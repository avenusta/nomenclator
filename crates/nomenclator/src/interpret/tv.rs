//! Runs the TV pipeline (passes 4-5). The pipeline resolves which series and season each
//! video belongs to (T1, T2, T9), which videos cover the same episodes and are therefore
//! versions of one episode (T15, T17), and where extras and sidecars are attached (E2, E5,
//! E11, T11, S2-S6) through `associations`.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use super::associations::{self, OwnerCandidate, OwnerCandidates, OwnerChoice};
use super::{
    Artwork, EntryIndex, Episode, Extra, FileRole, Item, OrphanExtra, PipelineOutput, Season,
    Series, SidecarFile, Version, collapse_same_named_wrappers, merge_provider_claims,
    parent_folder, resolution_of, sorted_by_path, unwrap_single_bracket,
    without_provider_and_edition_tags,
};
use crate::inputs::{
    AppliedHint, CallerInputs, EpisodeOrder, Hint, HintOutcome, Hints, MappedNumbering,
    NumberingMap, Overrides, SeriesInputs, episode_list, normalized, series_matches, settle,
};
use crate::lexical;
use crate::model::{ArtworkRole, EntryKind, Evidence, Finding, NfoRole, Rule};
use crate::parsers::episode_name::{AlternateIdentity, EpisodeReading, read_episode};
use crate::parsers::episode_name::{KnownSeasons, PresentationKind, Special};
use crate::parsers::file_types::file_name;
use crate::rules::Rules;

/// One representation of an episode: a video or a stack of parts.
struct Unit {
    /// One video, or the ordered parts.
    paths: Vec<String>,
    /// Its name: a video stem or a stack name.
    name: String,
    reading: EpisodeReading,
    part_ambiguity: Option<&'static str>,
    /// The hinted facts compared with the reading (T20).
    hints: Vec<AppliedHint>,
    /// `true` if the caller split it into an episode of its own (X16).
    is_split: bool,
    /// The order of its coordinates, as set by a hint (X17).
    order_hint: Option<EpisodeOrder>,
    /// The order of its coordinates, if the caller states orders (X17).
    order: Option<EpisodeOrder>,
    /// The numbers that a numbering map replaced (X17).
    mapped_from: Option<MappedNumbering>,
    /// `true` if a numbering map gave a season other than its season folder's season (X17).
    numbering_contradicts_season: bool,
}

impl Unit {
    /// Returns the series folder (T1). A same-named wrapper folder above it is collapsed into
    /// it.
    fn series_folder(&self) -> Option<String> {
        self.reading
            .layout
            .series_folder
            .as_deref()
            .map(|folder| collapse_same_named_wrappers(folder).to_owned())
    }

    /// Returns `true` if the series title comes from the series folder (T2) and not from the
    /// file.
    fn is_named_by_folder(&self) -> bool {
        self.reading.layout.series_folder.is_some()
            && self.reading.name.series_path == self.reading.layout.series_folder
    }

    fn series_key(&self, rules: &Rules) -> SeriesKey {
        let file_title_key = (!self.is_named_by_folder())
            .then(|| self.reading.name.series_name.resolved_value())
            .flatten()
            .map(|title| lexical::title_key(rules, title));
        SeriesKey {
            folder: self.series_folder(),
            file_title_key,
        }
    }

    fn season(&self) -> Option<u16> {
        self.reading.name.season.resolved_value().copied()
    }

    fn coverage(&self) -> Coverage {
        let name = &self.reading.name;
        if let Some(special) = &name.special {
            return Coverage::Special(special.clone());
        }
        if let Some(episodes) = &name.episodes {
            Coverage::Episodes(episodes.clone())
        } else if let Some(absolute) = &name.absolute {
            Coverage::Absolute(absolute.clone())
        } else if let Some(date) = date_key(&name.date) {
            Coverage::Date(date)
        } else {
            Coverage::Unreferenced(self.paths[0].clone())
        }
    }

    /// Returns the release text after the episode reference and its title, without provider
    /// and edition tags (`S01E01 - Pilot - 1080p` becomes `1080p`). The label never contains
    /// the episode title (T17). Returns `None` if no other text follows.
    fn label(&self, rules: &Rules) -> Option<String> {
        let start = self
            .reading
            .layout
            .release_remainder_start_in_stem
            .unwrap_or(0);
        let rest = without_provider_and_edition_tags(rules, self.name.get(start..)?);
        let rest = unwrap_single_bracket(rules, rest.trim_matches([' ', '-', '_', '.']));
        (!rest.is_empty()).then(|| lexical::display_title(rules, rest))
    }
}

fn date_key(date: &Finding<String>) -> Option<String> {
    match date {
        Finding::Resolved { value, .. } => Some(value.clone()),
        Finding::Ambiguous { candidates, .. } => Some(candidates.join("|")),
        Finding::Unknown => None,
    }
}

/// The key of a series: its folder, and the file's title if the series title comes from a
/// file (T2). Two shows named by their files in one folder are two series.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SeriesKey {
    folder: Option<String>,
    file_title_key: Option<String>,
}

impl SeriesKey {
    fn is_series(&self) -> bool {
        self.folder.is_some() || self.file_title_key.is_some()
    }
}

/// The episodes that an episode item covers. Videos of one series with the same season and
/// coverage are versions of one episode (T17). A video without any reference is an episode
/// of its own.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Coverage {
    Episodes(Vec<u32>),
    Absolute(Vec<u32>),
    Date(String),
    /// A special without an episode number (X20).
    Special(Special),
    Unreferenced(String),
}

/// An item that extras and artwork can be attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TvOwner {
    Series(usize),
    Season(usize),
    Episode(usize),
}

/// The items during assembly. `owners` and `candidates` are parallel lists with one entry
/// per item, in item order. The candidates are moved out to be indexed once all items are
/// assembled.
#[derive(Default)]
struct Assembly {
    series: Vec<Series>,
    seasons: Vec<Season>,
    episodes: Vec<Episode>,
    /// Per episode, the folders whose extras can refer to it: the folders of its videos, its
    /// season and its series.
    episode_home_folders: Vec<BTreeSet<String>>,
    owners: Vec<TvOwner>,
    candidates: Vec<OwnerCandidate>,
    /// The sort key of each item's block: its series folder, or its first video if it has no
    /// series folder.
    block_keys: Vec<String>,
}

