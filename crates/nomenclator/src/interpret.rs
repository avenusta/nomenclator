//! Interprets a supplied tree into items and the files each item owns (layer 2).
//!
//! The passes index and validate the entries, evaluate exclusion and classify each file by
//! its own name. `interpret` then dispatches once by library type to the movie or TV
//! pipeline. The pipeline resolves ownership and identity, assembles versions and parts, and
//! attaches extras and sidecars through `associations`. Outputs are sorted, so enumeration
//! order does not change any result.

mod associations;
mod movies;
pub(crate) mod parts;
mod scopes;

pub use scopes::ScopeGroup;
mod tv;

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::ignore::{ExclusionReason, IgnoreFile, IgnoreRules, RuleDiagnostic};
use crate::inputs::{
    AppliedHint, CallerInputs, EpisodeOrder, Hints, InputDiagnostic, MappedNumbering, Overrides,
    hint_diagnostics, normalized, override_diagnostics, series_diagnostics,
};
use crate::lexical::{self, BracketClass, TokenKind};
use crate::model::{ArtworkRole, EntryKind, ExtraKind, Finding, Format3d, SidecarFlag};
use crate::parsers::episode_name::AlternateIdentity;
use crate::parsers::episode_name::Special;
use crate::parsers::extras::{ExtrasContext, LibraryType, classify_extra_in};
use crate::parsers::file_types::is_disc_structure_folder;
pub(crate) use crate::parsers::file_types::parent_folder;
use crate::parsers::file_types::{UnsupportedVideoReason, file_name};
use crate::rules::{Rules, RulesIdentity, Samples};

/// The result of `interpret` for a supplied tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Interpretation {
    /// Movies, or series with their seasons and episodes, and extras no item owns,
    /// sorted by location.
    pub items: Vec<Item>,
    /// Each owner's extras, keyed by the owner: a movie's or episode's primary video
    /// path, a series' or season's folder.
    pub extras_of: BTreeMap<String, Vec<Extra>>,
    /// Subtitles attached to each video, features and extras alike.
    pub video_subtitles: BTreeMap<String, Vec<SidecarFile>>,
    /// Entries that an exclusion rule removed (deliberately excluded).
    pub excluded_entries: Vec<ExcludedEntry>,
    /// Entries that stay undecided because a rule file that could exclude them is unreadable
    /// (incomplete scope).
    pub undecided_entries: Vec<UndecidedEntry>,
    /// Paths that violate the input contract (invalid input).
    pub invalid_entries: Vec<InvalidEntry>,
    /// In-scope files that no item claims: unrecognized names, orphan sidecars, other files.
    pub unclaimed_files: Vec<String>,
    /// Problems in the supplied rule files.
    pub rule_diagnostics: Vec<RuleDiagnostic>,
    /// The unclaimed files that are release junk (X22). They stay in `unclaimed_files`.
    pub release_junk: Vec<String>,
    /// Caller inputs that no item took (T20).
    pub input_diagnostics: Vec<InputDiagnostic>,
    /// The sort key of each item of `items`, which `merge_scopes` orders the items of
    /// several scopes by (X23).
    #[serde(skip)]
    pub(crate) item_keys: Vec<String>,
    /// The rules this result was produced under.
    pub rules: RulesIdentity,
}

/// One interpreted item, tagged by `kind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Item {
    /// A movie with its versions and files.
    Movie(Box<Movie>),
    /// A series with its own extras and artwork.
    Series(Box<Series>),
    /// A season of a series.
    Season(Box<Season>),
    /// An episode coverage of a series.
    Episode(Box<Episode>),
    /// An extra without an owner. It is listed as an item so that it stays visible.
    Extra(OrphanExtra),
}

