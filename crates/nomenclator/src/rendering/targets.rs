//! Computes the path that the profile gives each file of an interpretation. The planner
//! then checks these paths against each other and against the existing entries (R6).

use crate::model::AmbiguityReason;
use std::collections::{BTreeMap, BTreeSet};

use crate::interpret::{
    Artwork, Episode, Extra, Interpretation, Item, Movie, Season, Series, SidecarFile, Version,
    parent_folder,
};
use crate::lexical;
use crate::model::{EntryKind, ExtraKind, Finding, SidecarKind, resolved_values};
use crate::parsers::extras::{LibraryType, extras_folder_kind};
use crate::parsers::file_types::file_name;
use crate::rules::Rules;

use super::import::{LibraryMatch, LibraryWorks};
use super::{
    BlockReason, ExtraFacts, MovieLayout, NamingFacts, OutputProfile, SidecarFacts, render_path,
    render_work_folder,
};

type Destination = Result<String, Vec<BlockReason>>;

/// An existing file and the destination that the profile gives it.
#[derive(Debug, Clone)]
pub(crate) struct FileTarget {
    pub path: String,
    pub destination: Destination,
    /// The file that this file moves with: its item's primary video, or the first part of
    /// its extra. It is `None` for that file itself.
    pub moves_with: Option<String>,
    /// The series that the file belongs to, used to keep a series in one folder (R6).
    pub series: Option<SeriesKey>,
}

/// A series key: the series folder and title.
pub(crate) type SeriesKey = (Option<String>, Option<String>);

/// Returns the target of every file that the interpretation's items claim. Also returns
/// the target of each unclaimed file that is named after a video or is in a folder that
/// contains only one movie.
///
/// If `library` is set, the files are imported into that library (R17): category folders
/// are not kept, a work that matches a library work takes its year, provider IDs and
/// folder, and samples are left in place.
pub(crate) fn file_targets(
    rules: &Rules,
    interpretation: &Interpretation,
    profile: &OutputProfile,
    library: Option<&LibraryWorks>,
) -> Vec<FileTarget> {
    let mut builder = TargetBuilder {
        rules,
        interpretation,
        profile,
        targets: Vec::new(),
        videos: BTreeMap::new(),
        video_stems: BTreeMap::new(),
        movie_folders: Vec::new(),
        series: None,
        category: None,
        library,
        work_folder: None,
    };
    let series_by_location: BTreeMap<(Option<&String>, Option<&String>), &Series> = interpretation
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Series(series) => {
                Some(((series.path.as_ref(), series.title.as_ref()), &**series))
            }
            _ => None,
        })
        .collect();
    for item in &interpretation.items {
        builder.series = match item {
            Item::Series(series) => Some((series.path.clone(), series.title.clone())),
            Item::Season(season) => Some((season.series_path.clone(), season.series_name.clone())),
            Item::Episode(episode) => {
                Some((episode.series_path.clone(), episode.series_name.clone()))
                    .filter(|(path, name)| path.is_some() || name.is_some())
            }
            Item::Movie(_) | Item::Extra(_) => None,
        };
        builder.category = item_category(item).filter(|_| library.is_none());
        builder.work_folder = None;
        match item {
            Item::Movie(movie) => builder.add_movie(rules, movie),
            Item::Series(series) => builder.add_series(rules, series),
            Item::Season(season) => {
                let series = series_by_location
                    .get(&(season.series_path.as_ref(), season.series_name.as_ref()));
                builder.add_season(rules, season, series.copied());
            }
            Item::Episode(episode) => {
                let series = series_by_location
                    .get(&(episode.series_path.as_ref(), episode.series_name.as_ref()));
                builder.add_episode(rules, episode, series.copied());
            }
            // An import leaves samples in place (R17).
            Item::Extra(orphan) if library.is_some() && orphan.extra == ExtraKind::Sample => {}
            Item::Extra(orphan) => {
                for path in &orphan.paths {
                    builder.push(path, Err(vec![BlockReason::NoOwner]), None);
                }
            }
        }
    }
    builder.series = None;
    builder.category = None;
    builder.add_unclaimed_files();
    builder
        .targets
        .sort_by(|left, right| left.path.cmp(&right.path));
    builder.targets
}