pub(super) fn interpret(
    rules: &Rules,
    index: &EntryIndex,
    hints: &Hints,
    inputs: &CallerInputs,
) -> PipelineOutput {
    let overrides = &inputs.overrides;
    let known = &known_seasons(rules, index, hints);
    let mut units = collect_units(known, rules, index);
    name_unrepeated_titles_by_folder(rules, &mut units);
    offer_repeated_season_folder_texts_as_alternates(rules, &mut units);
    for unit in &mut units {
        apply_unit_hints(rules, hints, unit);
        unit.is_split = unit.paths.iter().any(|path| overrides.split.contains(path));
        apply_series_inputs(rules, &inputs.series, unit);
    }
    merge_units(&mut units, overrides);
    let mut by_series: BTreeMap<SeriesKey, Vec<&Unit>> = BTreeMap::new();
    for unit in &units {
        let key = unit.series_key(rules);
        // A video with neither a series nor a reference is unresolved (T3).
        let has_reference = !matches!(unit.coverage(), Coverage::Unreferenced(_));
        if key.is_series() || has_reference {
            by_series.entry(key).or_default().push(unit);
        }
    }
    let mut assembly = Assembly::default();
    let mut owner_of_video: BTreeMap<String, usize> = BTreeMap::new();
    for (key, series_units) in &by_series {
        if key.is_series() {
            assembly.add_series(rules, key, series_units, &mut owner_of_video);
        } else {
            for unit in series_units {
                assembly.add_episode(rules, None, None, &[unit], &mut owner_of_video);
            }
        }
    }

    let candidates = OwnerCandidates::new(std::mem::take(&mut assembly.candidates));
    let mut claimed_files: BTreeSet<String> = owner_of_video.keys().cloned().collect();
    let orphans = attach_extras(
        known,
        rules,
        index,
        &mut assembly,
        &candidates,
        &mut claimed_files,
    );
    let sidecars = associations::attach_sidecars(rules, index);
    claimed_files.extend(sidecars.claimed_files.iter().cloned());
    let episode_of_video = |video: &str| match owner_of_video.get(video) {
        Some(&candidate) => match assembly.owners[candidate] {
            TvOwner::Episode(episode) => Some(episode),
            _ => None,
        },
        None => None,
    };
    let mut attached_sidecars: Vec<(usize, Vec<SidecarFile>, Vec<SidecarFile>)> = Vec::new();
    for (video, subtitles) in &sidecars.subtitles_by_video {
        if let Some(episode) = episode_of_video(video) {
            attached_sidecars.push((episode, subtitles.clone(), Vec::new()));
        }
    }
    for (video, audio) in &sidecars.audio_by_video {
        if let Some(episode) = episode_of_video(video) {
            attached_sidecars.push((episode, Vec::new(), audio.clone()));
        }
    }
    for (episode, subtitles, audio) in attached_sidecars {
        assembly.episodes[episode].subtitles.extend(subtitles);
        assembly.episodes[episode].audio.extend(audio);
    }
    let (artwork, artwork_files) = associations::attach_artwork(
        rules,
        index,
        &candidates,
        &owner_of_video,
        &sidecars.stills_by_video,
        ArtworkRole::EpisodeStill,
    );
    claimed_files.extend(artwork_files);
    for (owner, artwork) in artwork {
        assembly.artwork_of(assembly.owners[owner]).extend(artwork);
    }
    let nfos = associations::attach_nfos(rules, index, &owner_of_video, |role, folder| {
        assembly.folder_owner(&candidates, role, folder?)
    });
    for (owner, nfo) in &nfos.generic_by_owner {
        claimed_files.insert(nfo.clone());
        *assembly.nfo_of(assembly.owners[*owner]) = Some(nfo.clone());
    }
    for episode in &mut assembly.episodes {
        for version in &mut episode.versions {
            version.nfo = nfos.of_version(&version.paths);
            claimed_files.extend(version.nfo.clone());
        }
        episode.nfo = episode.versions[0].nfo.clone();
    }
    claimed_files.extend(associations::carried_files(index, &claimed_files));

    assembly.apply_series_hints(rules, hints);
    assembly.sort_attachments();
    let extras_of = assembly.extras_by_owner();
    let mut blocks = assembly.into_item_blocks();
    blocks.extend(
        orphans
            .into_iter()
            .map(|orphan| (orphan.paths[0].clone(), vec![Item::Extra(orphan)])),
    );
    blocks.sort_by(|left, right| left.0.cmp(&right.0));
    let item_keys = blocks
        .iter()
        .flat_map(|(key, items)| std::iter::repeat_n(key.clone(), items.len()))
        .collect();

    PipelineOutput {
        items: blocks.into_iter().flat_map(|(_, items)| items).collect(),
        item_keys,
        extras_of,
        video_subtitles: sorted_by_path(sidecars.subtitles_by_video),
        claimed_files,
    }
}

