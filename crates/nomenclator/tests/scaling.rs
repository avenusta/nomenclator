//! Tests that interpretation and planning time grows linearly with the library size.
//! The tests build synthetic libraries of growing size in memory and check that the time
//! is roughly proportional to the size. `just bench` runs the large libraries in release
//! mode against absolute bounds.

use std::path::Path;
use std::time::{Duration, Instant};

use nomenclator::{CallerInputs, LibraryType, OutputProfile, Rules};

/// Returns the paths of movie folders, each with a video, its subtitle, artwork, an NFO
/// and an extra. Every fourth movie is at the library root, next to its files.
fn movie_library(movies: usize) -> Vec<String> {
    let mut paths = Vec::new();
    for number in 0..movies {
        let year = 1950 + number % 70;
        if number % 4 == 0 {
            let stem = format!("Loose Movie {number} ({year})");
            for suffix in [".mkv", ".en.srt", "-trailer.mkv", "-poster.jpg", ".nfo"] {
                paths.push(format!("{stem}{suffix}"));
            }
            continue;
        }
        let folder = format!("Movie Title {number} ({year})");
        let stem = format!("Movie.Title.{number}.{year}.1080p.BluRay.x264-GRP");
        paths.push(format!("{folder}/"));
        paths.push(format!("{folder}/{stem}.mkv"));
        paths.push(format!("{folder}/{stem}.en.srt"));
        paths.push(format!("{folder}/poster.jpg"));
        paths.push(format!("{folder}/movie.nfo"));
        paths.push(format!("{folder}/Extras/"));
        paths.push(format!("{folder}/Extras/Making Of.mkv"));
    }
    paths
}

/// Returns the paths of series folders with season folders of episodes, each with a
/// subtitle.
fn tv_library(series: usize, seasons: usize, episodes: usize) -> Vec<String> {
    let mut paths = Vec::new();
    for number in 0..series {
        let show = format!("Show Name {number} ({})", 1950 + number % 70);
        paths.push(format!("{show}/"));
        paths.push(format!("{show}/poster.jpg"));
        paths.push(format!("{show}/tvshow.nfo"));
        for season in 1..=seasons {
            let folder = format!("{show}/Season {season:02}");
            paths.push(format!("{folder}/"));
            paths.push(format!("{folder}/season.nfo"));
            for episode in 1..=episodes {
                let stem =
                    format!("{folder}/Show Name {number} - S{season:02}E{episode:02} - Title");
                paths.push(format!("{stem}.mkv"));
                paths.push(format!("{stem}.en.srt"));
            }
        }
    }
    paths
}

/// Returns the fastest time of three runs of interpreting `paths`, and of planning their
/// renames.
fn timings(library: LibraryType, paths: &[String]) -> (Duration, Duration) {
    let mut fastest = (Duration::MAX, Duration::MAX);
    for _ in 0..3 {
        let start = Instant::now();
        let interpretation = Rules::builtin().interpret(library, paths, &[]);
        let interpreted = start.elapsed();
        let plan = Rules::builtin().plan_renames(&interpretation, paths, &OutputProfile::default());
        let planned = start.elapsed() - interpreted;
        assert!(!interpretation.items.is_empty() && !plan.proposals.is_empty());
        fastest = (fastest.0.min(interpreted), fastest.1.min(planned));
    }
    fastest
}

fn time_to_plan(library: LibraryType, paths: &[String]) -> Duration {
    let (interpreted, planned) = timings(library, paths);
    interpreted + planned
}

/// Asserts that a library four times as large takes well under seven times as long. A
/// pairwise pass would take sixteen times as long.
fn assert_scales_linearly(library: LibraryType, small: &[String], large: &[String]) {
    assert_eq!(
        large.len(),
        small.len() * 4,
        "the large library is four times the small"
    );
    let small_time = time_to_plan(library, small);
    let large_time = time_to_plan(library, large);
    let ratio = large_time.as_secs_f64() / small_time.as_secs_f64();
    println!(
        "{} paths: {small_time:?}; {} paths: {large_time:?}",
        small.len(),
        large.len()
    );
    assert!(
        ratio < 7.0,
        "{} paths took {small_time:?}, {} paths took {large_time:?}: {ratio:.1}× for 4× the paths",
        small.len(),
        large.len()
    );
}

