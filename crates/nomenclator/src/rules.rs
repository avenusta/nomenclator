//! Provides the input vocabulary as one value: every word list that gives the meaning of an
//! existing name. Operations are methods on [`Rules`]. The free functions of the crate use
//! [`Rules::builtin`]. The output profile, not the rules, defines how names are written
//! (extras output folders, flag words, reserved device names).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{LazyLock, OnceLock};

use regex::Regex;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::data;
use crate::lexical;
use crate::model::{ArtworkRole, ExtraKind, Format3d, NfoRole};
use crate::parsers::episode_name::SpecialKind;
use crate::parsers::release::{ReleaseFact, ReleasePatterns, release_spelling_form};
use crate::rendering::template::Template;

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
    /// Bracket tag keys and the typed fact that each states (X29).
    pub(crate) tags: BTreeMap<String, TagFact>,
    pub(crate) release_group_endings: Vec<String>,
    pub(crate) video_extensions: BTreeSet<String>,
    pub(crate) unsupported_video_extensions: BTreeSet<String>,
    pub(crate) disc_image_extensions: BTreeSet<String>,
    pub(crate) disc_structure_folders: BTreeSet<String>,
    pub(crate) subtitle_extensions: BTreeSet<String>,
    pub(crate) audio_extensions: BTreeSet<String>,
    pub(crate) image_extensions: BTreeSet<String>,
    pub(crate) nfo_extensions: BTreeSet<String>,
    pub(crate) nfo_generic_names: BTreeMap<String, NfoRole>,
    /// Suffixes that mark an image as the still of the video that it is named after.
    pub(crate) still_image_suffixes: BTreeSet<String>,
    pub(crate) default_flag_words: BTreeSet<String>,
    pub(crate) forced_flag_words: BTreeSet<String>,
    pub(crate) hearing_impaired_flag_words: BTreeSet<String>,
    /// Words that mean hearing impaired beside another language, and otherwise the language
    /// of each mapped BCP 47 tag.
    pub(crate) hearing_impaired_or_language_words: BTreeMap<String, String>,
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
    /// Templates of episode tokens that a caller adds (X34).
    pub(crate) episode_tokens: BTreeSet<String>,
    /// Characters between words, other than the dash (M6).
    pub(crate) separators: BTreeSet<char>,
    /// The closing character of each bracket pair (M5).
    pub(crate) bracket_pairs: BTreeMap<char, char>,
    /// Words before an episode number (X30).
    pub(crate) episode_words: BTreeSet<String>,
    /// Prefixes joined to a number that state an episode of season 0 (T12).
    pub(crate) special_prefixes: BTreeSet<String>,
    /// Animation special markers and the kind of each (X20).
    pub(crate) animation_special_words: BTreeMap<String, SpecialKind>,
    /// Words that name a part before its number or letter (M11).
    pub(crate) part_words: BTreeSet<String>,
    /// Words that name a folder of episodes before its number (T18).
    pub(crate) disc_folder_words: BTreeSet<String>,
    /// Folder names that hold the subtitles of the videos beside them (S2).
    pub(crate) subtitle_folder_names: BTreeSet<String>,
    /// Aspect ratios, never a season and episode (T16).
    pub(crate) aspect_ratios: BTreeSet<String>,
    /// Audio codec words that a channel layout may follow (T16).
    pub(crate) audio_codec_words: BTreeSet<String>,
    /// Words after an episode number that mark the last episode of a release run (X20).
    pub(crate) final_episode_words: BTreeSet<String>,
    /// Words between separators that start another title of a movie (M16).
    pub(crate) alternate_title_words: BTreeSet<String>,
    /// Text that marks a release word spelling as a remux (X19).
    pub(crate) remux_markers: BTreeSet<String>,
    /// Text that marks a source spelling as a screener (X19).
    pub(crate) screener_markers: BTreeSet<String>,
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

/// The typed fact that a bracket tag key states (X29).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(tag = "fact", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum TagFact {
    Edition,
    ProviderId { id: IdShape },
}

/// The shape of a provider ID.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum IdShape {
    /// A positive integer.
    Integer,
    /// Letters, digits, `-`, `_` and `.`, starting with a letter or digit.
    Slug,
    /// A prefix, then a digit count within the range.
    Prefixed {
        prefix: String,
        min_digits: u8,
        max_digits: u8,
    },
}

impl IdShape {
    pub(crate) fn is_prefixed(&self) -> bool {
        matches!(self, IdShape::Prefixed { .. })
    }