/// A movie: one work, its versions, and the files that belong to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Movie {
    /// The title, from the owning folder's or the video's name.
    pub title: Option<String>,
    /// The other titles that the name giving the title states (M16).
    pub alternate_titles: Vec<String>,
    /// The folder or file whose name gave the title.
    pub title_source: Option<String>,
    /// The release year.
    pub year: Option<u16>,
    /// The folder or file whose name gave the year.
    pub year_source: Option<String>,
    /// The folder that owns the movie; `None` for a video that is its own movie.
    pub path: Option<String>,
    /// The collection or category folder that contains the movie's folder, or the movie's
    /// video if the movie has no folder (R10). `None` at the library root, and if that folder
    /// is another movie's folder.
    pub collection: Option<String>,
    /// Provider IDs stated in the names, keyed by provider.
    pub provider_ids: BTreeMap<String, Finding<String>>,
    /// Alternative representations, primary first.
    pub versions: Vec<Version>,
    /// Extras the movie owns.
    pub extras: Vec<Extra>,
    /// Subtitle files attached to the movie.
    pub subtitles: Vec<SidecarFile>,
    /// External audio files attached to the movie.
    pub audio: Vec<SidecarFile>,
    /// Artwork that belongs to the movie.
    pub artwork: Vec<Artwork>,
    /// The primary version's NFO, or the folder's generic NFO if the version has none (S9).
    pub nfo: Option<String>,
    /// The hinted facts that apply to the movie, with their outcomes (T20).
    pub hints: Vec<AppliedHint>,
    /// `true` if a video has an episode token in a season folder: series content in a movie
    /// library (M15).
    pub misplaced_series: bool,
    /// `true` if the evidence does not resolve the movie. `ambiguity_reasons` lists the reasons.
    pub ambiguous: bool,
    /// Why the movie is ambiguous, as stable `snake_case` codes.
    pub ambiguity_reasons: Vec<&'static str>,
}

/// A series: the work its seasons and episodes belong to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Series {
    /// The series title, from its folder or its files' names.
    pub title: Option<String>,
    /// The series year, if a name contains one.
    pub year: Option<u16>,
    /// The series folder (T1), also when the series title comes from a file name. `None` at the
    /// root.
    pub path: Option<String>,
    /// The series folder's provider IDs.
    pub provider_ids: BTreeMap<String, Finding<String>>,
    /// The text that precedes the episode code in every episode file of the series' season
    /// folders, as another title for the same series (T2). `None` if there is no such text or
    /// the files contain different texts.
    pub alternate_identity: Option<AlternateIdentity>,
    /// Extras the series owns.
    pub extras: Vec<Extra>,
    /// Artwork that belongs to the series.
    pub artwork: Vec<Artwork>,
    /// The series folder's `tvshow.nfo` (S9).
    pub nfo: Option<String>,
    /// The hinted facts that apply to the series, with their outcomes (T20).
    pub hints: Vec<AppliedHint>,
}

/// A season of a series, from its folder or its episodes' references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Season {
    /// The title of the series the season belongs to.
    pub series_name: Option<String>,
    /// The series folder; `None` at the root.
    pub series_path: Option<String>,
    /// The season number.
    pub season: u16,
    /// The season folder that contains its episodes. `None` if there is no such folder or there
    /// are several.
    pub path: Option<String>,
    /// The season folder's own provider IDs.
    pub provider_ids: BTreeMap<String, Finding<String>>,
    /// Extras the season owns.
    pub extras: Vec<Extra>,
    /// Artwork that belongs to the season.
    pub artwork: Vec<Artwork>,
    /// The season folder's `season.nfo` (S9).
    pub nfo: Option<String>,
}

/// One episode coverage of a series: the videos that cover the same episodes. A
/// multi-episode file is one episode item that lists every episode the file covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Episode {
    /// The title of the series the episode belongs to.
    pub series_name: Option<String>,
    /// The series year, if a name contains one.
    pub series_year: Option<u16>,
    /// The series folder; `None` at the root.
    pub series_path: Option<String>,
    /// The season number and the evidence for it.
    pub season: Finding<u16>,
    /// Episode numbers within the season, ranges expanded.
    pub episodes: Option<Vec<u32>>,
    /// The first episode covered.
    pub first_episode: Option<u32>,
    /// Absolute numbers, never mixed into `episodes` (T14).
    pub absolute: Option<Vec<u32>>,
    /// The air date of a dated episode.
    pub date: Finding<String>,
    /// The primary version's episode title (T19, T2).
    pub episode_title: Option<String>,
    /// The disc or per-episode folder that contains it (T18).
    pub path: Option<String>,
    /// The episode files' own provider IDs.
    pub provider_ids: BTreeMap<String, Finding<String>>,
    /// Videos with the same coverage, primary first (T17).
    pub versions: Vec<Version>,
    /// Extras the episode owns.
    pub extras: Vec<Extra>,
    /// Subtitle files attached to the episode.
    pub subtitles: Vec<SidecarFile>,
    /// External audio files attached to the episode.
    pub audio: Vec<SidecarFile>,
    /// Artwork that belongs to the episode.
    pub artwork: Vec<Artwork>,
    /// The primary version's NFO (S9).
    pub nfo: Option<String>,
    /// The hinted facts that apply to the episode, with their outcomes (T20).
    pub hints: Vec<AppliedHint>,
    /// The order of its season and episode numbers, if the caller states orders (X17).
    /// `absolute` for an episode with absolute numbers only.
    pub order: Option<EpisodeOrder>,
    /// The numbers that a caller's numbering map replaced (X17).
    pub mapped_from: Option<MappedNumbering>,
    /// The special the episode is, when its name states one instead of a number (X20).
    pub special: Option<Special>,
    /// `true` if the evidence does not resolve the episode. `ambiguity_reasons` lists the
    /// reasons.
    pub ambiguous: bool,
    /// Why the episode is ambiguous, as stable `snake_case` codes.
    pub ambiguity_reasons: Vec<&'static str>,
}

