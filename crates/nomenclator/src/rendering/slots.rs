//! Provides the template slots of an output profile (X33) and the check that the reader
//! reads back the names that a profile writes.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize};

use crate::interpret::Item;
use crate::model::Finding;
use crate::parsers::extras::LibraryType;
use crate::rendering::template::Template;
use crate::rendering::{NamingFacts, OutputProfile, ProviderTagStyle};
use crate::rules::{IdShape, Rules, TagFact};

/// A part of a written name that a profile can spell with a template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TemplateSlot {
    /// The name of a movie or a series. Fields: `title` (required), `year`.
    WorkName,
    /// The folder of a season. Field: `season` (required).
    SeasonFolder,
    /// The stem of an episode before its version and part. Fields: `reference` (required),
    /// `work_name`, `episode_title`.
    EpisodeFile,
    /// The provider ID in a work's folder. Fields: `provider` and `id` (both required).
    ProviderTag,
    /// The edition of a movie. Field: `edition` (required).
    EditionTag,
    /// The text after an identity that states a version, with its separator. Field: `label`
    /// (required).
    VersionLabel,
    /// The text at the end of a stem that states a part, with its separator. Field: `part`
    /// (required).
    Part,
}

impl TemplateSlot {
    /// Returns the required fields and the optional fields of the slot.
    fn fields(self) -> (&'static [&'static str], &'static [&'static str]) {
        match self {
            TemplateSlot::WorkName => (&["title"], &["year"]),
            TemplateSlot::SeasonFolder => (&["season"], &[]),
            TemplateSlot::EpisodeFile => (&["reference"], &["work_name", "episode_title"]),
            TemplateSlot::ProviderTag => (&["provider", "id"], &[]),
            TemplateSlot::EditionTag => (&["edition"], &[]),
            TemplateSlot::VersionLabel => (&["label"], &[]),
            TemplateSlot::Part => (&["part"], &[]),
        }
    }

    /// Checks that `template` uses only the fields of the slot and every required field
    /// outside an optional section.
    /// Fails with a description of the first fault.
    fn check(self, template: &Template) -> Result<(), String> {
        let (required, optional) = self.fields();
        if let Some(field) = template
            .fields()
            .into_iter()
            .find(|field| !required.contains(field) && !optional.contains(field))
        {
            return Err(format!(
                "`{}` uses the field `{field}`, which the slot does not have",
                template.source()
            ));
        }
        let used = template.required_fields();
        if let Some(field) = required.iter().find(|field| !used.contains(*field)) {
            return Err(format!(
                "`{}` does not use the required field `{field}`",
                template.source()
            ));
        }
        Ok(())
    }
}

/// Reads the `templates` of a profile, and fails for a template that is not valid for its
/// slot.
pub(crate) fn read_templates<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<TemplateSlot, Template>, D::Error> {
    let texts = BTreeMap::<TemplateSlot, String>::deserialize(deserializer)?;
    let mut templates = BTreeMap::new();
    for (slot, text) in texts {
        let template = Template::parse(&text).map_err(|error| {
            serde::de::Error::custom(format!(
                "template `{text}`: {:?} at offset {}",
                error.reason, error.offset
            ))
        })?;
        slot.check(&template).map_err(serde::de::Error::custom)?;
        templates.insert(slot, template);
    }
    Ok(templates)
}

/// A problem with the templates of an output profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfileDiagnostic {
    /// The kind of problem.
    pub reason: ProfileDiagnosticReason,
    /// The slot concerned, when the fact has one that the profile sets.
    pub slot: Option<TemplateSlot>,
    /// The fact that was not read back.
    pub fact: Option<String>,
    /// A description of the problem.
    pub message: Option<String>,
}

/// Writes the reason, the slot, the fact and the message, each that the diagnostic
/// states, joined by `: `.
impl std::fmt::Display for ProfileDiagnostic {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = vec![crate::model::serde_name(self.reason)];
        parts.extend(
            self.slot
                .iter()
                .map(|slot| format!("slot {}", crate::model::serde_name(slot))),
        );
        parts.extend(self.fact.iter().map(|fact| format!("fact {fact}")));
        parts.extend(self.message.clone());
        formatter.write_str(&parts.join(": "))
    }
}

/// The reason that a profile's templates are rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProfileDiagnosticReason {
    /// A template uses a field that its slot does not have, or leaves out a required one.
    InvalidTemplate,
    /// A name of the profile is in a rival vocabulary of the rules.
    RivalName,
    /// The reader does not return a fact from a name that the profile writes.
    DoesNotReadBack,
}

