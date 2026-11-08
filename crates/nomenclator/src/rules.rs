//! Provides the input vocabulary as one value: every word list that gives the meaning of an
//! existing name. Operations are methods on [`Rules`]. The free functions of the crate use
//! [`Rules::builtin`]. The output profile, not the rules, defines how names are written
//! (extras output folders, flag words, reserved device names).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{LazyLock, OnceLock};

use regex::Regex;
use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

use crate::data;
use crate::lexical;
use crate::model::{ArtworkRole, ExtraKind, Format3d, NfoRole};
use crate::parsers::release::{ReleaseFact, ReleasePatterns, release_spelling_form};

mod overlay;

pub use overlay::{Overlay, OverlayDiagnostic, OverlayDiagnosticReason, OverlayIdentity};

/// The identity of the rules that produced a result. A caller compares it with the identity
/// stored with a result to know whether the stored facts are still valid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RulesIdentity {
    /// The built-in rules version, [`crate::RULES_VERSION`].
    pub version: String,
    /// The languages that the caller declared, sorted. `None` means every language (X11).
    pub languages: Option<Vec<String>>,
    /// The overlays applied to the built-in rules, in declared order.
    pub overlays: Vec<OverlayIdentity>,
}

/// Every input vocabulary that the operations read. [`Rules::builtin`] builds it from the
/// defaults. The same rules and inputs always give the same result.
#[derive(Debug, Clone)]
pub struct Rules {
    identity: RulesIdentity,
    pub(crate) technical_words: BTreeSet<String>,
    pub(crate) title_like_release_words: BTreeSet<String>,
    pub(crate) edition_words: BTreeSet<String>,
    pub(crate) edition_phrases: Vec<Vec<String>>,
    pub(crate) format_3d_words: BTreeMap<String, Format3d>,
    pub(crate) provider_keys: BTreeSet<String>,
    pub(crate) release_group_endings: Vec<String>,
    pub(crate) video_extensions: BTreeSet<String>,
    pub(crate) unsupported_video_extensions: BTreeSet<String>,
    pub(crate) disc_image_extensions: BTreeSet<String>,
    pub(crate) disc_structure_folders: BTreeSet<String>,
    pub(crate) subtitle_extensions: BTreeSet<String>,
    pub(crate) audio_extensions: BTreeSet<String>,
    pub(crate) image_extensions: BTreeSet<String>,
    pub(crate) nfo_extension: String,
    pub(crate) nfo_generic_names: BTreeMap<String, NfoRole>,
    pub(crate) still_image_suffix: String,
    pub(crate) default_flag_words: BTreeSet<String>,
    pub(crate) forced_flag_words: BTreeSet<String>,
    pub(crate) hearing_impaired_flag_words: BTreeSet<String>,
    pub(crate) hearing_impaired_or_hindi_word: String,
    /// Each spelling (the tag itself included) and the BCP 47 tag it names.
    pub(crate) language_spellings: BTreeMap<String, String>,
    pub(crate) extras_folders: BTreeMap<String, ExtraKind>,
    pub(crate) extras_suffixes: BTreeMap<String, ExtraKind>,
    pub(crate) extras_video_names: BTreeMap<String, ExtraKind>,
    pub(crate) extras_audio_names: BTreeMap<String, ExtraKind>,
    pub(crate) artwork_roles: BTreeMap<String, ArtworkRole>,
    pub(crate) built_in_exclusions: BuiltInExclusions,
    pub(crate) season_words: BTreeSet<String>,
    pub(crate) series_season_words: BTreeSet<String>,
    pub(crate) specials_folder_names: BTreeSet<String>,
    pub(crate) video_heights: BTreeSet<String>,
    pub(crate) letter_folds: BTreeMap<char, String>,
    pub(crate) leading_articles: BTreeSet<String>,
    /// Number words (Roman numerals, spelled numbers) and the number they read as.
    pub(crate) number_words: BTreeMap<String, u32>,
    /// Endings that make a number an ordinal before a season word.
    pub(crate) ordinal_suffixes: BTreeSet<String>,
    /// File names, stem beginnings, extensions and image stems of release junk (X22).
    pub(crate) release_junk_names: BTreeSet<String>,
    pub(crate) release_junk_prefixes: BTreeSet<String>,
    pub(crate) release_junk_extensions: BTreeSet<String>,
    pub(crate) release_junk_images: BTreeSet<String>,
    /// Release word spellings after a title (X19), in [`release_spelling_form`], with the
    /// fact and the normalized value that each states.
    pub(crate) release_words: BTreeMap<String, (ReleaseFact, String)>,
    /// Audio channel words and the channel layout that each states (`6ch` is `5.1`).
    pub(crate) audio_channel_words: BTreeMap<String, String>,
    /// Bracketed tags that name an index site, not a release group.
    pub(crate) release_site_tags: BTreeSet<String>,
    pub(crate) presets: Presets,
    pub(crate) patterns: Patterns,
    /// The compiled release patterns, built on first use, because most callers never read
    /// release facts.
    release_patterns: OnceLock<ReleasePatterns>,
}

