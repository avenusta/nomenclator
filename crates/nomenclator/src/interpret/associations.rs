//! Attaches extras and sidecars to established owners (pass 6) for the movie and TV
//! pipelines. An extra with several equal owner candidates stays ambiguous. An extra without
//! an owner stays visible as an orphan.

use std::collections::{BTreeMap, BTreeSet};

use super::{Artwork, EntryIndex, Extra, FileRole, SidecarFile, parent_folder};
use crate::data;
use crate::lexical;
use crate::model::{ArtworkRole, EntryKind, ExtraKind, Finding, NfoRole, SidecarKind};
use crate::parsers::file_types::file_name;
use crate::parsers::nfo::{FolderVideoNames, classify_nfo_among};
use crate::parsers::sidecar::{Sidecar, parse_sidecar_descriptors};
use crate::rules::Rules;

/// An item that extras and folder assets can be attached to.
pub(crate) struct OwnerCandidate {
    /// The folder that owns the item; `None` for a video that is its own item.
    pub owning_folder: Option<String>,
    /// Folders that directly contain the item's media. `""` is the library root.
    pub media_folders: BTreeSet<String>,
    /// The item's names as comparison words: its videos' stems and stack names.
    pub names: Vec<Vec<String>>,
    pub title_words: Vec<String>,
    pub year: Option<u16>,
    /// `true` if the item is its owning folder's own work, not another work in the same folder.
    pub is_folder_identity: bool,
    /// `true` if generic artwork in its folder can belong to the item. `false` for an episode,
    /// whose folder belongs to its series or season.
    pub takes_folder_assets: bool,
}

impl OwnerCandidate {
    fn is_located_in(&self, folder: Option<&str>) -> bool {
        let is_owner = folder.is_some() && self.owning_folder.as_deref() == folder;
        is_owner || self.media_folders.contains(folder.unwrap_or(""))
    }
}

/// The owner candidates in item order, indexed by the folders they are located in (the
/// folder that owns each, and the folders that contain its media) and by their names.
pub(crate) struct OwnerCandidates {
    candidates: Vec<OwnerCandidate>,
    /// Per folder, the candidates it owns, in candidate order.
    by_owning_folder: BTreeMap<String, Vec<usize>>,
    /// Per folder (`""` for the library root), the candidates with media directly in it.
    by_media_folder: BTreeMap<String, Vec<usize>>,
    /// Per name in comparison words, the candidates that have that name.
    by_name: BTreeMap<Vec<String>, Vec<usize>>,
    /// Per title in comparison words, the candidates with that title.
    by_title_words: BTreeMap<Vec<String>, Vec<usize>>,
}

impl OwnerCandidates {
    pub(crate) fn new(candidates: Vec<OwnerCandidate>) -> Self {
        let mut by_owning_folder: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut by_media_folder: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut by_name: BTreeMap<Vec<String>, Vec<usize>> = BTreeMap::new();
        let mut by_title_words: BTreeMap<Vec<String>, Vec<usize>> = BTreeMap::new();
        for (position, candidate) in candidates.iter().enumerate() {
            for name in &candidate.names {
                by_name.entry(name.clone()).or_default().push(position);
            }
            if !candidate.title_words.is_empty() {
                by_title_words
                    .entry(candidate.title_words.clone())
                    .or_default()
                    .push(position);
            }
            if let Some(folder) = &candidate.owning_folder {
                by_owning_folder
                    .entry(folder.clone())
                    .or_default()
                    .push(position);
            }
            for folder in &candidate.media_folders {
                by_media_folder
                    .entry(folder.clone())
                    .or_default()
                    .push(position);
            }
        }
        OwnerCandidates {
            candidates,
            by_owning_folder,
            by_media_folder,
            by_name,
            by_title_words,
        }
    }

    pub(crate) fn get(&self, position: usize) -> &OwnerCandidate {
        &self.candidates[position]
    }

    /// Returns the candidates that `folder` owns, in candidate order.
    pub(crate) fn owned_by(&self, folder: &str) -> &[usize] {
        self.by_owning_folder.get(folder).map_or(&[], Vec::as_slice)
    }

