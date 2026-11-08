//! Provides the input vocabulary as one value: every word list that gives the meaning of an
//! existing name. Operations are methods on [`Rules`]. The free functions of the crate use
//! [`Rules::builtin`]. The output profile, not the rules, defines how names are written
//! (extras output folders, flag words, reserved device names).

use std::sync::{LazyLock, OnceLock};

use regex::Regex;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::data;
use crate::lexical;
use crate::parsers::release::ReleasePatterns;
use crate::rendering::template::Template;

mod overlay;
mod vocabulary;

pub(crate) use vocabulary::Vocabularies;

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
    pub(crate) vocab: Vocabularies,
    pub(crate) patterns: Patterns,
    /// The compiled release patterns, built on first use, because most callers never read
    /// release facts.
    release_patterns: OnceLock<ReleasePatterns>,
}

/// The typed fact that a bracket tag key states (X29).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "fact", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum TagFact {
    Edition,
    ProviderId { id: IdShape },
}

/// The shape of a provider ID.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
/// defaults are the recorded decisions. A field's name is the preset's name in an overlay,
/// and a variant's name is a value of the preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DateOrder {
    Ambiguous,
    DayFirst,
    MonthFirst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BareNumbers {
    Anywhere,
    SeasonFoldersOnly,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum IgnoreNesting {
    #[default]
    Stack,
    Nearest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Samples {
    Extras,
    Excluded,
}

impl Presets {
    /// Sets the preset `name` to `value`.
    /// Fails with [`PresetError::UnknownPreset`] if `name` is not a preset.
    /// Fails with [`PresetError::InvalidValue`] if `value` is not an alternative of the preset,
    /// or a year outside the preset's range.
    pub(crate) fn set(&mut self, name: &str, value: &str) -> Result<(), PresetError> {
        let serde_json::Value::Object(mut fields) =
            serde_json::to_value(*self).expect("the presets serialize")
        else {
            unreachable!("the presets serialize as a mapping");
        };
        let field = fields.get_mut(name).ok_or(PresetError::UnknownPreset)?;
        *field = if field.is_number() {
            value
                .parse::<u16>()
                .map(serde_json::Value::from)
                .map_err(|_| PresetError::InvalidValue)?
        } else {
            serde_json::Value::String(value.to_owned())
        };
        let presets: Presets = serde_json::from_value(serde_json::Value::Object(fields))
            .map_err(|_| PresetError::InvalidValue)?;
        let years = data::LOWEST_EARLIEST_YEAR..=data::LATEST_PLAUSIBLE_YEAR;
        let latest_years = data::EARLIEST_PLAUSIBLE_YEAR..=data::LATEST_PLAUSIBLE_YEAR;
        if !years.contains(&presets.earliest_plausible_year)
            || !latest_years.contains(&presets.latest_plausible_year)
        {
            return Err(PresetError::InvalidValue);
        }
        *self = presets;
        Ok(())
    }
}

pub(crate) enum PresetError {
    UnknownPreset,
    InvalidValue,
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

static BASE: LazyLock<Rules> = LazyLock::new(Rules::from_builtin_document);
static BUILTIN: LazyLock<Rules> = LazyLock::new(Rules::with_every_language);

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

    /// Returns the shared base: the built-in rules without the words of any language pack.
    fn base() -> &'static Rules {
        &BASE
    }

    /// Returns the identity of these rules.
    pub fn identity(&self) -> &RulesIdentity {
        &self.identity
    }

    /// Returns `true` if `name` is a no-media marker name of these rules (X14). The
    /// comparison ignores case. A caller that scans folders reads each such file and
    /// passes it as an ignore file.
    pub fn is_no_media_marker(&self, name: &str) -> bool {
        self.vocab
            .no_media_markers
            .contains(&lexical::fold_case_text(name))
    }

    /// Returns the tag key that `key` names, with its fact: `key` itself, or a provider key
    /// that `key` follows with `id` (M7).
    /// Returns `None` if no tag has that key.
    pub(crate) fn tag(&self, key: &str) -> Option<(String, &TagFact)> {
        if let Some(fact) = self.vocab.tags.get(key) {
            return Some((key.to_owned(), fact));
        }
        let provider = key.strip_suffix("id")?;
        match self.vocab.tags.get(provider)? {
            fact @ TagFact::ProviderId { .. } => Some((provider.to_owned(), fact)),
            TagFact::Edition => None,
        }
    }

    /// Returns `value` if it is a valid ID of `provider`.
    /// Returns `None` if `provider` is not a provider tag or `value` does not have its shape.
    pub(crate) fn valid_provider_id(&self, provider: &str, value: &str) -> Option<String> {
        match self.vocab.tags.get(provider)? {
            TagFact::ProviderId { id: shape } if shape.is_valid(value) => Some(value.to_owned()),
            _ => None,
        }
    }

    /// Returns the compiled release word and audio channel patterns. Compiles them on the
    /// first call.
    pub(crate) fn release_patterns(&self) -> &ReleasePatterns {
        self.release_patterns.get_or_init(|| {
            ReleasePatterns::compile(&self.vocab.release_words, &self.vocab.audio_channel_words)
        })
    }

    /// Returns rules with empty vocabularies.
    fn empty() -> Rules {
        let vocab = Vocabularies::default();
        Rules {
            identity: RulesIdentity {
                version: crate::RULES_VERSION.to_owned(),
                languages: None,
                overlays: Vec::new(),
            },
            patterns: Patterns::compile(&vocab),
            vocab,
            release_patterns: OnceLock::new(),
        }
    }

    /// Compiles the patterns again. Called after the vocabularies change.
    fn recompile(&mut self) {
        self.patterns = Patterns::compile(&self.vocab);
        self.release_patterns = OnceLock::new();
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

impl Patterns {
    fn compile(vocab: &Vocabularies) -> Self {
        // Longest first, so a word is never cut short by one it starts with. Each word is
        // also written in decomposed Unicode (X3), so `Série` matches however its accent
        // is encoded.
        let alternation = |words: &mut dyn Iterator<Item = &String>| {
            let mut escaped: Vec<String> = words
                .flat_map(|word| [word.nfc().collect::<String>(), word.nfd().collect()])
                .map(|word| regex::escape(&word))
                .collect();
            escaped.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
            escaped.dedup();
            if escaped.is_empty() {
                // An empty vocabulary must match nothing.
                "[^\\s\\S]".to_owned()
            } else {
                escaped.join("|")
            }
        };
        let ratios = alternation(&mut vocab.aspect_ratios.iter());
        let codecs = alternation(&mut vocab.audio_codec_words.iter());
        let seasons = alternation(&mut vocab.season_words.iter().chain(&vocab.series_season_words));
        let plain_seasons = alternation(&mut vocab.season_words.iter());
        let episodes = alternation(&mut vocab.episode_words.iter());
        let specials = alternation(&mut vocab.special_prefixes.iter());
        let animation = alternation(&mut vocab.animation_special_words.keys());
        let dashes: String = lexical::DASHES.iter().collect();
        let ordinals = alternation(&mut vocab.ordinal_suffixes.iter());
        let parts = alternation(&mut vocab.part_words.iter());
        let discs = alternation(&mut vocab.disc_folder_words.iter());
        let alternates = alternation(&mut vocab.alternate_title_words.iter());
        let finals = alternation(&mut vocab.final_episode_words.iter());
        let episode_tokens: Vec<&str> = vocab.episode_tokens.iter().map(String::as_str).collect();
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
            episode_tokens: compile_episode_tokens(&episode_tokens),
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
