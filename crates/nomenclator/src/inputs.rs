//! Provides the caller inputs that resolve facts which the names cannot resolve, such as
//! hints per path (T20). Inputs are read for one call and are not stored. Every input that
//! no item takes is reported.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::interpret::Item;
use crate::lexical;
use crate::model::ExtraKind;

/// The facts that the caller already knows about a library, in addition to its names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallerInputs {
    /// Facts per path (T20).
    #[serde(default)]
    pub hints: Vec<Hint>,
    /// Structural decisions: merged and split items, extras' kinds and owners (X16).
    #[serde(default)]
    pub overrides: Overrides,
    /// Per series, the episode order used in its names and its numbering maps (X17).
    #[serde(default)]
    pub series: Vec<SeriesInputs>,
}

/// Facts the caller knows from a catalogue, read only by validation (X18).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceMetadata {
    /// Movies' years.
    #[serde(default)]
    pub movies: Vec<ReferenceMovie>,
    /// Series' episode lists, in the aired order.
    #[serde(default)]
    pub series: Vec<ReferenceSeries>,
}

/// A movie's year, for the movie at `path` (its folder or a video) or with one of
/// `provider_ids`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceMovie {
    /// The movie's folder or one of its videos.
    #[serde(default)]
    pub path: Option<String>,
    /// Provider IDs by provider. A movie matches if one of its IDs is equal.
    #[serde(default)]
    pub provider_ids: BTreeMap<String, String>,
    /// The movie's year.
    pub year: u16,
}

/// A series' episodes as `[season, episode]`, in the aired order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceSeries {
    /// The series folder, or the title of a series that has no folder.
    pub series: String,
    /// Every episode the series has.
    pub episodes: Vec<(u16, u32)>,
}

/// What the caller knows about one series' numbering (X17).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesInputs {
    /// The series folder, or the title of a series that has no folder.
    pub series: String,
    /// The episode order used in the series' names. Defaults to `aired`. Under `absolute`, a
    /// number without a season is an absolute number.
    #[serde(default)]
    pub order: Option<EpisodeOrder>,
    /// Absolute numbers per aired season: `first` to `last` are that season's episodes 1, 2, ….
    #[serde(default)]
    pub absolute: Vec<AbsoluteRange>,
    /// Scene coordinates and their aired coordinates.
    #[serde(default)]
    pub scene: Vec<SceneMapping>,
}

/// An order episodes are numbered in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeOrder {
    /// The order in which the episodes aired. Providers use this order.
    Aired,
    /// The order of a disc release.
    Dvd,
    /// One number across all seasons.
    Absolute,
}

/// The absolute numbers of one aired season.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbsoluteRange {
    /// The aired season.
    pub season: u16,
    /// The absolute number of its first episode.
    pub first: u32,
    /// The absolute number of its last episode.
    pub last: u32,
}

/// One scene coordinate, `[season, episode]`, and its aired coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneMapping {
    /// The scene season and episode.
    pub from: (u16, u32),
    /// The aired season and episode.
    pub to: (u16, u32),
}

/// The numbers that a numbering map replaced, as evidence for an episode's coordinates (X17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MappedNumbering {
    /// The map that gave the coordinates.
    pub map: NumberingMap,
    /// The season in the name, if any.
    pub season: Option<u16>,
    /// The numbers in the name: absolute numbers, or scene episodes.
    pub numbers: Vec<u32>,
}

/// A caller-supplied numbering map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberingMap {
    /// Absolute numbers to aired coordinates.
    Absolute,
    /// Scene coordinates to aired ones.
    Scene,
}

/// Structural decisions the caller makes about items (X16).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Overrides {
    /// Groups of videos that are versions of one item. The item is the first video's item.
    #[serde(default)]
    pub merge: Vec<Vec<String>>,
    /// Videos that are items of their own, outside any folder's movie or episode.
    #[serde(default)]
    pub split: Vec<String>,
    /// Extras' kinds and owners.
    #[serde(default)]
    pub extras: Vec<ExtraOverride>,
}

