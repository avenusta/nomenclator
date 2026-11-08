//! Provides the constants that code reads directly: the canonical names that the writer
//! writes, year bounds, resolutions, and the documents of the built-in vocabularies, the
//! language packs and the message catalogues. Every other vocabulary is in
//! `data/builtin.yaml`.

use crate::model::{ArtworkRole, ExtraKind, NfoRole, SidecarFlag};

/// The normalized source value of a screener (X19).
pub(crate) const SCREENER_SOURCE_VALUE: &str = "screener";

/// The default earliest year that a name can state (M2).
pub(crate) const EARLIEST_PLAUSIBLE_YEAR: u16 = 1900;

/// The smallest value of the `earliest_plausible_year` preset (X12).
pub(crate) const LOWEST_EARLIEST_YEAR: u16 = 1800;

/// The default latest year that a name can state, and the largest value of the
/// `latest_plausible_year` preset (M2).
pub(crate) const LATEST_PLAUSIBLE_YEAR: u16 = 2099;

/// The built-in exclusion groups, each switchable by overlays (X13).
pub(crate) const BUILT_IN_IGNORE_GROUPS: &[&str] = &[
    "hidden_entries",
    "system_junk",
    "partial_downloads",
    "generated_metadata",
];

/// The episode words that every library reads. The canonical section holds `e`.
pub(crate) const EPISODE_BASE_WORDS: &[&str] = &["episode", "ep"];

/// Resolution words and the height that each states. An explicit height in the same name
/// takes precedence over them, because `4K` also names a restoration.
pub(crate) const RESOLUTION_WORDS: &[(&str, u32)] = &[("4k", 2160), ("8k", 4320), ("uhd", 2160)];

/// Heights that are a resolution on their own, without a scan type (`1080`).
pub(crate) const BARE_RESOLUTIONS: &[&str] = &["720", "1080", "2160"];

// The canonical names (X25). The writer writes each of them, and the rules add each to
// the input vocabulary named in its comment, so the reader reads every name that the
// writer writes. The input tables above hold only the other spellings that the reader
// accepts. An overlay cannot remove or redefine a canonical name.

/// The name of each artwork role (R19), read as `artwork_roles`.
pub(crate) const ARTWORK_OUTPUT_WORDS: &[(ArtworkRole, &str)] = &[
    (ArtworkRole::Poster, "poster"),
    (ArtworkRole::Backdrop, "fanart"),
    (ArtworkRole::Banner, "banner"),
    (ArtworkRole::Logo, "clearlogo"),
    (ArtworkRole::Thumb, "landscape"),
    (ArtworkRole::ClearArt, "clearart"),
    (ArtworkRole::DiscArt, "discart"),
];

/// The generic NFO name of each role (R19, S9), read as `nfo_names`.
pub(crate) const NFO_OUTPUT_NAMES: &[(NfoRole, &str)] = &[
    (NfoRole::Movie, "movie"),
    (NfoRole::TvShow, "tvshow"),
    (NfoRole::Season, "season"),
];

/// The extension of an NFO file (R19), read as `nfo_extensions`.
pub(crate) const NFO_EXTENSION: &str = "nfo";

/// The suffix of a video's still (S6, R19), `Video-thumb.jpg`, read as `still_image_suffixes`.
pub(crate) const STILL_IMAGE_SUFFIX: &str = "-thumb";

/// The stem of a theme song or theme video (R19), read as `extras_audio_names`.
pub(crate) const THEME_OUTPUT_STEM: &str = "theme";

/// The word of a season folder (R2), read as `season_words`.
pub(crate) const SEASON_FOLDER_WORD: &str = "Season";

/// The separator between the words of a written name, read as `separators`.
pub(crate) const SPACE_SEPARATOR: char = ' ';

/// The bracket pairs of a written name (R1, R15), read as `bracket_pairs`.
pub(crate) const CANONICAL_BRACKET_PAIRS: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}')];

/// The letter before an episode number in an episode token (R2), read as `episode_words`.
pub(crate) const EPISODE_OUTPUT_LETTER: &str = "e";