    /// Returns `true` if `value` has the form of this shape. A prefixed ID of only zeros has
    /// the form and is not valid.
    pub(crate) fn has_form(&self, value: &str) -> bool {
        match self {
            IdShape::Integer => {
                !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
            }
            IdShape::Slug => {
                value.starts_with(|c: char| c.is_alphanumeric())
                    && value
                        .chars()
                        .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
            }
            IdShape::Prefixed {
                prefix,
                min_digits,
                max_digits,
            } => value.strip_prefix(prefix.as_str()).is_some_and(|digits| {
                (usize::from(*min_digits)..=usize::from(*max_digits)).contains(&digits.len())
                    && digits.bytes().all(|byte| byte.is_ascii_digit())
            }),
        }
    }

    /// Returns `true` if `value` is an ID of this shape.
    pub(crate) fn is_valid(&self, value: &str) -> bool {
        self.has_form(value)
            && (matches!(self, IdShape::Slug)
                || value.bytes().any(|byte| (b'1'..=b'9').contains(&byte)))
    }
}

/// Choices between alternatives that the decision catalogue records as contested (X12). The
/// defaults are the recorded decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Presets {
    /// The first year a name's number can be (X5).
    pub earliest_plausible_year: u16,
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
            earliest_plausible_year: data::EARLIEST_PLAUSIBLE_YEAR,
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
            "earliest_plausible_year" => match value.parse::<u16>() {
                Ok(year)
                    if (data::LOWEST_EARLIEST_YEAR..=data::LATEST_PLAUSIBLE_YEAR)
                        .contains(&year) =>
                {
                    self.earliest_plausible_year = year;
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
    /// A part word and its number or letter `a` to `d` at the end of a name, after a
    /// separator or bracket, optionally bracketed itself: `part1`, ` - cd 2`, `(Disc 01)`,
    /// `]pt.b`.
    pub part_marker: Regex,
    /// A whole folder name of a disc word and its number (`Disc 1`).
    pub disc_folder: Regex,
    /// An alternate title word between separators (` AKA `).
    pub alternate_title_separator: Regex,
    /// A season word, its number, an episode word and its number (`Staffel 2 Folge 3`).
    pub season_episode_words: Regex,
    /// An episode word, then its number (`Ep 12`).
    pub episode_word: Regex,
    /// A special prefix joined to its number (`SP02`).
    pub special_prefix: Regex,
    /// An animation special marker with an optional number and release version.
    pub animation_special: Regex,
    /// The episode tokens that a caller adds, longest template first (X34).
    pub episode_tokens: Vec<EpisodeTokenPattern>,
    /// The suffix after an episode number: an optional `v2` and an optional final word.
    pub episode_suffix: Regex,
}

/// An episode token template, compiled. The pattern captures `episode` and may capture
/// `season`.
#[derive(Debug, Clone)]
pub(crate) struct EpisodeTokenPattern {
    pub pattern: Regex,
    pub states_season: bool,
}

/// The vocabularies that [`Patterns`] are compiled from.
#[derive(Default)]
struct PatternWords<'a> {
    aspect_ratios: Vec<&'a str>,
    audio_codecs: Vec<&'a str>,
    season_words: Vec<&'a str>,
    ordinal_suffixes: Vec<&'a str>,
    part_words: Vec<&'a str>,
    disc_folder_words: Vec<&'a str>,
    alternate_title_words: Vec<&'a str>,
    final_episode_words: Vec<&'a str>,
    plain_season_words: Vec<&'a str>,
    episode_words: Vec<&'a str>,
    special_prefixes: Vec<&'a str>,
    animation_special_words: Vec<&'a str>,
    episode_tokens: Vec<&'a str>,
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

    /// Returns the tag key that `key` names, with its fact: `key` itself, or a provider key
    /// that `key` follows with `id` (M7).
    /// Returns `None` if no tag has that key.
    pub(crate) fn tag(&self, key: &str) -> Option<(String, &TagFact)> {
        if let Some(fact) = self.tags.get(key) {
            return Some((key.to_owned(), fact));
        }
        let provider = key.strip_suffix("id")?;
        match self.tags.get(provider)? {
            fact @ TagFact::ProviderId { .. } => Some((provider.to_owned(), fact)),
            TagFact::Edition => None,
        }
    }

    /// Returns `value` if it is a valid ID of `provider`.
    /// Returns `None` if `provider` is not a provider tag or `value` does not have its shape.
    pub(crate) fn valid_provider_id(&self, provider: &str, value: &str) -> Option<String> {
        match self.tags.get(provider)? {
            TagFact::ProviderId { id: shape } if shape.is_valid(value) => Some(value.to_owned()),
            _ => None,
        }
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
        self.patterns = Patterns::compile(&PatternWords {
            aspect_ratios: as_strs(&self.aspect_ratios),
            audio_codecs: as_strs(&self.audio_codec_words),
            season_words: self
                .season_words
                .iter()
                .chain(&self.series_season_words)
                .map(String::as_str)
                .collect(),
            ordinal_suffixes: as_strs(&self.ordinal_suffixes),
            part_words: as_strs(&self.part_words),
            disc_folder_words: as_strs(&self.disc_folder_words),
            alternate_title_words: as_strs(&self.alternate_title_words),
            final_episode_words: as_strs(&self.final_episode_words),
            plain_season_words: as_strs(&self.season_words),
            episode_words: as_strs(&self.episode_words),
            special_prefixes: as_strs(&self.special_prefixes),
            episode_tokens: as_strs(&self.episode_tokens),
            animation_special_words: self
                .animation_special_words
                .keys()
                .map(String::as_str)
                .collect(),
        });
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
            tags: data::PROVIDER_TAGS
                .iter()
                .map(|(key, prefix)| {
                    let id = prefix.map_or(IdShape::Integer, |(prefix, min_digits, max_digits)| {
                        IdShape::Prefixed {
                            prefix: prefix.to_owned(),
                            min_digits,
                            max_digits,
                        }
                    });
                    ((*key).to_owned(), TagFact::ProviderId { id })
                })
                .collect(),
            release_group_endings: owned(data::RELEASE_GROUP_ENDINGS),
            video_extensions: owned(data::VIDEO_EXTENSIONS),
            unsupported_video_extensions: owned(data::UNSUPPORTED_VIDEO_EXTENSIONS),
            disc_image_extensions: owned(data::DISC_IMAGE_EXTENSIONS),
            disc_structure_folders: owned(data::DISC_STRUCTURE_FOLDERS),
            subtitle_extensions: owned(data::SUBTITLE_EXTENSIONS),
            audio_extensions: owned(data::AUDIO_EXTENSIONS),
            image_extensions: owned(data::IMAGE_EXTENSIONS),
            // The canonical names add the extension and the suffix (X25).
            nfo_extensions: BTreeSet::new(),
            nfo_generic_names: data::NFO_GENERIC_NAMES
                .iter()
                .map(|(word, role)| ((*word).to_owned(), *role))
                .collect(),
            still_image_suffixes: BTreeSet::new(),
            default_flag_words: owned(data::DEFAULT_FLAG_WORDS),
            forced_flag_words: owned(data::FORCED_FLAG_WORDS),
            hearing_impaired_flag_words: owned(data::HEARING_IMPAIRED_FLAG_WORDS),
            hearing_impaired_or_language_words: data::HEARING_IMPAIRED_OR_LANGUAGE_WORDS
                .iter()
                .map(|(word, tag)| ((*word).to_owned(), (*tag).to_owned()))
                .collect(),
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
            episode_tokens: BTreeSet::new(),
            // The canonical names add the space and the written bracket pairs (X25).
            separators: data::SEPARATORS.iter().copied().collect(),
            bracket_pairs: data::BRACKET_PAIRS.iter().copied().collect(),
            episode_words: data::EPISODE_BASE_WORDS
                .iter()
                .chain(data::EPISODE_WORDS)
                .map(|word| lexical::vocabulary_form(word))
                .collect(),
            special_prefixes: owned(data::SPECIAL_PREFIXES),
            animation_special_words: data::ANIMATION_SPECIAL_WORDS
                .iter()
                .map(|(word, kind)| ((*word).to_owned(), *kind))
                .collect(),
            part_words: data::DISC_WORDS
                .iter()
                .chain(data::OTHER_PART_WORDS)
                .map(|word| lexical::vocabulary_form(word))
                .collect(),
            disc_folder_words: owned(data::DISC_WORDS),
            subtitle_folder_names: owned(data::SUBTITLE_FOLDER_NAMES),
            aspect_ratios: owned(data::ASPECT_RATIOS),
            audio_codec_words: owned(data::AUDIO_CODEC_WORDS),
            final_episode_words: owned(data::FINAL_EPISODE_WORDS),
            alternate_title_words: owned(data::ALTERNATE_TITLE_WORDS),
            remux_markers: owned(data::REMUX_MARKERS),
            screener_markers: owned(data::SCREENER_MARKERS),
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
            patterns: Patterns::compile(&PatternWords::default()),
            release_patterns: OnceLock::new(),
        };
        rules.add_canonical_names();
        rules.recompile();
        rules
    }
}

