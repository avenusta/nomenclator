//! Suggests whether a folder's names look like a Movies or a TV library (4.4, X21). The
//! suggestion is evidence for the caller. No other operation reads it.

use serde::Serialize;

use crate::interpret::parent_folder;
use crate::model::{EntryKind, Finding};
use crate::parsers::episode_name::{PresentationKind, is_season_folder};
use crate::parsers::extras::{ExtrasContext, LibraryType, classify_extra_in};
use crate::rules::Rules;

/// The library type that a folder's names suggest, with the videos as evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryTypeSuggestion {
    /// `tv` if at least 90% of the videos have an episode signal, `movies` if at most 10%
    /// do, ambiguous between them, and unknown without videos.
    pub library: Finding<LibraryType>,
    /// The videos that have an episode signal: an explicit season and episode, an air
    /// date, a special, or a season folder above them. Sorted by path.
    pub episode_videos: Vec<String>,
    /// The other videos, extras left out. Sorted by path.
    pub movie_videos: Vec<String>,
}

impl Rules {
    /// Suggests a library type for `paths`, the root-relative paths of a library folder.
    /// Extras are left out of the evidence.
    pub fn suggest_library_type(&self, paths: &[String]) -> LibraryTypeSuggestion {
        let mut sorted: Vec<&String> = paths.iter().collect();
        sorted.sort();
        sorted.dedup();
        let context = ExtrasContext::new(None, sorted.iter().copied());
        let mut episode_videos = Vec::new();
        let mut movie_videos = Vec::new();
        for path in sorted {
            if path.ends_with('/') || !self.is_video_file(path).result {
                continue;
            }
            let extra = classify_extra_in(self, path, EntryKind::File, &context);
            if extra.extra.is_resolved() {
                continue;
            }
            if has_episode_signal(self, path) {
                episode_videos.push(path.clone());
            } else {
                movie_videos.push(path.clone());
            }
        }
        let total = episode_videos.len() + movie_videos.len();
        let library = if total == 0 {
            Finding::Unknown
        } else if episode_videos.len() * 10 >= total * 9 {
            resolved(LibraryType::Tv)
        } else if episode_videos.len() * 10 <= total {
            resolved(LibraryType::Movies)
        } else {
            Finding::Ambiguous {
                candidates: vec![LibraryType::Movies, LibraryType::Tv],
                reason: "mixed_names",
            }
        };
        LibraryTypeSuggestion {
            library,
            episode_videos,
            movie_videos,
        }
    }
}

fn resolved(library: LibraryType) -> Finding<LibraryType> {
    Finding::Resolved {
        value: library,
        evidence: None,
    }
}

/// Returns `true` if a video's name states an explicit season and episode, an air date or
/// a special, or if a season folder contains it. A bare number is no signal, because a
/// movie title can end in one.
fn has_episode_signal(rules: &Rules, path: &str) -> bool {
    if parent_folder(path).is_some_and(|folder| is_season_folder(rules, folder)) {
        return true;
    }
    let name = rules.parse_episode_name(path, EntryKind::File, None);
    let has_coordinates = name.season.is_resolved() && name.episodes.is_some();
    has_coordinates
        || name.special.is_some()
        || name.presentation_kind == Some(PresentationKind::Dated)
}
