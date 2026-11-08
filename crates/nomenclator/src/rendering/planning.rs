//! Plans renames for an interpreted library. Each file gets the path that the profile
//! gives it, and the planner checks the proposals against the files that stay and
//! against each other (R6). The planner performs no filesystem operations. The caller
//! executes the renames, schedules temporary names and rolls back.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

use crate::ignore::IgnoreFile;
use crate::inputs::CallerInputs;
use crate::interpret::{Interpretation, parent_folder};
use crate::lexical;
use crate::parsers::extras::LibraryType;
use crate::parsers::file_types::file_name;
use crate::rules::OverlayDiagnostic;
use crate::rules::{Rules, RulesIdentity};

use super::targets::{FileTarget, SeriesKey, file_targets};
use super::{BlockReason, OutputProfile};

/// A rename plan: the proposals in execution order, and the renames that cannot be
/// proposed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RenamePlan {
    /// The proposals in execution order. A proposal whose destination is the source of
    /// another proposal comes after that proposal.
    pub proposals: Vec<RenameProposal>,
    /// The blocked renames, sorted by source path.
    pub blocked: Vec<BlockedRename>,
    /// The files that the plan proposes to remove instead of moving, sorted by path (R13).
    /// The caller decides whether to remove them.
    pub removals: Vec<ProposedRemoval>,
    /// The rules this result was produced under.
    pub rules: RulesIdentity,
}

impl Rules {
    /// Plans the conversion of a library that `from` wrote into the layout of `to`, as one
    /// rename plan (R18). The names are interpreted with these rules extended to read the
    /// names that `from` writes. Fails if `from` writes a name that the rules cannot read
    /// as its kind.
    #[allow(clippy::too_many_arguments)]
    pub fn plan_conversion(
        &self,
        library: LibraryType,
        paths: &[String],
        ignore_files: &[IgnoreFile],
        inputs: &CallerInputs,
        from: &OutputProfile,
        to: &OutputProfile,
    ) -> Result<RenamePlan, Vec<OverlayDiagnostic>> {
        let rules = self.reading_profile(from)?;
        let interpretation = rules.interpret_with(library, paths, ignore_files, inputs);
        Ok(rules.plan_renames(&interpretation, paths, to))
    }
}

/// A file that the plan proposes to remove.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProposedRemoval {
    /// The file's root-relative path.
    pub path: String,
    /// The reason for the removal.
    pub reason: RemovalReason,
}

/// The reason for a proposed removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemovalReason {
    /// The file is release junk (X22).
    ReleaseJunk,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// A proposed move of one file from its current path to a new path.