/// A video that already has a target, used to place the files named after it.
struct PlacedVideo {
    destination: Destination,
    /// The file that the files named after this video move with.
    leader: String,
}

/// A movie's folder and its destination, used to place the unclaimed files under it.
struct MovieFolder {
    path: String,
    destination: Destination,
    leader: String,
    claimed_files: Vec<String>,
}

struct TargetBuilder<'a> {
    rules: &'a Rules,
    interpretation: &'a Interpretation,
    profile: &'a OutputProfile,
    targets: Vec<FileTarget>,
    /// The placed videos by their folder, then by path. The folder `None` is the library
    /// root.
    videos: BTreeMap<Option<String>, BTreeMap<String, PlacedVideo>>,
    /// The paths of placed videos by folder and case-folded stem. The index finds the
    /// video that a file is named after without a comparison with every video in the
    /// folder.
    video_stems: BTreeMap<(Option<String>, String), BTreeSet<String>>,
    movie_folders: Vec<MovieFolder>,
    /// The series of the item whose files are added.
    series: Option<SeriesKey>,
    /// The category folders above the item whose files are added. Its destinations keep
    /// them (R10).
    category: Option<String>,
    /// The library that the files are imported into (R17).
    library: Option<&'a LibraryWorks>,
    /// The rendered work folder of the item whose files are added, and the folder of the
    /// library work that replaces it (R17).
    work_folder: Option<(String, String)>,
}