    /// Returns the candidates that `title` matches, in candidate order. A candidate matches if
    /// `title` equals one of its names, or if `title` parses to its title with a year that
    /// does not differ.
    fn named_by(&self, rules: &Rules, title: &str) -> BTreeSet<usize> {
        let words = lexical::comparison_words(title);
        let mut named: BTreeSet<usize> = self
            .by_name
            .get(&words)
            .into_iter()
            .flatten()
            .copied()
            .collect();
        let parsed = rules.parse_movie_name(title, EntryKind::Directory);
        let Finding::Resolved { value, .. } = &parsed.title else {
            return named;
        };
        let titled = self.by_title_words.get(&lexical::comparison_words(value));
        named.extend(titled.into_iter().flatten().copied().filter(|&position| {
            match (&parsed.year, self.candidates[position].year) {
                (Finding::Resolved { value: year, .. }, Some(own_year)) => *year == own_year,
                _ => true,
            }
        }));
        named
    }

    /// Returns the candidates that `folder` owns or that have media in `folder`, in candidate
    /// order.
    fn located_in(&self, folder: Option<&str>) -> Vec<usize> {
        let owned = folder.map_or(&[][..], |folder| self.owned_by(folder));
        let holding = self
            .by_media_folder
            .get(folder.unwrap_or(""))
            .map_or(&[][..], Vec::as_slice);
        let located: BTreeSet<usize> = owned.iter().chain(holding).copied().collect();
        located.into_iter().collect()
    }
}

