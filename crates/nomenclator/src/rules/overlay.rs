//! Reads and applies overlays: caller data that adds words to or removes words from the
//! built-in vocabularies. Each overlay is one YAML or JSON document. [`Rules::from_overlays`]
//! applies a declared list of overlays together. It reports overlays that disagree instead of
//! choosing one by its position in the list.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::data;
use crate::lexical::{self, fold_case_text};
use crate::model::{ArtworkRole, ExtraKind, Format3d, NfoRole, SidecarFlag};
use crate::parsers::episode_name::SpecialKind;
use crate::parsers::extras::folded_vocabulary_form;
use crate::parsers::release::{ReleaseFact, release_spelling_form};
use crate::rendering::template::Template;
use crate::rendering::{OutputProfile, TemplateSlot, literal_words};
use crate::rules::{IdShape, PresetError, Rules, RulesIdentity, TagFact};

/// One overlay, read from a YAML or JSON document:
///
/// ```yaml
/// name: titles-ending-in-end   # required, unique among the overlays applied together
/// version: "1"                 # optional
/// title_like_release_words:
///   remove: [end]
/// extras_folders:
///   add: { Making Of: behind_the_scenes }
/// ```
///
/// Every vocabulary takes `add` and `remove`, or `replace`, which has the shape of `add` and
/// drops every other entry of the vocabulary except its canonical names. Word lists add a list; `extras_folders`,
/// `extras_suffixes`, `extras_video_names` and `extras_audio_names` add a map from word to
/// extra kind, `artwork_roles` from word to artwork role, `nfo_names` from name to NFO role,
/// `languages` from BCP 47 tag to its spellings, `release_words` from release fact to
/// value to spellings, `audio_channel_words` from word to channel layout,
/// `tags` from key to its fact, `bracket_pairs` from opening to closing character, `format_3d_words` from word to 3D layout, `letter_folds` from one letter to its spelling,
/// and `hearing_impaired_or_language_words` from word to BCP 47 tag. `remove` is always a
/// list of entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    name: String,
    version: Option<String>,
    changes: Vec<Change>,
    /// The line each change's entry is written on, for diagnostics.
    lines: BTreeMap<(Vocabulary, String), usize>,
}

/// The part of a [`RulesIdentity`] that one overlay contributes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OverlayIdentity {
    /// Returns the name of the overlay.
    pub name: String,
    /// The overlay's version, when it states one.
    pub version: Option<String>,
    /// A digest of the overlay's changes. Two overlays with equal changes have equal
    /// digests, regardless of their layout. Editing a change changes the digest.
    pub digest: String,
}

/// A problem with an overlay, or between overlays. No rules are built while any diagnostic
/// remains.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OverlayDiagnostic {
    /// The kind of problem.
    pub reason: OverlayDiagnosticReason,
    /// The overlay's name, when it has one.
    pub overlay: Option<String>,
    /// The 1-based line in the overlay's document, when one line is at fault.
    pub line: Option<usize>,
    /// The vocabulary concerned.
    pub vocabulary: Option<String>,
    /// The entry concerned, in the form the vocabulary compares.
    pub entry: Option<String>,
    /// The overlays whose changes to the entry disagree, in declared order.
    pub overlays: Vec<String>,
    /// The parser's description of a syntax or shape error.
    pub message: Option<String>,
}

/// The reason an overlay was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayDiagnosticReason {
    /// The document is not valid YAML or JSON, or not a mapping.
    Syntax,
    /// The document has no `name`.
    MissingName,
    /// A key that is not the name of a vocabulary.
    UnknownVocabulary,
    /// A vocabulary's `add` or `remove` has the wrong shape or an unknown kind.
    MalformedVocabulary,
    /// An entry that is empty once normalized.
    EmptyEntry,
    /// One overlay lists an entry twice in a vocabulary, however written, or both adds
    /// and removes it.
    DuplicateEntry,
    /// A word the name grammar already reads, such as a year (`2021`), an episode code
    /// (`S01E02`), a resolution (`1080p`) or a release version (`v2`).
    ReservedToken,
    /// An added word that another word class of the same kind also holds: release words
    /// (technical, title-like, edition), flag words, or season words.
    CollidingEntry,
    /// A declared language that has no language pack.
    UnknownLanguage,
    /// An entry outside a closed vocabulary, such as an unknown built-in ignore group.
    UnknownEntry,
    /// A preset that no catalogued decision offers.
    UnknownPreset,
    /// A preset value outside its catalogued alternatives.
    InvalidPresetValue,
    /// Two overlays applied together share a name.
    DuplicateOverlayName,
    /// Overlays change one entry differently: one adds what another removes, or they
    /// give it different kinds.
    ConflictingChange,
    /// An entry that the writer writes, such as `fanart` or `season` (X25). An overlay
    /// cannot remove or redefine it, so the reader reads every name that the writer writes.
    CanonicalEntry,
    /// A removed entry that the built-in rules do not contain, so the removal changes
    /// nothing (X28).
    IneffectiveRemoval,
}

/// The vocabularies an overlay can change, by their names in an overlay document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Vocabulary {
    TechnicalWords,
    TitleLikeReleaseWords,
    EditionWords,
    EditionPhrases,
    ExtrasFolders,
    ExtrasSuffixes,
    ExtrasVideoNames,
    ExtrasAudioNames,
    SeasonWords,
    SeriesSeasonWords,
    SpecialsFolders,
    Languages,
    DefaultFlagWords,
    ForcedFlagWords,
    HearingImpairedFlagWords,
    ArtworkRoles,
    NfoNames,
    Tags,
    EpisodeTokens,
    Separators,
    BracketPairs,
    EpisodeWords,
    SpecialPrefixes,
    AnimationSpecialWords,
    VideoExtensions,
    AudioExtensions,
    SubtitleExtensions,
    ImageExtensions,
    SystemJunkNames,
    SystemJunkFileSuffixes,
    PartialDownloadSuffixes,
    GeneratedMetadataFolders,
    GeneratedMetadataFolderSuffixes,
    LeadingArticles,
    NumberWords,
    OrdinalSuffixes,
    Presets,
    BuiltInIgnoreGroups,
    NoMediaMarkers,
    ReleaseJunkNames,
    ReleaseJunkPrefixes,
    ReleaseJunkExtensions,
    ReleaseJunkImages,
    ReleaseWords,
    AudioChannelWords,
    ReleaseSiteTags,
    PartWords,
    DiscFolderWords,
    Format3dWords,
    ReleaseGroupEndings,
    VideoHeights,
    LetterFolds,
    DiscImageExtensions,
    UnsupportedVideoExtensions,
    DiscStructureFolders,
    NfoExtensions,
    StillImageSuffixes,
    HearingImpairedOrLanguageWords,
    SubtitleFolders,
    AspectRatios,
    AudioCodecWords,
    FinalEpisodeWords,
    AlternateTitleWords,
    RemuxMarkers,
    ScreenerMarkers,
}

