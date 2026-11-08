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
use crate::model::{ExtraKind, SidecarFlag};
use crate::rendering::{OutputProfile, TemplateSlot, literal_words};
use crate::rules::vocabulary::{Shape, Vocabularies, Vocabulary};
use crate::rules::{PresetError, Presets, Rules, RulesIdentity, TagFact};

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
/// drops every other entry of the vocabulary except its canonical names. A word list adds a
/// list. A typed vocabulary adds a map from entry to value: `extras_folders`,
/// `extras_suffixes`, `extras_video_names` and `extras_audio_names` to an extra kind,
/// `artwork_roles` to an artwork role, `nfo_names` to an NFO role, `audio_channel_words` to
/// a channel layout, `tags` to a fact, `bracket_pairs` to a closing character,
/// `format_3d_words` to a 3D layout, `letter_folds` to a spelling,
/// `hearing_impaired_or_language_words` to a BCP 47 tag, `animation_special_words` to a
/// special kind and `number_words` to a number. `languages` maps a BCP 47 tag to its
/// spellings, and `release_words` maps a release fact to a value to its spellings. `remove`
/// is always a list of entries. `presets` maps each preset to its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    name: String,
    version: Option<String>,
    changes: Vec<Change>,
    /// The vocabularies that the overlay replaces.
    replaced: BTreeSet<Vocabulary>,
    /// The line each change's entry is written on, for diagnostics.
    lines: BTreeMap<(Vocabulary, String), usize>,
}

/// The part of a [`RulesIdentity`] that one overlay contributes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OverlayIdentity {
    /// The name of the overlay.
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

/// One change to one entry of one vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Change {
    vocabulary: Vocabulary,
    entry: String,
    action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Action {
    Remove,
    /// Adds the entry with its value, written as JSON text.
    Add(String),
}

impl Change {
    /// Returns the addition of `raw` with `value`, with the entry in the vocabulary's form.
    fn add(vocabulary: Vocabulary, raw: &str, value: &Value) -> Change {
        Change {
            vocabulary,
            entry: vocabulary.spec().form.normalize(raw),
            action: Action::Add(value.to_string()),
        }
    }

    fn is_add(&self) -> bool {
        matches!(self.action, Action::Add(_))
    }

    /// Returns the line that the digest of an overlay reads for this change.
    fn describe(&self) -> String {
        let action = match &self.action {
            Action::Remove => "remove".to_owned(),
            Action::Add(value) => format!("add {value}"),
        };
        format!("{}\t{action}\t{}\n", self.vocabulary.name(), self.entry)
    }
}

impl Vocabularies {
    /// Applies one change.
    pub(crate) fn apply(&mut self, change: &Change) {
        let table = self.table_mut(change.vocabulary);
        match &change.action {
            Action::Add(value) => {
                let value = serde_json::from_str(value).unwrap_or(Value::Null);
                table.insert(&change.entry, &value);
            }
            Action::Remove => table.remove(&change.entry),
        }
    }