/// Builds an extra from a file, with its title parsed like a movie name (E3). If the title
/// contains no spaces, the extra's title is the raw text of the parsed title's span: the
/// title kept its separators because its siblings contain spaces (M6).
pub(crate) fn extra_file(rules: &Rules, path: &str, kind: ExtraKind, title: Option<&str>) -> Extra {
    let parsed_title = title.map(|title| rules.parse_movie_name(title, EntryKind::Directory));
    let own_title =
        title
            .zip(parsed_title.as_ref())
            .and_then(|(raw, parsed)| match &parsed.title {
                Finding::Resolved {
                    evidence: Some(evidence),
                    ..
                } if !raw.contains(' ') => {
                    raw.get(evidence.span.0..evidence.span.1).map(str::to_owned)
                }
                title => title.resolved_value().cloned(),
            });
    Extra {
        paths: vec![path.to_owned()],
        extra: kind,
        title: own_title,
        year: parsed_title
            .as_ref()
            .and_then(|parsed| parsed.year.resolved_value().copied()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnerChoice {
    Owner(usize),
    /// Several candidates match equally.
    Tied,
    Orphan,
}

/// Chooses the owner of an extra found in `folder` (`None` at the library root). A titled
/// extra is assigned to the one item that its title matches. Otherwise, outside the root,
/// the extra is assigned to the folder's only item, or to the one item that is the folder's
/// own work (E2-E5).
pub(crate) fn choose_extra_owner(
    rules: &Rules,
    folder: Option<&str>,
    extra_title: Option<&str>,
    owner_is_forced: bool,
    candidates: &OwnerCandidates,
) -> OwnerChoice {
    if owner_is_forced {
        // The caller set the folder, so the folder's own item owns the extra (X16).
        let owning: Vec<usize> = candidates
            .located_in(folder)
            .into_iter()
            .filter(|&index| {
                let candidate = candidates.get(index);
                candidate.is_folder_identity && candidate.owning_folder.as_deref() == folder
            })
            .collect();
        return match owning.as_slice() {
            [only] => OwnerChoice::Owner(*only),
            [] => OwnerChoice::Orphan,
            _ => OwnerChoice::Tied,
        };
    }
    if let Some(title) = extra_title {
        let named: Vec<usize> = candidates
            .named_by(rules, title)
            .into_iter()
            .filter(|&index| candidates.get(index).is_located_in(folder))
            .collect();
        match named.as_slice() {
            [only] => return OwnerChoice::Owner(*only),
            [_, _, ..] => return OwnerChoice::Tied,
            [] => {}
        }
    }
    if folder.is_none() {
        return OwnerChoice::Orphan;
    }
    let located = candidates.located_in(folder);
    if let [only] = located.as_slice() {
        return OwnerChoice::Owner(*only);
    }
    let folder_works: Vec<usize> = located
        .iter()
        .copied()
        .filter(|&index| {
            let candidate = candidates.get(index);
            candidate.is_folder_identity && candidate.owning_folder.as_deref() == folder
        })
        .collect();
    match (folder_works.as_slice(), located.is_empty()) {
        ([only], _) => OwnerChoice::Owner(*only),
        (_, true) => OwnerChoice::Orphan,
        _ => OwnerChoice::Tied,
    }
}

/// Chooses the owner of a generic asset (artwork not named for a video) in `folder`. Only
/// the single item of a folder can own the asset. An asset at the library root has no owner
/// (S8).
pub(crate) fn choose_asset_owner(
    folder: Option<&str>,
    candidates: &OwnerCandidates,
) -> OwnerChoice {
    if folder.is_none() {
        return OwnerChoice::Orphan;
    }
    let located: Vec<usize> = candidates
        .located_in(folder)
        .into_iter()
        .filter(|&index| candidates.get(index).takes_folder_assets)
        .collect();
    match located.as_slice() {
        [only] => OwnerChoice::Owner(*only),
        [] => OwnerChoice::Orphan,
        _ => OwnerChoice::Tied,
    }
}

/// Sidecars attached to individual videos.
#[derive(Default)]
pub(crate) struct SidecarAttachments {
    pub subtitles_by_video: BTreeMap<String, Vec<SidecarFile>>,
    pub audio_by_video: BTreeMap<String, Vec<SidecarFile>>,
    /// Images named exactly like a video, or like it plus `-thumb` (S6).
    pub stills_by_video: BTreeMap<String, Vec<String>>,
    pub claimed_files: BTreeSet<String>,
}

/// The possible location of a sidecar's video: the sidecar's own folder, the folder above a
/// `Subs/` folder, or the folder above `Subs/<video stem>/` (S2).
struct SidecarPlace<'a> {
    video_folder: Option<&'a str>,
    /// In `Subs/<video stem>/`: the video stem that is the folder's name.
    stem_folder: Option<&'a str>,
    is_in_subtitle_folder: bool,
}

fn sidecar_place(path: &str, kind: SidecarKind) -> SidecarPlace<'_> {
    let folder = parent_folder(path);
    let is_subtitle_folder = |folder: &str| {
        let name = lexical::vocabulary_form(file_name(folder));
        data::SUBTITLE_FOLDER_NAMES.contains(&name.as_str())
    };
    match folder {
        _ if kind == SidecarKind::Image => SidecarPlace {
            video_folder: folder,
            stem_folder: None,
            is_in_subtitle_folder: false,
        },
        Some(folder) if is_subtitle_folder(folder) => SidecarPlace {
            video_folder: parent_folder(folder),
            stem_folder: None,
            is_in_subtitle_folder: true,
        },
        Some(folder) if parent_folder(folder).is_some_and(is_subtitle_folder) => SidecarPlace {
            video_folder: parent_folder(folder).and_then(parent_folder),
            stem_folder: Some(file_name(folder)),
            is_in_subtitle_folder: true,
        },
        _ => SidecarPlace {
            video_folder: folder,
            stem_folder: None,
            is_in_subtitle_folder: false,
        },
    }
}

/// Attaches subtitles, external audio and stills to videos (S2, S3, S5, S6). A sidecar
/// matches a video if its name starts with the full video stem followed by a delimiter. The
/// longest stem takes precedence, then the stem that matches letter case exactly. A sidecar
/// that still matches several videos stays unattached. A descriptor-only name in a subtitle
/// folder (`Subs/English.srt`) is attached to its folder's only feature.
pub(crate) fn attach_sidecars(rules: &Rules, index: &EntryIndex) -> SidecarAttachments {
    let mut videos: BTreeMap<Option<&str>, FolderVideos> = BTreeMap::new();
    for (path, role) in &index.files {
        let is_video = matches!(role, FileRole::FeatureVideo)
            || (matches!(role, FileRole::Extra { .. }) && rules.is_video_file(path).result);
        if is_video {
            videos.entry(parent_folder(path)).or_default().add(path);
        }
    }
    let no_videos = FolderVideos::default();
    let mut attachments = SidecarAttachments::default();
    for (path, role) in &index.files {
        let kind = match role {
            FileRole::Subtitle => SidecarKind::Subtitle,
            FileRole::Audio => SidecarKind::Audio,
            FileRole::Image => SidecarKind::Image,
            _ => continue,
        };
        let place = sidecar_place(path, kind);
        let nearby = videos.get(&place.video_folder).unwrap_or(&no_videos);
        let Some((video, sidecar)) = sidecar_video(rules, index, nearby, &place, path, kind) else {
            continue;
        };
        attachments.claimed_files.insert(path.clone());
        let attached = SidecarFile {
            path: path.clone(),
            language: sidecar.language,
            flags: sidecar.flags,
            title: sidecar.title,
        };
        match kind {
            SidecarKind::Subtitle => attachments
                .subtitles_by_video
                .entry(video.clone())
                .or_default()
                .push(attached),
            SidecarKind::Audio => attachments
                .audio_by_video
                .entry(video.clone())
                .or_default()
                .push(attached),
            SidecarKind::Image => attachments
                .stills_by_video
                .entry(video.clone())
                .or_default()
                .push(path.clone()),
        }
    }
    attachments
}

/// Groups paths by the folder that contains them (`None` for the library root), in the order
/// given.
fn by_folder<'a>(
    paths: impl IntoIterator<Item = &'a String>,
) -> BTreeMap<Option<&'a str>, Vec<&'a String>> {
    let mut grouped: BTreeMap<Option<&'a str>, Vec<&'a String>> = BTreeMap::new();
    for path in paths {
        grouped.entry(parent_folder(path)).or_default().push(path);
    }
    grouped
}

