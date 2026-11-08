//! Plans the import of a download folder into a library (6.4, R17). Each file of the
//! download gets a library-relative destination, and the planner checks the destinations
//! against the library's files and against each other. The planner performs no
//! filesystem operations.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::interpret::{Extra, Interpretation, Item};
use crate::lexical;
use crate::model::{ExtraKind, resolved_values};
use crate::parsers::extras::LibraryType;
use crate::rules::{Rules, RulesIdentity};

use super::planning::{PlannedFile, block_groups_with_a_blocked_file, destination_conflicts};
use super::targets::file_targets;
use super::{BlockedRename, NamingFacts, OutputProfile};

/// An import plan: the moves from the download into the library, the files that cannot
/// move, and the files that stay in the download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportPlan {
    /// The moves, sorted by source path.
    pub proposals: Vec<ImportProposal>,
    /// The blocked moves, sorted by source path.
    pub blocked: Vec<BlockedRename>,
    /// The download files that the import leaves in place, sorted by path.
    pub left_in_place: Vec<LeftFile>,
    /// The rules this result was produced under.
    pub rules: RulesIdentity,
}

/// A proposed move of one file from the download into the library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportProposal {
    /// The download-relative source path.
    pub from: String,
    /// The library-relative destination path.
    pub to: String,
    /// The video that this file moves with. It is `None` for the video itself.
    pub moves_with: Option<String>,
    /// `true` if no item claims the file and the file moves with its folder (R6).
    pub unclaimed: bool,
}

/// A download file that the import leaves in place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LeftFile {
    /// The download-relative path.
    pub path: String,
    /// The reason that the file stays.
    pub reason: LeftReason,
}

/// The reason that an import leaves a file in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LeftReason {
    /// The file is a sample.
    Sample,
    /// The file is release junk (X22).
    ReleaseJunk,
    /// No item claims the file, and the file follows no video or movie folder.
    Unclaimed,
}

/// The movies and series of a library, for matching the works of a download.
pub(crate) struct LibraryWorks {
    works: Vec<LibraryWork>,
}

/// A movie or series of the library.
pub(crate) struct LibraryWork {
    library: LibraryType,
    title_key: String,
    pub year: Option<u16>,
    pub provider_ids: BTreeMap<String, String>,
    /// The library-relative folder of the work. `None` for a movie without a folder.
    pub folder: Option<String>,
}

/// The library works that a download work matches.
pub(crate) enum LibraryMatch<'a> {
    None,
    One(&'a LibraryWork),
    Several,
}

impl LibraryWorks {
    fn new(rules: &Rules, library: &Interpretation) -> LibraryWorks {
        let works = library
            .items
            .iter()
            .filter_map(|item| {
                let (library, title, year, provider_ids, folder) = match item {
                    Item::Movie(movie) => (
                        LibraryType::Movies,
                        movie.title.as_ref()?,
                        movie.year,
                        &movie.provider_ids,
                        movie.path.clone(),
                    ),
                    Item::Series(series) => (
                        LibraryType::Tv,
                        series.title.as_ref()?,
                        series.year,
                        &series.provider_ids,
                        series.path.clone(),
                    ),
                    _ => return None,
                };
                Some(LibraryWork {
                    library,
                    title_key: lexical::title_comparison_key(rules, title),
                    year,
                    provider_ids: resolved_values(provider_ids),
                    folder,
                })
            })
            .collect();
        LibraryWorks { works }
    }

    /// Returns the library works that the work of `facts` matches: the same library type
    /// and title comparison key (P4), a year that is equal or missing on one side, and no provider with two
    /// different IDs. If some matches have the same year, only those count.
    pub(crate) fn find(&self, rules: &Rules, facts: &NamingFacts) -> LibraryMatch<'_> {
        let Some(title) = &facts.title else {
            return LibraryMatch::None;
        };
        let title_key = lexical::title_comparison_key(rules, title);
        let matches: Vec<&LibraryWork> = self
            .works
            .iter()
            .filter(|work| work.library == facts.library && work.title_key == title_key)
            .filter(|work| work.year.is_none() || facts.year.is_none() || work.year == facts.year)
            .filter(|work| {
                work.provider_ids.iter().all(|(provider, id)| {
                    facts
                        .provider_ids
                        .get(provider)
                        .is_none_or(|other| other == id)
                })
            })
            .collect();
        let same_year: Vec<&LibraryWork> = matches
            .iter()
            .copied()
            .filter(|work| work.year.is_some() && work.year == facts.year)
            .collect();
        let matches = if same_year.is_empty() {
            matches
        } else {
            same_year
        };
        match matches.as_slice() {
            [] => LibraryMatch::None,
            [work] => LibraryMatch::One(work),
            _ => LibraryMatch::Several,
        }
    }
}

