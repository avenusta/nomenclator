//! Checks properties of rendering and planning: names rendered under the default profile
//! parse back to the facts that the profile encodes, and plans do not depend on entry
//! order.

use std::collections::BTreeMap;

use nomenclator::model::{EntryKind, ExtraKind, Finding, SidecarFlag, SidecarKind};
use nomenclator::{
    ExtraFacts, Item, LibraryType, NamingFacts, OutputProfile, SidecarFacts, classify_extra,
    interpret, parse_episode_name, parse_movie_name, parse_series_name, parse_sidecar,
    plan_renames, render_names,
};

fn facts(library: LibraryType) -> NamingFacts {
    NamingFacts {
        library,
        title: None,
        year: None,
        provider_ids: BTreeMap::new(),
        edition: None,
        version_label: None,
        part: None,
        container: Some("mkv".to_owned()),
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

fn rendered(facts: &NamingFacts) -> String {
    let result = render_names(facts, &OutputProfile::default());
    result
        .path
        .unwrap_or_else(|| panic!("{facts:?} renders, got {:?}", result.blocked_reasons))
}

fn resolved<T: Clone>(finding: &Finding<T>) -> Option<T> {
    finding.resolved_value().cloned()
}

fn resolved_ids(ids: &BTreeMap<String, Finding<String>>) -> BTreeMap<String, String> {
    ids.iter()
        .filter_map(|(provider, finding)| Some((provider.clone(), resolved(finding)?)))
        .collect()
}

fn movie_samples() -> Vec<NamingFacts> {
    let mut samples = Vec::new();
    let titles = [
        "Northern Passage",
        "1917",
        "Mr. Holland's Opus",
        "Æon Flux",
        "The 4K Project",
    ];
    let years = [None, Some(2021)];
    let ids: [&[(&str, &str)]; 3] = [&[], &[("tmdb", "550")], &[("imdb", "tt0137523")]];
    let editions = [None, Some("Director's Cut")];
    for title in titles {
        for year in years {
            for provider_ids in ids {
                for edition in editions {
                    let mut sample = facts(LibraryType::Movies);
                    sample.title = Some(title.to_owned());
                    sample.year = year;
                    sample.provider_ids = provider_ids
                        .iter()
                        .map(|(provider, id)| (provider.to_string(), id.to_string()))
                        .collect();
                    sample.edition = edition.map(str::to_owned);
                    samples.push(sample);
                }
            }
        }
    }
    samples
}

#[test]
fn render_names_movie_parses_back_to_title_year_provider_id_and_edition() {
    for sample in movie_samples() {
        let path = rendered(&sample);
        let (folder, file) = path.split_once('/').expect("a movie folder and a file");
        for (name, kind) in [(folder, EntryKind::Directory), (file, EntryKind::File)] {
            let parsed = parse_movie_name(name, kind);
            assert_eq!(resolved(&parsed.title), sample.title, "title of {name}");
            if sample.year.is_some() {
                assert_eq!(resolved(&parsed.year), sample.year, "year of {name}");
            }
            assert_eq!(
                resolved_ids(&parsed.provider_ids),
                sample.provider_ids,
                "{name}"
            );
        }
        let parsed_file = parse_movie_name(file, EntryKind::File);
        assert_eq!(
            resolved(&parsed_file.edition),
            sample.edition,
            "edition of {file}"
        );
    }
}

#[test]
fn render_names_versions_and_parts_interpret_as_one_movie() {
    let mut base = facts(LibraryType::Movies);
    base.title = Some("Northern Passage".to_owned());
    base.year = Some(2021);
    base.provider_ids = BTreeMap::from([("tmdb".to_owned(), "550".to_owned())]);
    let mut paths = Vec::new();
    for (label, parts) in [("2160p", 2), ("1080p", 1)] {
        for part in 1..=parts {
            let video = NamingFacts {
                version_label: Some(label.to_owned()),
                part: (parts > 1).then_some(part),
                ..base.clone()
            };
            paths.push(rendered(&video));
            let subtitle = NamingFacts {
                sidecar: Some(SidecarFacts {
                    language: Some("pt-BR".to_owned()),
                    flags: vec![SidecarFlag::HearingImpaired, SidecarFlag::Forced],
                    title: None,
                    extension: Some("srt".to_owned()),
                }),
                ..video
            };
            paths.push(rendered(&subtitle));
        }
    }
    let interpretation = interpret(LibraryType::Movies, &paths, &[]);
    let [Item::Movie(movie)] = interpretation.items.as_slice() else {
        panic!("one movie, got {:?}", interpretation.items);
    };
    assert_eq!(movie.title.as_deref(), Some("Northern Passage"));
    assert_eq!(movie.year, Some(2021));
    assert_eq!(resolved_ids(&movie.provider_ids), base.provider_ids);
    let version_paths: Vec<&Vec<String>> = movie.versions.iter().map(|v| &v.paths).collect();
    assert_eq!(
        version_paths,
        [
            &vec![paths[0].clone(), paths[2].clone()],
            &vec![paths[4].clone()]
        ]
    );
    assert_eq!(movie.subtitles.len(), 3);
    for subtitle in &movie.subtitles {
        assert_eq!(resolved(&subtitle.language).as_deref(), Some("pt-BR"));
        assert_eq!(
            subtitle.flags,
            [SidecarFlag::Forced, SidecarFlag::HearingImpaired]
        );
    }
    assert!(interpretation.unclaimed_files.is_empty());
}

#[test]
fn render_names_episode_parses_back_to_series_season_episodes_and_title() {
    let episode_lists: [&[u32]; 4] = [&[1], &[2, 3], &[1, 3], &[100]];
    for season in [0, 1, 12] {
        for episodes in episode_lists {
            for (year, episode_title) in [(None, None), (Some(2019), Some("Low Tide"))] {
                let mut sample = facts(LibraryType::Tv);
                sample.title = Some("Harbor Lights".to_owned());
                sample.year = year;
                sample.provider_ids = BTreeMap::from([("tvdb".to_owned(), "81189".to_owned())]);
                sample.season = Some(season);
                sample.episodes = episodes.to_vec();
                sample.episode_title = episode_title.map(str::to_owned);
                let path = rendered(&sample);
                let parsed = parse_episode_name(&path, EntryKind::File, None);
                assert_eq!(resolved(&parsed.series_name), sample.title, "{path}");
                assert_eq!(resolved(&parsed.series_year), year, "{path}");
                assert_eq!(resolved(&parsed.season), Some(season), "{path}");
                assert_eq!(parsed.episodes.as_deref(), Some(episodes), "{path}");
                assert_eq!(parsed.episode_title, sample.episode_title, "{path}");
                let series_folder = path.split('/').next().expect("a series folder");
                let series = parse_series_name(series_folder);
                assert_eq!(resolved_ids(&series.provider_ids), sample.provider_ids);
            }
        }
    }
}

#[test]
fn render_names_dated_episode_parses_back_to_date() {
    let mut sample = facts(LibraryType::Tv);
    sample.title = Some("Evening Report".to_owned());
    sample.year = Some(2010);
    sample.date = Some("2024-05-17".to_owned());
    sample.episode_title = Some("Election Night".to_owned());
    let path = rendered(&sample);
    let parsed = parse_episode_name(&path, EntryKind::File, None);
    assert_eq!(resolved(&parsed.date).as_deref(), Some("2024-05-17"));
    assert_eq!(
        resolved(&parsed.series_name).as_deref(),
        Some("Evening Report")
    );
    assert_eq!(parsed.episode_title.as_deref(), Some("Election Night"));
}

#[test]
fn render_names_sidecar_parses_back_to_language_flags_and_title() {
    let mut video = facts(LibraryType::Movies);
    video.title = Some("Northern Passage".to_owned());
    video.year = Some(2021);
    let video_path = rendered(&video);
    let flag_sets: [&[SidecarFlag]; 3] = [
        &[],
        &[SidecarFlag::Forced],
        &[
            SidecarFlag::Default,
            SidecarFlag::Forced,
            SidecarFlag::HearingImpaired,
        ],
    ];
    for language in [None, Some("en"), Some("pt-BR"), Some("zh-Hant")] {
        for flags in flag_sets {
            for title in [None, Some("Commentary")] {
                let sidecar = NamingFacts {
                    sidecar: Some(SidecarFacts {
                        language: language.map(str::to_owned),
                        flags: flags.to_vec(),
                        title: title.map(str::to_owned),
                        extension: Some("srt".to_owned()),
                    }),
                    ..video.clone()
                };
                let path = rendered(&sidecar);
                let name = path.rsplit('/').next().expect("a file name");
                let video_name = video_path.rsplit('/').next().expect("a file name");
                let parsed = parse_sidecar(name, video_name, SidecarKind::Subtitle);
                assert!(parsed.matches, "{name}");
                assert_eq!(resolved(&parsed.language).as_deref(), language, "{name}");
                assert_eq!(parsed.flags, flags, "{name}");
                assert_eq!(resolved(&parsed.title).as_deref(), title, "{name}");
            }
        }
    }
}

#[test]
fn render_names_extra_classifies_back_to_kind() {
    let mut movie = facts(LibraryType::Movies);
    movie.title = Some("Northern Passage".to_owned());
    movie.year = Some(2021);
    let video_path = rendered(&movie);
    let kinds = [
        ExtraKind::Trailer,
        ExtraKind::BehindTheScenes,
        ExtraKind::DeletedScene,
        ExtraKind::Featurette,
        ExtraKind::Interview,
        ExtraKind::Scene,
        ExtraKind::Short,
        ExtraKind::Clip,
        ExtraKind::Blooper,
        ExtraKind::Sample,
        ExtraKind::ThemeVideo,
        ExtraKind::Other,
        ExtraKind::Extra,
    ];
    for kind in kinds {
        let extra = NamingFacts {
            extra: Some(ExtraFacts {
                kind,
                title: Some("Making Of".to_owned()),
                year: None,
                group: None,
            }),
            ..movie.clone()
        };
        let path = rendered(&extra);
        let context = vec![video_path.clone(), path.clone()];
        let classified =
            classify_extra(&path, EntryKind::File, Some(LibraryType::Movies), &context);
        assert_eq!(resolved(&classified.extra), Some(kind), "{path}");
    }
}

fn messy_library() -> Vec<String> {
    [
        "Northern Passage 2021/Northern.Passage.2021.1080p.mkv",
        "Northern Passage 2021/Northern.Passage.2021.1080p.en.forced.srt",
        "Northern Passage 2021/Northern.Passage.2021.1080p.nfo",
        "Northern Passage 2021/poster.jpg",
        "Northern Passage 2021/Trailers/Theatrical Trailer.mkv",
        "Quiet Harbor (2020)/Quiet Harbor (2020) - cd1.mkv",
        "Quiet Harbor (2020)/Quiet Harbor (2020) - cd2.mkv",
        "Quiet Harbor (2020)/Quiet Harbor (2020) [720p].mkv",
        "A/Movie.2020.mkv",
        "B/Movie.2020.mkv",
        "Con.mkv",
        "title00.mkv",
        "northern passage (2019)/readme.txt",
        "Northern Passage (2019).mkv",
    ]
    .iter()
    .map(|path| path.to_string())
    .collect()
}

fn plan_of(paths: &[String]) -> nomenclator::RenamePlan {
    let interpretation = interpret(LibraryType::Movies, paths, &[]);
    plan_renames(&interpretation, paths, &OutputProfile::default())
}

#[test]
fn plan_renames_ignores_entry_order() {
    let paths = messy_library();
    let expected = plan_of(&paths);
    assert!(!expected.proposals.is_empty() && !expected.blocked.is_empty());
    let mut reversed = paths.clone();
    reversed.reverse();
    assert_eq!(plan_of(&reversed), expected);
    for rotation in 1..paths.len() {
        let mut rotated = paths.clone();
        rotated.rotate_left(rotation);
        assert_eq!(plan_of(&rotated), expected, "rotated by {rotation}");
    }
}

#[test]
fn plan_renames_proposes_nothing_after_plan_is_applied() {
    let mut paths = messy_library();
    let plan = plan_of(&paths);
    for proposal in &plan.proposals {
        let entry = paths
            .iter_mut()
            .find(|path| **path == proposal.from)
            .expect("a proposal renames an entry");
        *entry = proposal.to.clone();
    }
    assert!(
        plan_of(&paths).proposals.is_empty(),
        "{:?}",
        plan_of(&paths)
    );
}