pub struct RenameProposal {
    /// The current root-relative path.
    pub from: String,
    /// The proposed root-relative path.
    pub to: String,
    /// `true` if the paths differ only in letter case or Unicode normalization.
    pub case_only: bool,
    /// `true` if the proposal is part of a rename cycle. The caller moves one file of the
    /// cycle to a temporary name first.
    pub requires_temporary_step: bool,
    /// The video that this file moves with. It is `None` for the video itself.
    pub moves_with: Option<String>,
    /// `true` if no item claims the file and the file moves with its folder (R6).
    pub unclaimed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// A file that cannot be renamed, with the reasons.
pub struct BlockedRename {
    /// The current root-relative path.
    pub from: String,
    /// Every reason that blocks the file.
    pub reasons: Vec<BlockReason>,
    /// The rendered destination, if one could be rendered.
    pub to: Option<String>,
    /// The staying paths, proposal sources or folders that the destination clashes with.
    pub conflicting_paths: Vec<String>,
}

/// Proposes renames for the files of `interpretation`'s items. `entries` holds every
/// root-relative path that the caller has, with directories ending in `/`. A file
/// without a proposal stays where it is.
///
/// Uses the built-in rules. [`Rules::plan_renames`] runs under another rules value.
#[must_use]
pub fn plan_renames(
    interpretation: &Interpretation,
    entries: &[String],
    profile: &OutputProfile,
) -> RenamePlan {
    Rules::builtin().plan_renames(interpretation, entries, profile)
}

impl Rules {
    /// Proposes renames for the files of `interpretation`'s items. `entries` holds every
    /// root-relative path that the caller has, with directories ending in `/`. A file
    /// without a proposal stays where it is.
    pub fn plan_renames(
        &self,
        interpretation: &Interpretation,
        entries: &[String],
        profile: &OutputProfile,
    ) -> RenamePlan {
        let rules = self;
        let mut seen_sources = BTreeSet::new();
        let unclaimed: BTreeSet<&String> = interpretation.unclaimed_files.iter().collect();
        let mut planned: Vec<PlannedFile> = file_targets(rules, interpretation, profile, None)
            .into_iter()
            .filter(|target| seen_sources.insert(target.path.clone()))
            .filter(|target| target.destination.as_ref() != Ok(&target.path))
            .map(|target| PlannedFile::new(target, &unclaimed))
            .collect();
        let existing_files: BTreeSet<String> = entries
            .iter()
            .chain(&seen_sources)
            .filter(|path| !path.ends_with('/'))
            .cloned()
            .collect();

        block_groups_with_a_blocked_file(&mut planned);
        loop {
            let has_kept_files_in_place = keep_split_series_in_place(&mut planned);
            let conflicts = destination_conflicts(&planned, &existing_files, true);
            if conflicts.is_empty() && !has_kept_files_in_place {
                break;
            }
            for (index, (reasons, conflicting_paths)) in conflicts {
                planned[index].reasons = reasons;
                planned[index].conflicting_paths = conflicting_paths;
            }
            block_groups_with_a_blocked_file(&mut planned);
        }

        let (moving, blocked): (Vec<PlannedFile>, Vec<PlannedFile>) = planned
            .into_iter()
            .partition(|file| file.reasons.is_empty());
        let removals = interpretation
            .release_junk
            .iter()
            .map(|path| ProposedRemoval {
                path: path.clone(),
                reason: RemovalReason::ReleaseJunk,
            })
            .collect();
        RenamePlan {
            rules: rules.identity().clone(),
            removals,
            proposals: in_execution_order(moving),
            blocked: blocked.into_iter().map(PlannedFile::into_blocked).collect(),
        }
    }
}

/// A file of a plan with its destination or block reasons.
pub(super) struct PlannedFile {
    pub from: String,
    pub to: Option<String>,
    pub reasons: Vec<BlockReason>,
    pub conflicting_paths: BTreeSet<String>,
    pub moves_with: Option<String>,
    pub unclaimed: bool,
    /// The group of files that move together or not at all: a video and the files that
    /// move with it.
    group: String,
    series: Option<SeriesKey>,
    /// `true` if the file is renamed inside its current folder because its series has a
    /// blocked file.
    is_kept_in_place: bool,
}

impl PlannedFile {
    /// Returns the planned file of `target`. `unclaimed` holds the files that no item
    /// claims.
    pub(super) fn new(target: FileTarget, unclaimed: &BTreeSet<&String>) -> PlannedFile {
        let (to, reasons) = match target.destination {
            Ok(to) => (Some(to), Vec::new()),
            Err(reasons) => (None, reasons),
        };
        PlannedFile {
            unclaimed: unclaimed.contains(&target.path),
            group: target.moves_with.clone().unwrap_or(target.path.clone()),
            from: target.path,
            to,
            reasons,
            conflicting_paths: BTreeSet::new(),
            moves_with: target.moves_with,
            series: target.series,
            is_kept_in_place: false,
        }
    }

    pub(super) fn into_blocked(self) -> BlockedRename {
        BlockedRename {
            from: self.from,
            reasons: self.reasons,
            to: self.to,
            conflicting_paths: self.conflicting_paths.into_iter().collect(),
        }
    }