/// The facts that [`Rules::check_profile`] writes and reads back.
const SAMPLE_TITLE: &str = "Northern Passage";
const SAMPLE_YEAR: u16 = 2021;
const SAMPLE_EDITION: &str = "Final Cut";
const SAMPLE_EPISODE_TITLE: &str = "Pilot";
const SAMPLE_SEASON: u16 = 2;
const SAMPLE_EPISODE: u32 = 3;

impl Rules {
    /// Writes one movie and one series under `profile`, reads the names back under
    /// [`Rules::reading_profile`], and reports each fact that does not come back. Returns no
    /// diagnostics for a profile without templates.
    #[must_use]
    pub fn check_profile(&self, profile: &OutputProfile) -> Vec<ProfileDiagnostic> {
        if profile.templates.is_empty() {
            return Vec::new();
        }
        let invalid: Vec<ProfileDiagnostic> = profile
            .templates
            .iter()
            .filter_map(|(slot, template)| {
                slot.check(template).err().map(|message| ProfileDiagnostic {
                    reason: ProfileDiagnosticReason::InvalidTemplate,
                    slot: Some(*slot),
                    fact: None,
                    message: Some(message),
                })
            })
            .collect();
        if !invalid.is_empty() {
            return invalid;
        }
        let reading = match self.reading_profile(profile) {
            Ok(reading) => reading,
            Err(found) => {
                return found
                    .into_iter()
                    .map(|diagnostic| ProfileDiagnostic {
                        reason: ProfileDiagnosticReason::RivalName,
                        slot: None,
                        fact: diagnostic.entry,
                        message: diagnostic.message,
                    })
                    .collect();
            }
        };
        let mut lost = BTreeMap::new();
        for (fact, slot) in reading
            .lost_movie_facts(profile)
            .into_iter()
            .chain(reading.lost_series_facts(profile))
        {
            lost.insert(fact, slot);
        }
        lost.into_iter()
            .map(|(fact, slot)| ProfileDiagnostic {
                reason: ProfileDiagnosticReason::DoesNotReadBack,
                slot: Some(slot).filter(|slot| profile.templates.contains_key(slot)),
                fact: Some(fact.to_owned()),
                message: None,
            })
            .collect()
    }

    fn sample_provider_ids(&self, order: &[String]) -> BTreeMap<String, String> {
        order
            .iter()
            .find_map(|provider| match self.vocab.tags.get(provider)? {
                TagFact::ProviderId { id } => Some((provider.clone(), sample_id(id))),
                TagFact::Edition => None,
            })
            .into_iter()
            .collect()
    }

    /// Returns the facts of a written movie that do not come back, each with its slot.
    fn lost_movie_facts(&self, profile: &OutputProfile) -> Vec<(&'static str, TemplateSlot)> {
        let provider_ids = self.sample_provider_ids(&profile.movie_provider_order);
        let write = |label: &str, part: Option<u32>| {
            let facts = NamingFacts {
                provider_ids: provider_ids.clone(),
                edition: Some(SAMPLE_EDITION.to_owned()),
                version_label: Some(label.to_owned()),
                part,
                ..sample_facts(LibraryType::Movies)
            };
            self.render_names(&facts, profile).path
        };
        let paths: Vec<String> = [
            write("2160p", Some(1)),
            write("2160p", Some(2)),
            write("1080p", None),
        ]
        .into_iter()
        .flatten()
        .collect();
        let interpretation = self.interpret(LibraryType::Movies, &paths, &[]);
        let movie = interpretation.items.iter().find_map(|item| match item {
            Item::Movie(movie) => Some(movie),
            _ => None,
        });
        let mut lost = Vec::new();
        let Some(movie) = movie.filter(|_| paths.len() == 3) else {
            return vec![("title", TemplateSlot::WorkName)];
        };
        if movie.title.as_deref() != Some(SAMPLE_TITLE) {
            lost.push(("title", TemplateSlot::WorkName));
        }
        if movie.year != Some(SAMPLE_YEAR) {
            lost.push(("year", TemplateSlot::WorkName));
        }
        if writes_provider_tag(profile) && !has_ids(&movie.provider_ids, &provider_ids) {
            lost.push(("provider_id", TemplateSlot::ProviderTag));
        }
        if !movie
            .versions
            .iter()
            .all(|version| version.edition.as_deref() == Some(SAMPLE_EDITION))
        {
            lost.push(("edition", TemplateSlot::EditionTag));
        }
        if !movie
            .versions
            .iter()
            .any(|version| version.paths.len() == 2)
        {
            lost.push(("part", TemplateSlot::Part));
        }
        let has_label = |label: &str| {
            movie.versions.iter().any(|version| {
                version
                    .label
                    .as_deref()
                    .is_some_and(|text| text.contains(label))
            })
        };
        if !has_label("2160p") || !has_label("1080p") {
            lost.push(("version_label", TemplateSlot::VersionLabel));
        }
        lost
    }