/// Formats an order as it is written in evidence: `dvd`.
/// Applies the caller's numbering for the unit's series (X17). Under an `absolute` order, a
/// number without a season is an absolute number. A scene map converts scene coordinates to
/// aired coordinates, and an absolute map converts absolute numbers to aired coordinates,
/// unless the season folder's name contains another season. The unit then records the order
/// of its coordinates.
fn apply_series_inputs(rules: &Rules, series: &[SeriesInputs], unit: &mut Unit) {
    let folder = unit.series_folder();
    let title = unit.reading.name.series_name.resolved_value().cloned();
    let input = series.iter().find(|input| {
        series_matches(rules, input, folder.as_deref(), title.as_deref())
            || unit.reading.layout.series_folder.as_deref() == Some(&normalized(&input.series))
    });
    let Some(input) = input else {
        unit.order = unit.order_hint;
        return;
    };
    let named_order = unit
        .order_hint
        .or(input.order)
        .unwrap_or(EpisodeOrder::Aired);
    let name = &mut unit.reading.name;
    let named_season = name.season.resolved_value().copied();
    if named_order == EpisodeOrder::Absolute
        && named_season.is_none()
        && name.absolute.is_none()
        && let Some(episodes) = name.episodes.take()
    {
        name.absolute = Some(episodes);
        name.first_episode = None;
        name.last_episode = None;
        name.presentation_kind = Some(PresentationKind::Absolute);
    }
    let mut mapped = false;
    if let (Some(season), Some(episodes)) = (named_season, name.episodes.clone()) {
        let aired: Option<Vec<(u16, u32)>> = episodes
            .iter()
            .map(|&episode| {
                input
                    .scene
                    .iter()
                    .find(|mapping| mapping.from == (season, episode))
                    .map(|mapping| mapping.to)
            })
            .collect();
        if let Some(aired) = aired.filter(|aired| same_season(aired)) {
            set_coordinates(name, &aired);
            unit.mapped_from = Some(MappedNumbering {
                map: NumberingMap::Scene,
                season: Some(season),
                numbers: episodes,
            });
            mapped = true;
        }
    } else if let (None, Some(absolute)) = (&name.episodes, name.absolute.clone()) {
        let aired: Option<Vec<(u16, u32)>> = absolute
            .iter()
            .map(|&number| {
                input
                    .absolute
                    .iter()
                    .find(|range| (range.first..=range.last).contains(&number))
                    .map(|range| (range.season, number - range.first + 1))
            })
            .collect();
        if let Some(aired) = aired.filter(|aired| same_season(aired)) {
            if named_season.is_some_and(|season| season != aired[0].0) {
                unit.numbering_contradicts_season = true;
            } else {
                set_coordinates(name, &aired);
                unit.mapped_from = Some(MappedNumbering {
                    map: NumberingMap::Absolute,
                    season: named_season,
                    numbers: absolute,
                });
                mapped = true;
            }
        }
    }
    unit.order = Some(if mapped {
        EpisodeOrder::Aired
    } else if name.episodes.is_some() {
        match named_order {
            EpisodeOrder::Absolute => EpisodeOrder::Aired,
            order => order,
        }
    } else {
        EpisodeOrder::Absolute
    })
    .filter(|order| *order != EpisodeOrder::Absolute || name.absolute.is_some());
}

fn same_season(coordinates: &[(u16, u32)]) -> bool {
    coordinates
        .first()
        .is_some_and(|(season, _)| coordinates.iter().all(|(other, _)| other == season))
}

/// Sets a reading's season and episodes to aired coordinates of one season.
fn set_coordinates(name: &mut crate::parsers::episode_name::EpisodeName, aired: &[(u16, u32)]) {
    let episodes: Vec<u32> = aired.iter().map(|(_, episode)| *episode).collect();
    name.season = Finding::Resolved {
        value: aired[0].0,
        evidence: None,
    };
    name.first_episode = episodes.first().copied();
    name.last_episode = (episodes.len() > 1)
        .then(|| episodes.last().copied())
        .flatten();
    name.presentation_kind = Some(if episodes.len() > 1 {
        PresentationKind::MultiEpisode
    } else {
        PresentationKind::Episode
    });
    name.is_episode = true;
    name.episodes = Some(episodes);
}

/// Applies the caller's merge overrides (X16). The units of a group take the season and
/// coverage of the first unit listed, so that they are versions of its episode.
fn merge_units(units: &mut [Unit], overrides: &Overrides) {
    for group in &overrides.merge {
        let first = group
            .iter()
            .find_map(|path| units.iter().position(|unit| unit.paths.contains(path)));
        let Some(first) = first else {
            continue;
        };
        let name = units[first].reading.name.clone();
        let series_folder = units[first].reading.layout.series_folder.clone();
        for unit in units.iter_mut() {
            if unit.paths.iter().any(|path| group.contains(path)) {
                let reading = &mut unit.reading.name;
                reading.season = name.season.clone();
                reading.episodes = name.episodes.clone();
                reading.first_episode = name.first_episode;
                reading.last_episode = name.last_episode;
                reading.absolute = name.absolute.clone();
                reading.date = name.date.clone();
                reading.series_name = name.series_name.clone();
                reading.series_path = name.series_path.clone();
                unit.reading.layout.series_folder = series_folder.clone();
            }
        }
    }
}

/// Returns the folders that season hints make season folders (T20): folders whose own name
/// contains no season, and folders that an authoritative hint applies to.
fn known_seasons(rules: &Rules, index: &EntryIndex, hints: &Hints) -> KnownSeasons {
    index
        .folders
        .iter()
        .filter_map(|folder| {
            let hint = hints.at(folder)?;
            let season = hint.season?;
            let named = named_folder_season(rules, folder);
            (named.is_none() || hint.authoritative).then(|| (folder.clone(), season))
        })
        .collect()
}

/// Returns the season in a folder's own name. Returns `None` if the name contains no season.
fn named_folder_season(rules: &Rules, folder: &str) -> Option<u16> {
    let parent = parent_folder(folder).map(file_name);
    rules
        .parse_season_folder(file_name(folder), parent, parent.is_some())
        .season
        .resolved_value()
        .copied()
}