/// The videos of one folder in path order, and by their case-folded stems.
#[derive(Default)]
struct FolderVideos<'a> {
    videos: Vec<&'a String>,
    by_folded_stem: BTreeMap<String, Vec<&'a String>>,
}

impl<'a> FolderVideos<'a> {
    fn add(&mut self, video: &'a String) {
        self.videos.push(video);
        self.by_folded_stem
            .entry(lexical::fold_case_text(video_stem(video)))
            .or_default()
            .push(video);
    }

    /// Returns the videos whose stem is `stem`, ignoring case, in path order.
    fn stemmed(&self, stem: &str) -> &[&'a String] {
        self.by_folded_stem
            .get(&lexical::fold_case_text(stem))
            .map_or(&[], Vec::as_slice)
    }

    /// Returns the videos whose stem is a prefix of `name`'s stem, ignoring case, in path
    /// order. `name` can be a sidecar of these videos only.
    fn prefixing(&self, name: &str) -> Vec<&'a String> {
        let folded = lexical::fold_case_text(lexical::stem(name, EntryKind::File));
        let mut found: Vec<&'a String> = folded
            .char_indices()
            .filter_map(|(index, character)| {
                self.by_folded_stem
                    .get(&folded[..index + character.len_utf8()])
            })
            .flatten()
            .copied()
            .collect();
        found.sort();
        found
    }
}