/// An extra's kind or owner, as the caller sets it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtraOverride {
    /// The video or theme audio.
    pub path: String,
    /// The kind of extra. If it is set for a feature video, the video becomes an extra.
    #[serde(default)]
    pub kind: Option<ExtraKind>,
    /// The folder whose item the extra belongs to.
    #[serde(default)]
    pub owner: Option<String>,
}

/// Facts the caller supplies for one root-relative path, a file or a folder. A hint
/// that contradicts a name makes the item ambiguous, unless it is authoritative.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hint {
    /// The file or folder that the hint applies to. A trailing `/` is ignored.
    pub path: String,
    /// `true` if the hint replaces a name value that contradicts it.
    #[serde(default)]
    pub authoritative: bool,
    /// A movie's title, or a series' title on its series folder.
    #[serde(default)]
    pub title: Option<String>,
    /// A movie's year, or a series' year on its series folder.
    #[serde(default)]
    pub year: Option<u16>,
    /// Provider IDs by provider, for a movie or a series.
    #[serde(default)]
    pub provider_ids: BTreeMap<String, String>,
    /// The series folder of the videos at or below the path (TV). It must be one of their
    /// folders.
    #[serde(default)]
    pub series_folder: Option<String>,
    /// The season of the videos at or below the path (TV).
    #[serde(default)]
    pub season: Option<u16>,
    /// The episodes a video covers (TV).
    #[serde(default)]
    pub episodes: Option<Vec<u32>>,
    /// The order of the coordinates of the videos at or below the path (TV, X17).
    #[serde(default)]
    pub episode_order: Option<EpisodeOrder>,
}

/// A hinted fact as an item applied it: evidence with source `hint`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppliedHint {
    /// The hint's path.
    pub path: String,
    /// `title`, `year`, `provider_id.<provider>`, `series_folder`, `season` or `episodes`.
    pub fact: String,
    /// The hinted value.
    pub value: String,
    /// The outcome of the hint.
    pub outcome: HintOutcome,
    /// The value in the names, if the names contain the fact.
    pub name_value: Option<String>,
}

/// The outcome of a hint for a fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HintOutcome {
    /// The names do not contain the fact. The hint supplies it.
    Filled,
    /// The names contain the same value.
    Agreed,
    /// The names contain another value, which is kept. The item is ambiguous.
    Contradicted,
    /// The names contain another value. The authoritative hint replaces it.
    Overrode,
}

impl HintOutcome {
    /// Returns `true` if the hinted value becomes the fact.
    pub(crate) fn takes_hint(self) -> bool {
        matches!(self, HintOutcome::Filled | HintOutcome::Overrode)
    }
}

/// An input that no item could take.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct InputDiagnostic {
    /// The kind of input: `hint`, `merge`, `split`, `extra_override` or `series`.
    pub input: &'static str,
    /// The input's path.
    pub path: String,
    /// The reason it was not taken.
    pub reason: InputDiagnosticReason,
    /// The fact that no item took, for `not_applicable`.
    pub fact: Option<String>,
}

/// The reason an input was not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputDiagnosticReason {
    /// The path is not in the listing.
    UnknownPath,
    /// No item at the path takes the fact (a year on an extra, episodes on a movie).
    NotApplicable,
    /// Several hints have the path. None of them applies.
    Duplicate,
}

/// The hints of one call by path, without the duplicated ones.
pub(crate) struct Hints<'a> {
    by_path: BTreeMap<String, &'a Hint>,
}