/// Compares the hints on a unit's folders and videos with its reading (T20): a series folder
/// that is one of its folders, a season, and for a video its episodes. A hint supplies a
/// fact that the names do not contain. A contradicted hint keeps the names' value and makes
/// the episode ambiguous, unless the hint is authoritative.
fn apply_unit_hints(rules: &Rules, hints: &Hints, unit: &mut Unit) {
    let mut along: Vec<(&Hint, bool)> = Vec::new();
    for path in &unit.paths {
        for found in hints.along(path) {
            if !along.iter().any(|(known, _)| std::ptr::eq(*known, found.0)) {
                along.push(found);
            }
        }
    }
    for (hint, is_video) in along {
        if let Some(folder) = &hint.series_folder {
            let folder = normalized(folder);
            let is_own_folder = unit.paths[0].starts_with(&format!("{folder}/"));
            if is_own_folder {
                let named = unit.reading.layout.series_folder.clone();
                let agrees = named.as_ref() == Some(&folder);
                let applied = settle(hint, "series_folder", folder.clone(), named, agrees);
                if applied.outcome.takes_hint() {
                    let series = rules.parse_series_name(file_name(&folder));
                    unit.reading.layout.series_folder = Some(folder.clone());
                    unit.reading.name.series_path = Some(folder);
                    unit.reading.name.series_name = series.title;
                    unit.reading.name.series_year = series.year;
                }
                unit.hints.push(applied);
            }
        }
        if let Some(season) = hint.season {
            // A folder's hint is compared with the folder's own name, which `known_seasons`
            // already applied to the reading. A video's hint is compared with the reading.
            let named = if is_video {
                unit.reading.name.season.resolved_value().copied()
            } else {
                named_folder_season(rules, &hint.path)
            };
            let applied = settle(
                hint,
                "season",
                season.to_string(),
                named.map(|named| named.to_string()),
                named == Some(season),
            );
            if applied.outcome.takes_hint() {
                unit.reading.name.season = Finding::Resolved {
                    value: season,
                    evidence: None,
                };
            }
            unit.hints.push(applied);
        }
        // A video without a series folder belongs to no series item, so its own title and
        // year hints settle its series name and year.
        if is_video && unit.reading.layout.series_folder.is_none() {
            if let Some(title) = &hint.title {
                let named = unit.reading.name.series_name.resolved_value().cloned();
                let agrees = named.as_deref().is_some_and(|named| {
                    lexical::title_comparison_key(rules, named)
                        == lexical::title_comparison_key(rules, title)
                });
                let applied = settle(hint, "title", title.clone(), named, agrees);
                if applied.outcome.takes_hint() {
                    unit.reading.name.series_name = Finding::Resolved {
                        value: title.clone(),
                        evidence: None,
                    };
                }
                unit.hints.push(applied);
            }
            if let Some(year) = hint.year {
                let named = unit.reading.name.series_year.resolved_value().copied();
                let applied = settle(
                    hint,
                    "year",
                    year.to_string(),
                    named.map(|named| named.to_string()),
                    named == Some(year),
                );
                if applied.outcome.takes_hint() {
                    unit.reading.name.series_year = Finding::Resolved {
                        value: year,
                        evidence: None,
                    };
                }
                unit.hints.push(applied);
            }
        }
        if let Some(order) = hint.episode_order {
            let value = crate::model::serde_name(order);
            unit.hints
                .push(settle(hint, "episode_order", value, None, false));
            unit.order_hint = Some(order);
        }
        if let (Some(episodes), true) = (&hint.episodes, is_video)
            && !episodes.is_empty()
        {
            let named = unit.reading.name.episodes.clone();
            let applied = settle(
                hint,
                "episodes",
                episode_list(episodes),
                named.as_deref().map(episode_list),
                named.as_ref() == Some(episodes),
            );
            if applied.outcome.takes_hint() {
                let name = &mut unit.reading.name;
                name.episodes = Some(episodes.clone());
                name.first_episode = episodes.first().copied();
                name.last_episode = (episodes.len() > 1)
                    .then(|| episodes.last().copied())
                    .flatten();
                name.is_episode = true;
                name.presentation_kind = Some(if episodes.len() > 1 {
                    PresentationKind::MultiEpisode
                } else {
                    PresentationKind::Episode
                });
            }
            unit.hints.push(applied);
        }
    }
}

/// Collects the units: feature videos and part stacks (T17).
fn collect_units(known: &KnownSeasons, rules: &Rules, index: &EntryIndex) -> Vec<Unit> {
    let feature_files: Vec<String> = index
        .files
        .iter()
        .filter(|(_, role)| matches!(role, FileRole::FeatureVideo))
        .map(|(path, _)| path.clone())
        .collect();
    let mut units = Vec::new();
    let mut consumed: BTreeSet<String> = BTreeSet::new();
    for stack in rules.group_parts(&feature_files).stacks {
        let extension = lexical::extension(file_name(&stack.paths[0]), EntryKind::File);
        let stack_file = format!("{}.{}", stack.name, extension.unwrap_or_default());
        let reading_path = match parent_folder(&stack.paths[0]) {
            Some(folder) => format!("{folder}/{stack_file}"),
            None => stack_file,
        };
        consumed.extend(stack.paths.iter().cloned());
        units.push(Unit {
            reading: read_episode(known, rules, &reading_path, EntryKind::File, None),
            name: stack.name,
            paths: stack.paths,
            part_ambiguity: stack.ambiguity_reason,
            hints: Vec::new(),
            is_split: false,
            order_hint: None,
            order: None,
            mapped_from: None,
            numbering_contradicts_season: false,
        });
    }
    for path in feature_files
        .iter()
        .filter(|path| !consumed.contains(*path))
    {
        units.push(Unit {
            paths: vec![path.clone()],
            name: lexical::stem(file_name(path), EntryKind::File).to_owned(),
            reading: read_episode(known, rules, path, EntryKind::File, None),
            part_ambiguity: None,
            hints: Vec::new(),
            is_split: false,
            order_hint: None,
            order: None,
            mapped_from: None,
            numbering_contradicts_season: false,
        });
    }
    units.sort_by(|left, right| left.paths.cmp(&right.paths));
    units
}

/// An episode's season and coverage.
type EpisodeCoordinates = (Option<u16>, Coverage);

/// Takes the series title from the series folder for units whose file-named series text no
/// other episode in the folder repeats. A file-named series text in a series folder is the
/// series title only if files of several episodes in that folder contain it
/// (`Cartoons/Peppa S01E01`, `Cartoons/Peppa S01E02`). Otherwise the text is the episode's
/// title, and the series title comes from the folder (T2): `The X-Files/Pilot - S01E01` is
/// `The X-Files`, episode title `Pilot`. A folder whose name is only a season marker never
/// provides a series title this way.
fn name_unrepeated_titles_by_folder(rules: &Rules, units: &mut [Unit]) {
    let file_series_text = |unit: &Unit| -> Option<(String, String)> {
        let folder = unit.reading.layout.series_folder.clone()?;
        if unit.is_named_by_folder() {
            return None;
        }
        let title = unit.reading.name.series_name.resolved_value()?;
        Some((folder, lexical::title_key(rules, title)))
    };
    // Per folder and series text, the distinct episodes (season and coverage) whose files
    // contain the text.
    let mut episodes_naming: BTreeMap<(String, String), BTreeSet<EpisodeCoordinates>> =
        BTreeMap::new();
    for unit in units.iter() {
        if let Some(key) = file_series_text(unit) {
            episodes_naming
                .entry(key)
                .or_default()
                .insert((unit.season(), unit.coverage()));
        }
    }
    for unit in units.iter_mut() {
        let Some(key) = file_series_text(unit) else {
            continue;
        };
        if episodes_naming[&key].len() > 1 {
            continue;
        }
        let folder = key.0;
        let folder_name = rules.parse_series_name(file_name(&folder));
        // A season marker alone (`Season/` at the root) is season structure, not a title.
        let parent = parent_folder(&folder).map(file_name);
        let as_season_folder =
            rules.parse_season_folder(file_name(&folder), parent, parent.is_some());
        if !folder_name.title.is_resolved()
            || as_season_folder.is_bare_season_folder
            || as_season_folder.season.is_resolved()
        {
            continue;
        }
        let name = &mut unit.reading.name;
        name.episode_title = name.series_name.resolved_value().cloned();
        name.series_name = folder_name.title;
        name.series_year = folder_name.year;
        name.series_path = Some(folder);
        name.alternate_identity = None;
    }
}