    /// Returns the facts of a written episode that do not come back, each with its slot.
    fn lost_series_facts(&self, profile: &OutputProfile) -> Vec<(&'static str, TemplateSlot)> {
        let provider_ids = self.sample_provider_ids(&profile.series_provider_order);
        let facts = NamingFacts {
            provider_ids: provider_ids.clone(),
            season: Some(SAMPLE_SEASON),
            episodes: vec![SAMPLE_EPISODE],
            episode_title: Some(SAMPLE_EPISODE_TITLE.to_owned()),
            ..sample_facts(LibraryType::Tv)
        };
        let paths: Vec<String> = self
            .render_names(&facts, profile)
            .path
            .into_iter()
            .collect();
        let interpretation = self.interpret(LibraryType::Tv, &paths, &[]);
        let mut lost = Vec::new();
        let series = interpretation.items.iter().find_map(|item| match item {
            Item::Series(series) => Some(series),
            _ => None,
        });
        match series {
            Some(series) => {
                if series.title.as_deref() != Some(SAMPLE_TITLE) {
                    lost.push(("title", TemplateSlot::WorkName));
                }
                if series.year != Some(SAMPLE_YEAR) {
                    lost.push(("year", TemplateSlot::WorkName));
                }
                if writes_provider_tag(profile) && !has_ids(&series.provider_ids, &provider_ids) {
                    lost.push(("provider_id", TemplateSlot::ProviderTag));
                }
            }
            None => lost.push(("title", TemplateSlot::WorkName)),
        }
        let episode = interpretation.items.iter().find_map(|item| match item {
            Item::Episode(episode) => Some(episode),
            _ => None,
        });
        match episode {
            Some(episode) => {
                if episode.season.resolved_value() != Some(&SAMPLE_SEASON) {
                    lost.push(("season", TemplateSlot::SeasonFolder));
                }
                if episode.episodes.as_deref() != Some(&[SAMPLE_EPISODE]) {
                    lost.push(("episode", TemplateSlot::EpisodeFile));
                }
                if episode.episode_title.as_deref() != Some(SAMPLE_EPISODE_TITLE) {
                    lost.push(("episode_title", TemplateSlot::EpisodeFile));
                }
            }
            None => lost.push(("episode", TemplateSlot::EpisodeFile)),
        }
        lost
    }
}

fn sample_id(shape: &IdShape) -> String {
    match shape {
        IdShape::Integer => "12345".to_owned(),
        IdShape::Slug => "sample-id".to_owned(),
        IdShape::Prefixed {
            prefix, min_digits, ..
        } => format!("{prefix}{}", "1".repeat(usize::from(*min_digits))),
    }
}

fn sample_facts(library: LibraryType) -> NamingFacts {
    NamingFacts {
        title: Some(SAMPLE_TITLE.to_owned()),
        year: Some(SAMPLE_YEAR),
        container: Some("mkv".to_owned()),
        ..NamingFacts::of(library)
    }
}

fn writes_provider_tag(profile: &OutputProfile) -> bool {
    profile.provider_tag_style != ProviderTagStyle::None
        || profile.templates.contains_key(&TemplateSlot::ProviderTag)
}

fn has_ids(found: &BTreeMap<String, Finding<String>>, written: &BTreeMap<String, String>) -> bool {
    written
        .iter()
        .all(|(provider, id)| found.get(provider).and_then(Finding::resolved_value) == Some(id))
}

/// Returns the distinct words that `template` writes as literal text, in template order.
pub(crate) fn literal_words(template: &Template) -> Vec<String> {
    let mut seen = BTreeSet::new();
    template
        .literal_text()
        .split(|character: char| !character.is_alphabetic())
        .filter(|word| !word.is_empty())
        .filter(|word| seen.insert((*word).to_owned()))
        .map(str::to_owned)
        .collect()
}