/// The word of a part number (R1), read as `part_words`.
pub(crate) const PART_OUTPUT_WORD: &str = "part";

/// The key of an edition tag, `{edition-X}` (R1), read by the edition tag pattern.
pub(crate) const EDITION_TAG_KEY: &str = "edition";

/// The one folder each kind of extra is written to (R1), read as `extras_folders`.
pub(crate) const EXTRAS_OUTPUT_FOLDERS: &[(ExtraKind, &str)] = &[
    (ExtraKind::Trailer, "Trailers"),
    (ExtraKind::BehindTheScenes, "Behind The Scenes"),
    (ExtraKind::DeletedScene, "Deleted Scenes"),
    (ExtraKind::Featurette, "Featurettes"),
    (ExtraKind::Interview, "Interviews"),
    (ExtraKind::Scene, "Scenes"),
    (ExtraKind::Short, "Shorts"),
    (ExtraKind::Clip, "Clips"),
    (ExtraKind::Blooper, "Bloopers"),
    (ExtraKind::Sample, "Samples"),
    (ExtraKind::ThemeSong, "Theme Music"),
    (ExtraKind::ThemeVideo, "Backdrops"),
    (ExtraKind::Other, "Other"),
    (ExtraKind::Extra, "Extras"),
    (ExtraKind::Opening, "Openings"),
    (ExtraKind::Ending, "Endings"),
];

/// Device names that no path component may use before its first dot on the portable target
/// (R5).
pub(crate) const RESERVED_DEVICE_NAMES: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// How each sidecar flag is written (R4), read as the flag's words.
pub(crate) const SIDECAR_FLAG_OUTPUT_WORDS: &[(SidecarFlag, &str)] = &[
    (SidecarFlag::Default, "default"),
    (SidecarFlag::Forced, "forced"),
    (SidecarFlag::HearingImpaired, "sdh"),
];

/// The built-in vocabularies, as one overlay document.
pub(crate) const BUILTIN: &str = include_str!("../data/builtin.yaml");

/// Language packs by BCP 47 tag: overlay documents that contain the words of each language
/// (X11).
pub(crate) const LANGUAGE_PACKS: &[(&str, &str)] = &[
    ("bg", include_str!("../data/languages/bg.yaml")),
    ("bs", include_str!("../data/languages/bs.yaml")),
    ("cs", include_str!("../data/languages/cs.yaml")),
    ("da", include_str!("../data/languages/da.yaml")),
    ("de", include_str!("../data/languages/de.yaml")),
    ("en", include_str!("../data/languages/en.yaml")),
    ("es", include_str!("../data/languages/es.yaml")),
    ("fi", include_str!("../data/languages/fi.yaml")),
    ("fr", include_str!("../data/languages/fr.yaml")),
    ("hr", include_str!("../data/languages/hr.yaml")),
    ("it", include_str!("../data/languages/it.yaml")),
    ("ja", include_str!("../data/languages/ja.yaml")),
    ("ko", include_str!("../data/languages/ko.yaml")),
    ("nl", include_str!("../data/languages/nl.yaml")),
    ("no", include_str!("../data/languages/no.yaml")),
    ("pl", include_str!("../data/languages/pl.yaml")),
    ("pt", include_str!("../data/languages/pt.yaml")),
    ("ro", include_str!("../data/languages/ro.yaml")),
    ("ru", include_str!("../data/languages/ru.yaml")),
    ("sk", include_str!("../data/languages/sk.yaml")),
    ("sl", include_str!("../data/languages/sl.yaml")),
    ("sr", include_str!("../data/languages/sr.yaml")),
    ("sv", include_str!("../data/languages/sv.yaml")),
    ("tr", include_str!("../data/languages/tr.yaml")),
    ("uk", include_str!("../data/languages/uk.yaml")),
];

/// Diagnostic message catalogues by BCP 47 tag (R13). English is the fallback.
pub(crate) const MESSAGE_CATALOGS: &[(&str, &str)] =
    &[("en", include_str!("../data/messages/en.yaml"))];