/// One representation: a file, a disc, or ordered parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Version {
    /// One path, or the ordered parts. A disc structure is its folder, ending in `/`.
    pub paths: Vec<String>,
    /// The label that distinguishes this version from the others. `None` if the movie has one
    /// version or the version's name contains nothing beyond the movie's name.
    pub label: Option<String>,
    /// `missing_part` or `duplicate_part` if the parts are not numbered 1, 2, … in sequence.
    pub ambiguity_reason: Option<&'static str>,
    /// The raw `{edition-X}` tag of the version's name, or of its folder's name if the version's
    /// name has none (M9).
    pub edition: Option<String>,
    /// The raw edition words after the title in the version's name, or in its folder's name if
    /// the version's name has none.
    pub edition_hint: Option<String>,
    /// The 3D layout in the version's name, or in its folder's name if the version's name has
    /// none.
    pub format_3d: Option<Format3d>,
    /// The NFO named after the version's video or its parts' shared name (S9).
    pub nfo: Option<String>,
}

/// An extra attached to its owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Extra {
    /// Its file, or its ordered parts.
    pub paths: Vec<String>,
    /// The kind of extra.
    pub extra: ExtraKind,
    /// The title from its name.
    pub title: Option<String>,
    /// The year from its name.
    pub year: Option<u16>,
}

/// An extra no item owns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrphanExtra {
    /// Its file, or its ordered parts.
    pub paths: Vec<String>,
    /// The kind of extra.
    pub extra: ExtraKind,
    /// The title from its name.
    pub title: Option<String>,
    /// The year from its name.
    pub year: Option<u16>,
    /// `true` if several items are equally likely owners.
    pub ambiguous: bool,
}

/// A subtitle or external audio file attached to a video.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SidecarFile {
    /// The sidecar's root-relative path.
    pub path: String,
    /// The language in its name.
    pub language: Finding<String>,
    /// The flags in its name: default, forced, hearing impaired.
    pub flags: Vec<SidecarFlag>,
    /// The track title in its name.
    pub title: Finding<String>,
}

/// An image and its role, from its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Artwork {
    /// The image's root-relative path.
    pub path: String,
    /// What the image is for, such as a poster.
    pub role: ArtworkRole,
}

/// An entry that an exclusion removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExcludedEntry {
    /// The excluded entry's path.
    pub path: String,
    /// Why it is excluded.
    pub reason: ExclusionReason,
    /// The entry or the folder above it that matched.
    pub matched_path: String,
    /// The rule file whose pattern matched; `None` for a built-in.
    pub rule_file: Option<String>,
}

/// An entry that an unreadable rule file could exclude.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UndecidedEntry {
    /// The undecided entry's path.
    pub path: String,
    /// The unreadable rule files that could decide it.
    pub unreadable_rule_files: Vec<String>,
}

/// A supplied path that breaks the input contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InvalidEntry {
    /// The path as supplied.
    pub path: String,
    /// Why it is invalid.
    pub reason: InvalidEntryReason,
}

/// Why a supplied path is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InvalidEntryReason {
    /// The path is empty or starts with `/`. Paths must be root-relative.
    NotRootRelative,
    /// The path has an empty, `.` or `..` component.
    InvalidComponent,
    /// The same path is supplied twice. The first one is kept.
    Duplicate,
}