const VOCABULARIES: &[(&str, Vocabulary)] = &[
    ("technical_words", Vocabulary::TechnicalWords),
    (
        "title_like_release_words",
        Vocabulary::TitleLikeReleaseWords,
    ),
    ("edition_words", Vocabulary::EditionWords),
    ("edition_phrases", Vocabulary::EditionPhrases),
    ("extras_folders", Vocabulary::ExtrasFolders),
    ("extras_suffixes", Vocabulary::ExtrasSuffixes),
    ("extras_video_names", Vocabulary::ExtrasVideoNames),
    ("extras_audio_names", Vocabulary::ExtrasAudioNames),
    ("season_words", Vocabulary::SeasonWords),
    ("series_season_words", Vocabulary::SeriesSeasonWords),
    ("specials_folders", Vocabulary::SpecialsFolders),
    ("languages", Vocabulary::Languages),
    ("default_flag_words", Vocabulary::DefaultFlagWords),
    ("forced_flag_words", Vocabulary::ForcedFlagWords),
    (
        "hearing_impaired_flag_words",
        Vocabulary::HearingImpairedFlagWords,
    ),
    ("artwork_roles", Vocabulary::ArtworkRoles),
    ("nfo_names", Vocabulary::NfoNames),
    ("tags", Vocabulary::Tags),
    ("episode_tokens", Vocabulary::EpisodeTokens),
    ("separators", Vocabulary::Separators),
    ("bracket_pairs", Vocabulary::BracketPairs),
    ("episode_words", Vocabulary::EpisodeWords),
    ("special_prefixes", Vocabulary::SpecialPrefixes),
    ("animation_special_words", Vocabulary::AnimationSpecialWords),
    ("video_extensions", Vocabulary::VideoExtensions),
    ("audio_extensions", Vocabulary::AudioExtensions),
    ("subtitle_extensions", Vocabulary::SubtitleExtensions),
    ("image_extensions", Vocabulary::ImageExtensions),
    ("system_junk_names", Vocabulary::SystemJunkNames),
    (
        "system_junk_file_suffixes",
        Vocabulary::SystemJunkFileSuffixes,
    ),
    (
        "partial_download_suffixes",
        Vocabulary::PartialDownloadSuffixes,
    ),
    (
        "generated_metadata_folders",
        Vocabulary::GeneratedMetadataFolders,
    ),
    (
        "generated_metadata_folder_suffixes",
        Vocabulary::GeneratedMetadataFolderSuffixes,
    ),
    ("leading_articles", Vocabulary::LeadingArticles),
    ("number_words", Vocabulary::NumberWords),
    ("ordinal_suffixes", Vocabulary::OrdinalSuffixes),
    ("presets", Vocabulary::Presets),
    ("built_in_ignore_groups", Vocabulary::BuiltInIgnoreGroups),
    ("no_media_markers", Vocabulary::NoMediaMarkers),
    ("release_junk_names", Vocabulary::ReleaseJunkNames),
    ("release_junk_prefixes", Vocabulary::ReleaseJunkPrefixes),
    ("release_junk_extensions", Vocabulary::ReleaseJunkExtensions),
    ("release_junk_images", Vocabulary::ReleaseJunkImages),
    ("release_words", Vocabulary::ReleaseWords),
    ("audio_channel_words", Vocabulary::AudioChannelWords),
    ("release_site_tags", Vocabulary::ReleaseSiteTags),
    ("part_words", Vocabulary::PartWords),
    ("disc_folder_words", Vocabulary::DiscFolderWords),
    ("format_3d_words", Vocabulary::Format3dWords),
    ("release_group_endings", Vocabulary::ReleaseGroupEndings),
    ("video_heights", Vocabulary::VideoHeights),
    ("letter_folds", Vocabulary::LetterFolds),
    ("disc_image_extensions", Vocabulary::DiscImageExtensions),
    (
        "unsupported_video_extensions",
        Vocabulary::UnsupportedVideoExtensions,
    ),
    ("disc_structure_folders", Vocabulary::DiscStructureFolders),
    ("nfo_extensions", Vocabulary::NfoExtensions),
    ("still_image_suffixes", Vocabulary::StillImageSuffixes),
    (
        "hearing_impaired_or_language_words",
        Vocabulary::HearingImpairedOrLanguageWords,
    ),
    ("subtitle_folders", Vocabulary::SubtitleFolders),
    ("aspect_ratios", Vocabulary::AspectRatios),
    ("audio_codec_words", Vocabulary::AudioCodecWords),
    ("final_episode_words", Vocabulary::FinalEpisodeWords),
    ("alternate_title_words", Vocabulary::AlternateTitleWords),
    ("remux_markers", Vocabulary::RemuxMarkers),
    ("screener_markers", Vocabulary::ScreenerMarkers),
];

impl Vocabulary {
    /// Returns `true` if the words of the vocabulary are read as whole name words. In such a
    /// vocabulary, a word that the grammar already reads can never take effect.
    fn holds_name_words(self) -> bool {
        matches!(
            self,
            Vocabulary::TechnicalWords
                | Vocabulary::TitleLikeReleaseWords
                | Vocabulary::EditionWords
                | Vocabulary::EditionPhrases
                | Vocabulary::SeasonWords
                | Vocabulary::SeriesSeasonWords
                | Vocabulary::SpecialsFolders
                | Vocabulary::DefaultFlagWords
                | Vocabulary::ForcedFlagWords
                | Vocabulary::HearingImpairedFlagWords
                | Vocabulary::Languages
                | Vocabulary::LeadingArticles
                | Vocabulary::NumberWords
                | Vocabulary::EpisodeWords
                | Vocabulary::AnimationSpecialWords
                | Vocabulary::PartWords
                | Vocabulary::DiscFolderWords
                | Vocabulary::Format3dWords
                | Vocabulary::FinalEpisodeWords
                | Vocabulary::AlternateTitleWords
        )
    }

    /// Returns the vocabularies that must not also contain a word of this one. A name word is
    /// read as one class only.
    fn rivals(self) -> &'static [Vocabulary] {
        const RELEASE: &[Vocabulary] = &[
            Vocabulary::TechnicalWords,
            Vocabulary::TitleLikeReleaseWords,
            Vocabulary::EditionWords,
        ];
        const FLAGS: &[Vocabulary] = &[
            Vocabulary::DefaultFlagWords,
            Vocabulary::ForcedFlagWords,
            Vocabulary::HearingImpairedFlagWords,
        ];
        const SEASONS: &[Vocabulary] = &[Vocabulary::SeasonWords, Vocabulary::SeriesSeasonWords];
        [RELEASE, FLAGS, SEASONS]
            .into_iter()
            .find(|group| group.contains(&self))
            .unwrap_or(&[])
    }

    fn name(self) -> &'static str {
        VOCABULARIES
            .iter()
            .find(|(_, vocabulary)| *vocabulary == self)
            .map(|(name, _)| *name)
            .expect("every vocabulary is named")
    }

    /// Normalizes `raw` into the form that the lookups of the vocabulary compare names in.
    fn normalize(self, raw: &str) -> String {
        let lower = lexical::vocabulary_form(raw.trim());
        match self {
            // A separator can be a space, which trimming would drop.
            Vocabulary::Separators => raw.to_owned(),
            Vocabulary::BracketPairs | Vocabulary::EpisodeTokens => raw.trim().to_owned(),
            Vocabulary::ExtrasFolders
            | Vocabulary::ExtrasSuffixes
            | Vocabulary::ExtrasVideoNames
            | Vocabulary::ExtrasAudioNames => folded_vocabulary_form(&lower),
            Vocabulary::EditionPhrases => phrase_words(&lower).join(" "),
            Vocabulary::VideoExtensions
            | Vocabulary::AudioExtensions
            | Vocabulary::SubtitleExtensions
            | Vocabulary::ImageExtensions
            | Vocabulary::DiscImageExtensions
            | Vocabulary::UnsupportedVideoExtensions
            | Vocabulary::NfoExtensions => lower.trim_start_matches('.').to_owned(),
            Vocabulary::SystemJunkNames
            | Vocabulary::SystemJunkFileSuffixes
            | Vocabulary::PartialDownloadSuffixes
            | Vocabulary::GeneratedMetadataFolders
            | Vocabulary::GeneratedMetadataFolderSuffixes
            | Vocabulary::NfoNames
            | Vocabulary::NoMediaMarkers
            | Vocabulary::ReleaseJunkNames
            | Vocabulary::ReleaseJunkPrefixes
            | Vocabulary::ReleaseJunkImages => fold_case_text(raw.trim()),
            Vocabulary::ReleaseJunkExtensions => lower.trim_start_matches('.').to_owned(),
            Vocabulary::ReleaseWords => release_spelling_form(raw),
            // Compared in a title's comparison form (P4): accents and punctuation dropped.
            Vocabulary::LeadingArticles | Vocabulary::NumberWords => {
                lexical::without_accents(&lexical::title_key(Rules::builtin(), raw))
            }
            _ => lower,
        }
    }
}