/// Sets repeated texts in season folders as alternate identities. In a season folder of a
/// trusted series folder, a file's differing text before its episode code is its episode's
/// title (T2). If files of several episodes in that season folder contain the same text,
/// the series title still comes from the trusted folder. The text is then set as the
/// series' `alternate_identity`, and no episode uses it as its title
/// (`Haus des Geldes (2017)/Season 01/Money Heist S01E01`, `Money Heist S01E02` → series
/// `Haus des Geldes`, alternate `Money Heist`).
fn offer_repeated_season_folder_texts_as_alternates(rules: &Rules, units: &mut [Unit]) {
    let set_aside_text = |unit: &Unit| -> Option<(String, String)> {
        let season_folder = unit.reading.layout.season_folder.clone()?;
        let title = unit
            .reading
            .layout
            .set_aside_file_title
            .as_ref()?
            .title_value()?;
        Some((season_folder, lexical::title_key(rules, title)))
    };
    let mut episodes_naming: BTreeMap<(String, String), BTreeSet<EpisodeCoordinates>> =
        BTreeMap::new();
    for unit in units.iter() {
        if let Some(key) = set_aside_text(unit) {
            episodes_naming
                .entry(key)
                .or_default()
                .insert((unit.season(), unit.coverage()));
        }
    }
    for unit in units.iter_mut() {
        let Some(key) = set_aside_text(unit) else {
            continue;
        };
        if episodes_naming[&key].len() < 2 {
            continue;
        }
        let parts = unit
            .reading
            .layout
            .set_aside_file_title
            .take()
            .expect("checked above");
        let name = &mut unit.reading.name;
        if name.episode_title.as_deref() == parts.title_value() {
            name.episode_title = None;
        }
        name.alternate_identity = parts.title_value().map(|title| AlternateIdentity {
            title: title.to_owned(),
            year: parts.year_finding().resolved_value().copied(),
        });
    }
}

/// Returns the alternate identity that all folder-named episodes of one series share (T2).
/// Returns `None` if the episodes have no alternate identity or have different ones.
fn shared_alternate_identity(rules: &Rules, units: &[&Unit]) -> Option<AlternateIdentity> {
    let mut by_title_key: BTreeMap<String, AlternateIdentity> = BTreeMap::new();
    for unit in units.iter().filter(|unit| unit.is_named_by_folder()) {
        if let Some(alternate) = &unit.reading.name.alternate_identity {
            by_title_key.insert(
                lexical::title_key(rules, &alternate.title),
                alternate.clone(),
            );
        }
    }
    (by_title_key.len() == 1)
        .then(|| by_title_key.into_values().next())
        .flatten()
}

/// The facts of a series that its episodes and seasons repeat.
#[derive(Clone)]
struct SeriesFacts {
    name: Option<String>,
    year: Option<u16>,
    path: Option<String>,
    block_key: String,
}

impl Assembly {
    /// Adds the series item, then per season its item and its episodes, then the episodes
    /// with no known season (T7).
    fn add_series(
        &mut self,
        rules: &Rules,
        key: &SeriesKey,
        units: &[&Unit],
        owner_of_video: &mut BTreeMap<String, usize>,
    ) {
        let years: BTreeSet<u16> = units
            .iter()
            .filter_map(|unit| unit.reading.name.series_year.resolved_value().copied())
            .collect();
        let facts = SeriesFacts {
            name: units[0].reading.name.series_name.resolved_value().cloned(),
            year: (years.len() == 1)
                .then(|| years.into_iter().next())
                .flatten(),
            path: key.folder.clone(),
            block_key: key
                .folder
                .clone()
                .unwrap_or_else(|| units[0].paths[0].clone()),
        };
        let (provider_ids, tags) = match (&key.file_title_key, &key.folder) {
            (None, Some(folder)) => {
                let name = rules.parse_series_name(file_name(folder));
                (name.provider_ids, name.tags)
            }
            _ => (BTreeMap::new(), BTreeMap::new()),
        };
        self.push_owner(
            TvOwner::Series(self.series.len()),
            &facts.block_key,
            OwnerCandidate {
                owning_folder: facts.path.clone(),
                media_folders: BTreeSet::new(),
                names: Vec::new(),
                title_words: facts
                    .name
                    .as_deref()
                    .map(lexical::comparison_words)
                    .unwrap_or_default(),
                year: facts.year,
                is_folder_identity: true,
                takes_folder_assets: true,
            },
        );
        self.series.push(Series {
            title: facts.name.clone(),
            year: facts.year,
            path: facts.path.clone(),
            provider_ids,
            tags,
            alternate_identity: shared_alternate_identity(rules, units),
            extras: Vec::new(),
            artwork: Vec::new(),
            nfo: None,
            hints: Vec::new(),
        });

        // A split unit is an episode of its own, with its path in its key (X16).
        // One coordinate in two orders is two episodes (X17).
        type CoverageKey = (Option<u16>, Coverage, Option<EpisodeOrder>, Option<String>);
        let mut by_coverage: BTreeMap<CoverageKey, Vec<&Unit>> = BTreeMap::new();
        for unit in units {
            let apart = unit.is_split.then(|| unit.paths[0].clone());
            by_coverage
                .entry((unit.season(), unit.coverage(), unit.order, apart))
                .or_default()
                .push(unit);
        }
        let seasons: BTreeSet<u16> = by_coverage
            .keys()
            .filter_map(|(season, _, _, _)| *season)
            .collect();
        for season in seasons.into_iter().map(Some).chain([None]) {
            let season_path = season.and_then(|season| season_folder_of(rules, units, season));
            if let Some(season) = season {
                self.add_season(rules, &facts, season, season_path.clone());
            }
            for (_, episode_units) in by_coverage
                .iter()
                .filter(|((unit_season, _, _, _), _)| *unit_season == season)
            {
                self.add_episode(
                    rules,
                    Some(&facts),
                    season_path.as_deref(),
                    episode_units,
                    owner_of_video,
                );
            }
        }
    }