/// Interprets a complete scope: every entry of the library, as root-relative paths
/// (directories end in `/`), with the contents of the rule files found in it.
///
/// Uses the built-in rules. [`Rules::interpret`] accepts other rules.
#[must_use]
pub fn interpret(
    library: LibraryType,
    paths: &[String],
    ignore_files: &[IgnoreFile],
) -> Interpretation {
    Rules::builtin().interpret(library, paths, ignore_files)
}

/// Interprets a complete scope like [`interpret`], with the caller inputs, such as hints per
/// path (T20).
///
/// Uses the built-in rules. [`Rules::interpret_with`] accepts other rules.
#[must_use]
pub fn interpret_with(
    library: LibraryType,
    paths: &[String],
    ignore_files: &[IgnoreFile],
    inputs: &CallerInputs,
) -> Interpretation {
    Rules::builtin().interpret_with(library, paths, ignore_files, inputs)
}

impl Rules {
    /// Interprets a complete scope: every entry of the library, as root-relative paths
    /// (directories end in `/`), with the contents of the rule files found in it.
    pub fn interpret(
        &self,
        library: LibraryType,
        paths: &[String],
        ignore_files: &[IgnoreFile],
    ) -> Interpretation {
        self.interpret_with(library, paths, ignore_files, &CallerInputs::default())
    }

    /// Interprets a complete scope like [`Rules::interpret`], with the caller inputs, such as
    /// hints per path (T20). Every input that no item takes is reported in
    /// `input_diagnostics`.
    pub fn interpret_with(
        &self,
        library: LibraryType,
        paths: &[String],
        ignore_files: &[IgnoreFile],
        inputs: &CallerInputs,
    ) -> Interpretation {
        let rules = self;
        let ignore_rules = rules.ignore_rules(ignore_files);
        let (index, outcomes) =
            EntryIndex::build(rules, library, paths, &ignore_rules, &inputs.overrides);
        let hints = Hints::new(&inputs.hints);
        let output = match library {
            LibraryType::Movies => movies::interpret(rules, &index, &hints, &inputs.overrides),
            LibraryType::Tv => tv::interpret(rules, &index, &hints, inputs),
        };
        let input_diagnostics = input_diagnostics(rules, inputs, paths, &output.items);
        let unclaimed_files: Vec<String> = index
            .files
            .keys()
            .filter(|path| !output.claimed_files.contains(*path))
            .cloned()
            .collect();
        let release_junk = unclaimed_files
            .iter()
            .filter(|path| rules.is_release_junk(path).result)
            .cloned()
            .collect();
        Interpretation {
            items: output.items,
            item_keys: output.item_keys,
            extras_of: output.extras_of,
            video_subtitles: output.video_subtitles,
            excluded_entries: outcomes.excluded_entries,
            undecided_entries: outcomes.undecided_entries,
            invalid_entries: outcomes.invalid_entries,
            unclaimed_files,
            rule_diagnostics: ignore_rules.diagnostics().to_vec(),
            release_junk,
            input_diagnostics,
            rules: rules.identity().clone(),
        }
    }
}

/// Returns the inputs that no item of `items` takes, and the inputs for paths outside
/// `paths` (T20, X16, X17), sorted.
pub(crate) fn input_diagnostics(
    rules: &Rules,
    inputs: &CallerInputs,
    paths: &[String],
    items: &[Item],
) -> Vec<InputDiagnostic> {
    let mut listed: BTreeSet<String> = BTreeSet::new();
    for path in paths {
        let mut current = Some(normalized(path));
        while let Some(path) = current {
            current = parent_folder(&path).map(str::to_owned);
            listed.insert(path);
        }
    }
    let mut diagnostics = hint_diagnostics(&inputs.hints, &listed, items);
    diagnostics.extend(override_diagnostics(&inputs.overrides, &listed, items));
    diagnostics.extend(series_diagnostics(rules, &inputs.series, &listed, items));
    diagnostics.sort();
    diagnostics
}

/// The output of a library pipeline.
#[derive(Default)]
struct PipelineOutput {
    items: Vec<Item>,
    /// The sort key of each item.
    item_keys: Vec<String>,
    extras_of: BTreeMap<String, Vec<Extra>>,
    video_subtitles: BTreeMap<String, Vec<SidecarFile>>,
    claimed_files: BTreeSet<String>,
}

