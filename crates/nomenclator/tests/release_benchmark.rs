//! Measures `parse_release` against the public release-name test sets in
//! `references/release-names/` (see `references/SOURCES.md`) and prints the match rate per
//! fact. The sets are not committed, so the test is ignored; `just release-benchmark` runs it.

use std::collections::BTreeMap;
use std::path::Path;

use nomenclator::model::{EntryKind, Finding};
use nomenclator::{ReleaseFacts, Rules};
use serde_json::Value;

/// The counts for one fact.
#[derive(Default)]
struct Tally {
    stated: usize,
    matched: usize,
    wrong: usize,
    extra: usize,
}

#[test]
#[ignore = "reads the release-name sets in references/, which are not committed"]
fn parse_release_matches_public_release_name_sets() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../references/release-names");
    let mut tallies: BTreeMap<&'static str, Tally> = BTreeMap::new();
    let mut names = 0;
    let Ok(text) = std::fs::read_to_string(folder.join("set2.json")) else {
        panic!("set2.json is missing: create it as references/SOURCES.md describes");
    };
    let cases: Vec<(String, Value)> = nomenclator::parse_document(&text).expect("valid JSON");
    for (path, expected) in &cases {
        if path.starts_with("__") || path.starts_with('-') || expected.get("options").is_some() {
            continue;
        }
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        // A name without a known extension is a folder name, as a caller would pass it.
        let rules = Rules::builtin();
        let is_file = rules.is_video_file(name).result
            || rules.is_subtitle_file(name, &[]).result
            || rules.is_audio_file(name).result
            || rules.is_image_file(name).result;
        let kind = if is_file {
            EntryKind::File
        } else {
            EntryKind::Directory
        };
        let facts = rules.parse_release(name, kind);
        names += 1;
        compare(&mut tallies, expected, &facts, name);
    }
    println!("{names} release names");
    println!(
        "{:<18} {:>6} {:>8} {:>6} {:>6} {:>6}",
        "fact", "stated", "matched", "rate", "wrong", "extra"
    );
    for (fact, tally) in &tallies {
        let rate = 100.0 * tally.matched as f64 / tally.stated.max(1) as f64;
        println!(
            "{fact:<18} {:>6} {:>8} {:>5.1}% {:>6} {:>6}",
            tally.stated, tally.matched, rate, tally.wrong, tally.extra
        );
    }
}

/// Counts one name's facts against the set's expectation, after mapping the set's values
/// onto this crate's values. A fact the set does not state counts as `extra` if the
/// parser reports it.
fn compare(
    tallies: &mut BTreeMap<&'static str, Tally>,
    expected: &Value,
    facts: &ReleaseFacts,
    name: &str,
) {
    // `RELEASE_MISSES=<fact>` prints that fact's misses and wrong values.
    let shown = std::env::var("RELEASE_MISSES").ok();
    let values = |field: &str| -> Vec<String> {
        match expected.get(field) {
            Some(Value::String(text)) => vec![text.clone()],
            Some(Value::Number(number)) => vec![number.to_string()],
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        }
    };
    let others = values("other");
    let scalar = |finding: &Finding<String>| match finding {
        Finding::Resolved { value, .. } => Some(value.clone()),
        Finding::Ambiguous { candidates, .. } => Some(candidates.join("|")),
        Finding::Unknown => None,
    };
    let mut tally = |fact: &'static str, wanted: Option<String>, got: Option<String>| {
        let tally = tallies.entry(fact).or_default();
        match (wanted, got) {
            (Some(wanted), got) => {
                tally.stated += 1;
                match &got {
                    Some(got) if *got == wanted => tally.matched += 1,
                    Some(_) => tally.wrong += 1,
                    None => {}
                }
                if shown.as_deref() == Some(fact) && got.as_ref() != Some(&wanted) {
                    println!("{fact}: {name}: expected {wanted}, got {got:?}");
                }
            }
            (None, Some(_)) => tally.extra += 1,
            (None, None) => {}
        }
    };
    tally(
        "resolution",
        values("screen_size")
            .first()
            .map(|size| size.to_lowercase()),
        scalar(&facts.resolution),
    );
    tally(
        "source",
        values("source")
            .first()
            .and_then(|source| mapped(SOURCES, source)),
        scalar(&facts.source),
    );
    tally(
        "video_codec",
        values("video_codec")
            .first()
            .and_then(|codec| mapped(VIDEO_CODECS, codec)),
        scalar(&facts.video_codec),
    );
    tally(
        "bit_depth",
        values("color_depth")
            .first()
            .map(|depth| depth.trim_end_matches("-bit").to_owned()),
        match &facts.bit_depth {
            Finding::Resolved { value, .. } => Some(value.to_string()),
            _ => None,
        },
    );
    let set_of = |wanted: Vec<String>, got: Vec<String>| {
        let wanted = (!wanted.is_empty()).then_some(wanted);
        let got_text = (!got.is_empty()).then(|| got.join(","));
        let matched = wanted
            .as_ref()
            .map(|wanted| wanted.iter().all(|value| got.contains(value)));
        (wanted.map(|wanted| wanted.join(",")), got_text, matched)
    };
    let hdr_wanted: Vec<String> = others
        .iter()
        .filter_map(|other| mapped(HDR, other))
        .collect();
    let hdr_got = facts
        .hdr
        .iter()
        .map(|marker| marker.value.clone())
        .collect();
    let (wanted, got, matched) = set_of(hdr_wanted, hdr_got);
    tally(
        "hdr",
        wanted.clone(),
        matched.map_or(
            got.clone(),
            |ok| if ok { wanted.clone() } else { got.clone() },
        ),
    );
    let audio_wanted: Vec<String> = values("audio_codec")
        .iter()
        .filter_map(|codec| mapped(AUDIO_CODECS, codec))
        .collect();
    let audio_got = facts
        .audio_codecs
        .iter()
        .map(|marker| marker.value.clone())
        .collect();
    let (wanted, got, matched) = set_of(audio_wanted, audio_got);
    tally(
        "audio_codecs",
        wanted.clone(),
        matched.map_or(
            got.clone(),
            |ok| if ok { wanted.clone() } else { got.clone() },
        ),
    );
    tally(
        "audio_channels",
        values("audio_channels").first().cloned(),
        scalar(&facts.audio_channels),
    );
    tally(
        "release_group",
        // The set keeps an index site's tag on the group (`LOL[ettv]`); the parser drops it.
        values("release_group").first().map(|group| {
            let group = group.to_lowercase();
            match group.find('[') {
                Some(open) if group.ends_with(']') && open > 0 => group[..open].to_owned(),
                _ => group,
            }
        }),
        scalar(&facts.release_group).map(|group| group.to_lowercase()),
    );
    tally(
        "checksum",
        values("crc32").first().map(|crc| crc.to_uppercase()),
        scalar(&facts.checksum),
    );
    let revision_wanted = others.iter().any(|other| other == "Proper");
    tally(
        "revision",
        revision_wanted.then(|| "yes".to_owned()),
        (!facts.revision.is_empty()).then(|| "yes".to_owned()),
    );
    tally(
        "streaming_service",
        values("streaming_service")
            .first()
            .map(|service| mapped(STREAMING_SERVICES, service).unwrap_or_else(|| service.clone())),
        scalar(&facts.streaming_service),
    );
    tally(
        "container",
        values("container")
            .first()
            .map(|container| container.to_lowercase()),
        facts.container.clone(),
    );
}

