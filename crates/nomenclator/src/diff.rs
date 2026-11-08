//! Compares the interpretations of one listing under two rule sets and reports the facts
//! that differ, per path (X15).

use crate::model::serde_name;
use std::collections::BTreeMap;
use std::fmt::Display;

use serde::Serialize;

use crate::ignore::IgnoreFile;
use crate::inputs::CallerInputs;
use crate::interpret::{Artwork, Extra, Interpretation, Item, SidecarFile, Version};
use crate::model::Finding;
use crate::parsers::extras::LibraryType;
use crate::rules::{Rules, RulesIdentity};

/// The facts that differ between two interpretations of one listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RulesDiff {
    /// The first rule set.
    pub a: RulesIdentity,
    /// The second rule set.
    pub b: RulesIdentity,
    /// Each path with a fact that differs, sorted by path.
    pub paths: Vec<PathDifference>,
}

/// The facts of one path that differ between the two interpretations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathDifference {
    /// The root-relative path.
    pub path: String,
    /// The differing facts, sorted by fact name.
    pub facts: Vec<FactDifference>,
}

/// A fact with its value under each rule set. A value is `None` if that rule set gives the
/// path no such fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FactDifference {
    /// The fact: `outcome` (`item`, `excluded`, `undecided`, `invalid`, `unclaimed`),
    /// `role`, `title`, `year`, `edition`, `edition_hint`, `version_label`, `series`,
    /// `series_year`, `season`, `episodes`, `absolute`, `date`, `episode_title`, `extra`,
    /// `owner`, `language`, `flags`, `artwork_role`, `reason` or `ambiguous`.
    pub fact: String,
    /// The value under the first rule set.
    pub a: Option<String>,
    /// The value under the second rule set.
    pub b: Option<String>,
}

/// Interprets the listing under `a` and under `b`, with the same caller `inputs`, and
/// returns the facts that differ.
pub fn diff_rules(
    a: &Rules,
    b: &Rules,
    library: LibraryType,
    paths: &[String],
    ignore_files: &[IgnoreFile],
    inputs: &CallerInputs,
) -> RulesDiff {
    let first = a.interpret_with(library, paths, ignore_files, inputs);
    let second = b.interpret_with(library, paths, ignore_files, inputs);
    RulesDiff {
        a: first.rules.clone(),
        b: second.rules.clone(),
        paths: diff_interpretations(&first, &second),
    }
}

/// Returns the facts that differ between two interpretations, per path.
#[must_use]
pub fn diff_interpretations(a: &Interpretation, b: &Interpretation) -> Vec<PathDifference> {
    let (first, second) = (facts_by_path(a), facts_by_path(b));
    let mut paths: Vec<&String> = first.keys().chain(second.keys()).collect();
    paths.sort();
    paths.dedup();
    let none = Facts::new();
    paths
        .into_iter()
        .filter_map(|path| {
            let (a_facts, b_facts) = (
                first.get(path).unwrap_or(&none),
                second.get(path).unwrap_or(&none),
            );
            let mut names: Vec<&&str> = a_facts.keys().chain(b_facts.keys()).collect();
            names.sort();
            names.dedup();
            let facts: Vec<FactDifference> = names
                .into_iter()
                .filter(|name| a_facts.get(**name) != b_facts.get(**name))
                .map(|name| FactDifference {
                    fact: (*name).to_owned(),
                    a: a_facts.get(*name).cloned(),
                    b: b_facts.get(*name).cloned(),
                })
                .collect();
            (!facts.is_empty()).then(|| PathDifference {
                path: path.clone(),
                facts,
            })
        })
        .collect()
}

type Facts = BTreeMap<&'static str, String>;

