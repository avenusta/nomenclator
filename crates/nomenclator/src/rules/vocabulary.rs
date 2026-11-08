//! Provides the registry of the input vocabularies: one row per vocabulary, and one table
//! type that stores, reads and changes the entries of any vocabulary.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::lexical;
use crate::model::{ArtworkRole, ExtraKind, Format3d, NfoRole};
use crate::parsers::episode_name::SpecialKind;
use crate::parsers::extras::folded_vocabulary_form;
use crate::parsers::release::{ReleaseFact, release_spelling_form};
use crate::rendering::template::Template;
use crate::rules::{IdShape, Presets, TagFact};

/// How an entry is normalized into the form that the lookups of its vocabulary compare.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Form {
    /// [`lexical::vocabulary_form`] of the trimmed text.
    Word,
    /// The text as written. A separator can be a space, which trimming would drop.
    Raw,
    /// The trimmed text with its case kept: bracket characters and token templates.
    Trimmed,
    /// [`lexical::vocabulary_form`] without separators, as extras words compare (E1).
    Extras,
    /// The words of an edition phrase without apostrophes, joined by one space.
    Phrase,
    /// [`lexical::vocabulary_form`] without a leading dot.
    Extension,
    /// The words of a release spelling, joined by one space (X26).
    ReleaseSpelling,
    /// A title comparison key without accents (P4).
    TitleKey,
}

impl Form {
    /// Returns `raw` in this form.
    pub(crate) fn normalize(self, raw: &str) -> String {
        let word = || lexical::vocabulary_form(raw.trim());
        match self {
            Form::Word => word(),
            Form::Raw => raw.to_owned(),
            Form::Trimmed => raw.trim().to_owned(),
            Form::Extras => folded_vocabulary_form(&word()),
            Form::Phrase => word()
                .replace(['\'', '’'], "")
                .split([' ', '.', '_', '-'])
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
            Form::Extension => word().trim_start_matches('.').to_owned(),
            Form::ReleaseSpelling => release_spelling_form(raw),
            Form::TitleKey => lexical::without_accents(&lexical::plain_title_key(raw)),
        }
    }
}

/// The layout of the `add` section of a vocabulary in an overlay document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    /// A list of entries.
    List,
    /// A map from each entry to its value.
    Map,
    /// A map from each value to its other spellings. The value is also an entry
    /// (`language_spellings`).
    Spellings,
    /// A map from release fact to value to spellings (`release_words`).
    ReleaseSpellings,
    /// A map from each entry to its value, written without `add` or `remove` (`presets`).
    Settings,
}

/// A group of vocabularies whose words must not overlap: a name word is read as one class
/// only (X10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rival {
    /// Technical, title-like and edition words.
    Release,
    /// Default, forced and hearing-impaired flag words.
    Flags,
    /// Season and series-season words.
    Seasons,
}

/// Checks the raw entry and the value of one addition, and returns the value in its stored
/// form. Fails with a description of the fault.
pub(crate) type Check = fn(&str, Value) -> Result<Value, String>;

/// One row of the registry.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Spec {
    /// The name of the vocabulary in an overlay document and in diagnostics.
    pub name: &'static str,
    pub form: Form,
    pub shape: Shape,
    /// `true` if the entries are read as whole name words, so a word that the grammar
    /// already reads cannot take effect (`reserved_token`).
    pub name_words: bool,
    pub rival: Option<Rival>,
    /// The only entries that the vocabulary can contain, if it is closed.
    pub closed: Option<&'static [&'static str]>,
    pub check: Check,
}

impl Spec {
    const fn new(form: Form, shape: Shape) -> Spec {
        Spec {
            name: "",
            form,
            shape,
            name_words: false,
            rival: None,
            closed: None,
            check: unchecked,
        }
    }

    const fn list(form: Form) -> Spec {
        Spec::new(form, Shape::List)
    }

    const fn map(form: Form) -> Spec {
        Spec::new(form, Shape::Map)
    }

    const fn name_words(self) -> Spec {
        Spec {
            name_words: true,
            ..self
        }
    }

    const fn rival(self, rival: Rival) -> Spec {
        Spec {
            rival: Some(rival),
            ..self
        }
    }

    const fn closed(self, entries: &'static [&'static str]) -> Spec {
        Spec {
            closed: Some(entries),
            ..self
        }
    }

    const fn check(self, check: Check) -> Spec {
        Spec { check, ..self }
    }
}

