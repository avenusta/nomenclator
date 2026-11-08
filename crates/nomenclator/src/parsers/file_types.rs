//! Classifies the kind of file that a path names, by its extension. For disc clips and
//! `idx`/`sub` pairs, the module also reads the folders and siblings of the path. A
//! recognized file type does not guarantee that the file plays.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::lexical;
use crate::model::EntryKind;
use crate::rules::Rules;

/// The reason a recognised video is not admitted as a video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedVideoReason {
    /// An `.iso` or `.img` disc image.
    DiscImage,
    /// A clip inside a `BDMV/` or `VIDEO_TS/` folder.
    DiscStructure,
    /// A video container outside the supported list (`.vob`, `.rmvb`, …).
    UnsupportedContainer,
}

/// The result of a video check: whether a path is a video, and why a recognised video is
/// not admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct VideoFileCheck {
    /// `true` if the file is a video that a title can be made of.
    pub result: bool,
    /// `true` if the file is recognisably a video but is reported instead of admitted.
    pub unsupported_video: bool,
    /// The reason the video is not admitted.
    pub unsupported_video_reason: Option<UnsupportedVideoReason>,
}

/// The result of a file type check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FileTypeCheck {
    /// `true` if the file is of the checked type.
    pub result: bool,
}

/// Checks whether `path` is a supported video, or a recognised video that is reported as
/// unsupported, under the built-in rules.
#[must_use]
pub fn is_video_file(path: &str) -> VideoFileCheck {
    Rules::builtin().is_video_file(path)
}

/// Checks whether `path` is an audio file, by its extension, under the built-in rules.
#[must_use]
pub fn is_audio_file(path: &str) -> FileTypeCheck {
    Rules::builtin().is_audio_file(path)
}

/// Checks whether `path` is an image, by its extension, under the built-in rules.
#[must_use]
pub fn is_image_file(path: &str) -> FileTypeCheck {
    Rules::builtin().is_image_file(path)
}

/// Checks whether `path` is a subtitle under the built-in rules. An `.idx` counts only beside
/// its `.sub`. That `.sub` is then part of the pair rather than a subtitle of its own.
#[must_use]
pub fn is_subtitle_file(path: &str, sibling_paths: &[String]) -> FileTypeCheck {
    Rules::builtin().is_subtitle_file(path, sibling_paths)
}

/// Returns `true` if `path` is release junk under the built-in rules.
///
/// Uses the built-in rules; [`Rules::is_release_junk`] takes others.
#[must_use]
pub fn is_release_junk(path: &str) -> FileTypeCheck {
    Rules::builtin().is_release_junk(path)
}

impl Rules {
    /// Returns whether `path` is release junk (X22): a tracker note or ad, a shortcut or an
    /// executable, or a screenshot or bitrate image. Media files are never junk.
    pub fn is_release_junk(&self, path: &str) -> FileTypeCheck {
        let name = file_name(path);
        let folded = lexical::fold_case_text(name);
        let stem = lexical::fold_case_text(lexical::stem(name, EntryKind::File));
        let is_media = self.is_video_file(path).result
            || self.is_audio_file(path).result
            || self.is_subtitle_file(path, &[]).result;
        let is_image = self.is_image_file(path).result;
        let image_stem = stem
            .trim_end_matches(|c: char| c.is_ascii_digit() || matches!(c, ' ' | '.' | '-' | '_'));
        let result = !is_media
            && (self.release_junk_names.contains(&folded)
                || extension(path)
                    .is_some_and(|extension| self.release_junk_extensions.contains(&extension))
                || self
                    .release_junk_prefixes
                    .iter()
                    .any(|prefix| stem.starts_with(prefix.as_str()))
                || (is_image && self.release_junk_images.contains(image_stem)));
        FileTypeCheck { result }
    }

    /// Checks whether `path` is a supported video, or a recognised video that is reported as
    /// unsupported.
    pub fn is_video_file(&self, path: &str) -> VideoFileCheck {
        let reason = unsupported_video_reason(self, path);
        VideoFileCheck {
            result: reason.is_none() && has_extension_in(path, &self.video_extensions),
            unsupported_video: reason.is_some(),
            unsupported_video_reason: reason,
        }
    }

    /// Checks whether `path` is an audio file, by its extension.
    pub fn is_audio_file(&self, path: &str) -> FileTypeCheck {
        FileTypeCheck {
            result: has_extension_in(path, &self.audio_extensions),
        }
    }

    /// Checks whether `path` is an image, by its extension.
    pub fn is_image_file(&self, path: &str) -> FileTypeCheck {
        FileTypeCheck {
            result: has_extension_in(path, &self.image_extensions),
        }
    }

    /// Checks whether `path` is a subtitle. An `.idx` counts only beside its `.sub`. That `.sub`
    /// is then part of the pair rather than a subtitle of its own.
    pub fn is_subtitle_file(&self, path: &str, sibling_paths: &[String]) -> FileTypeCheck {
        let has_sibling_with_extension = |wanted: &str| {
            let stem = lexical::vocabulary_form(lexical::stem(file_name(path), EntryKind::File));
            sibling_paths.iter().any(|sibling| {
                let sibling_name = file_name(sibling);
                lexical::vocabulary_form(lexical::stem(sibling_name, EntryKind::File)) == stem
                    && lexical::extension(sibling_name, EntryKind::File).as_deref() == Some(wanted)
            })
        };
        let result = match extension(path).as_deref() {
            Some("idx") => has_sibling_with_extension("sub"),
            Some("sub") => !has_sibling_with_extension("idx"),
            Some(other) => self.subtitle_extensions.contains(other),
            None => false,
        };
        FileTypeCheck { result }
    }
}

fn unsupported_video_reason(rules: &Rules, path: &str) -> Option<UnsupportedVideoReason> {
    let extension = extension(path)?;
    if rules.disc_image_extensions.contains(&extension) {
        return Some(UnsupportedVideoReason::DiscImage);
    }
    let is_video_extension = rules.video_extensions.contains(&extension)
        || rules.unsupported_video_extensions.contains(&extension);
    if is_video_extension && is_inside_disc_structure(rules, path) {
        return Some(UnsupportedVideoReason::DiscStructure);
    }
    rules
        .unsupported_video_extensions
        .contains(&extension)
        .then_some(UnsupportedVideoReason::UnsupportedContainer)
}

fn is_inside_disc_structure(rules: &Rules, path: &str) -> bool {
    let mut folders = path.split('/').rev().skip(1);
    folders.any(|folder| is_disc_structure_folder(rules, folder))
}

/// Returns `true` if a folder name is a disc structure folder, such as `VIDEO_TS` (M13).
pub(crate) fn is_disc_structure_folder(rules: &Rules, name: &str) -> bool {
    rules
        .disc_structure_folders
        .contains(&lexical::vocabulary_form(name))
}

fn has_extension_in(path: &str, extensions: &BTreeSet<String>) -> bool {
    extension(path).is_some_and(|extension| extensions.contains(&extension))
}

fn extension(path: &str) -> Option<String> {
    lexical::extension(file_name(path), EntryKind::File)
}

/// Returns the folder that contains a root-relative path. A trailing `/` marks a folder
/// and is not a component. Returns `None` if the path is at the library root.
pub(crate) fn parent_folder(path: &str) -> Option<&str> {
    path.trim_end_matches('/')
        .rfind('/')
        .map(|slash| &path[..slash])
}

/// Returns the last component of a root-relative path.
pub(crate) fn file_name(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
}
