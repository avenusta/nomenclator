//! Runs the movie pipeline (passes 4-5). The pipeline resolves which folder owns each video
//! (M1), which videos are one work and which name identifies the work (M2, M3, M8, M10), how
//! its versions and parts are ordered (M11, M13), and which videos are misplaced series
//! content (M15). Extras and sidecars are then attached through `associations`.

use crate::model::AmbiguityReason;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use super::associations::{self, OwnerCandidate, OwnerCandidates, OwnerChoice};
use super::{
    EntryIndex, FileRole, Item, Movie, OrphanExtra, PipelineOutput, Version,
    collapse_same_named_wrappers, merge_provider_claims, parent_folder, resolution_of,
    sorted_by_path, unwrap_single_bracket, without_provider_and_edition_tags,
};
use crate::inputs::{HintOutcome, Hints, Overrides, settle_identity};
use crate::lexical;
use crate::model::{ArtworkRole, EntryKind, Finding, NfoRole};
use crate::parsers::episode_name::is_season_folder;
use crate::parsers::file_types::file_name;
use crate::parsers::movie_name::MovieName;
use crate::rules::Rules;

/// One representation of a work: a video, a stack of parts, or a disc structure.
struct Unit {
    /// One video, the ordered parts, or a disc folder ending in `/`.
    paths: Vec<String>,
    /// Its name: a video stem, a stack name, or a disc folder's name.
    name: String,
    /// The folder that contains it; `None` at the library root.
    folder: Option<String>,
    parsed: MovieName,
    part_ambiguity: Option<AmbiguityReason>,
}

impl Unit {
    fn title(&self) -> Option<&str> {
        finding_value(&self.parsed.title).map(String::as_str)
    }

    fn year(&self) -> Option<u16> {
        finding_value(&self.parsed.year).copied()
    }

    fn title_words(&self) -> Vec<String> {
        self.title()
            .map(lexical::comparison_words)
            .unwrap_or_default()
    }

    fn has_episode_marker(&self, rules: &Rules) -> bool {
        lexical::has_episode_marker(rules, &self.name)
    }

    /// Returns `true` if the unit is an episode-coded video directly in a season folder (M15).
    fn is_in_season_folder(&self, rules: &Rules) -> bool {
        self.has_episode_marker(rules)
            && self.paths.iter().any(|path| {
                parent_folder(path).is_some_and(|folder| is_season_folder(rules, folder))
            })
    }
}

fn finding_value<T>(finding: &Finding<T>) -> Option<&T> {
    match finding {
        Finding::Resolved { value, .. } => Some(value),
        _ => None,
    }
}

/// The folder that owns a unit, with its parsed name.
struct Owner {
    /// The owning folder. A same-named wrapper folder above it is collapsed into it.
    path: String,
    /// The name that units are compared with: the name of the folder that directly contains
    /// them.
    name: String,
    parsed: MovieName,
}

impl Owner {
    /// Returns `true` if the folder name has a year or a provider ID. Such a folder owns
    /// everything it contains (M1) and gives the work its name (M2).
    fn is_anchored(&self) -> bool {
        finding_value(&self.parsed.year).is_some() || !self.parsed.provider_ids.is_empty()
    }

    fn title_words(&self) -> Vec<String> {
        finding_value(&self.parsed.title)
            .map(|title| lexical::comparison_words(title))
            .unwrap_or_default()
    }

    /// Returns the position where the owner's name ends in `stem`. Returns `None` unless
    /// `stem` starts with the owner's name and then ends or continues with a version
    /// separator (`-`, `_`, `.`, `[`, ` -`, ` [`).
    fn name_end_in(&self, stem: &str) -> Option<usize> {
        let length = lexical::caseless_prefix_length(stem, &self.name)?;
        let rest = &stem[length..];
        let continues_with_label = rest.is_empty()
            || rest.starts_with(['-', '_', '.', '['])
            || rest.starts_with(" -")
            || rest.starts_with(" [");
        continues_with_label.then_some(length)
    }

    /// Returns `true` if the unit is the folder's own work: the unit's name starts with the
    /// folder's name, the unit has no title, or the unit's title agrees with the folder's.
    fn is_own_work(&self, rules: &Rules, unit: &Unit) -> bool {
        self.name_end_in(&unit.name).is_some()
            || unit.title().is_none()
            || names_agree(rules, &self.parsed, &unit.parsed)
    }

