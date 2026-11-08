//! Renders names from facts under an output profile, and plans renames for an
//! interpreted library.
//!
//! Rendering is stricter than parsing: each fact has one output spelling (R1-R5). The
//! renderer writes the path of one file from its facts. The planner turns an
//! interpretation and the caller's entries into rename proposals. It performs no
//! filesystem operations (R6).

mod import;
mod planning;
mod profile;
mod slots;
pub(crate) mod targets;
pub(crate) mod template;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::data;
use crate::model::{ExtraKind, NfoRole, SidecarFlag};
use crate::parsers::extras::LibraryType;
use crate::rules::Rules;

pub use import::{ImportPlan, ImportProposal, LeftFile, LeftReason};
pub use planning::{BlockedRename, ProposedRemoval, RemovalReason, RenamePlan, RenameProposal};
pub use profile::{
    AssetItemKind, AssetRole, ExtensionCase, MovieLayout, OutputProfile, ProviderTagStyle,
    SeasonFolderStyle, Suppression, TargetFilesystem, VersionLabelStyle, WantedSubtitle,
};
pub(crate) use slots::literal_words;
pub use slots::{ProfileDiagnostic, ProfileDiagnosticReason, TemplateSlot};
pub use template::{RenderedTemplate, Template, TemplateError, TemplateErrorReason, TemplateFacts};

/// The structured facts that one name is rendered from. The renderer leaves out a
/// missing optional fact. A missing required fact blocks the name (R3).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamingFacts {
    /// The library type, which selects the layout: a movie folder, or series and season
    /// folders.
    pub library: LibraryType,
    /// The movie's or the series' title.
    pub title: Option<String>,
    /// The movie's or the series' year.
    pub year: Option<u16>,
    /// Provider IDs by provider name. The renderer writes the first one in the profile's
    /// provider order (R1, R2).
    #[serde(default)]
    pub provider_ids: BTreeMap<String, String>,
    /// A movie's edition, written as `{edition-X}` (R1).
    pub edition: Option<String>,
    /// The version label. It is written only if the work has several versions (M10).
    pub version_label: Option<String>,
    /// The 1-based part number of a stacked representation.
    pub part: Option<u32>,
    /// The video's extension.
    pub container: Option<String>,
    /// The season number; `0` is specials.
    pub season: Option<u16>,
    /// The episodes that the file covers. Contiguous episodes are written as a range (R2).
    #[serde(default)]
    pub episodes: Vec<u32>,
    /// The absolute episode numbers. They are never written. If they are the only episode
    /// reference, the name is blocked.
    #[serde(default)]
    pub absolute: Vec<u32>,
    /// An air date, `YYYY-MM-DD`.
    pub date: Option<String>,
    /// The episode's title, written after the episode reference.
    pub episode_title: Option<String>,
    /// The extra of the work to render instead of its video.
    pub extra: Option<ExtraFacts>,
    /// The sidecar to render for the video or extra that the other facts describe.
    pub sidecar: Option<SidecarFacts>,
    /// `true` if the movie has extras or files that belong to its folder, such as folder
    /// artwork. A flat movie layout keeps the folder of such a movie (R15).
    #[serde(default)]
    pub has_extras: bool,
    /// The asset to render for the item that the other facts describe (R19). The item is
    /// an episode if the facts have episodes, a date or absolute numbers, else a season
    /// if they have a season, else the movie or series.
    #[serde(default)]
    pub asset: Option<AssetFacts>,
}