#[derive(Default)]
struct EntryOutcomes {
    excluded_entries: Vec<ExcludedEntry>,
    undecided_entries: Vec<UndecidedEntry>,
    invalid_entries: Vec<InvalidEntry>,
}

/// The role of a file by its own name, before ownership is resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileRole {
    FeatureVideo,
    /// An `.iso` or `.img`: a disc structure in one file (M13).
    DiscImage,
    /// A video or theme audio that is an extra by its name or by its extras folder.
    Extra {
        kind: ExtraKind,
        /// The extra's title text, without its suffix.
        title: Option<String>,
        /// The folder whose title the extra belongs to; `None` at the library root.
        owner_folder: Option<String>,
        /// `true` if the caller set the owner (X16). The folder's own item then owns the extra.
        owner_is_forced: bool,
    },
    Subtitle,
    /// The `.sub` of an `idx`/`sub` pair. It is attached through its `.idx`.
    SubtitlePairPart,
    Audio,
    Image,
    /// A file inside a `VIDEO_TS/` or `BDMV/` folder. It belongs to its disc structure.
    DiscContent,
    Other,
}

/// The in-scope entries: valid, not excluded, not undecided.
pub(crate) struct EntryIndex {
    /// Files by root-relative path.
    pub files: BTreeMap<String, FileRole>,
    /// Folders without the trailing `/`, supplied or implied by a file below them.
    pub folders: BTreeSet<String>,
    /// Folders that contain a `VIDEO_TS/` or `BDMV/` folder: one disc structure each (M13).
    pub disc_folders: BTreeSet<String>,
}

impl EntryIndex {
    fn build(
        rules: &Rules,
        library: LibraryType,
        paths: &[String],
        ignore_rules: &IgnoreRules,
        overrides: &Overrides,
    ) -> (EntryIndex, EntryOutcomes) {
        let mut outcomes = EntryOutcomes::default();
        let mut sorted_paths: Vec<&String> = paths.iter().collect();
        sorted_paths.sort();
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut file_paths: Vec<String> = Vec::new();
        let mut folders: BTreeSet<String> = BTreeSet::new();
        for path in sorted_paths {
            if let Some(reason) = invalid_reason(path, &mut seen) {
                outcomes.invalid_entries.push(InvalidEntry {
                    path: path.clone(),
                    reason,
                });
                continue;
            }
            let is_folder = path.ends_with('/');
            let kind = if is_folder {
                EntryKind::Directory
            } else {
                EntryKind::File
            };
            let evaluation = ignore_rules.evaluate(path, kind);
            match (evaluation.excluded, evaluation.exclusion) {
                (Some(false), _) if is_folder => {
                    folders.insert(path.trim_end_matches('/').to_owned());
                }
                (Some(false), _) => file_paths.push(path.clone()),
                (Some(true), Some(exclusion)) => {
                    outcomes.excluded_entries.push(ExcludedEntry {
                        path: path.clone(),
                        reason: exclusion.reason,
                        matched_path: exclusion.path,
                        rule_file: exclusion.rule_file,
                    });
                }
                _ => outcomes.undecided_entries.push(UndecidedEntry {
                    path: path.clone(),
                    unreadable_rule_files: evaluation.unreadable_rule_files,
                }),
            }
        }
        for path in &file_paths {
            let mut folder = parent_folder(path);
            while let Some(current) = folder {
                folders.insert(current.to_owned());
                folder = parent_folder(current);
            }
        }
        let disc_folders = folders
            .iter()
            .filter(|folder| is_disc_structure_folder(rules, file_name(folder)))
            .filter_map(|folder| parent_folder(folder).map(str::to_owned))
            .collect();
        let context = ClassificationContext {
            extras: ExtrasContext::new(Some(library), &file_paths),
            files_by_folder: files_by_folder(&file_paths),
        };
        let mut files: BTreeMap<String, FileRole> = file_paths
            .iter()
            .map(|path| (path.clone(), classify_file(rules, path, &context)))
            .collect();
        // The caller's extras overrides set the kind and the owner (X16).
        for extra in &overrides.extras {
            let Some(role) = files.get_mut(&extra.path) else {
                continue;
            };
            let parent = parent_folder(&extra.path).map(str::to_owned);
            match (role.clone(), extra.kind) {
                (
                    FileRole::Extra {
                        kind,
                        title,
                        owner_folder,
                        ..
                    },
                    kind_override,
                ) => {
                    *role = FileRole::Extra {
                        kind: kind_override.unwrap_or(kind),
                        title,
                        owner_folder: extra.owner.clone().or(owner_folder),
                        owner_is_forced: extra.owner.is_some(),
                    };
                }
                (FileRole::FeatureVideo, Some(kind)) => {
                    let stem = lexical::stem(file_name(&extra.path), EntryKind::File);
                    *role = FileRole::Extra {
                        kind,
                        title: Some(lexical::display_title(stem)),
                        owner_folder: extra.owner.clone().or(parent),
                        owner_is_forced: extra.owner.is_some(),
                    };
                }
                _ => {}
            }
        }
        // Under the excluded-samples preset a sample is excluded, not an extra (E6, X12).
        if rules.presets.samples == Samples::Excluded {
            files.retain(|path, role| {
                let is_sample = matches!(
                    role,
                    FileRole::Extra {
                        kind: ExtraKind::Sample,
                        ..
                    }
                );
                if is_sample {
                    outcomes.excluded_entries.push(ExcludedEntry {
                        path: path.clone(),
                        reason: ExclusionReason::Sample,
                        matched_path: path.clone(),
                        rule_file: None,
                    });
                }
                !is_sample
            });
            outcomes
                .excluded_entries
                .sort_by(|first, second| first.path.cmp(&second.path));
        }
        let index = EntryIndex {
            files,
            folders,
            disc_folders,
        };
        (index, outcomes)
    }
}