    /// Returns `true` if the unit is a trusted file whose title or year differs from the
    /// folder's (M1, M2). A title that only repeats the folder's year (`Title 2008 (2008)`)
    /// does not differ (M3).
    fn is_contradicted_by(&self, rules: &Rules, unit: &Unit) -> bool {
        if !unit.parsed.trusted || self.name_end_in(&unit.name).is_some() {
            return false;
        }
        let folder_year = finding_value(&self.parsed.year).copied();
        let folder_words = self.title_words();
        let unit_words = unit.title_words();
        let repeats_folder_year = folder_year.is_some_and(|year| {
            unit.year() == Some(year)
                && unit_words.split_last().is_some_and(|(last, rest)| {
                    *last == year.to_string() && rest == folder_words.as_slice()
                })
        });
        let titles_differ = match (finding_value(&self.parsed.title), unit.title()) {
            (Some(_), Some(_)) => {
                !names_agree(rules, &self.parsed, &unit.parsed) && !repeats_folder_year
            }
            _ => false,
        };
        let years_differ =
            matches!((folder_year, unit.year()), (Some(folder), Some(own)) if folder != own);
        titles_differ || years_differ
    }
}

/// Returns `true` if a folder name and a file name name the same work (M1, M16): any of
/// the folder's title and alternate titles agrees with any of the file's.
fn names_agree(rules: &Rules, folder: &MovieName, file: &MovieName) -> bool {
    let titles = |name: &MovieName| -> Vec<String> {
        finding_value(&name.title)
            .cloned()
            .into_iter()
            .chain(
                name.alternate_titles
                    .iter()
                    .map(|alternate| alternate.title.clone()),
            )
            .collect()
    };
    let (folder_titles, file_titles) = (titles(folder), titles(file));
    folder_titles.iter().any(|folder_title| {
        file_titles
            .iter()
            .any(|file_title| lexical::titles_agree(rules, folder_title, file_title))
    })
}

/// A movie during assembly: its units and the sources of its identity.
struct Work<'a> {
    units: Vec<&'a Unit>,
    owner: Option<&'a Owner>,
    title: Option<(String, String)>,
    year: Option<(u16, String)>,
    is_folder_identity: bool,
    ambiguity_reasons: Vec<AmbiguityReason>,
}