/// The entries of one vocabulary, whatever their stored type.
pub(crate) trait Table {
    /// Returns every entry, in stored form.
    fn entries(&self) -> Vec<String>;
    fn contains(&self, entry: &str) -> bool;
    /// Fails if `value` is not a value of this vocabulary.
    fn read_value(&self, value: &Value) -> Result<(), String>;
    /// Adds `entry` with `value`. An entry or value of the wrong shape is skipped, because
    /// the overlay reader checked both.
    fn insert(&mut self, entry: &str, value: &Value);
    fn remove(&mut self, entry: &str);
}

/// The key type of a table: a word, or one character.
pub(crate) trait Key: Ord + Sized {
    fn read(entry: &str) -> Option<Self>;
    fn write(&self) -> String;
}

impl Key for String {
    fn read(entry: &str) -> Option<Self> {
        Some(entry.to_owned())
    }

    fn write(&self) -> String {
        self.clone()
    }
}

impl Key for char {
    fn read(entry: &str) -> Option<Self> {
        let mut characters = entry.chars();
        match (characters.next(), characters.next()) {
            (Some(character), None) => Some(character),
            _ => None,
        }
    }

    fn write(&self) -> String {
        self.to_string()
    }
}

impl<K: Key> Table for BTreeSet<K> {
    fn entries(&self) -> Vec<String> {
        self.iter().map(Key::write).collect()
    }

    fn contains(&self, entry: &str) -> bool {
        K::read(entry).is_some_and(|key| BTreeSet::contains(self, &key))
    }

    fn read_value(&self, _: &Value) -> Result<(), String> {
        Ok(())
    }

    fn insert(&mut self, entry: &str, _: &Value) {
        if let Some(key) = K::read(entry) {
            BTreeSet::insert(self, key);
        }
    }

    fn remove(&mut self, entry: &str) {
        if let Some(key) = K::read(entry) {
            BTreeSet::remove(self, &key);
        }
    }
}

impl<K: Key, V: DeserializeOwned> Table for BTreeMap<K, V> {
    fn entries(&self) -> Vec<String> {
        self.keys().map(Key::write).collect()
    }

    fn contains(&self, entry: &str) -> bool {
        K::read(entry).is_some_and(|key| self.contains_key(&key))
    }

    fn read_value(&self, value: &Value) -> Result<(), String> {
        V::deserialize(value)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn insert(&mut self, entry: &str, value: &Value) {
        if let (Some(key), Ok(value)) = (K::read(entry), V::deserialize(value)) {
            BTreeMap::insert(self, key, value);
        }
    }

    fn remove(&mut self, entry: &str) {
        if let Some(key) = K::read(entry) {
            BTreeMap::remove(self, &key);
        }
    }
}

/// The presets are settings, not entries: an overlay sets them and never lists them.
impl Table for Presets {
    fn entries(&self) -> Vec<String> {
        Vec::new()
    }

    fn contains(&self, _: &str) -> bool {
        false
    }

    fn read_value(&self, _: &Value) -> Result<(), String> {
        Ok(())
    }

    fn insert(&mut self, entry: &str, value: &Value) {
        if let Some(value) = value.as_str() {
            // The overlay reader checked the preset and its value.
            let _ = self.set(entry, value);
        }
    }