impl NamingFacts {
    /// Returns the facts of a work in `library` that states no fact.
    pub(crate) fn of(library: LibraryType) -> NamingFacts {
        NamingFacts {
            library,
            title: None,
            year: None,
            provider_ids: BTreeMap::new(),
            edition: None,
            version_label: None,
            part: None,
            container: None,
            season: None,
            episodes: Vec::new(),
            absolute: Vec::new(),
            date: None,
            episode_title: None,
            extra: None,
            sidecar: None,
            has_extras: false,
            asset: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
/// The facts of an asset: an NFO, artwork or theme file of an item (R19).
pub struct AssetFacts {
    /// The asset's role.
    pub role: AssetRole,
    /// The asset's extension. It is required (R3).
    pub extension: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
/// The facts of an extra, which is rendered into its kind's folder (R1, R2).
pub struct ExtraFacts {
    /// The extra kind, which selects the extras folder.
    pub kind: ExtraKind,
    /// The extra's title. It is required (R3).
    pub title: Option<String>,
    /// The extra's year, written after the title as `Title (Year)`.
    pub year: Option<u16>,
    /// The group folder that contains the extra inside its kind's folder (R1).
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
/// The facts of a sidecar, which is rendered after its video's stem (R4).
pub struct SidecarFacts {
    /// The language, as a BCP 47 tag.
    pub language: Option<String>,
    /// The sidecar flags, written in canonical order (S4).
    #[serde(default)]
    pub flags: Vec<SidecarFlag>,
    /// The sidecar's title, written before the language.
    pub title: Option<String>,
    /// The sidecar's extension, written in lowercase. It is required (R3).
    pub extension: Option<String>,
}

/// A proposed root-relative path, or the reasons that no path can be proposed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RenderedName {
    /// The proposed path. It is `None` if the name is blocked.
    pub path: Option<String>,
    /// Every reason that blocks the name. It is empty if the name is not blocked.
    pub blocked_reasons: Vec<BlockReason>,
}

/// A reason that a name cannot be rendered or a rename cannot be proposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockReason {
    /// A missing movie or series title (R3).
    MissingTitle,
    /// A missing container or sidecar extension (R3).
    MissingExtension,
    /// An episode without a season (R3).
    MissingSeason,
    /// An episode without episode numbers or a date (R3).
    MissingEpisodeReference,
    /// An episode with only absolute numbers, which are never written (R2, R3).
    AbsoluteWithoutSeasonMapping,
    /// An extra without a title (R3).
    MissingExtraTitle,
    /// A component that is a reserved device name on the target filesystem (R5).
    ReservedName,
    /// A component that exceeds the profile's byte limit (R5).
    ComponentTooLong,
    /// An item whose identity is ambiguous (R6).
    AmbiguousIdentity,
    /// An item that has conflicting provider IDs (R6).
    AmbiguousProviderId,
    /// An item whose parts have gaps or duplicates (M11, R6).
    AmbiguousParts,
    /// An episode whose season is ambiguous (R6).
    AmbiguousSeason,
    /// A sidecar whose language is ambiguous.
    AmbiguousSidecarLanguage,
    /// Series content in a Movies library (M15, R6).
    MisplacedSeries,
    /// An extra that no item owns (R6).
    NoOwner,
    /// A disc structure folder, which is never renamed (M13).
    DiscStructure,
    /// A destination that is an existing file that stays in place (R6).
    DestinationExists,
    /// A destination that another proposal also has (R6).
    DestinationCollision,
    /// A destination that differs from another path only in letter case or Unicode
    /// normalization (R6).
    DestinationDiffersOnlyInCaseOrNormalization,
    /// A blocked file in the group of files that move together (R6).
    MovesWithBlockedFile,
    /// An imported work that matches several works of the library (R17).
    AmbiguousLibraryWork,
    /// An asset role that the item kind cannot have, such as a movie's episode still
    /// (R19).
    AssetRoleNotForItem,
}

impl BlockReason {
    /// Returns `true` if the reason is a missing required fact (R3).
    pub(crate) fn is_missing_fact(self) -> bool {
        matches!(
            self,
            BlockReason::MissingTitle
                | BlockReason::MissingExtension
                | BlockReason::MissingSeason
                | BlockReason::MissingEpisodeReference
                | BlockReason::AbsoluteWithoutSeasonMapping
                | BlockReason::MissingExtraTitle
        )
    }

    /// Returns `true` if the reason is a name that the target filesystem cannot hold (R5).
    pub(crate) fn is_invalid_name(self) -> bool {
        matches!(
            self,
            BlockReason::ReservedName | BlockReason::ComponentTooLong
        )
    }
}

impl Rules {
    /// Renders the root-relative path that `profile` gives the file described by
    /// `facts`. If the path cannot be rendered, the result has no path and lists every
    /// block reason. A provider ID is written only if these rules read it back (X25).
    pub fn render_names(&self, facts: &NamingFacts, profile: &OutputProfile) -> RenderedName {
        match render_path(self, facts, profile) {
            Ok(path) => RenderedName {
                path: Some(path),
                blocked_reasons: Vec::new(),
            },
            Err(reasons) => RenderedName {
                path: None,
                blocked_reasons: reasons,
            },
        }
    }
}

pub(crate) fn render_path(
    rules: &Rules,
    facts: &NamingFacts,
    profile: &OutputProfile,
) -> Result<String, Vec<BlockReason>> {
    let components = compose(rules, facts, profile)?;
    finish_components(&components, profile)
}

/// Renders the work's folder path: the movie folder, or the series folder and, if
/// `facts` has a season, its season folder. Fails if the title is missing.
pub(crate) fn render_work_folder(
    rules: &Rules,
    facts: &NamingFacts,
    profile: &OutputProfile,
) -> Result<String, Vec<BlockReason>> {
    let writer = Writer {
        rules,
        facts,
        profile,
    };
    let work_name = writer.work_name().ok_or(vec![BlockReason::MissingTitle])?;
    let mut components = vec![writer.work_folder(&work_name)];
    if let (LibraryType::Tv, Some(season)) = (facts.library, facts.season) {
        components.push(writer.season_folder(season));
    }
    finish_components(&components, profile)
}

/// Returns the unfinished components of the path. Fails with the missing required
/// facts if any are missing.
fn compose(
    rules: &Rules,
    facts: &NamingFacts,
    profile: &OutputProfile,
) -> Result<Vec<String>, Vec<BlockReason>> {
    let writer = Writer {
        rules,
        facts,
        profile,
    };
    let mut missing = Vec::new();
    let work_name = writer.work_name();
    if work_name.is_none() {
        missing.push(BlockReason::MissingTitle);
    }
    let extension = match (&facts.asset, &facts.sidecar) {
        (Some(asset), _) => asset.extension.as_ref(),
        (None, Some(sidecar)) => sidecar.extension.as_ref(),
        (None, None) => facts.container.as_ref(),
    };
    let extension = writer
        .optional_text(extension)
        .map(|extension| match profile.extension_case {
            ExtensionCase::Lower => extension.to_lowercase(),
            ExtensionCase::Keep => extension,
        });
    if extension.is_none() {
        missing.push(BlockReason::MissingExtension);
    }
    let work_name = work_name.unwrap_or_default();
    let layout = match (&facts.asset, &facts.extra) {
        (Some(asset), _) => writer.asset_layout(asset, &work_name),
        (None, Some(extra)) => writer.extra_layout(extra),
        (None, None) => writer.feature_layout(&work_name),
    };
    let (folders, stem) = match layout {
        Ok(layout) => layout,
        Err(reason) => {
            missing.push(reason);
            return Err(missing);
        }
    };
    let Some(extension) = extension.filter(|_| missing.is_empty()) else {
        return Err(missing);
    };
    let mut components = Vec::new();
    if !writer.is_flat_movie() {
        components.push(writer.work_folder(&work_name));
    }
    components.extend(folders);
    let sidecar_suffix = facts
        .sidecar
        .as_ref()
        .filter(|_| facts.asset.is_none())
        .map(|sidecar| writer.sidecar_suffix(sidecar))
        .unwrap_or_default();
    components.push(format!("{stem}{sidecar_suffix}.{extension}"));
    Ok(components)
}

/// A writer of the text of `facts` under one profile.
struct Writer<'a> {
    /// The rules that read the written names back.
    rules: &'a Rules,
    facts: &'a NamingFacts,
    profile: &'a OutputProfile,
}

impl Writer<'_> {
    /// Returns `text` cleaned for the target filesystem. Returns `None` if `text` is
    /// missing or empty after cleaning.
    fn optional_text(&self, text: Option<&String>) -> Option<String> {
        text.map(|text| self.profile.clean_fact(text))
            .filter(|text| !text.is_empty())
    }

    /// Returns the text that the profile's template of `slot` writes for `fields`.
    /// Returns `None` if the profile has no template for the slot or a required field has no
    /// value. The target filesystem's rules apply to the text. Spaces at its ends are kept,
    /// because a slot can write its own separator.
    fn slot(&self, slot: TemplateSlot, fields: &[(&str, Option<String>)]) -> Option<String> {
        let template = self.profile.templates.get(&slot)?;
        let facts: TemplateFacts = fields
            .iter()
            .filter_map(|(name, value)| Some(((*name).to_owned(), value.clone()?)))
            .collect();
        let text = template.render(&facts).path?;
        let cleaned = self.profile.target_filesystem.clean_text(&text);
        let start = text.len() - text.trim_start().len();
        let end = text.trim_end().len();
        Some(format!("{}{cleaned}{}", &text[..start], &text[end..]))
    }

    /// Returns the text that states a part at the end of a stem (R1, R15).
    fn part_suffix(&self, part: u32) -> String {
        self.slot(TemplateSlot::Part, &[("part", Some(part.to_string()))])
            .unwrap_or_else(|| format!(" - {}{part}", data::PART_OUTPUT_WORD))
    }

    /// Returns `true` if the movie's files go without a movie folder (R15).
    fn is_flat_movie(&self) -> bool {
        self.facts.library == LibraryType::Movies
            && self.profile.movie_layout == MovieLayout::Flat
            && !self.facts.has_extras
            && self.facts.extra.is_none()
            && self.facts.asset.is_none()
    }

    /// Returns the folder name of `season` (R2, R15).
    fn season_folder(&self, season: u16) -> String {
        if season == 0
            && let Some(name) = self.optional_text(self.profile.specials_folder.as_ref())
        {
            return name;
        }
        if let Some(name) = self.slot(
            TemplateSlot::SeasonFolder,
            &[("season", Some(season.to_string()))],
        ) {
            return name;
        }
        match self.profile.season_folder_style {
            SeasonFolderStyle::Padded => format!("{} {season:02}", data::SEASON_FOLDER_WORD),
            SeasonFolderStyle::Unpadded => format!("{} {season}", data::SEASON_FOLDER_WORD),
        }
    }

    /// Returns the folder name of an extra kind (R1, R15).
    fn extras_folder(&self, kind: ExtraKind) -> String {
        self.optional_text(self.profile.extras_folders.get(&kind))
            .unwrap_or_else(|| extras_folder(kind).to_owned())
    }

    /// Returns the work name as `Title (Year)`: the movie name, or the series name that
    /// precedes the episode tokens. Returns `None` if the title is missing.
    fn work_name(&self) -> Option<String> {
        let title = self.optional_text(self.facts.title.as_ref())?;
        if let Some(name) = self.slot(
            TemplateSlot::WorkName,
            &[
                ("title", Some(title.clone())),
                ("year", self.facts.year.map(|year| year.to_string())),
            ],
        ) {
            return Some(name);
        }
        Some(match self.facts.year {
            Some(year) => format!("{title} ({year})"),
            None => title,
        })
    }

    /// Returns the work's folder name: the work name and the first provider ID in the
    /// profile's provider order that the rules read back: a known provider with a valid
    /// ID (X25).
    fn work_folder(&self, work_name: &str) -> String {
        let order = match self.facts.library {
            LibraryType::Movies => &self.profile.movie_provider_order,
            LibraryType::Tv => &self.profile.series_provider_order,
        };
        let tag = order.iter().find_map(|provider| {
            let id = self.optional_text(self.facts.provider_ids.get(provider))?;
            let id = self.rules.valid_provider_id(provider, &id)?;
            if let Some(tag) = self.slot(
                TemplateSlot::ProviderTag,
                &[
                    ("provider", Some(provider.clone())),
                    ("id", Some(id.clone())),
                ],
            ) {
                return Some(tag);
            }
            match self.profile.provider_tag_style {
                ProviderTagStyle::Braces => Some(format!("{{{provider}-{id}}}")),
                ProviderTagStyle::BracketsId => Some(format!("[{provider}id-{id}]")),
                ProviderTagStyle::None => None,
            }
        });
        match tag {
            Some(tag) => format!("{work_name} {tag}"),
            None => work_name.to_owned(),
        }
    }

    /// Returns the suffix that follows a feature's identity in its stem, such as
    /// ` - 1080p - part2`.
    fn version_and_part(&self) -> String {
        let mut suffix = String::new();
        if let Some(label) = self.optional_text(self.facts.version_label.as_ref()) {
            match (
                self.slot(
                    TemplateSlot::VersionLabel,
                    &[("label", Some(label.clone()))],
                ),
                self.profile.version_label_style,
            ) {
                (Some(text), _) => suffix.push_str(&text),
                (None, VersionLabelStyle::Dash) => suffix.push_str(&format!(" - {label}")),
                (None, VersionLabelStyle::Brackets) => suffix.push_str(&format!(" [{label}]")),
            }
        }
        if let Some(part) = self.facts.part {
            suffix.push_str(&self.part_suffix(part));
        }
        suffix
    }

    /// Returns the folders below the work folder and the stem of a movie or episode
    /// video. Fails if an episode has no season or no episode reference.
    fn feature_layout(&self, work_name: &str) -> Result<(Vec<String>, String), BlockReason> {
        let facts = self.facts;
        match facts.library {
            LibraryType::Movies => {
                let movie_folder = self.work_folder(work_name);
                let edition = self
                    .optional_text(facts.edition.as_ref())
                    .map(|edition| {
                        let edition = edition.replace(['{', '}'], "");
                        let tag = self
                            .slot(
                                TemplateSlot::EditionTag,
                                &[("edition", Some(edition.clone()))],
                            )
                            .unwrap_or_else(|| format!("{{{}-{edition}}}", data::EDITION_TAG_KEY));
                        format!(" {tag}")
                    })
                    .unwrap_or_default();
                let stem = format!("{movie_folder}{edition}{}", self.version_and_part());
                Ok((Vec::new(), stem))
            }
            LibraryType::Tv => {
                let reference = if !facts.episodes.is_empty() {
                    let season = facts.season.ok_or(BlockReason::MissingSeason)?;
                    episode_token(season, &facts.episodes)
                } else if let Some(date) = self.optional_text(facts.date.as_ref()) {
                    date
                } else if !facts.absolute.is_empty() {
                    return Err(BlockReason::AbsoluteWithoutSeasonMapping);
                } else {
                    return Err(BlockReason::MissingEpisodeReference);
                };
                let episode_title = self.optional_text(facts.episode_title.as_ref());
                let identity = self
                    .slot(
                        TemplateSlot::EpisodeFile,
                        &[
                            ("work_name", Some(work_name.to_owned())),
                            ("reference", Some(reference.clone())),
                            ("episode_title", episode_title.clone()),
                        ],
                    )
                    .unwrap_or_else(|| {
                        let title = episode_title
                            .map(|title| format!(" - {title}"))
                            .unwrap_or_default();
                        format!("{work_name} - {reference}{title}")
                    });
                let stem = format!("{identity}{}", self.version_and_part());
                let folders = facts.season.map(|season| self.season_folder(season));
                Ok((folders.into_iter().collect(), stem))
            }
        }
    }

    /// Returns the folders below the work folder and the stem of an extra. The extra goes
    /// into its kind's folder inside the movie, series or season folder. An episode's
    /// extra is named after its episode (R2). Fails if the extra has no title.
    fn extra_layout(&self, extra: &ExtraFacts) -> Result<(Vec<String>, String), BlockReason> {
        let title = self
            .optional_text(extra.title.as_ref())
            .ok_or(BlockReason::MissingExtraTitle)?;
        let mut stem = match extra.year {
            Some(year) => format!("{title} ({year})"),
            None => title,
        };
        let mut folders = Vec::new();
        if let (LibraryType::Tv, Some(season)) = (self.facts.library, self.facts.season) {
            folders.push(self.season_folder(season));
            if !self.facts.episodes.is_empty() {
                stem = format!("{} - {stem}", episode_token(season, &self.facts.episodes));
            }
        }
        folders.push(self.extras_folder(extra.kind));
        folders.extend(self.optional_text(extra.group.as_ref()));
        if let Some(part) = self.facts.part {
            stem.push_str(&self.part_suffix(part));
        }
        Ok((folders, stem))
    }

    /// Returns the folders below the work folder and the stem of an asset (R19). A movie's,
    /// series' or season's asset is in its folder, under a generic name. An episode's asset
    /// follows the episode's stem. Fails if the item kind cannot have the role.
    fn asset_layout(
        &self,
        asset: &AssetFacts,
        work_name: &str,
    ) -> Result<(Vec<String>, String), BlockReason> {
        let facts = self.facts;
        let is_episode = facts.library == LibraryType::Tv
            && (!facts.episodes.is_empty() || facts.date.is_some() || !facts.absolute.is_empty());
        if is_episode {
            let (folders, stem) = self.feature_layout(work_name)?;
            let stem = match asset.role {
                AssetRole::Nfo => stem,
                // An episode's thumb is its still (S6).
                AssetRole::Thumb | AssetRole::EpisodeStill => {
                    format!("{stem}{}", data::STILL_IMAGE_SUFFIX)
                }
                AssetRole::ThemeSong | AssetRole::ThemeVideo => {
                    return Err(BlockReason::AssetRoleNotForItem);
                }
                role => format!("{stem}-{}", artwork_word(role)),
            };
            return Ok((folders, stem));
        }
        let season = facts.season.filter(|_| facts.library == LibraryType::Tv);
        let mut folders: Vec<String> = season
            .map(|season| self.season_folder(season))
            .into_iter()
            .collect();
        let stem = match asset.role {
            AssetRole::EpisodeStill => return Err(BlockReason::AssetRoleNotForItem),
            AssetRole::ThemeSong | AssetRole::ThemeVideo if season.is_some() => {
                return Err(BlockReason::AssetRoleNotForItem);
            }
            AssetRole::ThemeSong => data::THEME_OUTPUT_STEM.to_owned(),
            AssetRole::ThemeVideo => {
                folders.push(self.extras_folder(ExtraKind::ThemeVideo));
                data::THEME_OUTPUT_STEM.to_owned()
            }
            AssetRole::Nfo => nfo_name(match (facts.library, season) {
                (LibraryType::Movies, _) => NfoRole::Movie,
                (LibraryType::Tv, None) => NfoRole::TvShow,
                (LibraryType::Tv, Some(_)) => NfoRole::Season,
            })
            .to_owned(),
            role => artwork_word(role).to_owned(),
        };
        Ok((folders, stem))
    }

    /// Returns the sidecar suffix, such as `.Commentary.en.forced`: the title, language
    /// and flags in canonical order (R4).
    fn sidecar_suffix(&self, sidecar: &SidecarFacts) -> String {
        let mut words: Vec<String> = Vec::new();
        words.extend(self.optional_text(sidecar.title.as_ref()));
        words.extend(self.optional_text(sidecar.language.as_ref()));
        let mut flags = sidecar.flags.clone();
        flags.sort();
        flags.dedup();
        words.extend(flags.iter().map(|flag| flag_word(*flag).to_owned()));
        words.iter().map(|word| format!(".{word}")).collect()
    }
}

/// Returns the episode token: `S01E02`, `S01E02-E03` for a contiguous range, or
/// `S01E01E03` for a list (R2).
pub(crate) fn episode_token(season: u16, episodes: &[u32]) -> String {
    let is_contiguous_range =
        episodes.len() > 1 && episodes.windows(2).all(|pair| pair[1] == pair[0] + 1);
    match episodes {
        [first, .., last] if is_contiguous_range => format!("S{season:02}E{first:02}-E{last:02}"),
        _ => {
            let listed: String = episodes
                .iter()
                .map(|episode| format!("E{episode:02}"))
                .collect();
            format!("S{season:02}{listed}")
        }
    }
}

fn extras_folder(kind: ExtraKind) -> &'static str {
    data::EXTRAS_OUTPUT_FOLDERS
        .iter()
        .find(|(folder_kind, _)| *folder_kind == kind)
        .map(|(_, folder)| *folder)
        .expect("every extra kind has an output folder")
}