pub(super) fn interpret(
    rules: &Rules,
    index: &EntryIndex,
    hints: &Hints,
    overrides: &Overrides,
) -> PipelineOutput {
    let units = collect_units(rules, index);
    let owners: Vec<Option<Owner>> = units.iter().map(|unit| owner_of(rules, unit)).collect();
    let mut works = group_into_works(rules, &units, &owners);
    apply_structure_overrides(&mut works, overrides);
    works.sort_by_key(|work| work_sort_key(work));

    let mut items: Vec<Movie> = works
        .iter()
        .map(|work| assemble_movie(rules, work))
        .collect();
    for movie in &mut items {
        apply_hints(rules, hints, movie);
    }
    set_collections(&mut items);
    let candidates = OwnerCandidates::new(works.iter().map(owner_candidate).collect());
    let owner_of_video: BTreeMap<String, usize> = works
        .iter()
        .enumerate()
        .flat_map(|(position, work)| {
            work.units
                .iter()
                .flat_map(|unit| unit.paths.iter())
                .map(move |path| (path.clone(), position))
        })
        .collect();

    let mut claimed_files: BTreeSet<String> = BTreeSet::new();
    claimed_files.extend(owner_of_video.keys().cloned());
    let orphans = attach_extras(rules, index, &candidates, &mut items, &mut claimed_files);
    let sidecars = associations::attach_sidecars(rules, index);
    claimed_files.extend(sidecars.claimed_files.iter().cloned());
    for (video, subtitles) in &sidecars.subtitles_by_video {
        if let Some(&owner) = owner_of_video.get(video) {
            items[owner].subtitles.extend(subtitles.iter().cloned());
        }
    }
    for (video, audio) in &sidecars.audio_by_video {
        if let Some(&owner) = owner_of_video.get(video) {
            items[owner].audio.extend(audio.iter().cloned());
        }
    }
    let (artwork, artwork_files) = associations::attach_artwork(
        rules,
        index,
        &candidates,
        &owner_of_video,
        &sidecars.stills_by_video,
        ArtworkRole::Thumb,
    );
    claimed_files.extend(artwork_files);
    for (owner, artwork) in artwork {
        items[owner].artwork.extend(artwork);
    }
    let nfos = associations::attach_nfos(rules, index, &owner_of_video, |role, folder| {
        match (role, associations::choose_asset_owner(folder, &candidates)) {
            (NfoRole::Movie, OwnerChoice::Owner(owner)) => Some(owner),
            _ => None,
        }
    });
    for (owner, movie) in items.iter_mut().enumerate() {
        for version in &mut movie.versions {
            version.nfo = nfos.of_version(&version.paths);
            claimed_files.extend(version.nfo.clone());
        }
        movie.nfo = movie.versions[0]
            .nfo
            .clone()
            .or_else(|| nfos.generic_by_owner.get(&owner).cloned());
        claimed_files.extend(movie.nfo.clone());
    }
    claimed_files.extend(associations::carried_files(index, &claimed_files));

    for movie in &mut items {
        movie
            .extras
            .sort_by(|left, right| left.paths.cmp(&right.paths));
        movie
            .subtitles
            .sort_by(|left, right| left.path.cmp(&right.path));
        movie
            .audio
            .sort_by(|left, right| left.path.cmp(&right.path));
        movie
            .artwork
            .sort_by(|left, right| left.path.cmp(&right.path));
    }
    let extras_of = items
        .iter()
        .filter(|movie| !movie.extras.is_empty())
        .map(|movie| (primary_path(movie).to_owned(), movie.extras.clone()))
        .collect();
    let mut all_items: Vec<Item> = items
        .into_iter()
        .map(|movie| Item::Movie(Box::new(movie)))
        .collect();
    all_items.extend(orphans.into_iter().map(Item::Extra));
    all_items.sort_by(|left, right| item_sort_key(left).cmp(&item_sort_key(right)));
    // NUL separates the parts, because no valid path contains it, so the keys sort like
    // the pairs.
    let item_keys = all_items
        .iter()
        .map(|item| {
            let (first, second) = item_sort_key(item);
            format!("{first}\0{second}")
        })
        .collect();

    PipelineOutput {
        items: all_items,
        item_keys,
        extras_of,
        video_subtitles: sorted_by_path(sidecars.subtitles_by_video),
        claimed_files,
    }
}

