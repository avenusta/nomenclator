//! Classifies a local artwork image by its name (S7).

use serde::Serialize;

use crate::lexical;
use crate::model::{ArtworkRole, EntryKind};
use crate::parsers::file_types::{file_name, parent_folder};
use crate::rules::Rules;

/// The artwork role of an image, read from its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ArtworkClassification {
    /// `None` for a file that is not an image.
    pub role: Option<ArtworkRole>,
    /// `true` if the image is the still of the given video (S6).
    pub is_episode_still: bool,
}

impl Rules {
    /// Classifies an image by its name. If `video_path` is given, an image beside that video
    /// named `<video stem>-thumb` is classified as the still of the video.
    pub fn classify_artwork(&self, path: &str, video_path: Option<&str>) -> ArtworkClassification {
        if !self.is_image_file(path).result {
            return ArtworkClassification {
                role: None,
                is_episode_still: false,
            };
        }
        let stem = lexical::stem(file_name(path), EntryKind::File);
        let is_episode_still =
            video_path.is_some_and(|video_path| is_still_of(self, path, stem, video_path));
        let role = if is_episode_still {
            ArtworkRole::EpisodeStill
        } else {
            named_role(self, stem).unwrap_or(ArtworkRole::Other)
        };
        ArtworkClassification {
            role: Some(role),
            is_episode_still,
        }
    }
}

fn is_still_of(rules: &Rules, image_path: &str, image_stem: &str, video_path: &str) -> bool {
    let video_stem = lexical::stem(file_name(video_path), EntryKind::File);
    parent_folder(image_path) == parent_folder(video_path)
        && lexical::vocabulary_form(image_stem)
            .strip_prefix(&lexical::vocabulary_form(video_stem))
            .is_some_and(|rest| rules.vocab.still_image_suffixes.contains(rest))
}

/// Returns the role that a role word in `stem` gives. The role word is the whole stem
/// (`poster`, `fanart2`) or follows a separator (`Heat (1995)-poster`), with an optional index.
/// Returns `None` if `stem` has no role word.
fn named_role(rules: &Rules, stem: &str) -> Option<ArtworkRole> {
    let folded = lexical::vocabulary_form(stem);
    let word = folded
        .rsplit(['-', '.', '_'])
        .next()
        .unwrap_or(&folded)
        .trim_end_matches(|character: char| character.is_ascii_digit());
    rules.vocab.artwork_roles.get(word).copied()
}