impl<'a> Hints<'a> {
    pub(crate) fn new(hints: &'a [Hint]) -> Self {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for hint in hints {
            *counts.entry(normalized(&hint.path)).or_default() += 1;
        }
        let by_path = hints
            .iter()
            .filter(|hint| counts[&normalized(&hint.path)] == 1)
            .map(|hint| (normalized(&hint.path), hint))
            .collect();
        Hints { by_path }
    }

    /// Returns the hint for exactly `path`.
    pub(crate) fn at(&self, path: &str) -> Option<&'a Hint> {
        self.by_path.get(&normalized(path)).copied()
    }

    /// Returns the hints of `path`'s folders, from the root down, then the hint of `path`
    /// itself.
    pub(crate) fn along(&self, path: &str) -> Vec<(&'a Hint, bool)> {
        let path = normalized(path);
        let mut found = Vec::new();
        let mut end = 0;
        while let Some(slash) = path[end..].find('/') {
            end += slash;
            if let Some(hint) = self.at(&path[..end]) {
                found.push((hint, false));
            }
            end += 1;
        }
        if let Some(hint) = self.at(&path) {
            found.push((hint, true));
        }
        found
    }
}

pub(crate) fn normalized(path: &str) -> String {
    path.trim_end_matches('/').to_owned()
}

/// Compares a hinted value with the names' value of the same fact and returns the applied
/// hint. Both values are as written for evidence. `agrees` is `true` if the two values state
/// the same fact.
pub(crate) fn settle(
    hint: &Hint,
    fact: &str,
    hinted: String,
    named: Option<String>,
    agrees: bool,
) -> AppliedHint {
    let outcome = match (&named, agrees) {
        (None, _) => HintOutcome::Filled,
        (Some(_), true) => HintOutcome::Agreed,
        (Some(_), false) if hint.authoritative => HintOutcome::Overrode,
        (Some(_), false) => HintOutcome::Contradicted,
    };
    AppliedHint {
        path: normalized(&hint.path),
        fact: fact.to_owned(),
        value: hinted,
        outcome,
        name_value: named,
    }
}

/// Returns the hints and facts that no item took, the hints for paths outside the listing,
/// and the paths that several hints have.
pub(crate) fn hint_diagnostics(
    hints: &[Hint],
    listed: &BTreeSet<String>,
    items: &[Item],
) -> Vec<InputDiagnostic> {
    let applied: BTreeSet<(String, String)> = items
        .iter()
        .flat_map(|item| match item {
            Item::Movie(movie) => movie.hints.as_slice(),
            Item::Series(series) => series.hints.as_slice(),
            Item::Episode(episode) => episode.hints.as_slice(),
            _ => &[],
        })
        .map(|applied| (applied.path.clone(), applied.fact.clone()))
        .collect();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for hint in hints {
        *counts.entry(normalized(&hint.path)).or_default() += 1;
    }
    let mut diagnostics = BTreeSet::new();
    for hint in hints {
        let path = normalized(&hint.path);
        let diagnostic = |reason, fact: Option<String>| InputDiagnostic {
            input: "hint",
            path: path.clone(),
            reason,
            fact,
        };
        if !listed.contains(&path) {
            diagnostics.insert(diagnostic(InputDiagnosticReason::UnknownPath, None));
        } else if counts[&path] > 1 {
            diagnostics.insert(diagnostic(InputDiagnosticReason::Duplicate, None));
        } else {
            for fact in hinted_facts(hint) {
                if !applied.contains(&(path.clone(), fact.clone())) {
                    diagnostics
                        .insert(diagnostic(InputDiagnosticReason::NotApplicable, Some(fact)));
                }
            }
        }
    }
    diagnostics.into_iter().collect()
}

fn hinted_facts(hint: &Hint) -> Vec<String> {
    let mut facts = Vec::new();
    let mut add = |present: bool, fact: &str| {
        if present {
            facts.push(fact.to_owned());
        }
    };
    add(hint.title.is_some(), "title");
    add(hint.year.is_some(), "year");
    add(hint.series_folder.is_some(), "series_folder");
    add(hint.episode_order.is_some(), "episode_order");
    add(hint.season.is_some(), "season");
    add(
        hint.episodes
            .as_ref()
            .is_some_and(|episodes| !episodes.is_empty()),
        "episodes",
    );
    facts.extend(
        hint.provider_ids
            .keys()
            .map(|provider| format!("provider_id.{}", lexical::vocabulary_form(provider))),
    );
    facts
}