fn sidecar_video<'a>(
    rules: &Rules,
    index: &EntryIndex,
    nearby: &FolderVideos<'a>,
    place: &SidecarPlace,
    path: &str,
    kind: SidecarKind,
) -> Option<(&'a String, Sidecar)> {
    let name = file_name(path);
    if let Some(stem_folder) = place.stem_folder {
        let [video] = nearby.stemmed(stem_folder) else {
            return None;
        };
        return Some((video, parse_sidecar_descriptors(rules, name, kind)));
    }

    let matching: Vec<(&'a String, Sidecar)> = nearby
        .prefixing(name)
        .into_iter()
        .map(|video| (video, rules.parse_sidecar(name, file_name(video), kind)))
        .filter(|(_, sidecar)| sidecar.matches)
        .collect();
    if let Some(longest) = matching
        .iter()
        .map(|(video, _)| video_stem(video).len())
        .max()
    {
        let mut best: Vec<(&'a String, Sidecar)> = matching
            .into_iter()
            .filter(|(video, _)| video_stem(video).len() == longest)
            .collect();
        if best.len() > 1 {
            best.retain(|(video, _)| name.starts_with(video_stem(video)));
        }
        return (best.len() == 1).then(|| best.remove(0));
    }

    if !place.is_in_subtitle_folder {
        return None;
    }
    let descriptors = parse_sidecar_descriptors(rules, name, kind);
    let is_descriptor_only = descriptors.matches
        && descriptors.title == Finding::Unknown
        && (descriptors.language != Finding::Unknown || !descriptors.flags.is_empty());
    let features: Vec<&'a String> = nearby
        .videos
        .iter()
        .copied()
        .filter(|video| index.files.get(*video) == Some(&FileRole::FeatureVideo))
        .collect();
    match features.as_slice() {
        [only] if is_descriptor_only => Some((only, descriptors)),
        _ => None,
    }
}

fn video_stem(path: &str) -> &str {
    lexical::stem(file_name(path), EntryKind::File)
}

/// Assigns artwork to owners by owner index: each video's stills (`still_role`: a movie's
/// `thumb`, an episode's `episode_still`), images named for a video with a role suffix
/// (`Heat (1995)-poster.jpg`), and generic images in a folder that contains a single item
/// (S6-S8).
pub(crate) fn attach_artwork(
    rules: &Rules,
    index: &EntryIndex,
    candidates: &OwnerCandidates,
    owner_of_video: &BTreeMap<String, usize>,
    stills_by_video: &BTreeMap<String, Vec<String>>,
    still_role: ArtworkRole,
) -> (BTreeMap<usize, Vec<Artwork>>, BTreeSet<String>) {
    let mut videos: BTreeMap<Option<&str>, FolderVideos> = BTreeMap::new();
    for video in owner_of_video.keys() {
        videos.entry(parent_folder(video)).or_default().add(video);
    }
    let no_videos = FolderVideos::default();
    let mut artwork: BTreeMap<usize, Vec<Artwork>> = BTreeMap::new();
    let mut claimed = BTreeSet::new();
    for (video, stills) in stills_by_video {
        let Some(&owner) = owner_of_video.get(video) else {
            continue;
        };
        for still in stills {
            claimed.insert(still.clone());
            artwork.entry(owner).or_default().push(Artwork {
                path: still.clone(),
                role: still_role,
            });
        }
    }
    for (path, role) in &index.files {
        if *role != FileRole::Image || claimed.contains(path) {
            continue;
        }
        let artwork_role = rules
            .classify_artwork(path, None)
            .role
            .unwrap_or(ArtworkRole::Other);
        let nearby = videos.get(&parent_folder(path)).unwrap_or(&no_videos);
        let owner = match video_named_by(path, artwork_role, nearby, owner_of_video) {
            Some(owner) => owner,
            None => match choose_asset_owner(parent_folder(path), candidates) {
                OwnerChoice::Owner(owner) => owner,
                OwnerChoice::Tied | OwnerChoice::Orphan => continue,
            },
        };
        claimed.insert(path.clone());
        artwork.entry(owner).or_default().push(Artwork {
            path: path.clone(),
            role: artwork_role,
        });
    }
    (artwork, claimed)
}

/// Returns the owner of the video that an image is named for, among the `nearby` videos in
/// its folder. The video has the longest stem that the image's stem starts with, followed by
/// `-` or `.` and a role word. Returns `None` if the role is `Other` or no video matches.
fn video_named_by(
    image_path: &str,
    role: ArtworkRole,
    nearby: &FolderVideos,
    owner_of_video: &BTreeMap<String, usize>,
) -> Option<usize> {
    if role == ArtworkRole::Other {
        return None;
    }
    let image_stem = video_stem(image_path);
    nearby
        .prefixing(file_name(image_path))
        .into_iter()
        .filter_map(|video| {
            let length = lexical::caseless_prefix_length(image_stem, video_stem(video))?;
            image_stem[length..]
                .starts_with(['-', '.'])
                .then_some((length, owner_of_video[video]))
        })
        .max()
        .map(|(_, owner)| owner)
}

/// The NFOs that are named for videos or owners (S9).
#[derive(Debug, Default)]
pub(crate) struct NfoAttachments {
    /// Each version's NFO, keyed by the video that it is named for or by the first part of the
    /// stack that it is named for.
    pub by_video: BTreeMap<String, String>,
    /// Each owner's generic NFO (`movie.nfo`, `tvshow.nfo`, `season.nfo`) by owner index.
    pub generic_by_owner: BTreeMap<usize, String>,
}

/// Attaches the NFOs of a tree (S9). `<video stem>.nfo` is attached to that video's version,
/// and an NFO with a stack's shared name to the stack's version. An NFO with a generic name
/// is attached to the owner that `generic_owner` returns for its role in its folder
/// (single-title folders only, S8). A video's own name takes precedence over a stack's name
/// for the same version. Among names that differ only in case, the first path takes
/// precedence. The other NFOs stay unclaimed.
pub(crate) fn attach_nfos(
    rules: &Rules,
    index: &EntryIndex,
    owner_of_video: &BTreeMap<String, usize>,
    generic_owner: impl Fn(NfoRole, Option<&str>) -> Option<usize>,
) -> NfoAttachments {
    let names_by_folder: BTreeMap<Option<&str>, FolderVideoNames> =
        by_folder(owner_of_video.keys().filter(|video| !video.ends_with('/')))
            .into_iter()
            .map(|(folder, videos)| {
                let videos: Vec<String> = videos.into_iter().cloned().collect();
                (folder, FolderVideoNames::new(rules, &videos))
            })
            .collect();
    let no_names = FolderVideoNames::new(rules, &[]);
    let mut by_video: BTreeMap<String, (u8, &String)> = BTreeMap::new();
    let mut generic: BTreeMap<usize, &String> = BTreeMap::new();
    for (path, role) in &index.files {
        if *role != FileRole::Other {
            continue;
        }
        let folder = parent_folder(path);
        // Only the videos in the same folder as an NFO affect the classification of its name.
        let names = names_by_folder.get(&folder).unwrap_or(&no_names);
        let Some(nfo_role) = classify_nfo_among(rules, path, names).role else {
            continue;
        };
        let stem = lexical::fold_case_text(video_stem(path));
        let named_video = names.videos_by_stem.get(&stem);
        let named_stack = names.stacks_by_name.get(&stem);
        let (video, rank) = match (nfo_role, named_video, named_stack) {
            (NfoRole::Video, Some(video), _) => (video.clone(), 0),
            (NfoRole::Movie, _, Some(first_part)) => (first_part.clone(), 1),
            _ => {
                if let Some(owner) = generic_owner(nfo_role, folder)
                    && generic.get(&owner).is_none_or(|current| path < *current)
                {
                    generic.insert(owner, path);
                }
                continue;
            }
        };
        if by_video
            .get(&video)
            .is_none_or(|current| (rank, path) < (current.0, current.1))
        {
            by_video.insert(video, (rank, path));
        }
    }
    NfoAttachments {
        by_video: by_video
            .into_iter()
            .map(|(video, (_, path))| (video, path.clone()))
            .collect(),
        generic_by_owner: generic
            .into_iter()
            .map(|(owner, path)| (owner, path.clone()))
            .collect(),
    }
}

impl NfoAttachments {
    /// Returns the NFO named for one of a version's paths. The NFO of the first path that has
    /// one takes precedence.
    pub(crate) fn of_version(&self, paths: &[String]) -> Option<String> {
        paths
            .iter()
            .find_map(|path| self.by_video.get(path).cloned())
    }
}

/// Returns the files that belong to a claimed file: a disc's contents, and the `.sub` of a
/// claimed `.idx`.
pub(crate) fn carried_files(index: &EntryIndex, claimed: &BTreeSet<String>) -> Vec<String> {
    index
        .files
        .iter()
        .filter(|(path, role)| match role {
            // A claimed disc folder is one of the folders above the file.
            FileRole::DiscContent => path
                .match_indices('/')
                .any(|(slash, _)| claimed.contains(&path[..=slash])),
            FileRole::SubtitlePairPart => {
                let stem = lexical::stem(path, EntryKind::File);
                claimed.contains(&format!("{stem}.idx")) || claimed.contains(&format!("{stem}.IDX"))
            }
            _ => false,
        })
        .map(|(path, _)| path.clone())
        .collect()
}