    fn add_season(
        &mut self,
        rules: &Rules,
        series: &SeriesFacts,
        season: u16,
        path: Option<String>,
    ) {
        let provider_ids = path
            .as_deref()
            .map(|folder| {
                let parent = parent_folder(folder).map(file_name);
                rules
                    .parse_season_folder(file_name(folder), parent, parent.is_some())
                    .provider_ids
            })
            .unwrap_or_default();
        self.push_owner(
            TvOwner::Season(self.seasons.len()),
            &series.block_key,
            OwnerCandidate {
                owning_folder: path.clone(),
                media_folders: BTreeSet::new(),
                names: Vec::new(),
                title_words: Vec::new(),
                year: None,
                is_folder_identity: true,
                takes_folder_assets: true,
            },
        );
        self.seasons.push(Season {
            series_name: series.name.clone(),
            series_path: series.path.clone(),
            season,
            path,
            provider_ids,
            extras: Vec::new(),
            artwork: Vec::new(),
            nfo: None,
        });
    }

    /// Adds one episode item for units with the same coverage. The units' files are its
    /// versions.
    fn add_episode(
        &mut self,
        rules: &Rules,
        series: Option<&SeriesFacts>,
        season_path: Option<&str>,
        units: &[&Unit],
        owner_of_video: &mut BTreeMap<String, usize>,
    ) {
        let first = units[0];
        let episode_folders: BTreeSet<Option<&String>> = units
            .iter()
            .map(|unit| unit.reading.layout.episode_folder.as_ref())
            .collect();
        let path = match episode_folders.into_iter().collect::<Vec<_>>().as_slice() {
            [Some(folder)] => Some((*folder).clone()),
            _ => None,
        };
        let provider_ids = merge_provider_claims(
            units.iter().map(|unit| &unit.reading.name.provider_ids),
            crate::model::CONFLICTING_PROVIDER_ID,
        );
        let mut ambiguity_reasons: Vec<&'static str> = Vec::new();
        if let Finding::Ambiguous { reason, .. } = first.reading.name.date {
            ambiguity_reasons.push(reason);
        }
        if crate::model::has_ambiguous_value(&provider_ids) {
            ambiguity_reasons.push(crate::model::CONFLICTING_PROVIDER_ID);
        }
        ambiguity_reasons.extend(units.iter().filter_map(|unit| unit.part_ambiguity));
        let hints: Vec<AppliedHint> = units
            .iter()
            .flat_map(|unit| unit.hints.iter().cloned())
            .collect();
        if hints
            .iter()
            .any(|applied| applied.outcome == HintOutcome::Contradicted)
        {
            ambiguity_reasons.push("hint_contradicts_name");
        }
        if units.iter().any(|unit| unit.numbering_contradicts_season) {
            ambiguity_reasons.push("numbering_contradicts_season");
        }
        ambiguity_reasons.dedup();

        let videos: Vec<&String> = units.iter().flat_map(|unit| &unit.paths).collect();
        let media_folders: BTreeSet<String> = videos
            .iter()
            .map(|video| parent_folder(video).unwrap_or_default().to_owned())
            .collect();
        let mut home_folders = media_folders.clone();
        home_folders.extend(season_path.map(str::to_owned));
        home_folders.extend(series.and_then(|series| series.path.clone()));
        let mut names: Vec<Vec<String>> = units
            .iter()
            .map(|unit| lexical::comparison_words(&unit.name))
            .collect();
        names.extend(videos.iter().map(|video| {
            lexical::comparison_words(lexical::stem(file_name(video), EntryKind::File))
        }));
        for video in &videos {
            owner_of_video.insert((*video).clone(), self.candidates.len());
        }
        let block_key = series.map_or_else(|| videos[0].clone(), |series| series.block_key.clone());
        self.push_owner(
            TvOwner::Episode(self.episodes.len()),
            &block_key,
            OwnerCandidate {
                owning_folder: path.clone(),
                media_folders,
                names,
                title_words: Vec::new(),
                year: None,
                is_folder_identity: path.is_some(),
                takes_folder_assets: path.is_some(),
            },
        );
        self.episode_home_folders.push(home_folders);
        let name = &first.reading.name;
        let versions = ordered_versions(rules, units);
        self.episodes.push(Episode {
            series_name: series.and_then(|series| series.name.clone()),
            series_year: series.and_then(|series| series.year),
            series_path: series.and_then(|series| series.path.clone()),
            season: name.season.clone(),
            episodes: name.episodes.clone(),
            first_episode: name.first_episode,
            absolute: name.absolute.clone(),
            date: name.date.clone(),
            episode_title: primary_unit(units, &versions)
                .reading
                .name
                .episode_title
                .clone(),
            path,
            provider_ids,
            tags: merge_provider_claims(
                units.iter().map(|unit| &unit.reading.name.tags),
                crate::model::CONFLICTING_TAG,
            ),
            versions,
            extras: Vec::new(),
            subtitles: Vec::new(),
            audio: Vec::new(),
            artwork: Vec::new(),
            nfo: None,
            hints,
            order: first.order,
            mapped_from: first.mapped_from.clone(),
            special: name.special.clone(),
            ambiguous: !ambiguity_reasons.is_empty(),
            ambiguity_reasons,
        });
    }