impl TargetBuilder<'_> {
    fn push(&mut self, path: &str, destination: Destination, moves_with: Option<&str>) {
        self.targets.push(FileTarget {
            path: path.to_owned(),
            destination,
            moves_with: moves_with
                .filter(|leader| *leader != path)
                .map(str::to_owned),
            series: self.series.clone(),
        });
    }

    /// Renders the destination for `facts`. Returns `item_reasons` instead if any are
    /// present, so that a file with an uncertain identity is never renamed.
    fn render(&self, facts: &NamingFacts, item_reasons: &[BlockReason]) -> Destination {
        if item_reasons.is_empty() {
            render_path(self.rules, facts, self.profile).map(|path| self.place(path))
        } else {
            Err(item_reasons.to_vec())
        }
    }

    /// Returns `path` in the item's library work folder, if it has one, or below its
    /// category folders.
    fn place(&self, path: String) -> String {
        if let Some((rendered, library_folder)) = &self.work_folder {
            if path == *rendered {
                return library_folder.clone();
            }
            if let Some(rest) = path
                .strip_prefix(rendered.as_str())
                .and_then(|rest| rest.strip_prefix('/'))
            {
                return format!("{library_folder}/{rest}");
            }
        }
        match &self.category {
            Some(category) => format!("{category}/{path}"),
            None => path,
        }
    }

    /// Returns `facts` completed from the library work that the work matches (R17): the
    /// missing year and provider IDs. Records the library work's folder for `place`.
    /// Adds `ambiguous_library_work` to `item_reasons` if several library works match.
    fn with_library_work(
        &mut self,
        mut facts: NamingFacts,
        item_reasons: &mut Vec<BlockReason>,
    ) -> NamingFacts {
        let Some(library) = self.library else {
            return facts;
        };
        match library.find(self.rules, &facts) {
            LibraryMatch::None => {}
            LibraryMatch::Several => {
                item_reasons.push(BlockReason::AmbiguousLibraryWork);
                item_reasons.sort();
                item_reasons.dedup();
            }
            LibraryMatch::One(work) => {
                facts.year = facts.year.or(work.year);
                for (provider, id) in &work.provider_ids {
                    facts
                        .provider_ids
                        .entry(provider.clone())
                        .or_insert_with(|| id.clone());
                }
                let folder_facts = NamingFacts {
                    season: None,
                    ..facts.clone()
                };
                if let (Some(folder), Ok(rendered)) = (
                    &work.folder,
                    render_work_folder(self.rules, &folder_facts, self.profile),
                ) {
                    self.work_folder = Some((rendered, folder.clone()));
                }
            }
        }
        facts
    }

    /// Adds a video, which is a feature or an extra, and its attached sidecars.
    fn add_video(
        &mut self,
        path: &str,
        facts: &NamingFacts,
        item_reasons: &[BlockReason],
        leader: &str,
    ) {
        if path.ends_with('/') {
            self.push(path, Err(vec![BlockReason::DiscStructure]), Some(leader));
            return;
        }
        let destination = self.render(facts, item_reasons);
        self.push(path, destination.clone(), Some(leader));
        let stem = lexical::stem(file_name(path), EntryKind::File);
        self.video_stems
            .entry((
                parent_folder(path).map(str::to_owned),
                lexical::fold_case_text(stem),
            ))
            .or_default()
            .insert(path.to_owned());
        self.videos
            .entry(parent_folder(path).map(str::to_owned))
            .or_default()
            .insert(
                path.to_owned(),
                PlacedVideo {
                    destination,
                    leader: leader.to_owned(),
                },
            );
        let subtitles = self.interpretation.video_subtitles.get(path);
        for subtitle in subtitles.into_iter().flatten() {
            self.add_sidecar(subtitle, facts, item_reasons, leader);
        }
    }

    fn add_sidecar(
        &mut self,
        sidecar: &SidecarFile,
        video_facts: &NamingFacts,
        item_reasons: &[BlockReason],
        leader: &str,
    ) {
        let language = match &sidecar.language {
            Finding::Resolved { value, .. } => Some(value.clone()),
            Finding::Ambiguous { .. } => {
                self.push(
                    &sidecar.path,
                    Err(vec![BlockReason::AmbiguousSidecarLanguage]),
                    Some(leader),
                );
                return;
            }
            Finding::Unknown => None,
        };
        let facts = NamingFacts {
            sidecar: Some(SidecarFacts {
                language,
                flags: sidecar.flags.clone(),
                title: sidecar.title.resolved_value().cloned(),
                extension: extension_of(&sidecar.path),
            }),
            ..video_facts.clone()
        };
        let destination = self.render(&facts, item_reasons);
        self.push(&sidecar.path, destination, Some(leader));
    }

    /// Adds an item's extras. `owner_facts` places them in the movie, series, season or
    /// episode.
    fn add_extras(
        &mut self,
        rules: &Rules,
        extras: &[Extra],
        owner_facts: &NamingFacts,
        item_reasons: &[BlockReason],
        leader: Option<&str>,
    ) {
        for extra in extras {
            // An import leaves samples in place (R17).
            if self.library.is_some() && extra.extra == ExtraKind::Sample {
                continue;
            }
            let title = extra.title.clone().unwrap_or_else(|| {
                lexical::stem(file_name(&extra.paths[0]), EntryKind::File).to_owned()
            });
            let extra_leader = leader.unwrap_or(&extra.paths[0]).to_owned();
            for (index, path) in extra.paths.iter().enumerate() {
                let facts = NamingFacts {
                    extra: Some(ExtraFacts {
                        kind: extra.extra,
                        title: Some(title.clone()),
                        year: extra.year,
                        group: extra_group(rules, path).map(str::to_owned),
                    }),
                    part: part_number(&extra.paths, index),
                    container: extension_of(path),
                    ..owner_facts.clone()
                };
                self.add_video(path, &facts, item_reasons, &extra_leader);
            }
        }
    }

    fn add_versions(
        &mut self,
        versions: &[Version],
        base: &NamingFacts,
        item_reasons: &[BlockReason],
        leader: &str,
    ) {
        for version in versions {
            let version_label = version.label.clone().filter(|_| versions.len() > 1);
            for (index, path) in version.paths.iter().enumerate() {
                let facts = NamingFacts {
                    edition: version.edition.clone(),
                    version_label: version_label.clone(),
                    part: part_number(&version.paths, index),
                    container: extension_of(path),
                    ..base.clone()
                };
                self.add_video(path, &facts, item_reasons, leader);
            }
            if let Some(nfo) = &version.nfo {
                let facts = NamingFacts {
                    edition: version.edition.clone(),
                    version_label: version_label.clone(),
                    container: extension_of(nfo),
                    ..base.clone()
                };
                self.add_version_nfo(nfo, &version.paths[0], &facts, item_reasons);
            }
        }
    }

    /// Adds a version's NFO (S9). An NFO named after its video follows that video's stem.
    /// An NFO named after the shared name of the parts gets the version's new name
    /// without a part number.
    fn add_version_nfo(
        &mut self,
        nfo: &str,
        first_video: &str,
        facts: &NamingFacts,
        item_reasons: &[BlockReason],
    ) {
        let destination = match self.follow_video_stem(nfo) {
            Some((destination, _)) => destination,
            None => self.render(facts, item_reasons),
        };
        self.push(nfo, destination, Some(first_video));
    }

    /// Adds external audio files. Each file goes with the video whose stem it contains.
    /// If no video stem matches, the file goes with `leader`.
    fn add_audio(
        &mut self,
        rules: &Rules,
        audio: &[SidecarFile],
        video_facts: &BTreeMap<String, NamingFacts>,
        item_reasons: &[BlockReason],
        leader: &str,
    ) {
        for track in audio {
            let video = video_facts
                .keys()
                .filter(|video| parent_folder(video) == parent_folder(&track.path))
                .filter(|video| {
                    rules
                        .parse_sidecar(file_name(&track.path), file_name(video), SidecarKind::Audio)
                        .matches
                })
                .max_by_key(|video| video.len())
                .map_or(leader, String::as_str);
            if let Some(facts) = video_facts.get(video) {
                self.add_sidecar(track, facts, item_reasons, leader);
            }
        }
    }

    fn add_movie(&mut self, rules: &Rules, movie: &Movie) {
        let leader = movie.versions[0].paths[0].clone();
        let mut item_reasons = movie_block_reasons(movie);
        let base = NamingFacts {
            title: movie.title.clone(),
            year: movie.year,
            provider_ids: resolved_values(&movie.provider_ids),
            has_extras: movie_has_folder_files(movie),
            ..empty_facts(LibraryType::Movies)
        };
        let base = self.with_library_work(base, &mut item_reasons);
        self.add_versions(&movie.versions, &base, &item_reasons, &leader);
        self.add_extras(rules, &movie.extras, &base, &item_reasons, Some(&leader));
        let video_facts = self.facts_of_videos(movie_video_paths(movie), &base);
        self.add_audio(rules, &movie.audio, &video_facts, &item_reasons, &leader);
        let folder_destination = self.folder_destination(&base, &item_reasons);
        let folder = movie
            .path
            .as_deref()
            .map(|path| (path, &folder_destination));
        self.add_artwork(&movie.artwork, folder, &leader);
        if let Some(nfo) = movie
            .nfo
            .as_deref()
            .filter(|nfo| movie.versions[0].nfo.as_deref() != Some(nfo))
            && let Some(destination) = follow_folder(nfo, folder)
        {
            self.push(nfo, destination, Some(&leader));
        }
        // A flat movie's unclaimed files have no folder to follow, so they are not moved.
        let is_flat = self.profile.movie_layout == MovieLayout::Flat && !base.has_extras;
        if let Some(folder) = movie.path.as_ref().filter(|_| !is_flat) {
            self.movie_folders.push(MovieFolder {
                path: folder.clone(),
                destination: folder_destination,
                leader,
                claimed_files: claimed_files_of_movie(movie),
            });
        }
    }

    fn add_series(&mut self, rules: &Rules, series: &Series) {
        let mut item_reasons = series_block_reasons(Some(series));
        let facts = series_facts(Some(series), series.title.clone(), series.year);
        let facts = self.with_library_work(facts, &mut item_reasons);
        self.add_extras(rules, &series.extras, &facts, &item_reasons, None);
        let folder_destination = self.folder_destination(&facts, &item_reasons);
        self.add_folder_files(
            &series.artwork,
            series.nfo.as_deref(),
            series.path.as_deref(),
            &folder_destination,
        );
    }

    fn add_season(&mut self, rules: &Rules, season: &Season, series: Option<&Series>) {
        let mut item_reasons = series_block_reasons(series);
        let facts = series_facts(
            series,
            season.series_name.clone(),
            series.and_then(|s| s.year),
        );
        let facts = NamingFacts {
            season: Some(season.season),
            ..self.with_library_work(facts, &mut item_reasons)
        };
        self.add_extras(rules, &season.extras, &facts, &item_reasons, None);
        let folder_destination = self.folder_destination(&facts, &item_reasons);
        self.add_folder_files(
            &season.artwork,
            season.nfo.as_deref(),
            season.path.as_deref(),
            &folder_destination,
        );
    }

    fn add_episode(&mut self, rules: &Rules, episode: &Episode, series: Option<&Series>) {
        let leader = episode.versions[0].paths[0].clone();
        let mut item_reasons = series_block_reasons(series);
        item_reasons.extend(ambiguity_block_reasons(&episode.ambiguity_reasons));
        if matches!(episode.season, Finding::Ambiguous { .. }) {
            item_reasons.push(BlockReason::AmbiguousSeason);
        }
        item_reasons.sort();
        item_reasons.dedup();
        let series_facts = series_facts(series, episode.series_name.clone(), episode.series_year);
        let base = NamingFacts {
            season: episode.season.resolved_value().copied(),
            episodes: episode.episodes.clone().unwrap_or_default(),
            absolute: episode.absolute.clone().unwrap_or_default(),
            date: episode.date.resolved_value().cloned(),
            episode_title: episode.episode_title.clone(),
            ..self.with_library_work(series_facts, &mut item_reasons)
        };
        self.add_versions(&episode.versions, &base, &item_reasons, &leader);
        self.add_extras(rules, &episode.extras, &base, &item_reasons, Some(&leader));
        let videos = episode
            .versions
            .iter()
            .flat_map(|version| version.paths.clone())
            .collect();
        let video_facts = self.facts_of_videos(videos, &base);
        self.add_audio(rules, &episode.audio, &video_facts, &item_reasons, &leader);
        self.add_artwork(&episode.artwork, None, &leader);
    }

    /// Returns the facts that each feature video was rendered with, for its audio tracks.
    fn facts_of_videos(
        &self,
        videos: Vec<String>,
        base: &NamingFacts,
    ) -> BTreeMap<String, NamingFacts> {
        videos
            .into_iter()
            .map(|video| {
                let facts = NamingFacts {
                    container: extension_of(&video),
                    ..base.clone()
                };
                (video, facts)
            })
            .collect()
    }

    fn folder_destination(&self, facts: &NamingFacts, item_reasons: &[BlockReason]) -> Destination {
        if item_reasons.is_empty() {
            render_work_folder(self.rules, facts, self.profile).map(|path| self.place(path))
        } else {
            Err(item_reasons.to_vec())
        }
    }

    /// Adds artwork. Artwork named after a video follows that video's stem. Other artwork
    /// keeps its place in the item's folder.
    fn add_artwork(
        &mut self,
        artwork: &[Artwork],
        folder: Option<(&str, &Destination)>,
        leader: &str,
    ) {
        for image in artwork {
            if let Some((destination, _)) = self.follow_video_stem(&image.path) {
                self.push(&image.path, destination, Some(leader));
            } else if let Some(destination) = follow_folder(&image.path, folder) {
                self.push(&image.path, destination, Some(leader));
            }
        }
    }

    /// Adds the files of a series or season folder. Each file keeps its place in the
    /// folder.
    fn add_folder_files(
        &mut self,
        artwork: &[Artwork],
        nfo: Option<&str>,
        folder: Option<&str>,
        folder_destination: &Destination,
    ) {
        let files = artwork.iter().map(|image| image.path.as_str()).chain(nfo);
        for path in files {
            let folder = folder.map(|path| (path, folder_destination));
            if let Some(destination) = follow_folder(path, folder) {
                self.push(path, destination, None);
            }
        }
    }

    /// Returns the destination of a file named after a video in the same folder, and the
    /// file that it moves with. The longest matching stem wins. Returns `None` if no video
    /// stem matches.
    fn follow_video_stem(&self, path: &str) -> Option<(Destination, String)> {
        let name = file_name(path);
        let folder = parent_folder(path).map(str::to_owned);
        let placed_videos = self.videos.get(&folder)?;
        // Finds the longest stem that starts the name and is followed by `.` or `-`. If the
        // stems of several videos differ only in case, the last path wins.
        let (placed, stem_length) = name
            .char_indices()
            .filter(|(_, character)| matches!(character, '.' | '-'))
            .map(|(length, _)| length)
            .rev()
            .find_map(|length| {
                let key = (folder.clone(), lexical::fold_case_text(&name[..length]));
                let video = self
                    .video_stems
                    .get(&key)?
                    .iter()
                    .rfind(|video| *video != path)?;
                Some((&placed_videos[video], length))
            })?;
        let destination = match &placed.destination {
            Ok(video_destination) => {
                let new_stem = lexical::stem(file_name(video_destination), EntryKind::File);
                let rest = &name[stem_length..];
                let folder = parent_folder(video_destination)
                    .map(|folder| format!("{folder}/"))
                    .unwrap_or_default();
                Ok(format!("{folder}{new_stem}{rest}"))
            }
            Err(_) => Err(vec![BlockReason::MovesWithBlockedFile]),
        };
        Some((destination, placed.leader.clone()))
    }

    /// Adds the unclaimed files. A file named after a video moves with it. Another file
    /// under a folder that contains only one movie keeps its place in that folder.
    fn add_unclaimed_files(&mut self) {
        // Maps each folder to its last movie, and each folder to the movie folders that
        // claim files under it.
        let mut movie_folder_at: BTreeMap<&str, usize> = BTreeMap::new();
        let mut claiming_under: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for (position, folder) in self.movie_folders.iter().enumerate() {
            movie_folder_at.insert(&folder.path, position);
            for claimed in &folder.claimed_files {
                // A disc version (`Heat (1995)/`) is also under its own folder.
                let disc_folder = claimed.strip_suffix('/');
                for above in disc_folder.into_iter().chain(folders_above(claimed)) {
                    claiming_under
                        .entry(above)
                        .or_default()
                        .insert(&folder.path);
                }
            }
        }
        let mut targets = Vec::new();
        for path in &self.interpretation.unclaimed_files {
            // Release junk is a proposed removal, not a move (R13).
            if self.interpretation.release_junk.contains(path) {
                continue;
            }
            if let Some((destination, leader)) = self.follow_video_stem(path) {
                targets.push((path, destination, leader));
                continue;
            }
            // Uses the deepest movie folder above the file if no other movie claims files
            // under it.
            let exclusive_folder = folders_above(path)
                .find_map(|above| movie_folder_at.get(above))
                .map(|&position| &self.movie_folders[position])
                .filter(|folder| {
                    claiming_under
                        .get(folder.path.as_str())
                        .is_none_or(|claiming| claiming.iter().all(|other| *other == folder.path))
                });
            if let Some(folder) = exclusive_folder
                && let Some(destination) =
                    follow_folder(path, Some((&folder.path, &folder.destination)))
            {
                targets.push((path, destination, folder.leader.clone()));
            }
        }
        for (path, destination, leader) in targets {
            self.push(path, destination, Some(&leader));
        }
    }
}