/// Returns the canonical name of an artwork asset role (R19, X25). Called only for the
/// roles that are artwork with a name of their own.
fn artwork_word(role: AssetRole) -> &'static str {
    let artwork = role
        .artwork_role()
        .expect("only artwork roles have an artwork name");
    data::ARTWORK_OUTPUT_WORDS
        .iter()
        .find(|(output_role, _)| *output_role == artwork)
        .map(|(_, word)| *word)
        .expect("every named artwork role has an output word")
}

/// Returns the canonical generic NFO name of a role (S9, X25).
fn nfo_name(role: NfoRole) -> &'static str {
    data::NFO_OUTPUT_NAMES
        .iter()
        .find(|(output_role, _)| *output_role == role)
        .map(|(_, name)| *name)
        .expect("every NFO role has an output name")
}

fn flag_word(flag: SidecarFlag) -> &'static str {
    data::SIDECAR_FLAG_OUTPUT_WORDS
        .iter()
        .find(|(word_flag, _)| *word_flag == flag)
        .map(|(_, word)| *word)
        .expect("every sidecar flag has an output word")
}

/// Joins the finished components into a path. Fails if a component is a reserved name
/// or exceeds the profile's byte limit (R5).
fn finish_components(
    components: &[String],
    profile: &OutputProfile,
) -> Result<String, Vec<BlockReason>> {
    let filesystem = profile.target_filesystem;
    let finished: Vec<String> = components
        .iter()
        .map(|component| filesystem.finish_component(component))
        .collect();
    let mut reasons = Vec::new();
    if finished
        .iter()
        .any(|component| profile.is_reserved(component))
    {
        reasons.push(BlockReason::ReservedName);
    }
    if finished
        .iter()
        .any(|component| component.len() > profile.max_component_bytes)
    {
        reasons.push(BlockReason::ComponentTooLong);
    }
    if reasons.is_empty() {
        Ok(finished.join("/"))
    } else {
        Err(reasons)
    }
}