#[test]
fn interpret_movie_library_scales_linearly() {
    assert_scales_linearly(
        LibraryType::Movies,
        &movie_library(400),
        &movie_library(1600),
    );
}

#[test]
fn interpret_tv_library_scales_linearly() {
    assert_scales_linearly(
        LibraryType::Tv,
        &tv_library(30, 2, 10),
        &tv_library(120, 2, 10),
    );
}

/// Returns the first `entries` paths of a library that `generate` builds with growing
/// size, so that each benchmark library has exactly that many entries.
fn library_of(entries: usize, generate: impl Fn(usize) -> Vec<String>) -> Vec<String> {
    let mut size = 1;
    loop {
        let mut paths = generate(size);
        if paths.len() >= entries {
            paths.truncate(entries);
            return paths;
        }
        size *= 2;
    }
}

/// Measures interpretation and planning of movie and TV libraries of 1k, 10k and 100k
/// entries, and the re-interpretation of one scope of the largest (X23). Writes the
/// times to `target/benchmarks.json`, which CI keeps for each run. The bounds are
/// meaningful only in release mode (`just bench`).
#[test]
#[ignore = "large libraries; run in release mode with `just bench`"]
fn large_libraries_plan_within_bounds() {
    let mut records = Vec::new();
    for entries in [1_000, 10_000, 100_000] {
        let cases = [
            (
                "movies",
                LibraryType::Movies,
                library_of(entries, movie_library),
            ),
            (
                "tv",
                LibraryType::Tv,
                library_of(entries, |series| tv_library(series, 2, 20)),
            ),
        ];
        for (name, library, paths) in cases {
            let (interpreted, planned) = timings(library, &paths);
            let scoped = time_to_interpret_one_scope(library, &paths);
            println!(
                "{name}, {entries} entries: interpreted in {interpreted:?}, planned in {planned:?}, one scope in {scoped:?}"
            );
            records.push(format!(
                r#"{{"library":"{name}","entries":{entries},"interpret_ms":{:.3},"plan_ms":{:.3},"scope_ms":{:.3}}}"#,
                interpreted.as_secs_f64() * 1000.0,
                planned.as_secs_f64() * 1000.0,
                scoped.as_secs_f64() * 1000.0
            ));
            // CI runners are slower than a workstation, so the bound is generous.
            let limit = Duration::from_secs(10);
            assert!(
                interpreted < limit,
                "{name} {entries} interpreted in {interpreted:?}"
            );
            assert!(planned < limit, "{name} {entries} planned in {planned:?}");
            assert!(
                scoped < interpreted,
                "{name} {entries}: one scope took {scoped:?}, the whole library {interpreted:?}"
            );
        }
    }
    let report = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/benchmarks.json");
    std::fs::write(&report, format!("[\n{}\n]\n", records.join(",\n")))
        .expect("the benchmark report is written");
}

/// Returns the fastest time of three runs of interpreting the scope group of the last
/// path.
fn time_to_interpret_one_scope(library: LibraryType, paths: &[String]) -> Duration {
    let inputs = CallerInputs::default();
    let rules = Rules::builtin();
    let last = paths.last().expect("a library has paths");
    let groups = Rules::scope_groups(paths, &inputs);
    let group = groups
        .iter()
        .find(|group| group.contains(&Rules::scope_of(last).map(str::to_owned)))
        .expect("every scope is in a group");
    (0..3)
        .map(|_| {
            let start = Instant::now();
            let interpretation = rules.interpret_scopes(library, group, paths, &[], &inputs);
            assert!(!interpretation.items.is_empty());
            start.elapsed()
        })
        .min()
        .expect("three runs")
}