/// Attaches extras to the item that their title matches or to their folder's work (E11).
/// The other extras stay visible as orphans.
fn attach_extras(
    rules: &Rules,
    index: &EntryIndex,
    candidates: &OwnerCandidates,
    items: &mut [Movie],
    claimed_files: &mut BTreeSet<String>,
) -> Vec<OrphanExtra> {
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
        let extra = associations::extra_file(rules, path, *kind, title.as_deref());
        match associations::choose_extra_owner(
            rules,
            owner_folder.as_deref(),
            title.as_deref(),
            *owner_is_forced,
            candidates,
        ) {
            OwnerChoice::Owner(owner) => items[owner].extras.push(extra),
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

/// Collects the units: videos, part stacks, disc folders and disc images (M11, M13). A
/// folder is a part if it contains a disc structure or exactly one feature video.
fn collect_units(rules: &Rules, index: &EntryIndex) -> Vec<Unit> {
    let feature_files: Vec<String> = index
        .files
        .iter()
        .filter(|(_, role)| matches!(role, FileRole::FeatureVideo | FileRole::DiscImage))
        .map(|(path, _)| path.clone())
        .collect();
    let mut features_by_folder: BTreeMap<&str, Vec<&String>> = BTreeMap::new();
    for path in &feature_files {
        if let Some(folder) = parent_folder(path) {
            features_by_folder.entry(folder).or_default().push(path);
        }
    }
    let mut folder_part_content: BTreeMap<String, String> = BTreeMap::new();
    for folder in &index.folders {
        if index.disc_folders.contains(folder) {
            folder_part_content.insert(format!("{folder}/"), format!("{folder}/"));
            continue;
        }
        let videos = features_by_folder
            .get(folder.as_str())
            .map_or(&[][..], Vec::as_slice);
        if let [only] = videos {
            folder_part_content.insert(format!("{folder}/"), (*only).clone());
        }
    }
    let stack_candidates: Vec<String> = feature_files
        .iter()
        .chain(folder_part_content.keys())
        .cloned()
        .collect();

    let mut units = Vec::new();
    let mut consumed: BTreeSet<String> = BTreeSet::new();
    for stack in rules.group_parts(&stack_candidates).stacks {
        let is_folder_stack = stack.paths[0].ends_with('/');
        let paths: Vec<String> = if is_folder_stack {
            stack
                .paths
                .iter()
                .map(|folder| folder_part_content[folder].clone())
                .collect()
        } else {
            stack.paths.clone()
        };
        let parsed = if is_folder_stack {
            rules.parse_movie_name(&stack.name, EntryKind::Directory)
        } else {
            let extension = lexical::extension(file_name(&paths[0]), EntryKind::File);
            let name = format!("{}.{}", stack.name, extension.unwrap_or_default());
            rules.parse_movie_name(&name, EntryKind::File)
        };
        consumed.extend(paths.iter().cloned());
        units.push(Unit {
            folder: parent_folder(&stack.paths[0]).map(str::to_owned),
            name: stack.name,
            paths,
            parsed,
            part_ambiguity: stack.ambiguity_reason,
        });
    }
    for path in feature_files
        .iter()
        .filter(|path| !consumed.contains(*path))
    {
        let name = file_name(path);
        units.push(Unit {
            paths: vec![path.clone()],
            name: lexical::stem(name, EntryKind::File).to_owned(),
            folder: parent_folder(path).map(str::to_owned),
            parsed: rules.parse_movie_name(name, EntryKind::File),
            part_ambiguity: None,
        });
    }
    for folder in &index.disc_folders {
        let disc = format!("{folder}/");
        if consumed.contains(&disc) {
            continue;
        }
        let name = file_name(folder);
        units.push(Unit {
            paths: vec![disc],
            name: name.to_owned(),
            folder: Some(folder.clone()),
            parsed: rules.parse_movie_name(name, EntryKind::Directory),
            part_ambiguity: None,
        });
    }
    units.sort_by(|left, right| left.paths.cmp(&right.paths));
    units
}

/// Returns the folder that owns a unit. A folder owns a unit if the folder name has a
/// provider ID or a year, if its title is a prefix of the unit's title or name, or if the
/// unit's name has no title (M1). Returns `None` if no folder owns the unit.
fn owner_of(rules: &Rules, unit: &Unit) -> Option<Owner> {
    let folder = unit.folder.as_deref()?;
    let name = file_name(folder);
    let owner = Owner {
        path: collapse_same_named_wrappers(folder).to_owned(),
        name: name.to_owned(),
        parsed: rules.parse_movie_name(name, EntryKind::Directory),
    };
    let folder_words = owner.title_words();
    let title_is_prefixed =
        !folder_words.is_empty() && unit.title_words().starts_with(&folder_words);
    let owns = owner.is_anchored()
        || title_is_prefixed
        || owner.name_end_in(&unit.name).is_some()
        || unit.title().is_none();
    owns.then_some(owner)
}

/// Groups units into works (M10). An anchored folder contains one work. Another owning
/// folder contains its own work, split by differing years, and the works stored in it under
/// their own titles. Loose units are one work if their title and year agree.
fn group_into_works<'a>(
    rules: &Rules,
    units: &'a [Unit],
    owners: &'a [Option<Owner>],
) -> Vec<Work<'a>> {
    let mut by_owner: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut loose: Vec<&'a Unit> = Vec::new();
    for (position, unit) in units.iter().enumerate() {
        match &owners[position] {
            Some(owner) => by_owner.entry(&owner.path).or_default().push(position),
            None => loose.push(unit),
        }
    }

    let mut works = Vec::new();
    for positions in by_owner.values() {
        let owner = owners[positions[0]].as_ref().expect("grouped by owner");
        let owned: Vec<&Unit> = positions.iter().map(|&position| &units[position]).collect();
        if owner.is_anchored() {
            works.push(anchored_work(rules, owner, owned));
        } else {
            works.extend(titled_folder_works(rules, owner, owned));
        }
    }
    for group in group_by_identity(loose) {
        works.push(work_named_by_unit(None, group));
    }
    works
}