/// Splits an edition phrase into words the same way names are split, with apostrophes
/// dropped.
fn phrase_words(text: &str) -> Vec<String> {
    text.replace(['\'', '’'], "")
        .split([' ', '.', '_', '-'])
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Change {
    vocabulary: Vocabulary,
    entry: String,
    action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Action {
    /// Removes every entry of the vocabulary, before the additions of a `replace`.
    Clear,
    Remove,
    Add(Meaning),
}

/// What an added entry means in its vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Meaning {
    Word,
    Extra(ExtraKind),
    Artwork(ArtworkRole),
    Nfo(NfoRole),
    Language(String),
    Number(u32),
    Preset(String),
    Release(ReleaseFact, String),
    ChannelLayout(String),
    Format3d(Format3d),
    SpelledLetter(String),
    Tag(TagFact),
    Special(SpecialKind),
    ClosingBracket(char),
}

impl Action {
    fn describe(&self) -> String {
        match self {
            Action::Clear => "replace".to_owned(),
            Action::Remove => "remove".to_owned(),
            Action::Add(Meaning::Word) => "add".to_owned(),
            Action::Add(Meaning::Extra(kind)) => format!("add {kind:?}"),
            Action::Add(Meaning::Artwork(role)) => format!("add {role:?}"),
            Action::Add(Meaning::Nfo(role)) => format!("add {role:?}"),
            Action::Add(Meaning::Language(tag)) => format!("add {tag}"),
            Action::Add(Meaning::Number(number)) => format!("add {number}"),
            Action::Add(Meaning::Preset(value)) => format!("set {value}"),
            Action::Add(Meaning::Release(fact, value)) => format!("add {fact:?} {value}"),
            Action::Add(Meaning::ChannelLayout(layout)) => format!("add {layout}"),
            Action::Add(Meaning::Format3d(format)) => format!("add {format:?}"),
            Action::Add(Meaning::SpelledLetter(spelled)) => format!("add {spelled}"),
            Action::Add(Meaning::Tag(fact)) => format!("add {fact:?}"),
            Action::Add(Meaning::Special(kind)) => format!("add {kind:?}"),
            Action::Add(Meaning::ClosingBracket(close)) => format!("add {close}"),
        }
    }
}

impl Overlay {
    /// Reads one overlay document, YAML or JSON.
    /// Fails with a diagnostic for each problem that the document has.
    pub fn parse(text: &str) -> Result<Overlay, Vec<OverlayDiagnostic>> {
        let document: Value = crate::parse_document(text).map_err(|error| {
            vec![OverlayDiagnostic {
                line: error.line,
                message: Some(error.message),
                ..diagnostic(OverlayDiagnosticReason::Syntax)
            }]
        })?;
        let Value::Object(fields) = document else {
            return Err(vec![OverlayDiagnostic {
                message: Some("an overlay is a mapping of vocabularies".to_owned()),
                ..diagnostic(OverlayDiagnosticReason::Syntax)
            }]);
        };
        let scalar = |key: &str| match fields.get(key) {
            Some(Value::String(text)) => Some(text.clone()),
            Some(Value::Number(number)) => Some(number.to_string()),
            _ => None,
        };
        let name = scalar("name").filter(|name| !name.trim().is_empty());
        let version = scalar("version");
        let mut diagnostics = Vec::new();
        if name.is_none() {
            diagnostics.push(diagnostic(OverlayDiagnosticReason::MissingName));
        }
        let mut changes = Vec::new();
        let mut lines = BTreeMap::new();
        let mut seen = BTreeSet::new();
        for (key, value) in &fields {
            let key = key.as_str();
            if key == "name" || key == "version" {
                continue;
            }
            let at = |reason| OverlayDiagnostic {
                overlay: name.clone(),
                line: line_of_key(text, key),
                vocabulary: Some(key.to_owned()),
                ..diagnostic(reason)
            };
            let Some(&(_, vocabulary)) = VOCABULARIES.iter().find(|(known, _)| *known == key)
            else {
                diagnostics.push(at(OverlayDiagnosticReason::UnknownVocabulary));
                continue;
            };
            if vocabulary == Vocabulary::Presets {
                match read_presets(text, key, value) {
                    Ok(presets) => {
                        for change in presets {
                            seen.insert((vocabulary, change.entry.clone()));
                            changes.push(change);
                        }
                    }
                    Err(found) => {
                        diagnostics.extend(found.into_iter().map(|diagnostic| OverlayDiagnostic {
                            overlay: name.clone(),
                            ..diagnostic
                        }));
                    }
                }
                continue;
            }
            match read_changes(vocabulary, value) {
                Ok(read) => {
                    let mut entry_lines = EntryLines::new(text, line_of_key(text, key));
                    for (raw, change) in read {
                        let line = entry_lines.find(&raw);
                        let at_entry = |reason| OverlayDiagnostic {
                            line,
                            entry: Some(change.entry.clone()).filter(|entry| !entry.is_empty()),
                            ..at(reason)
                        };
                        if change.action == Action::Clear {
                            changes.push(change);
                        } else if change.entry.is_empty() {
                            diagnostics.push(at_entry(OverlayDiagnosticReason::EmptyEntry));
                        } else if vocabulary == Vocabulary::BuiltInIgnoreGroups
                            && !data::BUILT_IN_IGNORE_GROUPS.contains(&change.entry.as_str())
                        {
                            diagnostics.push(at_entry(OverlayDiagnosticReason::UnknownEntry));
                        } else if !seen.insert((vocabulary, change.entry.clone())) {
                            diagnostics.push(at_entry(OverlayDiagnosticReason::DuplicateEntry));
                        } else if vocabulary.holds_name_words()
                            && matches!(change.action, Action::Add(_))
                            && change.entry.split(' ').any(lexical::is_reserved_word)
                        {
                            diagnostics.push(at_entry(OverlayDiagnosticReason::ReservedToken));
                        } else {
                            if let Some(line) = line {
                                lines.insert((vocabulary, change.entry.clone()), line);
                            }
                            changes.push(change);
                        }
                    }
                }
                Err(message) => diagnostics.push(OverlayDiagnostic {
                    message: Some(message),
                    ..at(OverlayDiagnosticReason::MalformedVocabulary)
                }),
            }
        }
        match name {
            Some(name) if diagnostics.is_empty() => {
                changes.sort();
                changes.dedup();
                Ok(Overlay {
                    name,
                    version,
                    changes,
                    lines,
                })
            }
            _ => {
                // The document's keys are read in sorted order, so the diagnostics are put
                // back in the order of their lines.
                diagnostics.sort_by_key(|diagnostic| diagnostic.line.unwrap_or(0));
                Err(diagnostics)
            }
        }
    }

    /// Returns the name of the overlay.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the version of the overlay.
    /// Returns `None` if the overlay states no version.
    #[must_use]
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    fn identity(&self) -> OverlayIdentity {
        // FNV-1a over the sorted changes: stable across builds and platforms.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for change in &self.changes {
            let line = format!(
                "{}\t{}\t{}\n",
                change.vocabulary.name(),
                change.action.describe(),
                change.entry
            );
            for byte in line.bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
        }
        OverlayIdentity {
            name: self.name.clone(),
            version: self.version.clone(),
            digest: format!("{hash:016x}"),
        }
    }
}

fn diagnostic(reason: OverlayDiagnosticReason) -> OverlayDiagnostic {
    OverlayDiagnostic {
        reason,
        overlay: None,
        line: None,
        vocabulary: None,
        entry: None,
        overlays: Vec::new(),
        message: None,
    }
}

/// A search for the lines that entries are written on, below the key of their vocabulary,
/// in the order that the entries are read. A text written twice is found on its next line.
struct EntryLines<'a> {
    lines: Vec<&'a str>,
    start: usize,
    found: BTreeMap<String, usize>,
}

impl<'a> EntryLines<'a> {
    fn new(text: &'a str, key_line: Option<usize>) -> Self {
        EntryLines {
            lines: text.lines().collect(),
            start: key_line.map_or(0, |line| line - 1),
            found: BTreeMap::new(),
        }
    }

    fn find(&mut self, raw: &str) -> Option<usize> {
        // A repeated text is looked for below its last line (a 1-based line is the 0-based
        // index of the line after it); a one-line document keeps it on the same line.
        let previous = self.found.get(raw).copied();
        let from = previous.unwrap_or(self.start);
        let line = (from..self.lines.len())
            .find(|index| self.lines[*index].contains(raw))
            .map(|index| index + 1)
            .or(previous)?;
        self.found.insert(raw.to_owned(), line);
        Some(line)
    }
}

/// Returns the 1-based number of the first line that contains `key` as a mapping key, in YAML
/// or JSON.
/// Returns `None` if no line contains `key` as a mapping key.
fn line_of_key(text: &str, key: &str) -> Option<usize> {
    let quoted = format!("\"{key}\"");
    text.lines()
        .position(|line| {
            let line = line.trim_start().trim_start_matches(['{', ',', ' ']);
            [key, quoted.as_str()].iter().any(|form| {
                line.strip_prefix(form)
                    .is_some_and(|rest| rest.trim_start().starts_with(':'))
            })
        })
        .map(|index| index + 1)
}

/// Reads the presets that the `presets` section of an overlay sets: a mapping from preset to
/// value. Checks each value against the catalogued alternatives.
/// Fails with a diagnostic for each preset or value that is not catalogued.
fn read_presets(
    text: &str,
    key: &str,
    value: &Value,
) -> Result<Vec<Change>, Vec<OverlayDiagnostic>> {
    let at = |reason, line, entry: &str| OverlayDiagnostic {
        line,
        vocabulary: Some(key.to_owned()),
        entry: Some(entry.to_owned()),
        ..diagnostic(reason)
    };
    let Value::Object(fields) = value else {
        return Err(vec![OverlayDiagnostic {
            line: line_of_key(text, key),
            vocabulary: Some(key.to_owned()),
            message: Some("expected a mapping from preset to value".to_owned()),
            ..diagnostic(OverlayDiagnosticReason::MalformedVocabulary)
        }]);
    };
    let mut entry_lines = EntryLines::new(text, line_of_key(text, key));
    let mut changes = Vec::new();
    let mut diagnostics = Vec::new();
    for (preset, value) in fields {
        let preset = preset.as_str();
        let line = entry_lines.find(preset);
        let value = match value {
            Value::String(text) => fold_case_text(text.trim()),
            Value::Number(number) => number.to_string(),
            _ => String::new(),
        };
        match crate::rules::Presets::default().set(preset, &value) {
            Ok(()) => changes.push(Change {
                vocabulary: Vocabulary::Presets,
                entry: preset.to_owned(),
                action: Action::Add(Meaning::Preset(value)),
            }),
            Err(PresetError::UnknownPreset) => {
                diagnostics.push(at(OverlayDiagnosticReason::UnknownPreset, line, preset));
            }
            Err(PresetError::InvalidValue) => {
                diagnostics.push(at(
                    OverlayDiagnosticReason::InvalidPresetValue,
                    line,
                    preset,
                ));
            }
        }
    }
    if diagnostics.is_empty() {
        Ok(changes)
    } else {
        Err(diagnostics)
    }
}