/// Compiles the templates of episode tokens, longest first.
fn compile_episode_tokens(templates: &[&str]) -> Vec<EpisodeTokenPattern> {
    let mut sorted = templates.to_vec();
    sorted.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    sorted
        .into_iter()
        .map(|text| {
            let template = Template::parse(text).expect("an overlay checks its token templates");
            EpisodeTokenPattern {
                pattern: Regex::new(&format!("(?i){}", template.token_pattern()))
                    .expect("valid episode token pattern"),
                states_season: template.fields().contains("season"),
            }
        })
        .collect()
}

fn as_strs(words: &BTreeSet<String>) -> Vec<&str> {
    words.iter().map(String::as_str).collect()
}

/// Returns the entries of a built-in table in the vocabulary form (X25).
fn owned<C: FromIterator<String>>(list: &[&str]) -> C {
    list.iter()
        .map(|word| lexical::vocabulary_form(word))
        .collect()
}

impl Patterns {
    fn compile(words: &PatternWords) -> Self {
        let alternation = |list: &mut dyn Iterator<Item = String>| {
            let mut escaped: Vec<String> = list.map(|word| regex::escape(&word)).collect();
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
        let ratios = alternation(&mut words.aspect_ratios.iter().map(|ratio| (*ratio).to_owned()));
        let codecs = alternation(&mut words.audio_codecs.iter().map(|codec| (*codec).to_owned()));
        // Words in both composed and decomposed Unicode forms (X3), so `Série` matches
        // however its accent is encoded.
        let unicode_alternation = |list: &[&str]| {
            alternation(&mut list.iter().flat_map(|word| {
                [
                    word.nfc().collect::<String>(),
                    word.nfd().collect::<String>(),
                ]
            }))
        };
        let seasons = unicode_alternation(&words.season_words);
        let plain_seasons = unicode_alternation(&words.plain_season_words);
        let episodes = unicode_alternation(&words.episode_words);
        let specials = unicode_alternation(&words.special_prefixes);
        let animation = unicode_alternation(&words.animation_special_words);
        let dashes: String = lexical::DASHES.iter().collect();
        let ordinals = alternation(
            &mut words
                .ordinal_suffixes
                .iter()
                .map(|suffix| (*suffix).to_owned()),
        );
        let parts = alternation(&mut words.part_words.iter().map(|word| (*word).to_owned()));
        let discs = alternation(
            &mut words
                .disc_folder_words
                .iter()
                .map(|word| (*word).to_owned()),
        );
        let alternates = alternation(
            &mut words
                .alternate_title_words
                .iter()
                .map(|word| (*word).to_owned()),
        );
        let finals = alternation(
            &mut words
                .final_episode_words
                .iter()
                .map(|word| (*word).to_owned()),
        );
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
            part_marker: Regex::new(&format!(
                r"(?i)^(.*[ ._\-\[\]()])?(?:{parts})[ ._-]?(\d+|[a-d])\)?$"
            ))
            .expect("valid part marker pattern"),
            disc_folder: Regex::new(&format!(r"(?i)^(?:{discs})[ ._-]*\d{{1,2}}$"))
                .expect("valid disc folder pattern"),
            episode_tokens: compile_episode_tokens(&words.episode_tokens),
            alternate_title_separator: Regex::new(&format!(r"(?i)[ ._](?:{alternates})[ ._]"))
                .expect("valid alternate title pattern"),
            season_episode_words: Regex::new(&format!(
                r"(?i)(?:{plain_seasons})[ ._-]?(\d{{1,4}})[ ._-]*(?:{episodes})[ ._-]?(\d{{1,6}})"
            ))
            .expect("valid season episode words pattern"),
            episode_word: Regex::new(&format!(
                r"(?i)({episodes})(?:[ ._]*[{dashes}][ ._]*|[ ._]?)(\d{{1,6}})"
            ))
            .expect("valid episode pattern"),
            special_prefix: Regex::new(&format!(r"(?i)(?:{specials})(\d{{1,3}})"))
                .expect("valid special pattern"),
            animation_special: Regex::new(&format!(
                r"(?i)({animation})(?:[ ._-]*(\d{{1,3}}))?(?:v\d{{1,2}})?"
            ))
            .expect("valid animation special pattern"),
            episode_suffix: Regex::new(&format!(r"(?i)^(?:v\d{{1,2}})?(?:{finals})?"))
                .expect("valid episode suffix pattern"),
        }
    }
}