fn mapped(table: &[(&str, &str)], value: &str) -> Option<String> {
    table
        .iter()
        .find(|(theirs, _)| *theirs == value)
        .map(|(_, ours)| (*ours).to_owned())
}

/// The sets' source values and this crate's.
const SOURCES: &[(&str, &str)] = &[
    ("HDTV", "hdtv"),
    ("Web", "web"),
    ("Blu-ray", "bluray"),
    ("DVD", "dvd"),
    ("Digital TV", "digital_tv"),
    ("Ultra HD Blu-ray", "uhd_bluray"),
    ("TV", "tv"),
    ("HD-DVD", "hd_dvd"),
    ("VHS", "vhs"),
    ("Satellite", "satellite"),
    ("Camera", "camera"),
    ("Laserdisc", "laserdisc"),
    ("Analog HDTV", "analog_hdtv"),
    ("Ultra HDTV", "ultra_hdtv"),
    ("HD Telecine", "hd_telecine"),
    ("HD Camera", "hd_camera"),
    ("HD Telesync", "hd_telesync"),
    ("Telesync", "telesync"),
    ("Telecine", "telecine"),
    ("Workprint", "workprint"),
    ("Video on Demand", "video_on_demand"),
];

/// The sets' video codec values and this crate's.
const VIDEO_CODECS: &[(&str, &str)] = &[
    ("H.264", "h264"),
    ("H.265", "h265"),
    ("Xvid", "xvid"),
    ("DivX", "divx"),
    ("MPEG-2", "mpeg2"),
    ("VC-1", "vc1"),
    ("VP9", "vp9"),
    ("AV1", "av1"),
];

/// The sets' HDR values (under `other`) and this crate's.
const HDR: &[(&str, &str)] = &[
    ("HDR10", "hdr10"),
    ("HDR10+", "hdr10_plus"),
    ("Dolby Vision", "dolby_vision"),
];

/// The sets' audio codec values and this crate's.
const AUDIO_CODECS: &[(&str, &str)] = &[
    ("Dolby Digital", "dolby_digital"),
    ("Dolby Digital Plus", "dolby_digital_plus"),
    ("AAC", "aac"),
    ("DTS-HD", "dts_hd"),
    ("DTS", "dts"),
    ("FLAC", "flac"),
    ("Dolby Atmos", "dolby_atmos"),
    ("Opus", "opus"),
    ("MP2", "mp2"),
    ("LPCM", "lpcm"),
    ("Dolby TrueHD", "dolby_truehd"),
    ("Vorbis", "vorbis"),
    ("MP3", "mp3"),
    ("PCM", "pcm"),
    ("DTS:X", "dts_x"),
];

/// The sets' streaming service values and this crate's.
const STREAMING_SERVICES: &[(&str, &str)] = &[
    ("Amazon Prime", "amazon_prime"),
    ("Netflix", "netflix"),
    ("iTunes", "itunes"),
    ("Hulu", "hulu"),
    ("DC Universe", "dc_universe"),
    ("Viki", "viki"),
];