/// Returns the category folders above an item (R10): the folders above a movie's or
/// series' own folder, or above its first video if it has no folder. Returns `None` at
/// the library root.
fn item_category(item: &Item) -> Option<String> {
    let first_video = |versions: &[crate::interpret::Version]| {
        versions
            .first()
            .and_then(|version| version.paths.first())
            .and_then(|path| parent_folder(path.trim_end_matches('/')))
            .map(str::to_owned)
    };
    match item {
        Item::Movie(movie) => match &movie.path {
            Some(folder) => parent_folder(folder).map(str::to_owned),
            None => first_video(&movie.versions),
        },
        Item::Series(series) => series
            .path
            .as_deref()
            .and_then(parent_folder)
            .map(str::to_owned),
        Item::Season(season) => season
            .series_path
            .as_deref()
            .and_then(parent_folder)
            .map(str::to_owned),
        Item::Episode(episode) => match &episode.series_path {
            Some(folder) => parent_folder(folder).map(str::to_owned),
            None => first_video(&episode.versions),
        },
        Item::Extra(_) => None,
    }
}

/// Returns the subfolder of an extras folder that contains the extra, if that subfolder
/// is not an extras folder itself. For `Featurettes/On Location/01. Adrift.mkv` it
/// returns `On Location` (R1).
fn extra_group<'a>(rules: &Rules, path: &'a str) -> Option<&'a str> {
    let mut folders = path.rsplit('/').skip(1);
    let group = folders.next()?;
    let extras_folder = folders.next()?;
    (extras_folder_kind(rules, extras_folder).is_some()
        && extras_folder_kind(rules, group).is_none())
    .then_some(group)
}