    /// Removes every entry of `vocabulary`.
    fn clear(&mut self, vocabulary: Vocabulary) {
        let table = self.table_mut(vocabulary);
        for entry in table.entries() {
            table.remove(&entry);
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
        let mut diagnostics = Vec::new();
        if name.is_none() {
            diagnostics.push(diagnostic(OverlayDiagnosticReason::MissingName));
        }
        let mut overlay = Overlay {
            name: name.clone().unwrap_or_default(),
            version: scalar("version"),
            changes: Vec::new(),
            replaced: BTreeSet::new(),
            lines: BTreeMap::new(),
        };
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
            let Some(vocabulary) = Vocabulary::named(key) else {
                diagnostics.push(at(OverlayDiagnosticReason::UnknownVocabulary));
                continue;
            };
            let (replaces, read) = match read_changes(vocabulary, value) {
                Ok(read) => read,
                Err(message) => {
                    diagnostics.push(OverlayDiagnostic {
                        message: Some(message),
                        ..at(OverlayDiagnosticReason::MalformedVocabulary)
                    });
                    continue;
                }
            };
            if replaces {
                overlay.replaced.insert(vocabulary);
            }
            let mut entry_lines = EntryLines::new(text, line_of_key(text, key));
            for (raw, change) in read {
                let line = entry_lines.find(&raw);
                let at_entry = |reason| OverlayDiagnostic {
                    line,
                    entry: Some(change.entry.clone()),
                    ..at(reason)
                };
                if let Some(reason) = entry_fault(&change) {
                    diagnostics.push(at_entry(reason));
                } else if !seen.insert((vocabulary, change.entry.clone())) {
                    diagnostics.push(at_entry(OverlayDiagnosticReason::DuplicateEntry));
                } else {
                    if let Some(line) = line {
                        overlay
                            .lines
                            .insert((vocabulary, change.entry.clone()), line);
                    }
                    overlay.changes.push(change);
                }
            }
        }
        if name.is_none() || !diagnostics.is_empty() {
            // The document's keys are read in sorted order, so the diagnostics are put back
            // in the order of their lines.
            diagnostics.sort_by_key(|diagnostic| diagnostic.line.unwrap_or(0));
            return Err(diagnostics);
        }
        overlay.changes.sort();
        Ok(overlay)
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
        // FNV-1a over the replaced vocabularies and the sorted changes: stable across
        // builds and platforms.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let replaced = self
            .replaced
            .iter()
            .map(|vocabulary| format!("{}\treplace\n", vocabulary.name()));
        for line in replaced.chain(self.changes.iter().map(Change::describe)) {
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

    /// Returns a diagnostic about `change` of this overlay, on the line of its entry.
    fn diagnostic(&self, change: &Change, reason: OverlayDiagnosticReason) -> OverlayDiagnostic {
        OverlayDiagnostic {
            overlay: Some(self.name.clone()),
            line: self
                .lines
                .get(&(change.vocabulary, change.entry.clone()))
                .copied(),
            vocabulary: Some(change.vocabulary.name().to_owned()),
            entry: Some(change.entry.clone()),
            ..diagnostic(reason)
        }
    }
}

/// Returns the fault of one change on its own: an empty entry, an entry outside a closed
/// vocabulary, a preset that is unknown or set to a value outside its alternatives, or a
/// name word that the grammar already reads.
/// Returns `None` if the change has no such fault.
fn entry_fault(change: &Change) -> Option<OverlayDiagnosticReason> {
    let spec = change.vocabulary.spec();
    if change.entry.is_empty() {
        return Some(OverlayDiagnosticReason::EmptyEntry);
    }
    if spec
        .closed
        .is_some_and(|entries| !entries.contains(&change.entry.as_str()))
    {
        return Some(OverlayDiagnosticReason::UnknownEntry);
    }
    let Action::Add(value) = &change.action else {
        return None;
    };
    if spec.shape == Shape::Settings {
        let value: String = serde_json::from_str(value).unwrap_or_default();
        return match Presets::default().set(&change.entry, &value) {
            Ok(()) => None,
            Err(PresetError::UnknownPreset) => Some(OverlayDiagnosticReason::UnknownPreset),
            Err(PresetError::InvalidValue) => Some(OverlayDiagnosticReason::InvalidPresetValue),
        };
    }
    (spec.name_words && change.entry.split(' ').any(lexical::is_reserved_word))
        .then_some(OverlayDiagnosticReason::ReservedToken)
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

/// Reads the changes of one vocabulary's section, each with the raw text of its entry.
/// Returns `true` with them if the section replaces the vocabulary.
/// Fails if the section does not have the shape of the vocabulary, or if a value is not a
/// value of the vocabulary.
fn read_changes(
    vocabulary: Vocabulary,
    section: &Value,
) -> Result<(bool, Vec<(String, Change)>), String> {
    let spec = vocabulary.spec();
    if spec.shape == Shape::Settings {
        let settings: BTreeMap<String, Value> = read(section)?;
        let changes = settings
            .into_iter()
            .map(|(entry, value)| {
                let value = match value {
                    Value::String(text) => fold_case_text(text.trim()),
                    Value::Number(number) => number.to_string(),
                    _ => String::new(),
                };
                let change = Change::add(vocabulary, &entry, &Value::String(value));
                (entry, change)
            })
            .collect();
        return Ok((false, changes));
    }
    let Value::Object(fields) = section else {
        return Err("expected a mapping with `add`, `remove` or `replace`".to_owned());
    };
    if fields.contains_key("replace") && fields.len() > 1 {
        return Err("expected `replace` without `add` and `remove`".to_owned());
    }
    let empty = Vocabularies::default();
    let mut changes = Vec::new();
    for (key, value) in fields {
        match key.as_str() {
            "add" | "replace" => {
                for (raw, value) in read_additions(spec.shape, value)? {
                    let value = (spec.check)(&raw, value)?;
                    empty.table(vocabulary).read_value(&value)?;
                    let change = Change::add(vocabulary, &raw, &value);
                    changes.push((raw, change));
                }
            }
            "remove" => {
                for raw in read::<Vec<String>>(value)? {
                    let change = Change {
                        vocabulary,
                        entry: spec.form.normalize(&raw),
                        action: Action::Remove,
                    };
                    changes.push((raw, change));
                }
            }
            _ => return Err("expected only `add`, `remove` or `replace`".to_owned()),
        }
    }
    Ok((fields.contains_key("replace"), changes))
}

/// Reads the raw entries of an `add` section, each with its value, as the section's shape
/// lays them out.
fn read_additions(shape: Shape, section: &Value) -> Result<Vec<(String, Value)>, String> {
    Ok(match shape {
        Shape::List => read::<Vec<String>>(section)?
            .into_iter()
            .map(|entry| (entry, Value::Null))
            .collect(),
        Shape::Map | Shape::Settings => read::<BTreeMap<String, Value>>(section)?
            .into_iter()
            .collect(),
        Shape::Spellings => read::<BTreeMap<String, Vec<String>>>(section)?
            .into_iter()
            .flat_map(|(value, spellings)| {
                let tag = Value::String(value.clone());
                std::iter::once(value)
                    .chain(spellings)
                    .map(move |spelling| (spelling, tag.clone()))
            })
            .collect(),
        Shape::ReleaseSpellings => {
            let mut additions = Vec::new();
            let facts: BTreeMap<String, BTreeMap<String, Vec<String>>> = read(section)?;
            for (fact, values) in facts {
                for (value, spellings) in values {
                    let pair = Value::Array(vec![
                        Value::String(fact.clone()),
                        Value::String(value.clone()),
                    ]);
                    additions.extend(
                        spellings
                            .into_iter()
                            .map(|spelling| (spelling, pair.clone())),
                    );
                }
            }
            additions
        }
    })
}

fn read<T: DeserializeOwned>(value: &Value) -> Result<T, String> {
    T::deserialize(value).map_err(|error| error.to_string())
}

/// Returns the canonical names (X25) as additions to the input vocabularies that read
/// them.
fn canonical_changes() -> Vec<Change> {
    fn value(fact: impl Serialize) -> Value {
        serde_json::to_value(fact).expect("a canonical value serializes")
    }
    let word = |vocabulary, word: &str| Change::add(vocabulary, word, &Value::Null);
    let mut changes: Vec<Change> = data::ARTWORK_OUTPUT_WORDS
        .iter()
        .map(|(role, word)| Change::add(Vocabulary::artwork_roles, word, &value(role)))
        .collect();
    changes.extend(
        data::NFO_OUTPUT_NAMES
            .iter()
            .map(|(role, name)| Change::add(Vocabulary::nfo_names, name, &value(role))),
    );
    changes.extend(
        data::EXTRAS_OUTPUT_FOLDERS
            .iter()
            .map(|(kind, folder)| Change::add(Vocabulary::extras_folders, folder, &value(kind))),
    );
    changes.push(Change::add(
        Vocabulary::extras_audio_names,
        data::THEME_OUTPUT_STEM,
        &value(ExtraKind::ThemeSong),
    ));
    changes.push(Change::add(
        Vocabulary::tags,
        data::EDITION_TAG_KEY,
        &value(TagFact::Edition),
    ));
    changes.extend(data::CANONICAL_BRACKET_PAIRS.iter().map(|(open, close)| {
        Change::add(Vocabulary::bracket_pairs, &open.to_string(), &value(close))
    }));
    changes.push(word(Vocabulary::season_words, data::SEASON_FOLDER_WORD));
    changes.push(word(
        Vocabulary::separators,
        &data::SPACE_SEPARATOR.to_string(),
    ));
    changes.push(word(Vocabulary::episode_words, data::EPISODE_OUTPUT_LETTER));
    changes.push(word(Vocabulary::part_words, data::PART_OUTPUT_WORD));
    changes.push(word(Vocabulary::nfo_extensions, data::NFO_EXTENSION));
    changes.push(word(
        Vocabulary::still_image_suffixes,
        data::STILL_IMAGE_SUFFIX,
    ));
    changes.extend(
        data::SIDECAR_FLAG_OUTPUT_WORDS
            .iter()
            .map(|(flag, flag_word)| {
                let vocabulary = match flag {
                    SidecarFlag::Default => Vocabulary::default_flag_words,
                    SidecarFlag::Forced => Vocabulary::forced_flag_words,
                    SidecarFlag::HearingImpaired => Vocabulary::hearing_impaired_flag_words,
                };
                word(vocabulary, flag_word)
            }),
    );
    changes
}

impl Rules {
    /// Returns the built-in rules: the built-in document, then the canonical names.
    pub(crate) fn from_builtin_document() -> Rules {
        let builtin =
            Overlay::parse(data::BUILTIN).expect("the built-in document is a valid overlay");
        let mut rules = Rules::empty();
        for change in &builtin.changes {
            rules.vocab.apply(change);
        }
        rules.add_canonical_names();
        rules.recompile();
        rules
    }

    /// Adds the canonical names (X25) to the input vocabularies. Called whenever the
    /// vocabularies are built, so no other change can drop a canonical name.
    fn add_canonical_names(&mut self) {
        for change in canonical_changes() {
            self.vocab.apply(&change);
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
                .filter(|change| change.is_add())
                .map(|change| (change.vocabulary, change.entry.as_str()))
                .collect();
            for (_, pack) in &others {
                for change in &pack.changes {
                    if !kept.contains(&(change.vocabulary, change.entry.as_str())) {
                        base.vocab
                            .table_mut(change.vocabulary)
                            .remove(&change.entry);
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
                    diagnostics
                        .push(overlay.diagnostic(change, OverlayDiagnosticReason::CanonicalEntry));
                }
            }
        }
        // Language packs are the first layer and user overlays the second (X32), so an
        // overlay can change a word of a pack. Only overlays of one layer can conflict.
        let (packs, users) = overlays.split_at(overlays.len() - user_overlays.len());
        // Every user overlay's action on each entry, in declared order.
        let mut actions: BTreeMap<(Vocabulary, &str), Vec<(&str, &Action)>> = BTreeMap::new();
        let mut replacing: BTreeMap<Vocabulary, Vec<&str>> = BTreeMap::new();
        for overlay in users {
            for change in &overlay.changes {
                actions
                    .entry((change.vocabulary, change.entry.as_str()))
                    .or_default()
                    .push((overlay.name.as_str(), &change.action));
            }
            for vocabulary in &overlay.replaced {
                replacing
                    .entry(*vocabulary)
                    .or_default()
                    .push(&overlay.name);
            }
        }
        let conflict = |vocabulary: Vocabulary, entry: Option<&str>, names: &[&str]| {
            let mut overlays: Vec<String> = Vec::new();
            for name in names {
                if !overlays.iter().any(|known| known == name) {
                    overlays.push((*name).to_owned());
                }
            }
            OverlayDiagnostic {
                vocabulary: Some(vocabulary.name().to_owned()),
                entry: entry.map(str::to_owned),
                overlays,
                ..diagnostic(OverlayDiagnosticReason::ConflictingChange)
            }
        };
        for (vocabulary, names) in &replacing {
            if names.len() > 1 {
                diagnostics.push(conflict(*vocabulary, None, names));
            }
        }
        for ((vocabulary, entry), entry_actions) in &actions {
            let (_, first) = entry_actions[0];
            if entry_actions.iter().any(|(_, action)| *action != first) {
                let names: Vec<&str> = entry_actions.iter().map(|(name, _)| *name).collect();
                diagnostics.push(conflict(*vocabulary, Some(entry), &names));
            }
        }
        let pack_entries: BTreeSet<(Vocabulary, &str)> = packs
            .iter()
            .flat_map(|pack| &pack.changes)
            .map(|change| (change.vocabulary, change.entry.as_str()))
            .collect();
        for overlay in user_overlays {
            for change in &overlay.changes {
                let key = (change.vocabulary, change.entry.as_str());
                let is_added = actions[&key]
                    .iter()
                    .any(|(_, action)| matches!(action, Action::Add(_)))
                    || pack_entries.contains(&key);
                let is_builtin = Rules::builtin()
                    .vocab
                    .table(change.vocabulary)
                    .contains(&change.entry);
                if change.action == Action::Remove && !is_added && !is_builtin {
                    diagnostics.push(
                        overlay.diagnostic(change, OverlayDiagnosticReason::IneffectiveRemoval),
                    );
                }
            }
        }
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }
        let mut rules = base;
        for change in packs.iter().flat_map(|pack| &pack.changes) {
            rules.vocab.apply(change);
        }
        for vocabulary in replacing.keys() {
            rules.vocab.clear(*vocabulary);
        }
        for ((vocabulary, entry), entry_actions) in &actions {
            rules.vocab.apply(&Change {
                vocabulary: *vocabulary,
                entry: (*entry).to_owned(),
                action: entry_actions[0].1.clone(),
            });
        }
        let collisions = rules.collisions(users);
        if !collisions.is_empty() {
            return Err(collisions);
        }
        // A language pack that is not declared, or a replace, may have removed a canonical
        // name.
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
    /// extras folder names, its specials folder name, and the literal words of its
    /// `season_folder`, `part` and `edition_tag` slots, in the form that the writer writes
    /// them (X25, X33). The identity lists the additions as an overlay named `profile`.
    /// Returns a copy of these rules if the profile writes no name of its own.
    /// Fails if a name is in a rival vocabulary.
    pub fn reading_profile(
        &self,
        profile: &OutputProfile,
    ) -> Result<Rules, Vec<OverlayDiagnostic>> {
        let word = |vocabulary, text: &str| Change::add(vocabulary, text, &Value::Null);
        let mut changes: Vec<Change> = profile
            .extras_folders
            .iter()
            .map(|(kind, folder)| {
                let kind = serde_json::to_value(kind).expect("an extra kind serializes");
                Change::add(
                    Vocabulary::extras_folders,
                    &profile.clean_fact(folder),
                    &kind,
                )
            })
            .collect();
        changes.extend(
            profile
                .specials_folder
                .iter()
                .map(|folder| word(Vocabulary::specials_folders, &profile.clean_fact(folder))),
        );
        let slot_words = [
            (TemplateSlot::SeasonFolder, Vocabulary::season_words),
            (TemplateSlot::Part, Vocabulary::part_words),
        ];
        for (slot, vocabulary) in slot_words {
            let words = profile.templates.get(&slot).map(literal_words);
            changes.extend(
                words
                    .into_iter()
                    .flatten()
                    .map(|text| word(vocabulary, &text)),
            );
        }
        let edition_keys = profile
            .templates
            .get(&TemplateSlot::EditionTag)
            .map(literal_words);
        let edition = serde_json::to_value(TagFact::Edition).expect("a tag fact serializes");
        changes.extend(
            edition_keys
                .into_iter()
                .flatten()
                .map(|key| Change::add(Vocabulary::tags, &key, &edition)),
        );
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
            replaced: BTreeSet::new(),
            lines: BTreeMap::new(),
        };
        let mut rules = self.clone();
        for change in &overlay.changes {
            rules.vocab.apply(change);
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
            for change in overlay.changes.iter().filter(|change| change.is_add()) {
                let Some(rival) = change.vocabulary.spec().rival else {
                    continue;
                };
                let others: Vec<&str> = Vocabulary::ALL
                    .iter()
                    .filter(|other| {
                        **other != change.vocabulary
                            && other.spec().rival == Some(rival)
                            && self.vocab.table(**other).contains(&change.entry)
                    })
                    .map(|other| other.name())
                    .collect();
                if !others.is_empty() {
                    diagnostics.push(OverlayDiagnostic {
                        message: Some(format!("also in {}", others.join(", "))),
                        ..overlay.diagnostic(change, OverlayDiagnosticReason::CollidingEntry)
                    });
                }
            }
        }
        diagnostics
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
        let builtin = &Rules::builtin().vocab;
        let language_vocabularies = [
            Vocabulary::season_words,
            Vocabulary::series_season_words,
            Vocabulary::episode_words,
            Vocabulary::leading_articles,
            Vocabulary::ordinal_suffixes,
        ];
        for vocabulary in language_vocabularies {
            for word in builtin.table(vocabulary).entries() {
                assert!(
                    shared_base.contains(&word) || in_packs(vocabulary, &word),
                    "{} `{word}` is in no language pack",
                    vocabulary.name()
                );
            }
        }
    }

    #[test]
    fn every_language_pack_is_named_after_its_tag_and_adds_only() {
        for (tag, pack) in language_packs() {
            assert_eq!(pack.name, format!("language-{tag}"));
            assert!(pack.changes.iter().all(Change::is_add));
        }
    }

    #[test]
    fn declaring_every_language_keeps_every_built_in_word() {
        let all: Vec<String> = Rules::language_packs().map(str::to_owned).collect();
        let rules = Rules::for_languages(&all, &[]).expect("the packs agree with each other");
        for vocabulary in Vocabulary::ALL {
            let declared = rules.vocab.table(*vocabulary);
            for word in Rules::builtin().vocab.table(*vocabulary).entries() {
                assert!(declared.contains(&word), "{} `{word}`", vocabulary.name());
            }
        }
    }
}