/// Choices between alternatives that the decision catalogue records as contested (X12). The
/// defaults are the recorded decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Presets {
    /// The last year a name's number can be (X5).
    pub latest_plausible_year: u16,
    /// How a day- or month-first date with both parts at most 12 is read (T13).
    pub date_order: DateOrder,
    /// Where bare episode numbers are read (T6).
    pub bare_numbers: BareNumbers,
    /// How nested rule files combine (I4).
    pub ignore_nesting: IgnoreNesting,
    /// What a sample is (E6).
    pub samples: Samples,
}

impl Default for Presets {
    fn default() -> Self {
        Presets {
            latest_plausible_year: data::LATEST_PLAUSIBLE_YEAR,
            date_order: DateOrder::Ambiguous,
            bare_numbers: BareNumbers::Anywhere,
            ignore_nesting: IgnoreNesting::Stack,
            samples: Samples::Extras,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DateOrder {
    Ambiguous,
    DayFirst,
    MonthFirst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BareNumbers {
    Anywhere,
    SeasonFoldersOnly,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum IgnoreNesting {
    #[default]
    Stack,
    Nearest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Samples {
    Extras,
    Excluded,
}

impl Presets {
    /// Sets the preset `name` to `value`.
    /// Fails with [`PresetError::UnknownPreset`] if `name` is not a preset.
    /// Fails with [`PresetError::InvalidValue`] if `value` is not an alternative of the preset.
    pub(crate) fn set(&mut self, name: &str, value: &str) -> Result<(), PresetError> {
        let invalid = Err(PresetError::InvalidValue);
        match name {
            "latest_plausible_year" => match value.parse::<u16>() {
                Ok(year)
                    if (data::EARLIEST_PLAUSIBLE_YEAR..=data::LATEST_PLAUSIBLE_YEAR)
                        .contains(&year) =>
                {
                    self.latest_plausible_year = year;
                }
                _ => return invalid,
            },
            "date_order" => {
                self.date_order = match value {
                    "ambiguous" => DateOrder::Ambiguous,
                    "day_first" => DateOrder::DayFirst,
                    "month_first" => DateOrder::MonthFirst,
                    _ => return invalid,
                }
            }
            "bare_numbers" => {
                self.bare_numbers = match value {
                    "anywhere" => BareNumbers::Anywhere,
                    "season_folders_only" => BareNumbers::SeasonFoldersOnly,
                    _ => return invalid,
                }
            }
            "ignore_nesting" => {
                self.ignore_nesting = match value {
                    "stack" => IgnoreNesting::Stack,
                    "nearest" => IgnoreNesting::Nearest,
                    _ => return invalid,
                }
            }
            "samples" => {
                self.samples = match value {
                    "extras" => Samples::Extras,
                    "excluded" => Samples::Excluded,
                    _ => return invalid,
                }
            }
            _ => return Err(PresetError::UnknownPreset),
        }
        Ok(())
    }
}

pub(crate) enum PresetError {
    UnknownPreset,
    InvalidValue,
}

/// The names that are excluded wherever they appear, regardless of the rule files (I6-I9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuiltInExclusions {
    /// The groups in force, of [`data::BUILT_IN_IGNORE_GROUPS`] (X13).
    pub groups: BTreeSet<String>,
    /// Marker file names that exclude their folder (I3).
    pub no_media_markers: BTreeSet<String>,
    pub system_junk_names: BTreeSet<String>,
    pub system_junk_file_suffixes: Vec<String>,
    pub partial_download_file_suffixes: Vec<String>,
    pub generated_metadata_directory_names: BTreeSet<String>,
    pub generated_metadata_directory_suffixes: Vec<String>,
}

impl Default for BuiltInExclusions {
    fn default() -> Self {
        Rules::builtin().built_in_exclusions.clone()
    }
}

/// Patterns compiled from the vocabularies, once per [`Rules`].
#[derive(Debug, Clone)]
pub(crate) struct Patterns {
    pub aspect_ratio: Regex,
    pub audio_layout: Regex,
    /// A season word, then its number (`Season 2`).
    pub season_word_then_number: Regex,
    /// An ordinal, then a season word (`3. Staffel`).
    pub ordinal_then_season_word: Regex,
}

static BUILTIN: LazyLock<Rules> = LazyLock::new(Rules::from_builtin_tables);

impl Default for Rules {
    fn default() -> Self {
        Rules::builtin().clone()
    }
}

impl Rules {
    /// Returns the built-in rules: the vocabularies that the decisions accept.
    #[must_use]
    pub fn builtin() -> &'static Rules {
        &BUILTIN
    }

    /// Returns the identity of these rules.
    pub fn identity(&self) -> &RulesIdentity {
        &self.identity
    }

    /// Returns `true` if `name` is a no-media marker name of these rules (X14). The
    /// comparison ignores case. A caller that scans folders reads each such file and
    /// passes it as an ignore file.
    pub fn is_no_media_marker(&self, name: &str) -> bool {
        self.built_in_exclusions
            .no_media_markers
            .contains(&lexical::fold_case_text(name))
    }

    /// Returns the compiled release word and audio channel patterns. Compiles them on the
    /// first call.
    pub(crate) fn release_patterns(&self) -> &ReleasePatterns {
        self.release_patterns.get_or_init(|| {
            ReleasePatterns::compile(&self.release_words, &self.audio_channel_words)
        })
    }

    /// Compiles the patterns again. Called after the vocabularies change.
    fn recompile(&mut self) {
        let season_words: Vec<&str> = self
            .season_words
            .iter()
            .chain(&self.series_season_words)
            .map(String::as_str)
            .collect();
        let ordinal_suffixes: Vec<&str> =
            self.ordinal_suffixes.iter().map(String::as_str).collect();
        self.patterns = Patterns::compile(
            data::ASPECT_RATIOS,
            data::AUDIO_CODEC_WORDS,
            &season_words,
            &ordinal_suffixes,
        );
        self.release_patterns = OnceLock::new();
    }

    fn from_builtin_tables() -> Rules {
        let pairs = |list: &[(&str, ExtraKind)]| {
            list.iter()
                .map(|(word, kind)| ((*word).to_owned(), *kind))
                .collect()
        };
        let mut language_spellings = BTreeMap::new();
        for (tag, spellings) in data::LANGUAGES {
            for spelling in std::iter::once(tag).chain(spellings.iter()) {
                language_spellings
                    .entry((*spelling).to_owned())
                    .or_insert_with(|| (*tag).to_owned());
            }
        }
        let number_words = data::ROMAN_NUMERALS
            .iter()
            .enumerate()
            .chain(data::SPELLED_NUMBERS.iter().enumerate())
            .map(|(index, word)| ((*word).to_owned(), index as u32 + 1))
            .collect();
        let season_words: BTreeSet<String> = owned(data::SEASON_WORDS);
        let series_season_words: BTreeSet<String> = owned(data::SERIES_SEASON_WORDS);
        let all_season_words: Vec<&str> = season_words
            .iter()
            .chain(&series_season_words)
            .map(String::as_str)
            .collect();
        let mut release_words = BTreeMap::new();
        for (fact, value, spellings) in data::RELEASE_WORDS {
            for spelling in *spellings {
                release_words
                    .entry(release_spelling_form(spelling))
                    .or_insert_with(|| (*fact, (*value).to_owned()));
            }
        }
        let audio_channel_words: BTreeMap<String, String> = data::AUDIO_CHANNEL_WORDS
            .iter()
            .map(|(word, layout)| (lexical::vocabulary_form(word), (*layout).to_owned()))
            .collect();
        let patterns = Patterns::compile(
            data::ASPECT_RATIOS,
            data::AUDIO_CODEC_WORDS,
            &all_season_words,
            data::ORDINAL_SUFFIXES,
        );
        let mut rules = Rules {
            identity: RulesIdentity {
                version: crate::RULES_VERSION.to_owned(),
                languages: None,
                overlays: Vec::new(),
            },
            technical_words: owned(data::TECHNICAL_WORDS),
            title_like_release_words: owned(data::TITLE_LIKE_RELEASE_WORDS),
            edition_words: owned(data::EDITION_WORDS),
            edition_phrases: data::EDITION_PHRASES
                .iter()
                .map(|phrase| owned(phrase))
                .collect(),
            format_3d_words: data::FORMAT_3D_WORDS
                .iter()
                .map(|(word, format)| ((*word).to_owned(), *format))
                .collect(),
            provider_keys: owned(data::PROVIDER_KEYS),
            release_group_endings: owned(data::RELEASE_GROUP_ENDINGS),
            video_extensions: owned(data::VIDEO_EXTENSIONS),
            unsupported_video_extensions: owned(data::UNSUPPORTED_VIDEO_EXTENSIONS),
            disc_image_extensions: owned(data::DISC_IMAGE_EXTENSIONS),
            disc_structure_folders: owned(data::DISC_STRUCTURE_FOLDERS),
            subtitle_extensions: owned(data::SUBTITLE_EXTENSIONS),
            audio_extensions: owned(data::AUDIO_EXTENSIONS),
            image_extensions: owned(data::IMAGE_EXTENSIONS),
            nfo_extension: data::NFO_EXTENSION.to_owned(),
            nfo_generic_names: data::NFO_GENERIC_NAMES
                .iter()
                .map(|(word, role)| ((*word).to_owned(), *role))
                .collect(),
            still_image_suffix: data::STILL_IMAGE_SUFFIX.to_owned(),
            default_flag_words: owned(data::DEFAULT_FLAG_WORDS),
            forced_flag_words: owned(data::FORCED_FLAG_WORDS),
            hearing_impaired_flag_words: owned(data::HEARING_IMPAIRED_FLAG_WORDS),
            hearing_impaired_or_hindi_word: data::HEARING_IMPAIRED_OR_HINDI_WORD.to_owned(),
            language_spellings,
            extras_folders: pairs(data::EXTRAS_FOLDERS),
            extras_suffixes: pairs(data::EXTRAS_SUFFIXES),
            extras_video_names: pairs(data::EXTRAS_VIDEO_NAMES),
            extras_audio_names: pairs(data::EXTRAS_AUDIO_NAMES),
            artwork_roles: data::ARTWORK_ROLES
                .iter()
                .map(|(word, role)| ((*word).to_owned(), *role))
                .collect(),
            built_in_exclusions: BuiltInExclusions {
                groups: owned(data::BUILT_IN_IGNORE_GROUPS),
                no_media_markers: owned(data::NO_MEDIA_MARKERS),
                system_junk_names: owned(data::SYSTEM_JUNK_NAMES),
                system_junk_file_suffixes: owned(data::SYSTEM_JUNK_FILE_SUFFIXES),
                partial_download_file_suffixes: owned(data::PARTIAL_DOWNLOAD_FILE_SUFFIXES),
                generated_metadata_directory_names: owned(data::GENERATED_METADATA_DIRECTORY_NAMES),
                generated_metadata_directory_suffixes: owned(
                    data::GENERATED_METADATA_DIRECTORY_SUFFIXES,
                ),
            },
            season_words,
            series_season_words,
            specials_folder_names: owned(data::SPECIALS_FOLDER_NAMES),
            video_heights: owned(data::VIDEO_HEIGHTS),
            letter_folds: data::LETTER_FOLDS
                .iter()
                .map(|(letter, spelled)| (*letter, (*spelled).to_owned()))
                .collect(),
            leading_articles: owned(data::LEADING_ARTICLES),
            number_words,
            ordinal_suffixes: owned(data::ORDINAL_SUFFIXES),
            release_junk_names: owned(data::RELEASE_JUNK_NAMES),
            release_junk_prefixes: owned(data::RELEASE_JUNK_PREFIXES),
            release_junk_extensions: owned(data::RELEASE_JUNK_EXTENSIONS),
            release_junk_images: owned(data::RELEASE_JUNK_IMAGES),
            release_words,
            audio_channel_words,
            release_site_tags: owned(data::RELEASE_SITE_TAGS),
            presets: Presets::default(),
            patterns,
            release_patterns: OnceLock::new(),
        };
        rules.add_canonical_names();
        rules.recompile();
        rules
    }
}

/// Returns the entries of a built-in table in the vocabulary form (X25).
fn owned<C: FromIterator<String>>(list: &[&str]) -> C {
    list.iter()
        .map(|word| lexical::vocabulary_form(word))
        .collect()
}

impl Patterns {
    fn compile(
        aspect_ratios: &[&str],
        audio_codecs: &[&str],
        season_words: &[&str],
        ordinal_suffixes: &[&str],
    ) -> Self {
        let alternation = |words: &mut dyn Iterator<Item = String>| {
            let mut escaped: Vec<String> = words.map(|word| regex::escape(&word)).collect();
            // Longest first, so a word is never cut short by one it starts with.
            escaped.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
            escaped.dedup();
            if escaped.is_empty() {
                // An empty vocabulary must match nothing.
                "[^\\s\\S]".to_owned()
            } else {
                escaped.join("|")
            }
        };
        let ratios = alternation(&mut aspect_ratios.iter().map(|ratio| (*ratio).to_owned()));
        let codecs = alternation(&mut audio_codecs.iter().map(|codec| (*codec).to_owned()));
        // Season words in both composed and decomposed Unicode forms (X3), so `Série`
        // matches however its accent is encoded.
        let seasons = alternation(&mut season_words.iter().flat_map(|word| {
            [
                word.nfc().collect::<String>(),
                word.nfd().collect::<String>(),
            ]
        }));
        let ordinals = alternation(&mut ordinal_suffixes.iter().map(|suffix| (*suffix).to_owned()));
        Patterns {
            aspect_ratio: Regex::new(&format!("(?i)(?:{ratios})"))
                .expect("valid aspect ratio pattern"),
            audio_layout: Regex::new(&format!(r"(?i)(?:{codecs})[ .]?\d\.\d(?:x\d)?"))
                .expect("valid audio layout pattern"),
            season_word_then_number: Regex::new(&format!(r"(?i)({seasons})[ ._-]*(\d{{1,5}})"))
                .expect("valid season word pattern"),
            ordinal_then_season_word: Regex::new(&format!(
                r"(?i)(\d{{1,2}})(?:{ordinals})[ ._-]*({seasons})"
            ))
            .expect("valid ordinal season pattern"),
        }
    }
}
