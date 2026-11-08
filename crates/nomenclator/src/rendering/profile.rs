//! Provides the declarative output profile and the naming rules of the target
//! filesystem (R5).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::data;
use crate::lexical;
use crate::model::{ArtworkRole, ExtraKind, SidecarFlag};
use crate::validation::{DiagnosticRule, SeverityOverride};

/// The settings that control how structured facts are written. Every field has a
/// default (R1-R5).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OutputProfile {
    /// The providers whose ID a movie name may contain. The first provider with an ID is
    /// written.
    pub movie_provider_order: Vec<String>,
    /// The providers whose ID a series folder may contain. The first provider with an ID
    /// is written.
    pub series_provider_order: Vec<String>,
    /// The filesystem whose naming rules apply (R5). Defaults to `portable`.
    pub target_filesystem: TargetFilesystem,
    /// The maximum length of a path component, in UTF-8 bytes. Defaults to 255.
    pub max_component_bytes: usize,
    /// Names that no path component may use before its first dot, compared without case,
    /// in addition to the device names that the target filesystem reserves (R5). Defaults
    /// to no names.
    pub reserved_names: Vec<String>,
    /// The validation rules reported at another severity or not reported (R7). Rename
    /// planning ignores this field.
    pub severity: BTreeMap<DiagnosticRule, SeverityOverride>,
    /// The asset roles that each item of a kind needs (R12). Validation reports each
    /// missing role as `missing_asset`. Rename planning ignores this field.
    pub required_assets: BTreeMap<AssetItemKind, Vec<AssetRole>>,
    /// The subtitles that each feature video needs (R12). Validation reports each missing
    /// one as `missing_subtitle`. Rename planning ignores this field.
    pub wanted_subtitles: Vec<WantedSubtitle>,
    /// The diagnostics that validation leaves out (R13). Rename planning ignores this
    /// field.
    pub suppressions: Vec<Suppression>,
    /// The place of a movie's files (R15). Defaults to `folder`.
    pub movie_layout: MovieLayout,
    /// The spelling of a season folder's number (R15). Defaults to `padded`.
    pub season_folder_style: SeasonFolderStyle,
    /// The name of the season 0 folder (R15). Defaults to `Season 00`, which follows
    /// `season_folder_style`.
    pub specials_folder: Option<String>,
    /// The folder name of each extra kind that does not use its R1 folder (R15).
    pub extras_folders: BTreeMap<ExtraKind, String>,
    /// The spelling of the provider ID in a work's folder and file name (R15). Defaults
    /// to `braces`.
    pub provider_tag_style: ProviderTagStyle,
    /// The text that replaces each listed text in a fact before the target filesystem's
    /// rules apply (R15). A longer listed text is replaced before a shorter one that it
    /// starts with.
    pub character_replacements: BTreeMap<String, String>,
    /// The spelling of a version label in a feature's stem (R15). Defaults to `dash`.
    pub version_label_style: VersionLabelStyle,
    /// The case of a written extension (R15). Defaults to `lower`.
    pub extension_case: ExtensionCase,
}

/// The place of a movie's files.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MovieLayout {
    /// Each movie has its own folder (R1).
    #[default]
    Folder,
    /// A movie's files are at the root, or in its category folder. A movie that has
    /// extras or folder files keeps its folder.
    Flat,
}

/// The spelling of a season folder's number.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeasonFolderStyle {
    /// `Season 01` (R2).
    #[default]
    Padded,
    /// `Season 1`.
    Unpadded,
}

/// The spelling of the provider ID in a work's name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderTagStyle {
    /// `{tmdb-329865}` (R1).
    #[default]
    Braces,
    /// `[tmdbid-329865]`.
    BracketsId,
    /// No provider ID is written.
    None,
}

/// The spelling of a version label in a feature's stem.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionLabelStyle {
    /// `Arrival (2016) - 2160p` (R1).
    #[default]
    Dash,
    /// `Arrival (2016) [2160p]`.
    Brackets,
}

/// The case of a written extension.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionCase {
    /// The extension in lowercase (R1).
    #[default]
    Lower,
    /// The extension as the facts give it.
    Keep,
}

/// Diagnostics that validation leaves out: those of `rule`, or of every rule if `rule` is
/// unset, with a path that one of `paths` matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suppression {
    /// The rule. Unset for every rule.
    #[serde(default)]
    pub rule: Option<DiagnosticRule>,
    /// Globs in gitignore syntax, matched against root-relative paths.
    pub paths: Vec<String>,
}

/// An item kind that can need assets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetItemKind {
    /// A movie.
    Movie,
    /// A series.
    Series,
    /// A season of a series.
    Season,
    /// An episode.
    Episode,
}

/// An asset role that an item can need.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetRole {
    /// A poster image.
    Poster,
    /// A backdrop image.
    Backdrop,
    /// A banner image.
    Banner,
    /// A logo image.
    Logo,
    /// A thumb image.
    Thumb,
    /// An episode's still image.
    EpisodeStill,
    /// An NFO file.
    Nfo,
    /// A theme song.
    ThemeSong,
    /// A theme video.
    ThemeVideo,
}

/// The asset role of each artwork role, the one mapping between the two (R12, R19).
const ARTWORK_ASSET_ROLES: &[(ArtworkRole, AssetRole)] = &[
    (ArtworkRole::Poster, AssetRole::Poster),
    (ArtworkRole::Backdrop, AssetRole::Backdrop),
    (ArtworkRole::Banner, AssetRole::Banner),
    (ArtworkRole::Logo, AssetRole::Logo),
    (ArtworkRole::Thumb, AssetRole::Thumb),
    (ArtworkRole::EpisodeStill, AssetRole::EpisodeStill),
];