/// Reads the changes that the `add` and `remove` of one vocabulary make, with entries
/// normalized.
/// Fails if `value` does not have the shape of the vocabulary.
fn read_changes(vocabulary: Vocabulary, value: &Value) -> Result<Vec<(String, Change)>, String> {
    let Value::Object(fields) = value else {
        return Err("expected a mapping with `add`, `remove` or `replace`".to_owned());
    };
    let mut changes = Vec::new();
    if fields.contains_key("replace") && fields.len() > 1 {
        return Err("expected `replace` without `add` and `remove`".to_owned());
    }
    for (key, value) in fields {
        match key.as_str() {
            "replace" => {
                changes.push((
                    String::new(),
                    Change {
                        vocabulary,
                        entry: String::new(),
                        action: Action::Clear,
                    },
                ));
                for (raw, meaning) in read_additions(vocabulary, value)? {
                    let entry = vocabulary.normalize(&raw);
                    changes.push((
                        raw,
                        Change {
                            vocabulary,
                            entry,
                            action: Action::Add(meaning),
                        },
                    ));
                }
            }
            "add" => {
                for (raw, meaning) in read_additions(vocabulary, value)? {
                    let entry = vocabulary.normalize(&raw);
                    changes.push((
                        raw,
                        Change {
                            vocabulary,
                            entry,
                            action: Action::Add(meaning),
                        },
                    ));
                }
            }
            "remove" => {
                for raw in read::<Vec<String>>(value)? {
                    let entry = vocabulary.normalize(&raw);
                    changes.push((
                        raw,
                        Change {
                            vocabulary,
                            entry,
                            action: Action::Remove,
                        },
                    ));
                }
            }
            _ => return Err("expected only `add`, `remove` or `replace`".to_owned()),
        }
    }
    Ok(changes)
}

fn read_additions(vocabulary: Vocabulary, value: &Value) -> Result<Vec<(String, Meaning)>, String> {
    let pairs = |meanings: BTreeMap<String, Meaning>| meanings.into_iter().collect();
    Ok(match vocabulary {
        Vocabulary::ExtrasFolders
        | Vocabulary::ExtrasSuffixes
        | Vocabulary::ExtrasVideoNames
        | Vocabulary::ExtrasAudioNames => pairs(
            read::<BTreeMap<String, ExtraKind>>(value)?
                .into_iter()
                .map(|(word, kind)| (word, Meaning::Extra(kind)))
                .collect(),
        ),
        Vocabulary::ArtworkRoles => pairs(
            read::<BTreeMap<String, ArtworkRole>>(value)?
                .into_iter()
                .map(|(word, role)| (word, Meaning::Artwork(role)))
                .collect(),
        ),
        Vocabulary::NfoNames => pairs(
            read::<BTreeMap<String, NfoRole>>(value)?
                .into_iter()
                .map(|(word, role)| (word, Meaning::Nfo(role)))
                .collect(),
        ),
        Vocabulary::NumberWords => read::<BTreeMap<String, u32>>(value)?
            .into_iter()
            .map(|(word, number)| (word, Meaning::Number(number)))
            .collect(),
        Vocabulary::Languages => read::<BTreeMap<String, Vec<String>>>(value)?
            .into_iter()
            .flat_map(|(tag, spellings)| {
                let tag = lexical::vocabulary_form(tag.trim());
                std::iter::once(tag.clone())
                    .chain(spellings)
                    .map(move |spelling| (spelling, Meaning::Language(tag.clone())))
            })
            .collect(),
        Vocabulary::ReleaseWords => {
            let mut additions = Vec::new();
            for (fact, values) in
                read::<BTreeMap<ReleaseFact, BTreeMap<String, Vec<String>>>>(value)?
            {
                for (value, spellings) in values {
                    let value = fold_case_text(value.trim());
                    if fact == ReleaseFact::BitDepth && value.parse::<u8>().is_err() {
                        return Err(format!("bit depth `{value}` is not a number"));
                    }
                    additions.extend(
                        spellings
                            .into_iter()
                            .map(|spelling| (spelling, Meaning::Release(fact, value.clone()))),
                    );
                }
            }
            additions
        }
        Vocabulary::AudioChannelWords => {
            let mut additions = Vec::new();
            for (word, layout) in read::<BTreeMap<String, String>>(value)? {
                let layout = layout.trim().to_owned();
                if !is_channel_layout(&layout) {
                    return Err(format!(
                        "channel layout `{layout}` is not a layout such as `5.1`"
                    ));
                }
                additions.push((word, Meaning::ChannelLayout(layout)));
            }
            additions
        }
        Vocabulary::Tags => {
            let mut additions = Vec::new();
            for (key, fact) in read::<BTreeMap<String, TagFact>>(value)? {
                let is_key = key.starts_with(|c: char| c.is_ascii_alphabetic())
                    && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                if !is_key {
                    return Err(format!("tag key `{key}` is not letters, digits and `_`"));
                }
                if let TagFact::ProviderId {
                    id:
                        IdShape::Prefixed {
                            prefix,
                            min_digits,
                            max_digits,
                        },
                } = &fact
                    && (prefix.is_empty() || *min_digits == 0 || min_digits > max_digits)
                {
                    return Err(format!("tag `{key}` has an invalid ID shape"));
                }
                additions.push((key, Meaning::Tag(fact)));
            }
            additions
        }
        Vocabulary::EpisodeTokens => {
            let tokens = read::<Vec<String>>(value)?;
            for token in &tokens {
                check_episode_token(token)?;
            }
            tokens
                .into_iter()
                .map(|token| (token, Meaning::Word))
                .collect()
        }
        Vocabulary::Separators => {
            let separators = read::<Vec<String>>(value)?;
            if let Some(separator) = separators.iter().find(|separator| {
                let mut characters = separator.chars();
                !matches!(
                    (characters.next(), characters.next()),
                    (Some(character), None)
                        if !character.is_alphanumeric() && !matches!(character, '-' | '/')
                )
            }) {
                return Err(format!(
                    "separator `{separator}` is not one character other than a letter, a digit, `-` or `/`"
                ));
            }
            separators
                .into_iter()
                .map(|separator| (separator, Meaning::Word))
                .collect()
        }
        Vocabulary::BracketPairs => {
            let mut additions = Vec::new();
            for (open, close) in read::<BTreeMap<String, String>>(value)? {
                let mut closes = close.trim().chars();
                let (Some(closing), None, 1) =
                    (closes.next(), closes.next(), open.trim().chars().count())
                else {
                    return Err(format!(
                        "bracket pair `{open}` `{close}` is not one character each"
                    ));
                };
                additions.push((open, Meaning::ClosingBracket(closing)));
            }
            additions
        }
        Vocabulary::AnimationSpecialWords => read::<BTreeMap<String, SpecialKind>>(value)?
            .into_iter()
            .map(|(word, kind)| (word, Meaning::Special(kind)))
            .collect(),
        Vocabulary::Format3dWords => read::<BTreeMap<String, Format3d>>(value)?
            .into_iter()
            .map(|(word, format)| (word, Meaning::Format3d(format)))
            .collect(),
        Vocabulary::HearingImpairedOrLanguageWords => read::<BTreeMap<String, String>>(value)?
            .into_iter()
            .map(|(word, tag)| {
                let tag = lexical::vocabulary_form(tag.trim());
                (word, Meaning::Language(tag))
            })
            .collect(),
        Vocabulary::LetterFolds => {
            let mut additions = Vec::new();
            for (letter, spelled) in read::<BTreeMap<String, String>>(value)? {
                let spelled = lexical::vocabulary_form(spelled.trim());
                if letter.trim().chars().count() != 1 || spelled.is_empty() {
                    return Err(format!(
                        "letter fold `{letter}` is not one letter with a spelling"
                    ));
                }
                additions.push((letter, Meaning::SpelledLetter(spelled)));
            }
            additions
        }
        Vocabulary::VideoHeights => {
            let heights = read::<Vec<String>>(value)?;
            if let Some(height) = heights.iter().find(|height| {
                !(3..=4).contains(&height.trim().len())
                    || !height.trim().bytes().all(|byte| byte.is_ascii_digit())
            }) {
                return Err(format!("video height `{height}` is not 3 or 4 digits"));
            }
            heights
                .into_iter()
                .map(|height| (height, Meaning::Word))
                .collect()
        }
        _ => read::<Vec<String>>(value)?
            .into_iter()
            .map(|word| (word, Meaning::Word))
            .collect(),
    })
}

/// Returns `true` if `layout` is a channel layout such as `5.1`: a digit from 1 to 9, a dot
/// and a digit from 0 to 2.
fn is_channel_layout(layout: &str) -> bool {
    let bytes = layout.as_bytes();
    bytes.len() == 3
        && (b'1'..=b'9').contains(&bytes[0])
        && bytes[1] == b'.'
        && (b'0'..=b'2').contains(&bytes[2])
}