/// Applies the caller's split and merge overrides (X16). A split video leaves its work and
/// becomes a work of its own, named by its own name and owned by no folder. Merged videos
/// join the work of the first video listed, which keeps its identity.
fn apply_structure_overrides<'a>(works: &mut Vec<Work<'a>>, overrides: &Overrides) {
    let holds = |unit: &Unit, paths: &[String]| unit.paths.iter().any(|path| paths.contains(path));
    let mut split_off: Vec<Work<'a>> = Vec::new();
    for work in works.iter_mut() {
        if work.units.len() < 2 {
            continue;
        }
        let (split, kept): (Vec<&Unit>, Vec<&Unit>) = work
            .units
            .iter()
            .partition(|unit| holds(unit, &overrides.split));
        if !split.is_empty() && !kept.is_empty() {
            work.units = kept;
            split_off.extend(
                split
                    .into_iter()
                    .map(|unit| work_named_by_unit(None, vec![unit])),
            );
        }
    }
    works.extend(split_off);
    for group in &overrides.merge {
        let position_of = |works: &[Work], path: &String| {
            works
                .iter()
                .position(|work| work.units.iter().any(|unit| unit.paths.contains(path)))
        };
        while let Some(target) = group.iter().find_map(|path| position_of(works, path)) {
            let Some(other) = group
                .iter()
                .filter_map(|path| position_of(works, path))
                .find(|&position| position != target)
            else {
                break;
            };
            let work = works.remove(other);
            let target = if other < target { target - 1 } else { target };
            works[target].units.extend(work.units);
        }
    }
}

/// Builds the one work of a folder with a year or provider ID. The work's name comes from
/// the folder. A missing year comes from the videos that agree with the folder's title. A
/// trusted video that contradicts the folder makes the work ambiguous, unless the folder has
/// a provider ID.
fn anchored_work<'a>(rules: &Rules, owner: &'a Owner, units: Vec<&'a Unit>) -> Work<'a> {
    let title = finding_value(&owner.parsed.title)
        .map(|title| (title.clone(), owner.path.clone()))
        .or_else(|| {
            units
                .iter()
                .find_map(|unit| Some((unit.title()?.to_owned(), unit.paths[0].clone())))
        });
    let year = finding_value(&owner.parsed.year)
        .map(|year| (*year, owner.path.clone()))
        .or_else(|| {
            let agreeing: Vec<&&Unit> = units
                .iter()
                .filter(|unit| unit.year().is_some() && owner.is_own_work(rules, unit))
                .collect();
            unique_year(&agreeing)
        });
    let is_contradicted = owner.parsed.provider_ids.is_empty()
        && units
            .iter()
            .any(|unit| owner.is_contradicted_by(rules, unit));
    let mut work = Work {
        units,
        owner: Some(owner),
        title,
        year,
        is_folder_identity: true,
        ambiguity_reasons: Vec::new(),
    };
    if is_contradicted {
        work.ambiguity_reasons
            .push(AmbiguityReason::ContradictsOwningFolder);
    }
    work
}

/// Builds the works of a folder that owns units by title only: its own work, one work per
/// year if its videos contain several years, and the works stored in it under longer
/// titles.
fn titled_folder_works<'a>(rules: &Rules, owner: &'a Owner, units: Vec<&'a Unit>) -> Vec<Work<'a>> {
    let (own, others): (Vec<&Unit>, Vec<&Unit>) = units
        .into_iter()
        .partition(|unit| owner.is_own_work(rules, unit));
    let years: BTreeSet<u16> = own.iter().filter_map(|unit| unit.year()).collect();
    let mut own_groups: BTreeMap<Option<u16>, Vec<&Unit>> = BTreeMap::new();
    for unit in own {
        let year = if years.len() <= 1 { None } else { unit.year() };
        own_groups.entry(year).or_default().push(unit);
    }
    let mut works: Vec<Work> = own_groups
        .into_values()
        .map(|group| {
            let year = unique_year(&group.iter().collect::<Vec<_>>());
            Work {
                units: group,
                owner: Some(owner),
                title: finding_value(&owner.parsed.title)
                    .map(|title| (title.clone(), owner.path.clone())),
                year,
                is_folder_identity: true,
                ambiguity_reasons: Vec::new(),
            }
        })
        .collect();
    works.extend(
        group_by_identity(others)
            .into_iter()
            .map(|group| work_named_by_unit(Some(owner), group)),
    );
    works
}