impl Rules {
    /// Plans the import of the files of `download` into the library that `library`
    /// interprets. `library_entries` holds every library-relative path, with directories
    /// ending in `/`. A destination that is an existing library file is blocked, so the
    /// import never replaces a file.
    pub fn plan_import(
        &self,
        download: &Interpretation,
        library: &Interpretation,
        library_entries: &[String],
        profile: &OutputProfile,
    ) -> ImportPlan {
        let works = LibraryWorks::new(self, library);
        let unclaimed: BTreeSet<&String> = download.unclaimed_files.iter().collect();
        let mut seen_sources = BTreeSet::new();
        let mut planned: Vec<PlannedFile> = file_targets(self, download, profile, Some(&works))
            .into_iter()
            .filter(|target| seen_sources.insert(target.path.clone()))
            .map(|target| PlannedFile::new(target, &unclaimed))
            .collect();
        let library_files: BTreeSet<String> = library_entries
            .iter()
            .filter(|path| !path.ends_with('/'))
            .cloned()
            .collect();

        block_groups_with_a_blocked_file(&mut planned);
        loop {
            // The download's sources are outside the library, so no library file moves
            // away.
            let conflicts = destination_conflicts(&planned, &library_files, false);
            if conflicts.is_empty() {
                break;
            }
            for (index, (reasons, conflicting_paths)) in conflicts {
                planned[index].reasons = reasons;
                planned[index].conflicting_paths = conflicting_paths;
            }
            block_groups_with_a_blocked_file(&mut planned);
        }

        let mut left_in_place: Vec<LeftFile> = download
            .release_junk
            .iter()
            .map(|path| LeftFile {
                path: path.clone(),
                reason: LeftReason::ReleaseJunk,
            })
            .collect();
        let orphan_samples = download.items.iter().filter_map(|item| match item {
            Item::Extra(orphan) if orphan.extra == ExtraKind::Sample => Some(&orphan.paths),
            _ => None,
        });
        let samples = download
            .items
            .iter()
            .flat_map(item_extras)
            .chain(download.extras_of.values().flatten())
            .filter(|extra| extra.extra == ExtraKind::Sample)
            .map(|extra| &extra.paths)
            .chain(orphan_samples)
            .flatten();
        for path in samples {
            if seen_sources.insert(path.clone()) {
                left_in_place.push(LeftFile {
                    path: path.clone(),
                    reason: LeftReason::Sample,
                });
            }
        }
        let junk: BTreeSet<&String> = download.release_junk.iter().collect();
        for path in &download.unclaimed_files {
            if !seen_sources.contains(path) && !junk.contains(path) {
                left_in_place.push(LeftFile {
                    path: path.clone(),
                    reason: LeftReason::Unclaimed,
                });
            }
        }
        left_in_place.sort_by(|left, right| left.path.cmp(&right.path));
        left_in_place.dedup_by(|left, right| left.path == right.path);

        let (moving, blocked): (Vec<PlannedFile>, Vec<PlannedFile>) = planned
            .into_iter()
            .partition(|file| file.reasons.is_empty());
        let mut proposals: Vec<ImportProposal> = moving
            .into_iter()
            .map(|file| ImportProposal {
                to: file.to.expect("moving files have a destination"),
                from: file.from,
                moves_with: file.moves_with,
                unclaimed: file.unclaimed,
            })
            .collect();
        proposals.sort_by(|left, right| left.from.cmp(&right.from));
        let mut blocked: Vec<BlockedRename> =
            blocked.into_iter().map(PlannedFile::into_blocked).collect();
        blocked.sort_by(|left, right| left.from.cmp(&right.from));
        ImportPlan {
            proposals,
            blocked,
            left_in_place,
            rules: self.identity().clone(),
        }
    }
}

/// Returns the extras that an item owns.
fn item_extras(item: &Item) -> &[Extra] {
    match item {
        Item::Movie(movie) => &movie.extras,
        Item::Series(series) => &series.extras,
        Item::Season(season) => &season.extras,
        Item::Episode(episode) => &episode.extras,
        Item::Extra(_) => &[],
    }
}