/// Checks that `text` is an episode token template: it uses `episode` once, may use `season`
/// once, and has no optional section (X34).
/// Fails with a description of the fault.
fn check_episode_token(text: &str) -> Result<(), String> {
    let template = Template::parse(text)
        .map_err(|error| format!("episode token `{text}`: {:?}", error.reason))?;
    let fields = template.fields();
    if !fields.contains("episode")
        || fields
            .iter()
            .any(|field| !matches!(*field, "episode" | "season"))
        || template.has_optional_or_repeated_field()
    {
        return Err(format!(
            "episode token `{text}` must use `episode` once, may use `season` once, and has no optional section"
        ));
    }
    Ok(())
}

fn read<T: DeserializeOwned>(value: &Value) -> Result<T, String> {
    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
}

/// Returns the canonical names (X25) as additions to the input vocabularies that read
/// them.
fn canonical_changes() -> Vec<Change> {
    let add = |vocabulary: Vocabulary, word: &str, meaning: Meaning| Change {
        vocabulary,
        entry: vocabulary.normalize(word),
        action: Action::Add(meaning),
    };
    let mut changes: Vec<Change> = data::ARTWORK_OUTPUT_WORDS
        .iter()
        .map(|(role, word)| add(Vocabulary::ArtworkRoles, word, Meaning::Artwork(*role)))
        .collect();
    changes.extend(
        data::NFO_OUTPUT_NAMES
            .iter()
            .map(|(role, name)| add(Vocabulary::NfoNames, name, Meaning::Nfo(*role))),
    );
    changes.extend(
        data::EXTRAS_OUTPUT_FOLDERS
            .iter()
            .map(|(kind, folder)| add(Vocabulary::ExtrasFolders, folder, Meaning::Extra(*kind))),
    );
    changes.push(add(
        Vocabulary::ExtrasAudioNames,
        data::THEME_OUTPUT_STEM,
        Meaning::Extra(ExtraKind::ThemeSong),
    ));
    changes.push(add(
        Vocabulary::SeasonWords,
        data::SEASON_FOLDER_WORD,
        Meaning::Word,
    ));
    changes.push(add(
        Vocabulary::Tags,
        data::EDITION_TAG_KEY,
        Meaning::Tag(TagFact::Edition),
    ));
    changes.push(Change {
        vocabulary: Vocabulary::Separators,
        entry: data::SPACE_SEPARATOR.to_string(),
        action: Action::Add(Meaning::Word),
    });
    changes.extend(data::CANONICAL_BRACKET_PAIRS.iter().map(|(open, close)| {
        add(
            Vocabulary::BracketPairs,
            &open.to_string(),
            Meaning::ClosingBracket(*close),
        )
    }));
    changes.push(add(
        Vocabulary::EpisodeWords,
        data::EPISODE_OUTPUT_LETTER,
        Meaning::Word,
    ));
    changes.push(add(
        Vocabulary::PartWords,
        data::PART_OUTPUT_WORD,
        Meaning::Word,
    ));
    changes.push(add(
        Vocabulary::NfoExtensions,
        data::NFO_EXTENSION,
        Meaning::Word,
    ));
    changes.push(add(
        Vocabulary::StillImageSuffixes,
        data::STILL_IMAGE_SUFFIX,
        Meaning::Word,
    ));
    changes.extend(data::SIDECAR_FLAG_OUTPUT_WORDS.iter().map(|(flag, word)| {
        let vocabulary = match flag {
            SidecarFlag::Default => Vocabulary::DefaultFlagWords,
            SidecarFlag::Forced => Vocabulary::ForcedFlagWords,
            SidecarFlag::HearingImpaired => Vocabulary::HearingImpairedFlagWords,
        };
        add(vocabulary, word, Meaning::Word)
    }));
    changes
}

impl Rules {
    /// Adds the canonical names (X25) to the input vocabularies. Called whenever the
    /// vocabularies are built, so no other change can drop a canonical name.
    pub(crate) fn add_canonical_names(&mut self) {
        for change in canonical_changes() {
            self.apply(change.vocabulary, &change.entry, &change.action);
        }
    }

    /// Returns the built-in rules with all `overlays` applied together. The order of
    /// `overlays` is their order in the identity and never breaks a tie.
    /// Fails if overlays change one entry differently or share a name. No rules are built in
    /// that case.
    pub fn from_overlays(overlays: &[Overlay]) -> Result<Rules, Vec<OverlayDiagnostic>> {
        Rules::build(None, overlays)
    }

    /// Returns the rules of a library in `languages` (BCP 47 tags with a language pack): the
    /// shared base and exactly the words of those languages, with `overlays` applied together
    /// with the packs. Drops words that only undeclared languages contain, unless an overlay
    /// adds them (X11). An empty `languages` selects every language, as
    /// [`Rules::from_overlays`] does.
    /// Fails if a declared language has no language pack. Fails for the same overlay
    /// conflicts as [`Rules::from_overlays`].
    pub fn for_languages(
        languages: &[String],
        overlays: &[Overlay],
    ) -> Result<Rules, Vec<OverlayDiagnostic>> {
        if languages.is_empty() {
            return Rules::build(None, overlays);
        }
        Rules::build(Some(languages), overlays)
    }