    fn remove(&mut self, _: &str) {}
}

/// Defines the vocabularies: one row per vocabulary, with its stored type and its [`Spec`].
/// The row's field name is the vocabulary's name in an overlay document.
macro_rules! vocabularies {
    ($($(#[doc = $doc:literal])* $field:ident: $type:ty = $spec:expr;)*) => {
        /// Every input vocabulary.
        #[derive(Debug, Clone, Default)]
        pub(crate) struct Vocabularies {
            $($(#[doc = $doc])* pub(crate) $field: $type,)*
        }

        /// A vocabulary, by its field in [`Vocabularies`].
        #[allow(non_camel_case_types)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        pub(crate) enum Vocabulary {
            $($field,)*
        }

        const SPECS: &[Spec] = &[$(Spec { name: stringify!($field), ..$spec },)*];

        impl Vocabulary {
            /// Every vocabulary, in registry order.
            pub(crate) const ALL: &[Vocabulary] = &[$(Vocabulary::$field,)*];
        }

        impl Vocabularies {
            /// Returns the table of `vocabulary`.
            pub(crate) fn table(&self, vocabulary: Vocabulary) -> &dyn Table {
                match vocabulary {
                    $(Vocabulary::$field => &self.$field,)*
                }
            }

            /// Returns the table of `vocabulary` for a change.
            pub(crate) fn table_mut(&mut self, vocabulary: Vocabulary) -> &mut dyn Table {
                match vocabulary {
                    $(Vocabulary::$field => &mut self.$field,)*
                }
            }
        }
    };
}

impl Vocabulary {
    /// Returns the registry row of the vocabulary.
    pub(crate) fn spec(self) -> &'static Spec {
        &SPECS[self as usize]
    }

    /// Returns the name of the vocabulary in an overlay document.
    pub(crate) fn name(self) -> &'static str {
        self.spec().name
    }

    /// Returns the vocabulary named `name` in an overlay document.
    /// Returns `None` if no vocabulary has that name.
    pub(crate) fn named(name: &str) -> Option<Vocabulary> {
        Vocabulary::ALL
            .iter()
            .copied()
            .find(|vocabulary| vocabulary.name() == name)
    }
}

use Form::{Extension, Extras, Phrase, Raw, ReleaseSpelling, TitleKey, Trimmed, Word};

vocabularies! {
    /// Release words that cut a title wherever they appear (M4).
    technical_words: BTreeSet<String> = Spec::list(Word).name_words().rival(Rival::Release);
    /// Release words that can also be title words (M4).
    title_like_release_words: BTreeSet<String> =
        Spec::list(Word).name_words().rival(Rival::Release);
    /// Edition words after a title (M9).
    edition_words: BTreeSet<String> = Spec::list(Word).name_words().rival(Rival::Release);
    /// Edition phrases, each stored as its words joined by one space (M9).
    edition_phrases: BTreeSet<String> = Spec::list(Phrase).name_words();
    extras_folders: BTreeMap<String, ExtraKind> = Spec::map(Extras);
    extras_suffixes: BTreeMap<String, ExtraKind> = Spec::map(Extras);
    extras_video_names: BTreeMap<String, ExtraKind> = Spec::map(Extras);
    extras_audio_names: BTreeMap<String, ExtraKind> = Spec::map(Extras);
    season_words: BTreeSet<String> = Spec::list(Word).name_words().rival(Rival::Seasons);
    series_season_words: BTreeSet<String> =
        Spec::list(Word).name_words().rival(Rival::Seasons);
    specials_folders: BTreeSet<String> = Spec::list(Word).name_words();
    /// Each spelling, the tag itself included, and the BCP 47 tag that it names.
    language_spellings: BTreeMap<String, String> =
        Spec::new(Word, Shape::Spellings).name_words().check(language_tag);
    default_flag_words: BTreeSet<String> = Spec::list(Word).name_words().rival(Rival::Flags);
    forced_flag_words: BTreeSet<String> = Spec::list(Word).name_words().rival(Rival::Flags);
    hearing_impaired_flag_words: BTreeSet<String> =
        Spec::list(Word).name_words().rival(Rival::Flags);
    artwork_roles: BTreeMap<String, ArtworkRole> = Spec::map(Word);
    nfo_names: BTreeMap<String, NfoRole> = Spec::map(Word);
    /// Bracket tag keys and the typed fact that each states (X29).
    tags: BTreeMap<String, TagFact> = Spec::map(Word).check(tag);
    /// Templates of episode tokens (X34).
    episode_tokens: BTreeSet<String> = Spec::list(Trimmed).check(episode_token);
    /// Characters between words, other than the dash (M6).
    separators: BTreeSet<char> = Spec::list(Raw).check(separator);
    /// The closing character of each bracket pair (M5).
    bracket_pairs: BTreeMap<char, char> = Spec::map(Trimmed).check(bracket_pair);
    /// Words before an episode number (X30).
    episode_words: BTreeSet<String> = Spec::list(Word).name_words();
    /// Prefixes joined to a number that state an episode of season 0 (T12).
    special_prefixes: BTreeSet<String> = Spec::list(Word);
    /// Animation special markers and the kind of each (X20).
    animation_special_words: BTreeMap<String, SpecialKind> = Spec::map(Word).name_words();
    video_extensions: BTreeSet<String> = Spec::list(Extension);
    audio_extensions: BTreeSet<String> = Spec::list(Extension);
    subtitle_extensions: BTreeSet<String> = Spec::list(Extension);
    image_extensions: BTreeSet<String> = Spec::list(Extension);
    system_junk_names: BTreeSet<String> = Spec::list(Word);
    system_junk_file_suffixes: BTreeSet<String> = Spec::list(Word);
    partial_download_suffixes: BTreeSet<String> = Spec::list(Word);
    generated_metadata_folders: BTreeSet<String> = Spec::list(Word);
    generated_metadata_folder_suffixes: BTreeSet<String> = Spec::list(Word);
    leading_articles: BTreeSet<String> = Spec::list(TitleKey).name_words();
    /// Number words (Roman numerals, spelled numbers) and the number that each reads as.
    number_words: BTreeMap<String, u32> = Spec::map(TitleKey).name_words();
    /// Endings that make a number an ordinal before a season word.
    ordinal_suffixes: BTreeSet<String> = Spec::list(Word);
    presets: Presets = Spec::new(Word, Shape::Settings);
    /// The built-in exclusion groups in force (X13).
    built_in_ignore_groups: BTreeSet<String> =
        Spec::list(Word).closed(crate::data::BUILT_IN_IGNORE_GROUPS);
    /// Marker file names that exclude their folder (X14).
    no_media_markers: BTreeSet<String> = Spec::list(Word);
    release_junk_names: BTreeSet<String> = Spec::list(Word);
    release_junk_prefixes: BTreeSet<String> = Spec::list(Word);
    release_junk_extensions: BTreeSet<String> = Spec::list(Extension);
    release_junk_images: BTreeSet<String> = Spec::list(Word);
    /// Release word spellings after a title (X19), with the fact and the normalized value
    /// that each states.
    release_words: BTreeMap<String, (ReleaseFact, String)> =
        Spec::new(ReleaseSpelling, Shape::ReleaseSpellings).check(release_value);
    /// Audio channel words and the channel layout that each states (`6ch` is `5.1`).
    audio_channel_words: BTreeMap<String, String> = Spec::map(Word).check(channel_layout);
    /// Bracketed tags that name an index site, not a release group.
    release_site_tags: BTreeSet<String> = Spec::list(Word);
    /// Words that name a part before its number or letter (M11).
    part_words: BTreeSet<String> = Spec::list(Word).name_words();
    /// Words that name a folder of episodes before its number (T18).
    disc_folder_words: BTreeSet<String> = Spec::list(Word).name_words();
    format_3d_words: BTreeMap<String, Format3d> = Spec::map(Word).name_words();
    /// Endings that mark a leading bracket as a release group (M5).
    release_group_endings: BTreeSet<String> = Spec::list(Word);
    video_heights: BTreeSet<String> = Spec::list(Word).check(video_height);
    /// Letters and the spelling that each compares as (P4).
    letter_folds: BTreeMap<char, String> = Spec::map(Word).check(letter_fold);
    disc_image_extensions: BTreeSet<String> = Spec::list(Extension);
    unsupported_video_extensions: BTreeSet<String> = Spec::list(Extension);
    disc_structure_folders: BTreeSet<String> = Spec::list(Word);
    nfo_extensions: BTreeSet<String> = Spec::list(Extension);
    /// Suffixes that mark an image as the still of the video that it is named after (S6).
    still_image_suffixes: BTreeSet<String> = Spec::list(Word);
    /// Words that mean hearing impaired beside another language, and otherwise the language
    /// of each mapped BCP 47 tag (S4).
    hearing_impaired_or_language_words: BTreeMap<String, String> =
        Spec::map(Word).check(language_tag);
    /// Folder names that hold the subtitles of the videos beside them (S2).
    subtitle_folders: BTreeSet<String> = Spec::list(Word);
    /// Aspect ratios, never a season and episode (T16).
    aspect_ratios: BTreeSet<String> = Spec::list(Word);
    /// Audio codec words that a channel layout may follow (T16).
    audio_codec_words: BTreeSet<String> = Spec::list(Word);
    /// Words after an episode number that mark the last episode of a release run (X20).
    final_episode_words: BTreeSet<String> = Spec::list(Word).name_words();
    /// Words between separators that start another title of a movie (M16).
    alternate_title_words: BTreeSet<String> = Spec::list(Word).name_words();
    /// Text that marks a release word spelling as a remux (X19).
    remux_markers: BTreeSet<String> = Spec::list(Word);
    /// Text that marks a source spelling as a screener (X19).
    screener_markers: BTreeSet<String> = Spec::list(Word);
}

/// Accepts any value.
fn unchecked(_: &str, value: Value) -> Result<Value, String> {
    Ok(value)
}

/// Stores a language tag in vocabulary form.
fn language_tag(_: &str, value: Value) -> Result<Value, String> {
    Ok(match value {
        Value::String(tag) => Value::String(lexical::vocabulary_form(tag.trim())),
        other => other,
    })
}

/// Checks a tag key and the ID shape of a provider tag (X29).
fn tag(key: &str, value: Value) -> Result<Value, String> {
    let is_key = key.starts_with(|c: char| c.is_ascii_alphabetic())
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !is_key {
        return Err(format!("tag key `{key}` is not letters, digits and `_`"));
    }
    if let Ok(TagFact::ProviderId {
        id:
            IdShape::Prefixed {
                prefix,
                min_digits,
                max_digits,
            },
    }) = TagFact::deserialize(&value)
        && (prefix.is_empty() || min_digits == 0 || min_digits > max_digits)
    {
        return Err(format!("tag `{key}` has an invalid ID shape"));
    }
    Ok(value)
}

/// Checks that `text` is an episode token template: it uses `episode` once, may use
/// `season` once, and has no optional section (X34).
fn episode_token(text: &str, value: Value) -> Result<Value, String> {
    let template = Template::parse(text.trim())
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
    Ok(value)
}

/// Checks that a separator is one character other than a letter, a digit, `-` or `/`.
fn separator(text: &str, value: Value) -> Result<Value, String> {
    match char::read(text) {
        Some(character) if !character.is_alphanumeric() && !matches!(character, '-' | '/') => {
            Ok(value)
        }
        _ => Err(format!(
            "separator `{text}` is not one character other than a letter, a digit, `-` or `/`"
        )),
    }
}

/// Checks that a bracket pair is one character each.
fn bracket_pair(open: &str, value: Value) -> Result<Value, String> {
    let close = value.as_str().map(str::trim).unwrap_or_default();
    match (char::read(open.trim()), char::read(close)) {
        (Some(_), Some(_)) => Ok(Value::String(close.to_owned())),
        _ => Err(format!(
            "bracket pair `{open}` `{close}` is not one character each"
        )),
    }
}

/// Checks that a video height is three or four digits.
fn video_height(text: &str, value: Value) -> Result<Value, String> {
    let height = text.trim();
    if (3..=4).contains(&height.len()) && height.bytes().all(|byte| byte.is_ascii_digit()) {
        Ok(value)
    } else {
        Err(format!("video height `{text}` is not 3 or 4 digits"))
    }
}

/// Checks that a letter fold is one letter with a spelling, and stores the spelling in
/// vocabulary form.
fn letter_fold(letter: &str, value: Value) -> Result<Value, String> {
    let spelled = lexical::vocabulary_form(value.as_str().unwrap_or_default().trim());
    if char::read(letter.trim()).is_none() || spelled.is_empty() {
        return Err(format!(
            "letter fold `{letter}` is not one letter with a spelling"
        ));
    }
    Ok(Value::String(spelled))
}

/// Checks that a channel layout is a digit from 1 to 9, a dot and a digit from 0 to 2
/// (`5.1`).
fn channel_layout(_: &str, value: Value) -> Result<Value, String> {
    let layout = value.as_str().unwrap_or_default().trim().to_owned();
    let bytes = layout.as_bytes();
    let is_layout = bytes.len() == 3
        && (b'1'..=b'9').contains(&bytes[0])
        && bytes[1] == b'.'
        && (b'0'..=b'2').contains(&bytes[2]);
    if is_layout {
        Ok(Value::String(layout))
    } else {
        Err(format!(
            "channel layout `{layout}` is not a layout such as `5.1`"
        ))
    }
}

/// Stores a release value case-folded, and checks that a bit depth is a number.
fn release_value(_: &str, value: Value) -> Result<Value, String> {
    let Value::Array(pair) = value else {
        return Ok(value);
    };
    let (Some(fact), Some(text)) = (pair.first(), pair.get(1).and_then(Value::as_str)) else {
        return Ok(Value::Array(pair));
    };
    let text = lexical::fold_case_text(text.trim());
    if ReleaseFact::deserialize(fact).ok() == Some(ReleaseFact::BitDepth)
        && text.parse::<u8>().is_err()
    {
        return Err(format!("bit depth `{text}` is not a number"));
    }
    Ok(Value::Array(vec![fact.clone(), Value::String(text)]))
}