fn invalid_reason<'a>(path: &'a str, seen: &mut BTreeSet<&'a str>) -> Option<InvalidEntryReason> {
    if path.is_empty() || path.starts_with('/') {
        return Some(InvalidEntryReason::NotRootRelative);
    }
    let has_invalid_component = path
        .trim_end_matches('/')
        .split('/')
        .any(|component| component.is_empty() || component == "." || component == "..");
    if has_invalid_component {
        return Some(InvalidEntryReason::InvalidComponent);
    }
    (!seen.insert(path)).then_some(InvalidEntryReason::Duplicate)
}

/// An index of the in-scope files, built once and used to classify each file.
struct ClassificationContext {
    extras: ExtrasContext,
    /// Files by the folder that contains them. The key is `None` at the library root.
    files_by_folder: BTreeMap<Option<String>, Vec<String>>,
}

/// Groups files by the folder that contains them, in the order given.
fn files_by_folder<'a>(
    paths: impl IntoIterator<Item = &'a String>,
) -> BTreeMap<Option<String>, Vec<String>> {
    let mut by_folder: BTreeMap<Option<String>, Vec<String>> = BTreeMap::new();
    for path in paths {
        let folder = parent_folder(path).map(str::to_owned);
        by_folder.entry(folder).or_default().push(path.clone());
    }
    by_folder
}

/// Returns a file's role from its own name and, for extras folders and `idx`/`sub` pairs,
/// from the other in-scope files.
fn classify_file(rules: &Rules, path: &str, context: &ClassificationContext) -> FileRole {
    let is_inside_disc_structure = path
        .split('/')
        .rev()
        .skip(1)
        .any(|folder| is_disc_structure_folder(rules, folder));
    if is_inside_disc_structure {
        return FileRole::DiscContent;
    }
    // Release junk is never artwork or a sidecar: it stays an unclaimed file (X22).
    if rules.is_release_junk(path).result {
        return FileRole::Other;
    }
    let video = rules.is_video_file(path);
    let is_audio = rules.is_audio_file(path).result;
    if video.result || is_audio {
        let classification = classify_extra_in(rules, path, EntryKind::File, &context.extras);
        if let Finding::Resolved { value: kind, .. } = classification.extra {
            return FileRole::Extra {
                kind,
                title: classification.title,
                owner_folder: classification.owner,
                owner_is_forced: false,
            };
        }
        return if video.result {
            FileRole::FeatureVideo
        } else {
            FileRole::Audio
        };
    }
    if video.unsupported_video_reason == Some(UnsupportedVideoReason::DiscImage) {
        return FileRole::DiscImage;
    }
    let siblings = context
        .files_by_folder
        .get(&parent_folder(path).map(str::to_owned))
        .map_or(&[][..], Vec::as_slice);
    if rules.is_subtitle_file(path, siblings).result {
        return FileRole::Subtitle;
    }
    if lexical::extension(file_name(path), EntryKind::File).as_deref() == Some("sub") {
        return FileRole::SubtitlePairPart;
    }
    if rules.is_image_file(path).result {
        return FileRole::Image;
    }
    FileRole::Other
}