    /// Returns the rules of `languages` with the overlay `documents` applied, as
    /// [`Rules::for_languages`]. Fails with the diagnostics of every document that is not
    /// a valid overlay, or with the diagnostics of the overlays together.
    pub fn from_documents(
        languages: &[String],
        documents: &[String],
    ) -> Result<Rules, Vec<OverlayDiagnostic>> {
        let mut overlays = Vec::new();
        let mut diagnostics = Vec::new();
        for document in documents {
            match Overlay::parse(document) {
                Ok(overlay) => overlays.push(overlay),
                Err(found) => diagnostics.extend(found),
            }
        }
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }
        Rules::for_languages(languages, &overlays)
    }

    /// Returns the languages that have a pack, as BCP 47 tags.
    pub fn language_packs() -> impl Iterator<Item = &'static str> {
        data::LANGUAGE_PACKS.iter().map(|(tag, _)| *tag)
    }

    fn build(
        languages: Option<&[String]>,
        user_overlays: &[Overlay],
    ) -> Result<Rules, Vec<OverlayDiagnostic>> {
        let mut base = Rules::builtin().clone();
        let mut declared: Option<Vec<String>> = None;
        let mut overlays: Vec<Overlay> = Vec::new();
        if let Some(languages) = languages {
            let mut tags: Vec<String> = languages
                .iter()
                .map(|tag| lexical::vocabulary_form(tag.trim()))
                .collect();
            tags.sort();
            tags.dedup();
            let unknown: Vec<OverlayDiagnostic> = tags
                .iter()
                .filter(|tag| !Rules::language_packs().any(|known| known == tag.as_str()))
                .map(|tag| OverlayDiagnostic {
                    entry: Some(tag.clone()),
                    ..diagnostic(OverlayDiagnosticReason::UnknownLanguage)
                })
                .collect();
            if !unknown.is_empty() {
                return Err(unknown);
            }
            let (chosen, others): (Vec<_>, Vec<_>) = language_packs()
                .into_iter()
                .partition(|(tag, _)| tags.iter().any(|wanted| wanted == tag));
            overlays.extend(chosen.into_iter().map(|(_, pack)| pack));
            overlays.extend(user_overlays.iter().cloned());
            let kept: BTreeSet<(Vocabulary, &str)> = overlays
                .iter()
                .flat_map(|overlay| &overlay.changes)
                .filter(|change| matches!(change.action, Action::Add(_)))
                .map(|change| (change.vocabulary, change.entry.as_str()))
                .collect();
            for (_, pack) in &others {
                for change in &pack.changes {
                    if !kept.contains(&(change.vocabulary, change.entry.as_str())) {
                        base.apply(change.vocabulary, &change.entry, &Action::Remove);
                    }
                }
            }
            declared = Some(tags);
        } else {
            overlays.extend(user_overlays.iter().cloned());
        }
        let overlays = overlays.as_slice();
        let mut diagnostics = Vec::new();
        for (index, overlay) in overlays.iter().enumerate() {
            let is_first_of_name = overlays[..index]
                .iter()
                .all(|earlier| earlier.name != overlay.name);
            let is_repeated = overlays[index + 1..]
                .iter()
                .any(|later| later.name == overlay.name);
            if is_first_of_name && is_repeated {
                diagnostics.push(OverlayDiagnostic {
                    overlay: Some(overlay.name.clone()),
                    ..diagnostic(OverlayDiagnosticReason::DuplicateOverlayName)
                });
            }
        }
        let canonical: BTreeSet<(Vocabulary, String)> = canonical_changes()
            .into_iter()
            .map(|change| (change.vocabulary, change.entry))
            .collect();
        for overlay in user_overlays {
            for change in &overlay.changes {
                if canonical.contains(&(change.vocabulary, change.entry.clone())) {
                    diagnostics.push(OverlayDiagnostic {
                        overlay: Some(overlay.name.clone()),
                        line: overlay
                            .lines
                            .get(&(change.vocabulary, change.entry.clone()))
                            .copied(),
                        vocabulary: Some(change.vocabulary.name().to_owned()),
                        entry: Some(change.entry.clone()),
                        ..diagnostic(OverlayDiagnosticReason::CanonicalEntry)
                    });
                }
            }
        }
        // Language packs are the first layer and user overlays the second (X32), so an
        // overlay can change a word of a pack. Only overlays of one layer can conflict.
        let (packs, users) = overlays.split_at(overlays.len() - user_overlays.len());
        // Every user overlay's action on each entry, in declared order.
        let mut actions: BTreeMap<(Vocabulary, &str), Vec<(&str, &Action)>> = BTreeMap::new();
        for overlay in users {
            for change in &overlay.changes {
                actions
                    .entry((change.vocabulary, change.entry.as_str()))
                    .or_default()
                    .push((overlay.name.as_str(), &change.action));
            }
        }
        for ((vocabulary, entry), entry_actions) in &actions {
            let (_, first) = entry_actions[0];
            let replaced_twice = *first == Action::Clear && entry_actions.len() > 1;
            if replaced_twice || entry_actions.iter().any(|(_, action)| *action != first) {
                let mut names: Vec<String> = Vec::new();
                for (name, _) in entry_actions {
                    if !names.iter().any(|known| known == name) {
                        names.push((*name).to_owned());
                    }
                }
                diagnostics.push(OverlayDiagnostic {
                    vocabulary: Some(vocabulary.name().to_owned()),
                    entry: Some((*entry).to_owned()).filter(|entry| !entry.is_empty()),
                    overlays: names,
                    ..diagnostic(OverlayDiagnosticReason::ConflictingChange)
                });
            }
        }
        for overlay in user_overlays {
            for change in &overlay.changes {
                let is_added = actions[&(change.vocabulary, change.entry.as_str())]
                    .iter()
                    .any(|(_, action)| matches!(action, Action::Add(_)))
                    || packs.iter().flat_map(|pack| &pack.changes).any(|added| {
                        added.vocabulary == change.vocabulary && added.entry == change.entry
                    });
                if change.action == Action::Remove
                    && !is_added
                    && !Rules::builtin().contains(change.vocabulary, &change.entry)
                {
                    diagnostics.push(OverlayDiagnostic {
                        overlay: Some(overlay.name.clone()),
                        line: overlay
                            .lines
                            .get(&(change.vocabulary, change.entry.clone()))
                            .copied(),
                        vocabulary: Some(change.vocabulary.name().to_owned()),
                        entry: Some(change.entry.clone()),
                        ..diagnostic(OverlayDiagnosticReason::IneffectiveRemoval)
                    });
                }
            }
        }
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }
        let mut rules = base;
        for change in packs.iter().flat_map(|pack| &pack.changes) {
            rules.apply(change.vocabulary, &change.entry, &change.action);
        }
        for ((vocabulary, entry), entry_actions) in &actions {
            rules.apply(*vocabulary, entry, entry_actions[0].1);
        }
        let collisions = rules.collisions(users);
        if !collisions.is_empty() {
            return Err(collisions);
        }
        // A language pack that is not declared may have removed a canonical name.
        rules.add_canonical_names();
        rules.recompile();
        rules.identity = RulesIdentity {
            version: crate::RULES_VERSION.to_owned(),
            languages: declared,
            overlays: user_overlays.iter().map(Overlay::identity).collect(),
        };
        Ok(rules)
    }

    /// Returns these rules extended to read the names that `profile` writes (R18): its
    /// extras folder names and its specials folder name, in the form that the writer
    /// writes them (X25). The identity lists the additions
    /// as an overlay named `profile`. Returns a copy of these rules if the profile writes
    /// no name of its own. Fails if a name is in a rival vocabulary.
    pub fn reading_profile(
        &self,
        profile: &OutputProfile,
    ) -> Result<Rules, Vec<OverlayDiagnostic>> {
        let mut changes: Vec<Change> = profile
            .extras_folders
            .iter()
            .map(|(kind, folder)| Change {
                vocabulary: Vocabulary::ExtrasFolders,
                entry: Vocabulary::ExtrasFolders.normalize(&profile.clean_fact(folder)),
                action: Action::Add(Meaning::Extra(*kind)),
            })
            .collect();
        changes.extend(profile.specials_folder.iter().map(|folder| Change {
            vocabulary: Vocabulary::SpecialsFolders,
            entry: Vocabulary::SpecialsFolders.normalize(&profile.clean_fact(folder)),
            action: Action::Add(Meaning::Word),
        }));
        let word_changes = [
            (TemplateSlot::SeasonFolder, Vocabulary::SeasonWords),
            (TemplateSlot::Part, Vocabulary::PartWords),
        ];
        for (slot, vocabulary) in word_changes {
            let words = profile.templates.get(&slot).map(literal_words);
            changes.extend(words.into_iter().flatten().map(|word| Change {
                vocabulary,
                entry: vocabulary.normalize(&word),
                action: Action::Add(Meaning::Word),
            }));
        }
        let edition_keys = profile
            .templates
            .get(&TemplateSlot::EditionTag)
            .map(literal_words);
        changes.extend(edition_keys.into_iter().flatten().map(|key| Change {
            vocabulary: Vocabulary::Tags,
            entry: Vocabulary::Tags.normalize(&key),
            action: Action::Add(Meaning::Tag(TagFact::Edition)),
        }));
        changes.retain(|change| !change.entry.is_empty());
        if changes.is_empty() {
            return Ok(self.clone());
        }
        changes.sort();
        changes.dedup();
        let overlay = Overlay {
            name: "profile".to_owned(),
            version: None,
            changes,
            lines: BTreeMap::new(),
        };
        let mut rules = self.clone();
        for change in &overlay.changes {
            rules.apply(change.vocabulary, &change.entry, &change.action);
        }
        let collisions = rules.collisions(std::slice::from_ref(&overlay));
        if !collisions.is_empty() {
            return Err(collisions);
        }
        rules.recompile();
        rules.identity.overlays.push(overlay.identity());
        Ok(rules)
    }

    /// Reports the words that the overlays add to a vocabulary while a rival vocabulary still
    /// contains them.
    fn collisions(&self, overlays: &[Overlay]) -> Vec<OverlayDiagnostic> {
        let mut diagnostics = Vec::new();
        for overlay in overlays {
            for change in &overlay.changes {
                if !matches!(change.action, Action::Add(_)) {
                    continue;
                }
                let others: Vec<&str> = change
                    .vocabulary
                    .rivals()
                    .iter()
                    .filter(|rival| **rival != change.vocabulary)
                    .filter(|rival| {
                        self.word_set(**rival)
                            .is_some_and(|words| words.contains(&change.entry))
                    })
                    .map(|rival| rival.name())
                    .collect();
                if !others.is_empty() {
                    diagnostics.push(OverlayDiagnostic {
                        overlay: Some(overlay.name.clone()),
                        line: overlay
                            .lines
                            .get(&(change.vocabulary, change.entry.clone()))
                            .copied(),
                        vocabulary: Some(change.vocabulary.name().to_owned()),
                        entry: Some(change.entry.clone()),
                        message: Some(format!("also in {}", others.join(", "))),
                        ..diagnostic(OverlayDiagnosticReason::CollidingEntry)
                    });
                }
            }
        }
        diagnostics
    }

    fn word_set(&self, vocabulary: Vocabulary) -> Option<&BTreeSet<String>> {
        Some(match vocabulary {
            Vocabulary::TechnicalWords => &self.technical_words,
            Vocabulary::TitleLikeReleaseWords => &self.title_like_release_words,
            Vocabulary::EditionWords => &self.edition_words,
            Vocabulary::SeasonWords => &self.season_words,
            Vocabulary::SeriesSeasonWords => &self.series_season_words,
            Vocabulary::DefaultFlagWords => &self.default_flag_words,
            Vocabulary::ForcedFlagWords => &self.forced_flag_words,
            Vocabulary::HearingImpairedFlagWords => &self.hearing_impaired_flag_words,
            _ => return None,
        })
    }

    /// Returns the entries of `vocabulary`, in the form that the vocabulary compares.
    fn entries(&self, vocabulary: Vocabulary) -> Vec<String> {
        fn keys<V>(map: &BTreeMap<String, V>) -> Vec<String> {
            map.keys().cloned().collect()
        }
        fn words(set: &BTreeSet<String>) -> Vec<String> {
            set.iter().cloned().collect()
        }
        let exclusions = &self.built_in_exclusions;
        match vocabulary {
            Vocabulary::SpecialsFolders => words(&self.specials_folder_names),
            Vocabulary::LeadingArticles => words(&self.leading_articles),
            Vocabulary::OrdinalSuffixes => words(&self.ordinal_suffixes),
            Vocabulary::VideoExtensions => words(&self.video_extensions),
            Vocabulary::AudioExtensions => words(&self.audio_extensions),
            Vocabulary::SubtitleExtensions => words(&self.subtitle_extensions),
            Vocabulary::ImageExtensions => words(&self.image_extensions),
            Vocabulary::ReleaseJunkNames => words(&self.release_junk_names),
            Vocabulary::ReleaseJunkPrefixes => words(&self.release_junk_prefixes),
            Vocabulary::ReleaseJunkExtensions => words(&self.release_junk_extensions),
            Vocabulary::ReleaseJunkImages => words(&self.release_junk_images),
            Vocabulary::ReleaseSiteTags => words(&self.release_site_tags),
            Vocabulary::EpisodeWords => words(&self.episode_words),
            Vocabulary::SpecialPrefixes => words(&self.special_prefixes),
            Vocabulary::PartWords => words(&self.part_words),
            Vocabulary::DiscFolderWords => words(&self.disc_folder_words),
            Vocabulary::VideoHeights => words(&self.video_heights),
            Vocabulary::DiscImageExtensions => words(&self.disc_image_extensions),
            Vocabulary::UnsupportedVideoExtensions => words(&self.unsupported_video_extensions),
            Vocabulary::DiscStructureFolders => words(&self.disc_structure_folders),
            Vocabulary::NfoExtensions => words(&self.nfo_extensions),
            Vocabulary::StillImageSuffixes => words(&self.still_image_suffixes),
            Vocabulary::SubtitleFolders => words(&self.subtitle_folder_names),
            Vocabulary::AspectRatios => words(&self.aspect_ratios),
            Vocabulary::AudioCodecWords => words(&self.audio_codec_words),
            Vocabulary::FinalEpisodeWords => words(&self.final_episode_words),
            Vocabulary::AlternateTitleWords => words(&self.alternate_title_words),
            Vocabulary::RemuxMarkers => words(&self.remux_markers),
            Vocabulary::ScreenerMarkers => words(&self.screener_markers),
            Vocabulary::TechnicalWords => words(&self.technical_words),
            Vocabulary::TitleLikeReleaseWords => words(&self.title_like_release_words),
            Vocabulary::EditionWords => words(&self.edition_words),
            Vocabulary::SeasonWords => words(&self.season_words),
            Vocabulary::SeriesSeasonWords => words(&self.series_season_words),
            Vocabulary::DefaultFlagWords => words(&self.default_flag_words),
            Vocabulary::ForcedFlagWords => words(&self.forced_flag_words),
            Vocabulary::HearingImpairedFlagWords => words(&self.hearing_impaired_flag_words),
            Vocabulary::NumberWords => keys(&self.number_words),
            Vocabulary::ExtrasFolders => keys(&self.extras_folders),
            Vocabulary::ExtrasSuffixes => keys(&self.extras_suffixes),
            Vocabulary::ExtrasVideoNames => keys(&self.extras_video_names),
            Vocabulary::ExtrasAudioNames => keys(&self.extras_audio_names),
            Vocabulary::ArtworkRoles => keys(&self.artwork_roles),
            Vocabulary::NfoNames => keys(&self.nfo_generic_names),
            Vocabulary::Languages => keys(&self.language_spellings),
            Vocabulary::ReleaseWords => keys(&self.release_words),
            Vocabulary::AudioChannelWords => keys(&self.audio_channel_words),
            Vocabulary::Tags => keys(&self.tags),
            Vocabulary::AnimationSpecialWords => keys(&self.animation_special_words),
            Vocabulary::Format3dWords => keys(&self.format_3d_words),
            Vocabulary::HearingImpairedOrLanguageWords => {
                keys(&self.hearing_impaired_or_language_words)
            }
            Vocabulary::Presets => Vec::new(),
            Vocabulary::EpisodeTokens => words(&self.episode_tokens),
            Vocabulary::Separators => self.separators.iter().map(char::to_string).collect(),
            Vocabulary::BracketPairs => self.bracket_pairs.keys().map(char::to_string).collect(),
            Vocabulary::SystemJunkNames => words(&exclusions.system_junk_names),
            Vocabulary::BuiltInIgnoreGroups => words(&exclusions.groups),
            Vocabulary::NoMediaMarkers => words(&exclusions.no_media_markers),
            Vocabulary::GeneratedMetadataFolders => {
                words(&exclusions.generated_metadata_directory_names)
            }
            Vocabulary::SystemJunkFileSuffixes => exclusions.system_junk_file_suffixes.clone(),
            Vocabulary::PartialDownloadSuffixes => {
                exclusions.partial_download_file_suffixes.clone()
            }
            Vocabulary::GeneratedMetadataFolderSuffixes => {
                exclusions.generated_metadata_directory_suffixes.clone()
            }
            Vocabulary::ReleaseGroupEndings => self.release_group_endings.clone(),
            Vocabulary::EditionPhrases => self
                .edition_phrases
                .iter()
                .map(|phrase| phrase.join(" "))
                .collect(),
            Vocabulary::LetterFolds => self.letter_folds.keys().map(char::to_string).collect(),
        }
    }

    /// Returns `true` if `vocabulary` contains `entry`.
    fn contains(&self, vocabulary: Vocabulary, entry: &str) -> bool {
        self.entries(vocabulary).iter().any(|known| known == entry)
    }

    fn apply(&mut self, vocabulary: Vocabulary, entry: &str, action: &Action) {
        if *action == Action::Clear {
            for known in self.entries(vocabulary) {
                self.apply(vocabulary, &known, &Action::Remove);
            }
            return;
        }
        let entry = entry.to_owned();
        let exclusions = &mut self.built_in_exclusions;
        let words =
            match vocabulary {
                Vocabulary::TechnicalWords => &mut self.technical_words,
                Vocabulary::TitleLikeReleaseWords => &mut self.title_like_release_words,
                Vocabulary::EditionWords => &mut self.edition_words,
                Vocabulary::SeasonWords => &mut self.season_words,
                Vocabulary::SeriesSeasonWords => &mut self.series_season_words,
                Vocabulary::SpecialsFolders => &mut self.specials_folder_names,
                Vocabulary::DefaultFlagWords => &mut self.default_flag_words,
                Vocabulary::ForcedFlagWords => &mut self.forced_flag_words,
                Vocabulary::HearingImpairedFlagWords => &mut self.hearing_impaired_flag_words,
                Vocabulary::EpisodeWords => &mut self.episode_words,
                Vocabulary::SpecialPrefixes => &mut self.special_prefixes,
                Vocabulary::AnimationSpecialWords => {
                    return apply_to_map(
                        &mut self.animation_special_words,
                        entry,
                        action,
                        |meaning| match meaning {
                            Meaning::Special(kind) => Some(*kind),
                            _ => None,
                        },
                    );
                }
                Vocabulary::EpisodeTokens => &mut self.episode_tokens,
                Vocabulary::Separators => {
                    if let Some(separator) = entry.chars().next() {
                        match action {
                            Action::Add(_) => {
                                self.separators.insert(separator);
                            }
                            Action::Remove | Action::Clear => {
                                self.separators.remove(&separator);
                            }
                        }
                    }
                    return;
                }
                Vocabulary::BracketPairs => {
                    let Some(open) = entry.chars().next() else {
                        return;
                    };
                    match action {
                        Action::Add(Meaning::ClosingBracket(close)) => {
                            self.bracket_pairs.insert(open, *close);
                        }
                        Action::Remove => {
                            self.bracket_pairs.remove(&open);
                        }
                        Action::Add(_) | Action::Clear => {}
                    }
                    return;
                }
                Vocabulary::Tags => {
                    return apply_to_map(&mut self.tags, entry, action, |meaning| match meaning {
                        Meaning::Tag(fact) => Some(fact.clone()),
                        _ => None,
                    });
                }
                Vocabulary::Presets => {
                    if let Action::Add(Meaning::Preset(value)) = action {
                        // Checked when the overlay was read.
                        let _ = self.presets.set(&entry, value);
                    }
                    return;
                }
                Vocabulary::LeadingArticles => &mut self.leading_articles,
                Vocabulary::OrdinalSuffixes => &mut self.ordinal_suffixes,
                Vocabulary::NumberWords => {
                    return apply_to_map(&mut self.number_words, entry, action, |meaning| {
                        match meaning {
                            Meaning::Number(number) => Some(*number),
                            _ => None,
                        }
                    });
                }
                Vocabulary::VideoExtensions => &mut self.video_extensions,
                Vocabulary::AudioExtensions => &mut self.audio_extensions,
                Vocabulary::SubtitleExtensions => &mut self.subtitle_extensions,
                Vocabulary::ImageExtensions => &mut self.image_extensions,
                Vocabulary::SystemJunkNames => &mut exclusions.system_junk_names,
                Vocabulary::BuiltInIgnoreGroups => &mut exclusions.groups,
                Vocabulary::NoMediaMarkers => &mut exclusions.no_media_markers,
                Vocabulary::ReleaseJunkNames => &mut self.release_junk_names,
                Vocabulary::ReleaseJunkPrefixes => &mut self.release_junk_prefixes,
                Vocabulary::ReleaseJunkExtensions => &mut self.release_junk_extensions,
                Vocabulary::ReleaseJunkImages => &mut self.release_junk_images,
                Vocabulary::GeneratedMetadataFolders => {
                    &mut exclusions.generated_metadata_directory_names
                }
                Vocabulary::SystemJunkFileSuffixes => {
                    return apply_to_list(&mut exclusions.system_junk_file_suffixes, entry, action);
                }
                Vocabulary::PartialDownloadSuffixes => {
                    return apply_to_list(
                        &mut exclusions.partial_download_file_suffixes,
                        entry,
                        action,
                    );
                }
                Vocabulary::GeneratedMetadataFolderSuffixes => {
                    return apply_to_list(
                        &mut exclusions.generated_metadata_directory_suffixes,
                        entry,
                        action,
                    );
                }
                Vocabulary::EditionPhrases => {
                    let words: Vec<String> = entry.split(' ').map(str::to_owned).collect();
                    return apply_to_list(&mut self.edition_phrases, words, action);
                }
                Vocabulary::ExtrasFolders => {
                    return apply_to_map(&mut self.extras_folders, entry, action, extra_kind);
                }
                Vocabulary::ExtrasSuffixes => {
                    return apply_to_map(&mut self.extras_suffixes, entry, action, extra_kind);
                }
                Vocabulary::ExtrasVideoNames => {
                    return apply_to_map(&mut self.extras_video_names, entry, action, extra_kind);
                }
                Vocabulary::ExtrasAudioNames => {
                    return apply_to_map(&mut self.extras_audio_names, entry, action, extra_kind);
                }
                Vocabulary::ArtworkRoles => {
                    return apply_to_map(&mut self.artwork_roles, entry, action, |meaning| {
                        match meaning {
                            Meaning::Artwork(role) => Some(*role),
                            _ => None,
                        }
                    });
                }
                Vocabulary::NfoNames => {
                    return apply_to_map(&mut self.nfo_generic_names, entry, action, |meaning| {
                        match meaning {
                            Meaning::Nfo(role) => Some(*role),
                            _ => None,
                        }
                    });
                }
                Vocabulary::Languages => {
                    return apply_to_map(&mut self.language_spellings, entry, action, |meaning| {
                        match meaning {
                            Meaning::Language(tag) => Some(tag.clone()),
                            _ => None,
                        }
                    });
                }
                Vocabulary::ReleaseWords => {
                    return apply_to_map(&mut self.release_words, entry, action, |meaning| {
                        match meaning {
                            Meaning::Release(fact, value) => Some((*fact, value.clone())),
                            _ => None,
                        }
                    });
                }
                Vocabulary::AudioChannelWords => {
                    return apply_to_map(&mut self.audio_channel_words, entry, action, |meaning| {
                        match meaning {
                            Meaning::ChannelLayout(layout) => Some(layout.clone()),
                            _ => None,
                        }
                    });
                }
                Vocabulary::ReleaseSiteTags => &mut self.release_site_tags,
                Vocabulary::PartWords => &mut self.part_words,
                Vocabulary::DiscFolderWords => &mut self.disc_folder_words,
                Vocabulary::Format3dWords => {
                    return apply_to_map(&mut self.format_3d_words, entry, action, |meaning| {
                        match meaning {
                            Meaning::Format3d(format) => Some(*format),
                            _ => None,
                        }
                    });
                }
                Vocabulary::ReleaseGroupEndings => {
                    return apply_to_list(&mut self.release_group_endings, entry, action);
                }
                Vocabulary::VideoHeights => &mut self.video_heights,
                Vocabulary::LetterFolds => {
                    let Some(letter) = entry.chars().next() else {
                        return;
                    };
                    match action {
                        Action::Add(Meaning::SpelledLetter(spelled)) => {
                            self.letter_folds.insert(letter, spelled.clone());
                        }
                        Action::Remove => {
                            self.letter_folds.remove(&letter);
                        }
                        Action::Add(_) | Action::Clear => {}
                    }
                    return;
                }
                Vocabulary::DiscImageExtensions => &mut self.disc_image_extensions,
                Vocabulary::UnsupportedVideoExtensions => &mut self.unsupported_video_extensions,
                Vocabulary::DiscStructureFolders => &mut self.disc_structure_folders,
                Vocabulary::NfoExtensions => &mut self.nfo_extensions,
                Vocabulary::StillImageSuffixes => &mut self.still_image_suffixes,
                Vocabulary::HearingImpairedOrLanguageWords => {
                    return apply_to_map(
                        &mut self.hearing_impaired_or_language_words,
                        entry,
                        action,
                        |meaning| match meaning {
                            Meaning::Language(tag) => Some(tag.clone()),
                            _ => None,
                        },
                    );
                }
                Vocabulary::SubtitleFolders => &mut self.subtitle_folder_names,
                Vocabulary::AspectRatios => &mut self.aspect_ratios,
                Vocabulary::AudioCodecWords => &mut self.audio_codec_words,
                Vocabulary::FinalEpisodeWords => &mut self.final_episode_words,
                Vocabulary::AlternateTitleWords => &mut self.alternate_title_words,
                Vocabulary::RemuxMarkers => &mut self.remux_markers,
                Vocabulary::ScreenerMarkers => &mut self.screener_markers,
            };
        match action {
            Action::Add(_) => {
                words.insert(entry);
            }
            Action::Remove | Action::Clear => {
                words.remove(&entry);
            }
        }
    }
}

