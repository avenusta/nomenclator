//! Checks that the reader reads every name that the writer writes (X25): a library of
//! every kind of rendered name is interpreted back, under the built-in rules and under
//! the rules of each language pack alone, with every profile layout.

use std::collections::{BTreeMap, BTreeSet};

use nomenclator::model::{ArtworkRole, ExtraKind, SidecarFlag};
use nomenclator::{
    AssetFacts, AssetRole, ExtraFacts, Item, LibraryType, MovieLayout, NamingFacts, OutputProfile,
    ProviderTagStyle, Rules, SeasonFolderStyle, SidecarFacts, VersionLabelStyle,
};

const EXTRA_KINDS: &[ExtraKind] = &[
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
    ExtraKind::ThemeSong,
    ExtraKind::ThemeVideo,
    ExtraKind::Other,
    ExtraKind::Extra,
];

const ARTWORK: &[AssetRole] = &[
    AssetRole::Poster,
    AssetRole::Backdrop,
    AssetRole::Banner,
    AssetRole::Logo,
    AssetRole::Thumb,
];

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
        has_extras: true,
        asset: None,
    }
}

fn asset(base: &NamingFacts, role: AssetRole, extension: &str) -> NamingFacts {
    NamingFacts {
        asset: Some(AssetFacts {
            role,
            extension: Some(extension.to_owned()),
        }),
        ..base.clone()
    }
}

fn subtitle(base: &NamingFacts, flags: &[SidecarFlag]) -> NamingFacts {
    NamingFacts {
        sidecar: Some(SidecarFacts {
            language: Some("en".to_owned()),
            flags: flags.to_vec(),
            title: None,
            extension: Some("srt".to_owned()),
        }),
        ..base.clone()
    }
}

/// Returns the facts of every kind of name of one movie.
fn movie_names() -> Vec<NamingFacts> {
    let movie = NamingFacts {
        title: Some("Northern Passage".to_owned()),
        year: Some(2021),
        provider_ids: BTreeMap::from([("tmdb".to_owned(), "329865".to_owned())]),
        ..facts(LibraryType::Movies)
    };
    let mut names = Vec::new();
    for (label, edition) in [("1080p", None), ("2160p", Some("Director's Cut"))] {
        for part in [1, 2] {
            let version = NamingFacts {
                version_label: Some(label.to_owned()),
                edition: edition.map(str::to_owned),
                part: Some(part),
                ..movie.clone()
            };
            names.push(version.clone());
            if part == 1 && edition.is_none() {
                for flags in [
                    &[SidecarFlag::Default][..],
                    &[SidecarFlag::Forced],
                    &[SidecarFlag::HearingImpaired],
                ] {
                    names.push(subtitle(&version, flags));
                }
            }
        }
    }
    names.extend(ARTWORK.iter().map(|role| asset(&movie, *role, "jpg")));
    names.push(asset(&movie, AssetRole::Nfo, "nfo"));
    names.push(asset(&movie, AssetRole::ThemeSong, "mp3"));
    for kind in EXTRA_KINDS {
        let extension = if *kind == ExtraKind::ThemeSong {
            "mp3"
        } else {
            "mkv"
        };
        names.push(NamingFacts {
            extra: Some(ExtraFacts {
                kind: *kind,
                title: Some(format!("Extra {kind:?}")),
                year: None,
                group: None,
            }),
            container: Some(extension.to_owned()),
            ..movie.clone()
        });
    }
    names
}

/// Returns the facts of every kind of name of one series.
fn series_names() -> Vec<NamingFacts> {
    let series = NamingFacts {
        title: Some("Quiet Hours".to_owned()),
        year: Some(2019),
        provider_ids: BTreeMap::from([("tvdb".to_owned(), "81189".to_owned())]),
        ..facts(LibraryType::Tv)
    };
    let mut names = Vec::new();
    names.extend(ARTWORK.iter().map(|role| asset(&series, *role, "jpg")));
    names.push(asset(&series, AssetRole::Nfo, "nfo"));
    names.push(asset(&series, AssetRole::ThemeSong, "mp3"));
    for season in [0, 1, 12] {
        let season_facts = NamingFacts {
            season: Some(season),
            ..series.clone()
        };
        names.push(asset(&season_facts, AssetRole::Poster, "jpg"));
        names.push(asset(&season_facts, AssetRole::Nfo, "nfo"));
        for episodes in [vec![1], vec![2, 3], vec![5, 7]] {
            let episode = NamingFacts {
                episodes,
                episode_title: Some("Low Tide".to_owned()),
                ..season_facts.clone()
            };
            names.push(episode.clone());
            names.push(asset(&episode, AssetRole::EpisodeStill, "jpg"));
            names.push(asset(&episode, AssetRole::Nfo, "nfo"));
            names.push(subtitle(&episode, &[SidecarFlag::Forced]));
        }
    }
    names
}

