//! Classifies a local NFO file by its name (S9). The module never reads the contents.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::lexical;
use crate::model::{EntryKind, NfoRole};
use crate::parsers::file_types::{file_name, parent_folder};
use crate::rules::Rules;

/// The role of an NFO file, read from its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct NfoClassification {
    /// `None` if the file is not an NFO or its name gives no role.
    pub role: Option<NfoRole>,
}

/// Classifies an NFO file by its name. With the videos in scope, `<video stem>.nfo` belongs
/// to that video, and the shared name of a stack (`Movie-cd1.mkv` → `Movie.nfo`) belongs to
/// its movie. Otherwise the generic names `movie`, `tvshow` and `season` give their role. The
/// interpreter decides whether a generic name applies to its folder (one title only, S8).
/// Uses the built-in rules.
#[must_use]
pub fn classify_nfo(path: &str, video_paths: &[String]) -> NfoClassification {
    Rules::builtin().classify_nfo(path, video_paths)
}

impl Rules {
    /// Classifies an NFO file by its name, as [`classify_nfo`] does, under these rules.
    pub fn classify_nfo(&self, path: &str, video_paths: &[String]) -> NfoClassification {
        let folder = parent_folder(path).unwrap_or_default();
        let sibling_videos: Vec<String> = video_paths
            .iter()
            .filter(|video| parent_folder(video).unwrap_or_default() == folder)
            .cloned()
            .collect();
        classify_nfo_among(self, path, &FolderVideoNames::new(self, &sibling_videos))
    }
}

/// The case-folded names that an NFO beside the videos of a folder can have: the stem of each
/// video with the first video of that stem, and the shared name of each stack with its first
/// part.
pub(crate) struct FolderVideoNames {
    pub videos_by_stem: BTreeMap<String, String>,
    pub stacks_by_name: BTreeMap<String, String>,
}

impl FolderVideoNames {
    /// Collects the names of `videos`, the videos of one folder in path order.
    pub(crate) fn new(rules: &Rules, videos: &[String]) -> Self {
        let mut videos_by_stem = BTreeMap::new();
        for video in videos {
            let stem = lexical::fold_case_text(lexical::stem(file_name(video), EntryKind::File));
            videos_by_stem.entry(stem).or_insert_with(|| video.clone());
        }
        let mut stacks_by_name = BTreeMap::new();
        for stack in rules.group_parts(videos).stacks {
            let name = lexical::fold_case_text(&stack.name);
            stacks_by_name
                .entry(name)
                .or_insert_with(|| stack.paths[0].clone());
        }
        FolderVideoNames {
            videos_by_stem,
            stacks_by_name,
        }
    }
}

/// Classifies an NFO as `classify_nfo` does, against the names of the videos in the folder
/// of the NFO.
pub(crate) fn classify_nfo_among(
    rules: &Rules,
    path: &str,
    names: &FolderVideoNames,
) -> NfoClassification {
    let name = file_name(path);
    let is_nfo = lexical::extension(name, EntryKind::File)
        .is_some_and(|extension| rules.vocab.nfo_extensions.contains(&extension));
    if !is_nfo {
        return NfoClassification { role: None };
    }
    let stem = lexical::fold_case_text(lexical::stem(name, EntryKind::File));
    let role = if names.videos_by_stem.contains_key(&stem) {
        Some(NfoRole::Video)
    } else if names.stacks_by_name.contains_key(&stem) {
        Some(NfoRole::Movie)
    } else {
        rules.vocab.nfo_names.get(&stem).copied()
    };
    NfoClassification { role }
}