/// Returns every language pack, by tag. The packs are built-in data that the tests check.
fn language_packs() -> Vec<(&'static str, Overlay)> {
    data::LANGUAGE_PACKS
        .iter()
        .map(|(tag, document)| {
            let pack = Overlay::parse(document).expect("a language pack is a valid overlay");
            (*tag, pack)
        })
        .collect()
}

fn extra_kind(meaning: &Meaning) -> Option<ExtraKind> {
    match meaning {
        Meaning::Extra(kind) => Some(*kind),
        _ => None,
    }
}

fn apply_to_list<T: PartialEq>(list: &mut Vec<T>, entry: T, action: &Action) {
    list.retain(|existing| *existing != entry);
    if matches!(action, Action::Add(_)) {
        list.push(entry);
    }
}

fn apply_to_map<V>(
    map: &mut BTreeMap<String, V>,
    entry: String,
    action: &Action,
    value: impl Fn(&Meaning) -> Option<V>,
) {
    match action {
        Action::Add(meaning) => {
            if let Some(value) = value(meaning) {
                map.insert(entry, value);
            }
        }
        Action::Remove | Action::Clear => {
            map.remove(&entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checks that every built-in word that belongs to a language is in a pack. Narrowing
    /// drops the words of a language only if its pack lists them. The shared base is the
    /// rest.
    #[test]
    fn every_language_specific_built_in_word_is_in_a_pack() {
        let packs = language_packs();
        let in_packs = |vocabulary: Vocabulary, entry: &str| {
            packs.iter().any(|(_, pack)| {
                pack.changes
                    .iter()
                    .any(|change| change.vocabulary == vocabulary && change.entry == entry)
            })
        };
        let shared_base: Vec<String> = canonical_changes()
            .into_iter()
            .map(|change| change.entry)
            .chain(
                data::EPISODE_BASE_WORDS
                    .iter()
                    .map(|word| (*word).to_owned()),
            )
            .collect();
        let builtin = Rules::builtin();
        let language_words = [
            (
                Vocabulary::SeasonWords,
                builtin.season_words.iter().collect::<Vec<_>>(),
            ),
            (
                Vocabulary::SeriesSeasonWords,
                builtin.series_season_words.iter().collect(),
            ),
            (
                Vocabulary::EpisodeWords,
                builtin.episode_words.iter().collect(),
            ),
            (
                Vocabulary::LeadingArticles,
                builtin.leading_articles.iter().collect(),
            ),
            (
                Vocabulary::OrdinalSuffixes,
                builtin.ordinal_suffixes.iter().collect(),
            ),
        ];
        for (vocabulary, words) in language_words {
            for word in words {
                assert!(
                    shared_base.iter().any(|entry| entry == word) || in_packs(vocabulary, word),
                    "{} `{word}` is in no language pack",
                    vocabulary.name()
                );
            }
        }
        for word in crate::data::SPELLED_NUMBERS {
            assert!(
                in_packs(Vocabulary::NumberWords, word),
                "number `{word}` is in no pack"
            );
        }
    }

    #[test]
    fn every_language_pack_is_named_after_its_tag_and_adds_only() {
        for (tag, pack) in language_packs() {
            assert_eq!(pack.name, format!("language-{tag}"));
            assert!(
                pack.changes
                    .iter()
                    .all(|change| change.action != Action::Remove)
            );
        }
    }

    #[test]
    fn declaring_every_language_keeps_every_built_in_word() {
        let all: Vec<String> = Rules::language_packs().map(str::to_owned).collect();
        let rules = Rules::for_languages(&all, &[]).expect("the packs agree with each other");
        let builtin = Rules::builtin();
        assert!(builtin.season_words.is_subset(&rules.season_words));
        assert!(
            builtin
                .series_season_words
                .is_subset(&rules.series_season_words)
        );
        assert!(builtin.episode_words.is_subset(&rules.episode_words));
        assert!(builtin.leading_articles.is_subset(&rules.leading_articles));
        assert!(builtin.ordinal_suffixes.is_subset(&rules.ordinal_suffixes));
    }
}