    fn moving_destination(&self) -> Option<&str> {
        self.to.as_deref().filter(|_| self.reasons.is_empty())
    }
}

/// Returns the comparison form of `path`: NFC, then case-folded (X2, X3).
pub(super) fn comparison_key(path: &str) -> String {
    if path.is_ascii() {
        return lexical::fold_case_text(path);
    }
    lexical::fold_case_text(&path.nfc().collect::<String>())
}

/// Returns `path` and each folder above it.
fn path_and_folders(path: &str) -> Vec<&str> {
    let mut names = vec![path];
    let mut folder = parent_folder(path);
    while let Some(current) = folder {
        names.push(current);
        folder = parent_folder(current);
    }
    names
}

/// Blocks every file in a group that contains a blocked file (R6).
pub(super) fn block_groups_with_a_blocked_file(planned: &mut [PlannedFile]) {
    let blocked_groups: BTreeSet<String> = planned
        .iter()
        .filter(|file| !file.reasons.is_empty())
        .map(|file| file.group.clone())
        .collect();
    for file in planned.iter_mut() {
        if file.reasons.is_empty() && blocked_groups.contains(&file.group) {
            file.reasons.push(BlockReason::MovesWithBlockedFile);
        }
    }
}

/// Renames the other files of a series that has a blocked file inside their current
/// folders, so that the series keeps its folders (R6). Drops a file whose new path is
/// its current path. Returns `true` if any file changed.
fn keep_split_series_in_place(planned: &mut Vec<PlannedFile>) -> bool {
    let split_series: BTreeSet<SeriesKey> = planned
        .iter()
        .filter(|file| !file.reasons.is_empty())
        .filter_map(|file| file.series.clone())
        .collect();
    let mut has_changed = false;
    for file in planned.iter_mut() {
        let is_in_split_series = file
            .series
            .as_ref()
            .is_some_and(|series| split_series.contains(series));
        let Some(destination) = file.moving_destination() else {
            continue;
        };
        if !is_in_split_series || file.is_kept_in_place {
            continue;
        }
        let name = file_name(destination);
        let in_place = match parent_folder(&file.from) {
            Some(folder) => format!("{folder}/{name}"),
            None => name.to_owned(),
        };
        file.to = Some(in_place);
        file.is_kept_in_place = true;
        has_changed = true;
    }
    planned.retain(|file| file.moving_destination() != Some(file.from.as_str()));
    has_changed
}

/// Returns the moving files whose destination clashes, by index, with their block
/// reasons and the clashing paths. If `sources_are_existing` is `true`, the sources are
/// paths of `existing_files`, and a moving source does not stay.
pub(super) fn destination_conflicts(
    planned: &[PlannedFile],
    existing_files: &BTreeSet<String>,
    sources_are_existing: bool,
) -> BTreeMap<usize, (Vec<BlockReason>, BTreeSet<String>)> {
    let moving_sources: BTreeSet<&str> = planned
        .iter()
        .filter(|_| sources_are_existing)
        .filter(|file| file.moving_destination().is_some())
        .map(|file| file.from.as_str())
        .collect();
    let staying_files: BTreeSet<&str> = existing_files
        .iter()
        .map(String::as_str)
        .filter(|path| !moving_sources.contains(path))
        .collect();
    // Each name's comparison key is computed once because folders repeat across their
    // files.
    let mut key_of: BTreeMap<&str, String> = BTreeMap::new();
    let mut names_by_key: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let destinations = planned.iter().filter_map(PlannedFile::moving_destination);
    for path in staying_files.iter().copied().chain(destinations) {
        for name in path_and_folders(path) {
            key_of.entry(name).or_insert_with(|| comparison_key(name));
        }
    }
    for (name, key) in &key_of {
        names_by_key.entry(key).or_default().insert(name);
    }
    let mut sources_by_destination_key: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for file in planned {
        if let Some(destination) = file.moving_destination() {
            sources_by_destination_key
                .entry(&key_of[destination])
                .or_default()
                .push(&file.from);
        }
    }

    let mut conflicts = BTreeMap::new();
    for (index, file) in planned.iter().enumerate() {
        let Some(destination) = file.moving_destination() else {
            continue;
        };
        let mut reasons = Vec::new();
        let mut conflicting_paths = BTreeSet::new();
        for name in path_and_folders(destination) {
            if staying_files.contains(name) {
                reasons.push(BlockReason::DestinationExists);
                conflicting_paths.insert(name.to_owned());
            }
            let differing_names = names_by_key[key_of[name].as_str()]
                .iter()
                .filter(|other| **other != name);
            for other in differing_names {
                reasons.push(BlockReason::DestinationDiffersOnlyInCaseOrNormalization);
                conflicting_paths.insert((*other).to_owned());
            }
        }
        let other_sources = sources_by_destination_key[key_of[destination].as_str()]
            .iter()
            .filter(|source| **source != file.from);
        for source in other_sources {
            reasons.push(BlockReason::DestinationCollision);
            conflicting_paths.insert((*source).to_owned());
        }
        if !reasons.is_empty() {
            reasons.sort();
            reasons.dedup();
            conflicts.insert(index, (reasons, conflicting_paths));
        }
    }
    conflicts
}

/// Orders `moving` for execution. A proposal whose destination is the source of another
/// proposal comes after that proposal. The proposals of a cycle are marked as requiring
/// a temporary step and come in source order.
fn in_execution_order(mut moving: Vec<PlannedFile>) -> Vec<RenameProposal> {
    moving.sort_by(|left, right| left.from.cmp(&right.from));
    let index_by_source_key: BTreeMap<String, usize> = moving
        .iter()
        .enumerate()
        .map(|(index, file)| (comparison_key(&file.from), index))
        .collect();
    let dependency: Vec<Option<usize>> = moving
        .iter()
        .enumerate()
        .map(|(index, file)| {
            let destination = file.to.as_deref().expect("moving files have a destination");
            index_by_source_key
                .get(&comparison_key(destination))
                .copied()
                .filter(|source| *source != index)
        })
        .collect();

    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); moving.len()];
    for (index, source) in dependency.iter().enumerate() {
        if let Some(source) = source {
            dependents[*source].push(index);
        }
    }

    // The first ready file goes next. A file is ready once the source that it renames
    // into has moved. If no file is ready, the first waiting file leads to a cycle.
    let mut order: Vec<(usize, bool)> = Vec::new();
    let mut emitted = vec![false; moving.len()];
    let mut ready: BinaryHeap<Reverse<usize>> = (0..moving.len())
        .filter(|index| dependency[*index].is_none())
        .map(Reverse)
        .collect();
    let mut first_waiting = 0;
    while order.len() < moving.len() {
        let mut newly_emitted = Vec::new();
        if let Some(Reverse(index)) = ready.pop() {
            newly_emitted.push((index, false));
        } else {
            while emitted[first_waiting] {
                first_waiting += 1;
            }
            let mut visited = Vec::new();
            let mut position_of = BTreeMap::new();
            let mut current = first_waiting;
            while !position_of.contains_key(&current) {
                position_of.insert(current, visited.len());
                visited.push(current);
                current = dependency[current].expect("a waiting file has a dependency");
            }
            let mut cycle = visited[position_of[&current]..].to_vec();
            cycle.sort();
            newly_emitted.extend(cycle.into_iter().map(|index| (index, true)));
        }
        for &(index, _) in &newly_emitted {
            emitted[index] = true;
        }
        for &(index, _) in &newly_emitted {
            let now_ready = dependents[index]
                .iter()
                .filter(|dependent| !emitted[**dependent]);
            ready.extend(now_ready.copied().map(Reverse));
        }
        order.extend(newly_emitted);
    }

    let mut files: Vec<Option<PlannedFile>> = moving.into_iter().map(Some).collect();
    order
        .into_iter()
        .map(|(index, requires_temporary_step)| {
            let file = files[index].take().expect("each file is emitted once");
            let to = file.to.expect("moving files have a destination");
            RenameProposal {
                case_only: comparison_key(&file.from) == comparison_key(&to),
                from: file.from,
                to,
                requires_temporary_step,
                moves_with: file.moves_with,
                unclaimed: file.unclaimed,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moving(from: &str, to: &str) -> PlannedFile {
        PlannedFile {
            from: from.to_owned(),
            to: Some(to.to_owned()),
            reasons: Vec::new(),
            conflicting_paths: BTreeSet::new(),
            moves_with: None,
            unclaimed: false,
            group: from.to_owned(),
            series: None,
            is_kept_in_place: false,
        }
    }

    fn steps(proposals: &[RenameProposal]) -> Vec<(&str, &str, bool)> {
        proposals
            .iter()
            .map(|proposal| {
                (
                    proposal.from.as_str(),
                    proposal.to.as_str(),
                    proposal.requires_temporary_step,
                )
            })
            .collect()
    }

    #[test]
    fn in_execution_order_places_rename_after_source_it_renames_into() {
        let proposals = in_execution_order(vec![moving("a", "b"), moving("b", "c")]);
        assert_eq!(steps(&proposals), [("b", "c", false), ("a", "b", false)]);
    }

    #[test]
    fn in_execution_order_marks_cycle_as_requiring_temporary_step() {
        let proposals =
            in_execution_order(vec![moving("x", "a"), moving("a", "b"), moving("b", "a")]);
        assert_eq!(
            steps(&proposals),
            [("a", "b", true), ("b", "a", true), ("x", "a", false)]
        );
    }

    #[test]
    fn in_execution_order_matches_sources_ignoring_case() {
        let proposals = in_execution_order(vec![moving("A/x", "a/y"), moving("a/Y", "a/z")]);
        assert_eq!(
            steps(&proposals),
            [("a/Y", "a/z", false), ("A/x", "a/y", false)]
        );
    }
}