    /// Compares the hints on series folders with the series' title, year and provider IDs
    /// (T20), and copies a hinted title or year to the series' seasons and episodes. A
    /// contradicted hint makes the series' episodes ambiguous.
    fn apply_series_hints(&mut self, rules: &Rules, hints: &Hints) {
        for series in &mut self.series {
            let Some(hint) = series.path.as_deref().and_then(|path| hints.at(path)) else {
                continue;
            };
            if let Some(title) = &hint.title {
                let agrees = series.title.as_deref().is_some_and(|named| {
                    lexical::title_comparison_key(rules, named)
                        == lexical::title_comparison_key(rules, title)
                });
                let applied = settle(hint, "title", title.clone(), series.title.clone(), agrees);
                if applied.outcome.takes_hint() {
                    series.title = Some(title.clone());
                }
                series.hints.push(applied);
            }
            if let Some(year) = hint.year {
                let applied = settle(
                    hint,
                    "year",
                    year.to_string(),
                    series.year.map(|named| named.to_string()),
                    series.year == Some(year),
                );
                if applied.outcome.takes_hint() {
                    series.year = Some(year);
                }
                series.hints.push(applied);
            }
            for (provider, id) in &hint.provider_ids {
                let provider = lexical::vocabulary_form(provider);
                let named = series
                    .provider_ids
                    .get(&provider)
                    .and_then(Finding::resolved_value)
                    .cloned();
                let fact = format!("provider_id.{provider}");
                let applied = settle(
                    hint,
                    &fact,
                    id.clone(),
                    named.clone(),
                    named.as_ref() == Some(id),
                );
                match applied.outcome {
                    HintOutcome::Filled | HintOutcome::Overrode => {
                        series.provider_ids.insert(
                            provider,
                            Finding::Resolved {
                                value: id.clone(),
                                evidence: None,
                            },
                        );
                    }
                    HintOutcome::Contradicted => {
                        series.provider_ids.insert(
                            provider,
                            Finding::Ambiguous {
                                candidates: named.into_iter().chain([id.clone()]).collect(),
                                reason: "hint_contradicts_name",
                            },
                        );
                    }
                    HintOutcome::Agreed => {}
                }
                series.hints.push(applied);
            }
            let contradicted = series
                .hints
                .iter()
                .any(|applied| applied.outcome == HintOutcome::Contradicted);
            for season in &mut self.seasons {
                if season.series_path == series.path {
                    season.series_name = series.title.clone();
                }
            }
            for episode in &mut self.episodes {
                if episode.series_path == series.path {
                    episode.series_name = series.title.clone();
                    episode.series_year = series.year;
                    if contradicted {
                        episode.ambiguity_reasons.push("hint_contradicts_name");
                        episode.ambiguous = true;
                    }
                }
            }
        }
    }

    fn push_owner(&mut self, owner: TvOwner, block_key: &str, candidate: OwnerCandidate) {
        self.owners.push(owner);
        self.candidates.push(candidate);
        self.block_keys.push(block_key.to_owned());
    }

    fn extras_of(&mut self, owner: TvOwner) -> &mut Vec<Extra> {
        match owner {
            TvOwner::Series(index) => &mut self.series[index].extras,
            TvOwner::Season(index) => &mut self.seasons[index].extras,
            TvOwner::Episode(index) => &mut self.episodes[index].extras,
        }
    }

    fn artwork_of(&mut self, owner: TvOwner) -> &mut Vec<Artwork> {
        match owner {
            TvOwner::Series(index) => &mut self.series[index].artwork,
            TvOwner::Season(index) => &mut self.seasons[index].artwork,
            TvOwner::Episode(index) => &mut self.episodes[index].artwork,
        }
    }

    fn nfo_of(&mut self, owner: TvOwner) -> &mut Option<String> {
        match owner {
            TvOwner::Series(index) => &mut self.series[index].nfo,
            TvOwner::Season(index) => &mut self.seasons[index].nfo,
            TvOwner::Episode(index) => &mut self.episodes[index].nfo,
        }
    }

    /// Returns the one series (`tvshow`) or season (`season`) whose folder is `folder` (S8,
    /// S9). Returns `None` if there is no such item or there are several.
    fn folder_owner(
        &self,
        candidates: &OwnerCandidates,
        role: NfoRole,
        folder: &str,
    ) -> Option<usize> {
        let owners: Vec<usize> = candidates
            .owned_by(folder)
            .iter()
            .copied()
            .filter(|&candidate| {
                matches!(
                    (role, self.owners[candidate]),
                    (NfoRole::TvShow, TvOwner::Series(_)) | (NfoRole::Season, TvOwner::Season(_))
                )
            })
            .collect();
        let [owner] = owners[..] else {
            return None;
        };
        Some(owner)
    }

    fn sort_attachments(&mut self) {
        for owner in self.owners.clone() {
            self.extras_of(owner)
                .sort_by(|left, right| left.paths.cmp(&right.paths));
            self.artwork_of(owner)
                .sort_by(|left, right| left.path.cmp(&right.path));
        }
        for episode in &mut self.episodes {
            episode
                .subtitles
                .sort_by(|left, right| left.path.cmp(&right.path));
            episode
                .audio
                .sort_by(|left, right| left.path.cmp(&right.path));
        }
    }

    /// Returns the extras keyed by owner: a series' or season's folder, or an episode's primary
    /// video.
    fn extras_by_owner(&self) -> BTreeMap<String, Vec<Extra>> {
        let mut extras_of = BTreeMap::new();
        for owner in &self.owners {
            let (key, extras) = match *owner {
                TvOwner::Series(index) => {
                    (self.series[index].path.clone(), &self.series[index].extras)
                }
                TvOwner::Season(index) => (
                    self.seasons[index].path.clone(),
                    &self.seasons[index].extras,
                ),
                TvOwner::Episode(index) => (
                    Some(self.episodes[index].versions[0].paths[0].clone()),
                    &self.episodes[index].extras,
                ),
            };
            if let Some(key) = key
                && !extras.is_empty()
            {
                extras_of.insert(key, extras.clone());
            }
        }
        extras_of
    }

    /// Returns the items in assembly order, grouped into blocks by their sort key.
    fn into_item_blocks(self) -> Vec<(String, Vec<Item>)> {
        let mut series = self.series.into_iter();
        let mut seasons = self.seasons.into_iter();
        let mut episodes = self.episodes.into_iter();
        let mut blocks: Vec<(String, Vec<Item>)> = Vec::new();
        for (owner, block_key) in self.owners.into_iter().zip(self.block_keys) {
            let item = match owner {
                TvOwner::Series(_) => Item::Series(Box::new(series.next().expect("in order"))),
                TvOwner::Season(_) => Item::Season(Box::new(seasons.next().expect("in order"))),
                TvOwner::Episode(_) => Item::Episode(Box::new(episodes.next().expect("in order"))),
            };
            match blocks.last_mut() {
                Some((key, items)) if *key == block_key => items.push(item),
                _ => blocks.push((block_key, vec![item])),
            }
        }
        blocks
    }
}