/// Groups units in one folder with the same title and year into one work. A unit without a
/// year is a work of its own (M10).
fn group_by_identity(units: Vec<&Unit>) -> Vec<Vec<&Unit>> {
    let mut groups: BTreeMap<IdentityKey, Vec<&Unit>> = BTreeMap::new();
    for (position, unit) in units.into_iter().enumerate() {
        let key = IdentityKey {
            folder: unit.folder.clone(),
            title_words: unit.title_words(),
            year: unit.year(),
            undated_position: if unit.year().is_some() { 0 } else { position },
        };
        groups.entry(key).or_default().push(unit);
    }
    groups.into_values().collect()
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct IdentityKey {
    folder: Option<String>,
    title_words: Vec<String>,
    year: Option<u16>,
    /// A position that keeps each undated unit separate.
    undated_position: usize,
}

/// Builds a work named by its first unit, not by a folder.
fn work_named_by_unit<'a>(owner: Option<&'a Owner>, units: Vec<&'a Unit>) -> Work<'a> {
    let first = units[0];
    Work {
        title: first
            .title()
            .map(|title| (title.to_owned(), first.paths[0].clone())),
        year: first.year().map(|year| (year, first.paths[0].clone())),
        units,
        owner,
        is_folder_identity: false,
        ambiguity_reasons: Vec::new(),
    }
}

/// Returns the year that all dated units agree on, with the path of the first unit that
/// has it. Returns `None` if no unit has a year or the units have different years.
fn unique_year(units: &[&&Unit]) -> Option<(u16, String)> {
    let years: BTreeSet<u16> = units.iter().filter_map(|unit| unit.year()).collect();
    let [year] = years.into_iter().collect::<Vec<_>>()[..] else {
        return None;
    };
    let source = units.iter().find(|unit| unit.year() == Some(year))?;
    Some((year, source.paths[0].clone()))
}

fn work_sort_key(work: &Work) -> (Option<String>, Vec<String>) {
    (
        work.owner.map(|owner| owner.path.clone()),
        work.units[0].paths.clone(),
    )
}

fn assemble_movie(rules: &Rules, work: &Work) -> Movie {
    let owner_name = work.owner.map(|owner| owner.name.as_str());
    let mut versions: Vec<(VersionOrder, Version)> = work
        .units
        .iter()
        .map(|unit| {
            let label = version_label(rules, unit, work.owner);
            let order = VersionOrder {
                is_named_as_folder: owner_name.is_some_and(|name| {
                    lexical::fold_case_text(&unit.name) == lexical::fold_case_text(name)
                }),
                resolution: label.as_deref().map_or(0, resolution_of),
                label: label.as_deref().map(lexical::fold_case_text),
                first_path: unit.paths[0].clone(),
            };
            let folder_name = work.owner.map(|owner| &owner.parsed);
            let own_or_folder = |fact: fn(&MovieName) -> &Finding<String>| {
                finding_value(fact(&unit.parsed))
                    .or_else(|| folder_name.and_then(|folder| finding_value(fact(folder))))
                    .cloned()
            };
            let version = Version {
                paths: unit.paths.clone(),
                label,
                ambiguity_reason: unit.part_ambiguity,
                edition: own_or_folder(|name| &name.edition),
                edition_hint: own_or_folder(|name| &name.edition_hint),
                format_3d: finding_value(&unit.parsed.format_3d)
                    .or_else(|| folder_name.and_then(|folder| finding_value(&folder.format_3d)))
                    .copied(),
                nfo: None,
            };
            (order, version)
        })
        .collect();
    versions.sort_by(|left, right| left.0.sort_key().cmp(&right.0.sort_key()));
    let has_one_version = versions.len() == 1;
    let versions: Vec<Version> = versions
        .into_iter()
        .map(|(_, mut version)| {
            if has_one_version {
                version.label = None;
            }
            version
        })
        .collect();

    let provider_ids = merged_provider_ids(work);
    let is_anchored_by_provider = work
        .owner
        .is_some_and(|owner| !owner.parsed.provider_ids.is_empty());
    let mut ambiguity_reasons = work.ambiguity_reasons.clone();
    if !is_anchored_by_provider && work.units.iter().any(|unit| unit.has_episode_marker(rules)) {
        ambiguity_reasons.push(AmbiguityReason::EpisodeTokenInMovieLibrary);
    }
    if crate::model::has_ambiguous_value(&provider_ids) {
        ambiguity_reasons.push(AmbiguityReason::ConflictingProviderId);
    }
    ambiguity_reasons.extend(work.units.iter().filter_map(|unit| unit.part_ambiguity));
    ambiguity_reasons.dedup();

    let title_name = work.title.as_ref().and_then(|(_, source)| {
        work.owner
            .filter(|owner| &owner.path == source)
            .map(|owner| &owner.parsed)
            .or_else(|| {
                work.units
                    .iter()
                    .find(|unit| &unit.paths[0] == source)
                    .map(|unit| &unit.parsed)
            })
    });
    let alternate_titles = title_name
        .map(|name| {
            name.alternate_titles
                .iter()
                .map(|alternate| alternate.title.clone())
                .collect()
        })
        .unwrap_or_default();
    Movie {
        title: work.title.as_ref().map(|(title, _)| title.clone()),
        alternate_titles,
        title_source: work.title.as_ref().map(|(_, source)| source.clone()),
        year: work.year.as_ref().map(|(year, _)| *year),
        year_source: work.year.as_ref().map(|(_, source)| source.clone()),
        path: work.owner.map(|owner| owner.path.clone()),
        provider_ids,
        tags: merged_tags(work),
        versions,
        extras: Vec::new(),
        subtitles: Vec::new(),
        audio: Vec::new(),
        artwork: Vec::new(),
        nfo: None,
        hints: Vec::new(),
        collection: None,
        misplaced_series: work
            .units
            .iter()
            .any(|unit| unit.is_in_season_folder(rules)),
        ambiguous: !ambiguity_reasons.is_empty(),
        ambiguity_reasons,
    }
}

/// Sets each movie's collection (R10): the folder that contains the movie's folder, or its
/// video if it has no folder. A folder that is another movie's folder is not a collection.
fn set_collections(movies: &mut [Movie]) {
    let movie_folders: BTreeSet<String> = movies
        .iter()
        .filter_map(|movie| movie.path.clone())
        .collect();
    for movie in movies.iter_mut() {
        let container = match &movie.path {
            Some(folder) => parent_folder(folder).map(str::to_owned),
            None => movie
                .versions
                .first()
                .and_then(|version| version.paths.first())
                .and_then(|path| parent_folder(path.trim_end_matches('/')))
                .map(str::to_owned),
        };
        movie.collection = container.filter(|folder| !movie_folders.contains(folder));
    }
}

/// Compares the hints on the movie's folder, then on its videos, with the facts from its
/// names (T20). A hint supplies a fact that the names do not contain. A contradicted hint
/// makes the movie ambiguous unless the hint is authoritative.
fn apply_hints(rules: &Rules, hints: &Hints, movie: &mut Movie) {
    let video_paths = movie
        .versions
        .iter()
        .flat_map(|version| version.paths.iter().cloned());
    let paths: Vec<String> = movie.path.clone().into_iter().chain(video_paths).collect();
    for hint in paths.iter().filter_map(|path| hints.at(path)) {
        let applied = settle_identity(
            rules,
            hint,
            &mut movie.title,
            &mut movie.year,
            &mut movie.provider_ids,
        );
        for settled in &applied {
            // A hinted title or year comes from no name.
            if settled.outcome.takes_hint() && settled.fact == "title" {
                movie.title_source = None;
            }
            if settled.outcome.takes_hint() && settled.fact == "year" {
                movie.year_source = None;
            }
        }
        movie.hints.extend(applied);
    }
    if movie
        .hints
        .iter()
        .any(|applied| applied.outcome == HintOutcome::Contradicted)
    {
        movie
            .ambiguity_reasons
            .push(AmbiguityReason::HintContradictsName);
        movie.ambiguous = true;
    }
}

/// The sort order of versions: the version named exactly like its folder first, then by
/// resolution, highest first, then by label and path.
struct VersionOrder {
    is_named_as_folder: bool,
    resolution: u32,
    label: Option<String>,
    first_path: String,
}

impl VersionOrder {
    fn sort_key(&self) -> (bool, Reverse<u32>, Option<&String>, &String) {
        (
            !self.is_named_as_folder,
            Reverse(self.resolution),
            self.label.as_ref(),
            &self.first_path,
        )
    }
}

/// Returns the version label: the text after the owner's name, or after the unit's own
/// title and year (M10), without provider and edition tags, which are separate facts (R1).
/// `- 1080p` becomes `1080p`, `.directors.cut` becomes `directors cut`, and
/// `{tmdb-1} {edition-Cut} - 2160p` becomes `2160p`. Returns `None` if no other text
/// follows.
fn version_label(rules: &Rules, unit: &Unit, owner: Option<&Owner>) -> Option<String> {
    let stem = unit.name.as_str();
    let after_identity = [
        resolved_span_end(&unit.parsed.title),
        resolved_span_end(&unit.parsed.year),
    ]
    .into_iter()
    .flatten()
    .max()
    .map_or(0, |end| {
        end + usize::from(stem[end..].starts_with([')', ']']))
    });
    let start = owner
        .and_then(|owner| owner.name_end_in(stem))
        .unwrap_or(after_identity);
    let rest = without_provider_and_edition_tags(rules, stem.get(start..)?);
    let rest = unwrap_single_bracket(rules, rest.trim_matches([' ', '-', '_', '.']));
    (!rest.is_empty()).then(|| lexical::display_title(rules, rest))
}

fn resolved_span_end<T>(finding: &Finding<T>) -> Option<usize> {
    match finding {
        Finding::Resolved {
            evidence: Some(evidence),
            ..
        } => Some(evidence.span.1),
        _ => None,
    }
}

/// Merges the provider IDs of a work: the owning folder's IDs first, then the videos' IDs.
/// Returns an ambiguous finding that keeps every claim if one provider has different values
/// (M8).
fn merged_provider_ids(work: &Work) -> BTreeMap<String, Finding<String>> {
    let folder_ids = work.owner.map(|owner| &owner.parsed.provider_ids);
    let unit_ids = work.units.iter().map(|unit| &unit.parsed.provider_ids);
    merge_provider_claims(
        folder_ids.into_iter().chain(unit_ids),
        AmbiguityReason::ConflictingProviderId,
    )
}

/// Merges the `{key-value}` tags of a work: the owning folder's, then the videos'.
fn merged_tags(work: &Work) -> BTreeMap<String, Finding<String>> {
    let folder_tags = work.owner.map(|owner| &owner.parsed.tags);
    let unit_tags = work.units.iter().map(|unit| &unit.parsed.tags);
    merge_provider_claims(
        folder_tags.into_iter().chain(unit_tags),
        AmbiguityReason::ConflictingTag,
    )
}

fn owner_candidate(work: &Work) -> OwnerCandidate {
    let mut names: Vec<Vec<String>> = Vec::new();
    let mut media_folders = BTreeSet::new();
    for unit in &work.units {
        names.push(lexical::comparison_words(&unit.name));
        for path in &unit.paths {
            let stem = lexical::stem(file_name(path), EntryKind::File);
            names.push(lexical::comparison_words(stem));
            media_folders.insert(parent_folder(path).unwrap_or_default().to_owned());
        }
    }
    OwnerCandidate {
        owning_folder: work.owner.map(|owner| owner.path.clone()),
        media_folders,
        names,
        title_words: work
            .title
            .as_ref()
            .map(|(title, _)| lexical::comparison_words(title))
            .unwrap_or_default(),
        year: work.year.as_ref().map(|(year, _)| *year),
        is_folder_identity: work.is_folder_identity,
        takes_folder_assets: true,
    }
}

fn primary_path(movie: &Movie) -> &str {
    &movie.versions[0].paths[0]
}

fn item_sort_key(item: &Item) -> (&str, &str) {
    match item {
        Item::Movie(movie) => (
            movie.path.as_deref().unwrap_or(primary_path(movie)),
            primary_path(movie),
        ),
        Item::Extra(extra) => (&extra.paths[0], &extra.paths[0]),
        Item::Series(_) | Item::Season(_) | Item::Episode(_) => {
            unreachable!("the movie pipeline makes no series items")
        }
    }
}