/// Returns the inside of a text that is one bracketed token: `[test]` → `test`. Returns any
/// other text unchanged, such as `[a] [b]`.
pub(crate) fn unwrap_single_bracket(text: &str) -> &str {
    let tokens = lexical::tokenize(text);
    match tokens.as_slice() {
        [token] if matches!(token.kind, lexical::TokenKind::Bracket { .. }) => {
            let lexical::TokenKind::Bracket { inner } = token.kind else {
                return text;
            };
            text[inner.0..inner.1].trim()
        }
        _ => text,
    }
}

/// Returns the text with its provider-ID and edition tags removed and spaces collapsed.
pub(crate) fn without_provider_and_edition_tags(rules: &Rules, text: &str) -> String {
    let mut kept = String::new();
    let mut position = 0;
    for token in lexical::tokenize(text) {
        let TokenKind::Bracket { inner } = token.kind else {
            continue;
        };
        let is_tag = matches!(
            lexical::classify_bracket(rules, &text[inner.0..inner.1]),
            BracketClass::ProviderTag { .. }
                | BracketClass::BareImdbId(_)
                | BracketClass::EditionTag(_)
        );
        if is_tag {
            kept.push_str(&text[position..token.span.0]);
            position = token.span.1;
        }
    }
    if position == 0 {
        return text.to_owned();
    }
    kept.push_str(&text[position..]);
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Returns the vertical resolution that a label starts with (`1080p`, `4K`, `UHD`). Returns 0
/// if the label does not start with a resolution.
pub(crate) fn resolution_of(label: &str) -> u32 {
    label
        .split([' ', '.', '_', '-', '[', ']', '(', ')'])
        .find(|word| !word.is_empty())
        .and_then(lexical::resolution)
        .map_or(0, |(height, _)| height)
}

/// Merges provider-ID claim sets into one finding per provider, with the values in order of
/// precedence. Returns an ambiguous finding that keeps every claim if one provider has
/// different values (M8).
pub(crate) fn merge_provider_claims<'a>(
    claim_sets: impl Iterator<Item = &'a BTreeMap<String, Finding<String>>>,
) -> BTreeMap<String, Finding<String>> {
    let mut claims: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for ids in claim_sets {
        for (provider, finding) in ids {
            let values = claims.entry(provider.clone()).or_default();
            let claimed: Vec<String> = match finding {
                Finding::Resolved { value, .. } => vec![value.clone()],
                Finding::Ambiguous { candidates, .. } => candidates.clone(),
                Finding::Unknown => Vec::new(),
            };
            for value in claimed {
                if !values.contains(&value) {
                    values.push(value);
                }
            }
        }
    }
    claims
        .into_iter()
        .map(|(provider, mut values)| {
            let finding = if values.len() == 1 {
                Finding::Resolved {
                    value: values.remove(0),
                    evidence: None,
                }
            } else {
                Finding::Ambiguous {
                    candidates: values,
                    reason: crate::model::CONFLICTING_PROVIDER_ID,
                }
            };
            (provider, finding)
        })
        .collect()
}

/// Sorts each video's sidecars by path.
pub(crate) fn sorted_by_path(
    mut by_video: BTreeMap<String, Vec<SidecarFile>>,
) -> BTreeMap<String, Vec<SidecarFile>> {
    for sidecars in by_video.values_mut() {
        sidecars.sort_by(|left, right| left.path.cmp(&right.path));
    }
    by_video
}

/// Returns the outermost of nested folders that have the same name, so that
/// `Alien (1979)/Alien (1979)/` is one folder.
pub(crate) fn collapse_same_named_wrappers(folder: &str) -> &str {
    let mut folder = folder;
    while let Some(parent) = parent_folder(folder).filter(|parent| {
        lexical::fold_case_text(file_name(parent)) == lexical::fold_case_text(file_name(folder))
    }) {
        folder = parent;
    }
    folder
}