fn profiles() -> Vec<OutputProfile> {
    vec![
        OutputProfile::default(),
        OutputProfile {
            movie_layout: MovieLayout::Flat,
            season_folder_style: SeasonFolderStyle::Unpadded,
            provider_tag_style: ProviderTagStyle::BracketsId,
            version_label_style: VersionLabelStyle::Brackets,
            ..OutputProfile::default()
        },
    ]
}

fn every_rules() -> Vec<(String, Rules)> {
    let mut rules = vec![("built-in".to_owned(), Rules::builtin().clone())];
    for tag in Rules::language_packs() {
        let pack = Rules::for_languages(&[tag.to_owned()], &[]).expect("a pack builds");
        rules.push((tag.to_owned(), pack));
    }
    rules
}

fn render_all(rules: &Rules, profile: &OutputProfile, names: &[NamingFacts]) -> Vec<String> {
    let mut paths: Vec<String> = names
        .iter()
        .map(|facts| {
            rules
                .render_names(facts, profile)
                .path
                .unwrap_or_else(|| panic!("the writer renders {facts:?}"))
        })
        .collect();
    paths.sort();
    paths
}

#[test]
fn written_movie_names_read_back_under_every_rules() {
    for (rules_name, rules) in every_rules() {
        for profile in profiles() {
            let paths = render_all(&rules, &profile, &movie_names());
            let interpretation = rules.interpret(LibraryType::Movies, &paths, &[]);
            let context = format!("{rules_name} {profile:?}: {paths:#?}");
            assert!(
                interpretation.unclaimed_files.is_empty(),
                "{context}\nunclaimed {:?}",
                interpretation.unclaimed_files
            );
            let movies: Vec<_> = interpretation
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Movie(movie) => Some(movie),
                    _ => None,
                })
                .collect();
            assert_eq!(movies.len(), 1, "{context}");
            let movie = movies[0];
            assert_eq!(
                movie.title.as_deref(),
                Some("Northern Passage"),
                "{context}"
            );
            assert_eq!(movie.year, Some(2021), "{context}");
            assert_eq!(movie.versions.len(), 2, "{context}");
            assert!(movie.nfo.is_some(), "{context}");
            let kinds: BTreeSet<ExtraKind> = movie.extras.iter().map(|extra| extra.extra).collect();
            assert_eq!(kinds, EXTRA_KINDS.iter().copied().collect(), "{context}");
            let roles: BTreeSet<ArtworkRole> =
                movie.artwork.iter().map(|image| image.role).collect();
            let expected: BTreeSet<ArtworkRole> = [
                ArtworkRole::Poster,
                ArtworkRole::Backdrop,
                ArtworkRole::Banner,
                ArtworkRole::Logo,
                ArtworkRole::Thumb,
            ]
            .into();
            assert_eq!(roles, expected, "{context}");
            let flags: BTreeSet<SidecarFlag> = movie
                .subtitles
                .iter()
                .flat_map(|subtitle| subtitle.flags.clone())
                .collect();
            assert_eq!(flags.len(), 3, "{context}");
        }
    }
}

#[test]
fn written_series_names_read_back_under_every_rules() {
    for (rules_name, rules) in every_rules() {
        for profile in profiles() {
            let paths = render_all(&rules, &profile, &series_names());
            let interpretation = rules.interpret(LibraryType::Tv, &paths, &[]);
            let context = format!("{rules_name} {profile:?}: {paths:#?}");
            assert!(
                interpretation.unclaimed_files.is_empty(),
                "{context}\nunclaimed {:?}",
                interpretation.unclaimed_files
            );
            let mut series = Vec::new();
            let mut seasons = BTreeSet::new();
            let mut episodes = 0;
            for item in &interpretation.items {
                match item {
                    Item::Series(found) => series.push(found),
                    Item::Season(season) => {
                        assert!(season.nfo.is_some(), "{context}: season {}", season.season);
                        seasons.insert(season.season);
                    }
                    Item::Episode(episode) => {
                        episodes += 1;
                        assert!(episode.nfo.is_some(), "{context}: {:?}", episode.episodes);
                        assert!(
                            episode
                                .artwork
                                .iter()
                                .any(|image| image.role == ArtworkRole::EpisodeStill),
                            "{context}: {:?}",
                            episode.episodes
                        );
                        assert!(!episode.ambiguous, "{context}: {:?}", episode.episodes);
                    }
                    other => panic!("{context}: unexpected {other:?}"),
                }
            }
            assert_eq!(series.len(), 1, "{context}");
            assert_eq!(series[0].title.as_deref(), Some("Quiet Hours"), "{context}");
            assert_eq!(series[0].year, Some(2019), "{context}");
            assert!(series[0].nfo.is_some(), "{context}");
            assert_eq!(series[0].artwork.len(), ARTWORK.len(), "{context}");
            assert_eq!(seasons, BTreeSet::from([0, 1, 12]), "{context}");
            assert_eq!(episodes, 9, "{context}");
        }
    }
}