/// Returns the one season folder that contains a season's episodes, among folders whose own
/// name contains that season or no season (`Season/`). Returns `None` if there is no such
/// folder or there are several.
fn season_folder_of(rules: &Rules, units: &[&Unit], season: u16) -> Option<String> {
    let folders: BTreeSet<&String> = units
        .iter()
        .filter(|unit| unit.season() == Some(season))
        .filter_map(|unit| unit.reading.layout.season_folder.as_ref())
        .filter(|folder| {
            let parent = parent_folder(folder).map(file_name);
            let own_season = rules
                .parse_season_folder(file_name(folder), parent, parent.is_some())
                .season;
            own_season.resolved_value().is_none_or(|own| *own == season)
        })
        .collect();
    let [folder] = folders.into_iter().collect::<Vec<_>>()[..] else {
        return None;
    };
    Some(folder.clone())
}

/// Returns the unit of the primary version.
fn primary_unit<'a>(units: &[&'a Unit], versions: &[Version]) -> &'a Unit {
    units
        .iter()
        .find(|unit| unit.paths == versions[0].paths)
        .expect("every version is a unit")
}

/// The sort key of a version: unlabelled first, then resolution (highest first), label and
/// first path.
type VersionOrder = (bool, Reverse<u32>, Option<String>, String);

/// Orders the versions of one episode: the version with nothing after its episode reference
/// first, then by the resolution that its label starts with, highest first, then by label
/// and path. A single version has no label.
fn ordered_versions(rules: &Rules, units: &[&Unit]) -> Vec<Version> {
    let mut versions: Vec<(VersionOrder, Version)> = units
        .iter()
        .map(|unit| {
            let label = unit.label(rules);
            let order = (
                label.is_some(),
                Reverse(label.as_deref().map_or(0, resolution_of)),
                label.as_deref().map(lexical::fold_case_text),
                unit.paths[0].clone(),
            );
            let version = Version {
                paths: unit.paths.clone(),
                label,
                ambiguity_reason: unit.part_ambiguity,
                edition: None,
                edition_hint: None,
                format_3d: None,
                nfo: None,
            };
            (order, version)
        })
        .collect();
    versions.sort_by(|left, right| left.0.cmp(&right.0));
    let has_one_version = versions.len() == 1;
    versions
        .into_iter()
        .map(|(_, mut version)| {
            if has_one_version {
                version.label = None;
            }
            version
        })
        .collect()
}

/// Attaches extras to the episode whose season and episode their title contains (E5, T11),
/// or else to the item that their title matches or to their folder's series or season
/// (E11). The other extras stay visible as orphans.
fn attach_extras(
    known: &KnownSeasons,
    rules: &Rules,
    index: &EntryIndex,
    assembly: &mut Assembly,
    candidates: &OwnerCandidates,
    claimed_files: &mut BTreeSet<String>,
) -> Vec<OrphanExtra> {
    let episodes_by_home = episodes_by_home_folder(assembly);
    let mut orphans = Vec::new();
    for (path, role) in &index.files {
        let FileRole::Extra {
            kind,
            title,
            owner_folder,
            owner_is_forced,
        } = role
        else {
            continue;
        };
        claimed_files.insert(path.clone());
        let at_coordinates = episode_at_coordinates(
            known,
            rules,
            assembly,
            &episodes_by_home,
            path,
            title.as_deref(),
            owner_folder.as_deref(),
        );
        let (extra, choice) = match at_coordinates {
            // The episode code identifies the owner, and the text after it is the title.
            Some((candidate, title_after_code)) => (
                associations::extra_file(rules, path, *kind, title_after_code.as_deref()),
                OwnerChoice::Owner(candidate),
            ),
            None => (
                associations::extra_file(rules, path, *kind, title.as_deref()),
                associations::choose_extra_owner(
                    rules,
                    owner_folder.as_deref(),
                    title.as_deref(),
                    *owner_is_forced,
                    candidates,
                ),
            ),
        };
        match choice {
            OwnerChoice::Owner(candidate) => {
                let owner = assembly.owners[candidate];
                assembly.extras_of(owner).push(extra);
            }
            choice => orphans.push(OrphanExtra {
                paths: extra.paths,
                extra: extra.extra,
                title: extra.title,
                year: extra.year,
                ambiguous: choice == OwnerChoice::Tied,
            }),
        }
    }
    orphans
}

/// Returns, per home folder, the episodes (candidate and episode index) whose extras the
/// folder can contain, in candidate order.
fn episodes_by_home_folder(assembly: &Assembly) -> BTreeMap<String, Vec<(usize, usize)>> {
    let mut by_home: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    for (candidate, owner) in assembly.owners.iter().enumerate() {
        let TvOwner::Episode(index) = *owner else {
            continue;
        };
        for home in &assembly.episode_home_folders[index] {
            by_home
                .entry(home.clone())
                .or_default()
                .push((candidate, index));
        }
    }
    by_home
}

/// Returns the candidate of the one episode near the extra whose season and episodes the
/// extra's title contains (`Show S01E05-deleted`, `Extras/S01E01 - Behind The Scenes`), and
/// the text after the episode code (`Behind The Scenes`). Returns `None` if the title has no
/// episode code or no single episode matches.
fn episode_at_coordinates(
    known: &KnownSeasons,
    rules: &Rules,
    assembly: &Assembly,
    episodes_by_home: &BTreeMap<String, Vec<(usize, usize)>>,
    path: &str,
    title: Option<&str>,
    owner_folder: Option<&str>,
) -> Option<(usize, Option<String>)> {
    let title = title?;
    let reading_path = match parent_folder(path) {
        Some(folder) => format!("{folder}/{title}.mkv"),
        None => format!("{title}.mkv"),
    };
    let reading = read_episode(known, rules, &reading_path, EntryKind::File, None);
    let Finding::Resolved {
        value: season,
        evidence: Some(Evidence {
            rule: Rule::EpisodeToken,
            ..
        }),
    } = reading.name.season
    else {
        return None;
    };
    let episodes = reading.name.episodes?;
    let home = owner_folder.unwrap_or_default();
    let matching: Vec<usize> = episodes_by_home
        .get(home)
        .into_iter()
        .flatten()
        .filter(|(_, index)| {
            let episode = &assembly.episodes[*index];
            episode.season.resolved_value() == Some(&season)
                && episode.episodes.as_ref() == Some(&episodes)
        })
        .map(|(candidate, _)| *candidate)
        .collect();
    let [candidate] = matching[..] else {
        return None;
    };
    Some((candidate, reading.name.episode_title))
}