/// Returns every path that `interpretation` mentions, with the facts that it states about
/// the path.
fn facts_by_path(interpretation: &Interpretation) -> BTreeMap<String, Facts> {
    let mut by_path: BTreeMap<String, Facts> = BTreeMap::new();
    let mut add = |path: &str, facts: Facts| {
        by_path.entry(path.to_owned()).or_default().extend(facts);
    };
    for item in &interpretation.items {
        match item {
            Item::Movie(movie) => {
                let mut identity = Facts::new();
                set(&mut identity, "title", movie.title.as_ref());
                set(&mut identity, "year", movie.year);
                if movie.ambiguous {
                    identity.insert("ambiguous", reasons(&movie.ambiguity_reasons));
                }
                versions(&mut add, &movie.versions, "movie_video", &identity);
                let owner = movie.title.clone();
                extras(&mut add, &movie.extras, owner.as_ref());
                sidecars(&mut add, &movie.subtitles, "subtitle");
                sidecars(&mut add, &movie.audio, "audio");
                artwork(&mut add, &movie.artwork);
                nfo(&mut add, movie.nfo.as_ref());
            }
            Item::Series(series) => {
                extras(&mut add, &series.extras, series.title.as_ref());
                artwork(&mut add, &series.artwork);
                nfo(&mut add, series.nfo.as_ref());
            }
            Item::Season(season) => {
                let owner = format!(
                    "{} season {}",
                    season.series_name.as_deref().unwrap_or("?"),
                    season.season
                );
                extras(&mut add, &season.extras, Some(&owner));
                artwork(&mut add, &season.artwork);
                nfo(&mut add, season.nfo.as_ref());
            }
            Item::Episode(episode) => {
                let mut identity = Facts::new();
                set(&mut identity, "series", episode.series_name.as_ref());
                set(&mut identity, "series_year", episode.series_year);
                set(&mut identity, "season", finding(&episode.season));
                set(
                    &mut identity,
                    "episodes",
                    episode.episodes.as_deref().map(numbers),
                );
                set(
                    &mut identity,
                    "absolute",
                    episode.absolute.as_deref().map(numbers),
                );
                set(&mut identity, "date", finding(&episode.date));
                set(
                    &mut identity,
                    "episode_title",
                    episode.episode_title.as_ref(),
                );
                if episode.ambiguous {
                    identity.insert("ambiguous", reasons(&episode.ambiguity_reasons));
                }
                versions(&mut add, &episode.versions, "episode_video", &identity);
                let owner = episode.series_name.clone();
                extras(&mut add, &episode.extras, owner.as_ref());
                sidecars(&mut add, &episode.subtitles, "subtitle");
                sidecars(&mut add, &episode.audio, "audio");
                artwork(&mut add, &episode.artwork);
                nfo(&mut add, episode.nfo.as_ref());
            }
            Item::Extra(orphan) => {
                for path in &orphan.paths {
                    let mut facts = item_facts("extra");
                    facts.insert("extra", serde_name(orphan.extra));
                    set(&mut facts, "title", orphan.title.as_ref());
                    if orphan.ambiguous {
                        facts.insert("ambiguous", "owner".to_owned());
                    }
                    add(path, facts);
                }
            }
        }
    }
    let outcome = |outcome: &str, reason: Option<String>| {
        let mut facts = Facts::from([("outcome", outcome.to_owned())]);
        set(&mut facts, "reason", reason);
        facts
    };
    for entry in &interpretation.excluded_entries {
        add(
            &entry.path,
            outcome("excluded", Some(serde_name(entry.reason))),
        );
    }
    for entry in &interpretation.undecided_entries {
        add(&entry.path, outcome("undecided", None));
    }
    for entry in &interpretation.invalid_entries {
        add(
            &entry.path,
            outcome("invalid", Some(serde_name(entry.reason))),
        );
    }
    for path in &interpretation.unclaimed_files {
        add(path, outcome("unclaimed", None));
    }
    by_path
}

fn item_facts(role: &str) -> Facts {
    Facts::from([("outcome", "item".to_owned()), ("role", role.to_owned())])
}

fn versions(add: &mut impl FnMut(&str, Facts), versions: &[Version], role: &str, identity: &Facts) {
    for version in versions {
        for path in &version.paths {
            let mut facts = item_facts(role);
            facts.extend(identity.clone());
            set(&mut facts, "version_label", version.label.as_ref());
            set(&mut facts, "edition", version.edition.as_ref());
            set(&mut facts, "edition_hint", version.edition_hint.as_ref());
            add(path, facts);
        }
    }
}

fn extras(add: &mut impl FnMut(&str, Facts), extras: &[Extra], owner: Option<&String>) {
    for extra in extras {
        for path in &extra.paths {
            let mut facts = item_facts("extra");
            facts.insert("extra", serde_name(extra.extra));
            set(&mut facts, "title", extra.title.as_ref());
            set(&mut facts, "year", extra.year);
            set(&mut facts, "owner", owner);
            add(path, facts);
        }
    }
}

fn sidecars(add: &mut impl FnMut(&str, Facts), sidecars: &[SidecarFile], role: &str) {
    for sidecar in sidecars {
        let mut facts = item_facts(role);
        set(&mut facts, "language", finding(&sidecar.language));
        if !sidecar.flags.is_empty() {
            let flags: Vec<String> = sidecar.flags.iter().map(|flag| serde_name(*flag)).collect();
            facts.insert("flags", flags.join(", "));
        }
        set(&mut facts, "title", finding(&sidecar.title));
        add(&sidecar.path, facts);
    }
}

fn artwork(add: &mut impl FnMut(&str, Facts), artwork: &[Artwork]) {
    for image in artwork {
        let mut facts = item_facts("artwork");
        facts.insert("artwork_role", serde_name(image.role));
        add(&image.path, facts);
    }
}

fn nfo(add: &mut impl FnMut(&str, Facts), nfo: Option<&String>) {
    if let Some(path) = nfo {
        add(path, item_facts("nfo"));
    }
}

fn set(facts: &mut Facts, fact: &'static str, value: Option<impl Display>) {
    if let Some(value) = value {
        facts.insert(fact, value.to_string());
    }
}

fn finding<T: Display>(finding: &Finding<T>) -> Option<String> {
    match finding {
        Finding::Resolved { value, .. } => Some(value.to_string()),
        Finding::Ambiguous { candidates, .. } => {
            let candidates: Vec<String> = candidates.iter().map(T::to_string).collect();
            Some(format!("ambiguous: {}", candidates.join(" | ")))
        }
        Finding::Unknown => None,
    }
}

fn numbers(numbers: &[u32]) -> String {
    let numbers: Vec<String> = numbers.iter().map(u32::to_string).collect();
    numbers.join(", ")
}

/// Returns the names of `reasons`, joined by commas.
fn reasons(reasons: &[crate::model::AmbiguityReason]) -> String {
    let names: Vec<String> = reasons.iter().map(serde_name).collect();
    names.join(", ")
}