/// Returns the same relative path under the folder's destination. Returns `None` if
/// `path` is outside the folder.
fn follow_folder(path: &str, folder: Option<(&str, &Destination)>) -> Option<Destination> {
    let (folder, folder_destination) = folder?;
    let relative = path.strip_prefix(folder)?.strip_prefix('/')?;
    Some(match folder_destination {
        Ok(destination) => Ok(format!("{destination}/{relative}")),
        Err(reasons) => Err(reasons.clone()),
    })
}

/// Returns the folders above `path`, deepest first.
fn folders_above(path: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(parent_folder(path), |folder| parent_folder(folder))
}

/// Returns `true` if `movie` has extras, artwork or a folder NFO, which keep its folder
/// in a flat movie layout (R15). Artwork named after a video also counts, so that a
/// movie's images never lose their folder.
fn movie_has_folder_files(movie: &Movie) -> bool {
    let folder_nfo = movie
        .nfo
        .as_deref()
        .is_some_and(|nfo| movie.versions[0].nfo.as_deref() != Some(nfo));
    !movie.extras.is_empty() || !movie.artwork.is_empty() || folder_nfo
}

fn empty_facts(library: LibraryType) -> NamingFacts {
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

fn series_facts(series: Option<&Series>, title: Option<String>, year: Option<u16>) -> NamingFacts {
    NamingFacts {
        title,
        year,
        provider_ids: series
            .map(|series| resolved_values(&series.provider_ids))
            .unwrap_or_default(),
        ..empty_facts(LibraryType::Tv)
    }
}

/// Returns the block reasons for the interpreter's ambiguity reasons.
fn ambiguity_block_reasons(reasons: &[AmbiguityReason]) -> Vec<BlockReason> {
    reasons
        .iter()
        .map(|reason| match reason {
            AmbiguityReason::ConflictingProviderId => BlockReason::AmbiguousProviderId,
            AmbiguityReason::MissingPart | AmbiguityReason::DuplicatePart => {
                BlockReason::AmbiguousParts
            }
            _ => BlockReason::AmbiguousIdentity,
        })
        .collect()
}

fn movie_block_reasons(movie: &Movie) -> Vec<BlockReason> {
    let mut reasons = if movie.misplaced_series {
        vec![BlockReason::MisplacedSeries]
    } else {
        ambiguity_block_reasons(&movie.ambiguity_reasons)
    };
    if crate::model::has_ambiguous_value(&movie.provider_ids) {
        reasons.push(BlockReason::AmbiguousProviderId);
    }
    reasons.sort();
    reasons.dedup();
    reasons
}

fn series_block_reasons(series: Option<&Series>) -> Vec<BlockReason> {
    let conflicting =
        series.is_some_and(|series| crate::model::has_ambiguous_value(&series.provider_ids));
    if conflicting {
        vec![BlockReason::AmbiguousProviderId]
    } else {
        Vec::new()
    }
}

fn movie_video_paths(movie: &Movie) -> Vec<String> {
    movie
        .versions
        .iter()
        .flat_map(|version| version.paths.clone())
        .collect()
}

fn claimed_files_of_movie(movie: &Movie) -> Vec<String> {
    let mut files = movie_video_paths(movie);
    files.extend(movie.extras.iter().flat_map(|extra| extra.paths.clone()));
    files.extend(movie.subtitles.iter().map(|sidecar| sidecar.path.clone()));
    files.extend(movie.audio.iter().map(|sidecar| sidecar.path.clone()));
    files.extend(movie.artwork.iter().map(|image| image.path.clone()));
    files.extend(movie.nfo.clone());
    files.extend(
        movie
            .versions
            .iter()
            .filter_map(|version| version.nfo.clone()),
    );
    files
}

fn part_number(paths: &[String], index: usize) -> Option<u32> {
    (paths.len() > 1).then(|| u32::try_from(index + 1).expect("few parts"))
}

fn extension_of(path: &str) -> Option<String> {
    lexical::extension(file_name(path), EntryKind::File)
}