impl AssetRole {
    /// Returns the artwork role of this asset role. Returns `None` for an NFO or a theme
    /// file.
    pub(crate) fn artwork_role(self) -> Option<ArtworkRole> {
        ARTWORK_ASSET_ROLES
            .iter()
            .find(|(_, asset)| *asset == self)
            .map(|(artwork, _)| *artwork)
    }

    /// Returns the asset role that an artwork role satisfies. Returns `None` for an image
    /// whose name gives no role.
    pub(crate) fn of_artwork(role: ArtworkRole) -> Option<AssetRole> {
        ARTWORK_ASSET_ROLES
            .iter()
            .find(|(artwork, _)| *artwork == role)
            .map(|(_, asset)| *asset)
    }
}

/// A subtitle that a feature video needs: a language and the flags it must have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WantedSubtitle {
    /// A BCP 47 tag. A base language (`pt`) also accepts its regions (`pt-BR`).
    pub language: String,
    /// The flags that the subtitle must have.
    #[serde(default)]
    pub flags: Vec<SidecarFlag>,
}

impl Default for OutputProfile {
    fn default() -> Self {
        OutputProfile {
            movie_provider_order: vec!["tmdb".to_owned(), "imdb".to_owned()],
            series_provider_order: vec!["tvdb".to_owned(), "tmdb".to_owned()],
            target_filesystem: TargetFilesystem::default(),
            max_component_bytes: 255,
            reserved_names: Vec::new(),
            severity: BTreeMap::new(),
            required_assets: BTreeMap::new(),
            wanted_subtitles: Vec::new(),
            suppressions: Vec::new(),
            movie_layout: MovieLayout::default(),
            season_folder_style: SeasonFolderStyle::default(),
            specials_folder: None,
            extras_folders: BTreeMap::new(),
            provider_tag_style: ProviderTagStyle::default(),
            character_replacements: BTreeMap::new(),
            version_label_style: VersionLabelStyle::default(),
            extension_case: ExtensionCase::default(),
        }
    }
}

/// A filesystem that proposed names must be valid on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum TargetFilesystem {
    /// Names that are valid on Windows, macOS and Linux.
    #[default]
    Portable,
    /// Names that may contain any character except `/` and NUL.
    Posix,
}

impl OutputProfile {
    /// Returns the text of one fact as the writer writes it: the character replacements
    /// applied, then the target filesystem's cleaning (R5, R15). The reader registers a
    /// profile's own names in this form (X25).
    pub(crate) fn clean_fact(&self, text: &str) -> String {
        self.target_filesystem
            .clean_text(&self.replace_characters(text))
    }

    /// Returns `true` if the target filesystem or the profile reserves `component`.
    pub(crate) fn is_reserved(&self, component: &str) -> bool {
        let name = lexical::vocabulary_form(before_first_dot(component));
        self.target_filesystem.is_reserved(component)
            || self
                .reserved_names
                .iter()
                .any(|reserved| lexical::vocabulary_form(reserved) == name)
    }

    /// Returns `text` with the profile's character replacements applied. At each position
    /// the longest listed text that starts there is replaced.
    pub(crate) fn replace_characters(&self, text: &str) -> String {
        if self.character_replacements.is_empty() {
            return text.to_owned();
        }
        let mut replaced = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(character) = rest.chars().next() {
            let longest = self
                .character_replacements
                .iter()
                .filter(|(from, _)| !from.is_empty() && rest.starts_with(from.as_str()))
                .max_by_key(|(from, _)| from.len());
            match longest {
                Some((from, to)) => {
                    replaced.push_str(to);
                    rest = &rest[from.len()..];
                }
                None => {
                    replaced.push(character);
                    rest = &rest[character.len_utf8()..];
                }
            }
        }
        replaced
    }
}

impl TargetFilesystem {
    /// Returns the text of one fact as it may appear in a name. The text is converted to
    /// NFC, characters that the target forbids are replaced or dropped, and whitespace is
    /// collapsed and trimmed.
    pub(crate) fn clean_text(self, text: &str) -> String {
        let mut replaced = String::with_capacity(text.len());
        for character in text.nfc() {
            match (self, character) {
                (_, '/') => replaced.push('-'),
                (_, '\0') => {}
                (TargetFilesystem::Portable, ':') => replaced.push_str(" -"),
                (TargetFilesystem::Portable, '\\' | '|') => replaced.push('-'),
                (TargetFilesystem::Portable, '"') => replaced.push('\''),
                (TargetFilesystem::Portable, '<' | '>' | '?' | '*') => {}
                (TargetFilesystem::Portable, control) if control.is_control() => replaced.push(' '),
                (_, other) => replaced.push(other),
            }
        }
        replaced.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Returns the finished form of `component`. The portable target drops trailing dots
    /// and spaces.
    pub(crate) fn finish_component(self, component: &str) -> String {
        match self {
            TargetFilesystem::Portable => component.trim_end_matches(['.', ' ']).to_owned(),
            TargetFilesystem::Posix => component.to_owned(),
        }
    }

    /// Returns `true` if the target reserves `component`, which is the case if the part
    /// before the first dot is a device name.
    pub(crate) fn is_reserved(self, component: &str) -> bool {
        self == TargetFilesystem::Portable
            && data::RESERVED_DEVICE_NAMES
                .contains(&lexical::vocabulary_form(before_first_dot(component)).as_str())
    }
}

/// Returns the part of `component` before its first dot, without trailing spaces.
fn before_first_dot(component: &str) -> &str {
    component.split('.').next().unwrap_or_default().trim_end()
}