/// Returns the override paths that no item took as stated, and those outside the listing.
pub(crate) fn override_diagnostics(
    overrides: &Overrides,
    listed: &BTreeSet<String>,
    items: &[Item],
) -> Vec<InputDiagnostic> {
    let mut videos: BTreeSet<&str> = BTreeSet::new();
    let mut extras: BTreeSet<&str> = BTreeSet::new();
    for item in items {
        let (versions, item_extras): (&[crate::interpret::Version], &[crate::interpret::Extra]) =
            match item {
                Item::Movie(movie) => (&movie.versions, &movie.extras),
                Item::Episode(episode) => (&episode.versions, &episode.extras),
                Item::Series(series) => (&[], &series.extras),
                Item::Season(season) => (&[], &season.extras),
                Item::Extra(orphan) => {
                    extras.extend(orphan.paths.iter().map(String::as_str));
                    continue;
                }
            };
        videos.extend(
            versions
                .iter()
                .flat_map(|version| version.paths.iter().map(String::as_str)),
        );
        extras.extend(
            item_extras
                .iter()
                .flat_map(|extra| extra.paths.iter().map(String::as_str)),
        );
    }
    let mut diagnostics = BTreeSet::new();
    let mut check = |input: &'static str, path: &str, taken: bool| {
        let path = normalized(path);
        let reason = if !listed.contains(&path) {
            InputDiagnosticReason::UnknownPath
        } else if !taken {
            InputDiagnosticReason::NotApplicable
        } else {
            return;
        };
        diagnostics.insert(InputDiagnostic {
            input,
            path,
            reason,
            fact: None,
        });
    };
    for group in &overrides.merge {
        for path in group {
            check("merge", path, videos.contains(path.as_str()));
        }
    }
    for path in &overrides.split {
        check("split", path, videos.contains(path.as_str()));
    }
    for extra in &overrides.extras {
        check(
            "extra_override",
            &extra.path,
            extras.contains(extra.path.as_str()),
        );
        if let Some(owner) = &extra.owner {
            check("extra_override", owner, true);
        }
    }
    diagnostics.into_iter().collect()
}

/// Returns the series inputs that no episode took, and those that match no listed folder or
/// series.
pub(crate) fn series_diagnostics(
    rules: &crate::rules::Rules,
    series: &[SeriesInputs],
    listed: &BTreeSet<String>,
    items: &[Item],
) -> Vec<InputDiagnostic> {
    series
        .iter()
        .filter(|input| {
            !items.iter().any(|item| match item {
                Item::Episode(episode) => series_matches(
                    rules,
                    input,
                    episode.series_path.as_deref(),
                    episode.series_name.as_deref(),
                ),
                _ => false,
            })
        })
        .map(|input| {
            let path = normalized(&input.series);
            InputDiagnostic {
                input: "series",
                reason: if listed.contains(&path) {
                    InputDiagnosticReason::NotApplicable
                } else {
                    InputDiagnosticReason::UnknownPath
                },
                path,
                fact: None,
            }
        })
        .collect()
}

/// Returns `true` if series inputs match a series: by its folder, or by title if the series
/// has no folder.
pub(crate) fn series_matches(
    rules: &crate::rules::Rules,
    input: &SeriesInputs,
    folder: Option<&str>,
    title: Option<&str>,
) -> bool {
    let wanted = normalized(&input.series);
    match folder {
        Some(folder) => folder == wanted,
        None => title.is_some_and(|title| {
            crate::lexical::title_key(rules, title) == crate::lexical::title_key(rules, &wanted)
        }),
    }
}

/// Formats episode numbers as a hint value: `3, 4`.
pub(crate) fn episode_list(episodes: &[u32]) -> String {
    episodes
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
