//! Provides the data tables: vocabularies that can grow without code changes. All entries are
//! lowercase.

use crate::model::{ArtworkRole, ExtraKind, Format3d, NfoRole, SidecarFlag};
use crate::parsers::release::ReleaseFact;

/// Release words that cut a title wherever they appear.
pub(crate) const TECHNICAL_WORDS: &[&str] = &[
    "bluray", "bdrip", "brrip", "bdremux", "remux", "dvd", "dvdrip", "dvdscr", "dvd5", "dvd9",
    "hddvd", "hdtv", "pdtv", "webdl", "web-dl", "webrip", "hdrip", "hdtc", "telesync", "x264",
    "x265", "h264", "h265", "hevc", "av1", "xvid", "divx", "aac", "ac3", "eac3", "dts", "dts-hd",
    "dtshd", "truehd", "atmos", "ddp", "hdc", "ultrahd", "repack", "rerip", "sbs", "hsbs", "fsbs",
    "htab", "ftab", "mvc", "sdtv", "bdmv", "hdr10", "hdr10+", "dovi", "sbs3d", "bd25", "bd50",
    "dsr", "mp4", "mkv", "m4v", "avi", "wmv", "mpg", "mpeg",
];

/// Release words that can also be ordinary title words. They cut only after a year or a
/// technical word, or when nothing but release material follows them.
pub(crate) const TITLE_LIKE_RELEASE_WORDS: &[&str] = &[
    "web",
    "dl",
    "4k",
    "8k",
    "uhd",
    "hdr",
    "sdr",
    "blu-ray",
    "opus",
    "flac",
    "avc",
    "3d",
    "tab",
    "vhs",
    "german",
    "french",
    "multi",
    "multisub",
    "multisubs",
    "dubbed",
    "subbed",
    "limited",
    "proper",
    "internal",
    "dc",
    "ts",
    "cam",
    "stv",
    "dual",
    "end",
    "complete",
];

/// Edition words after a title. They are hints, never editions. They cut a title the same way
/// as title-like release words.
pub(crate) const EDITION_WORDS: &[&str] = &[
    "extended",
    "unrated",
    "uncut",
    "remastered",
    "restored",
    "theatrical",
    "imax",
    "criterion",
    "despecialized",
    "workprint",
    "hybrid",
];

/// Edition phrases of several words, compared without apostrophes. Their words count as
/// edition words where the phrase appears.
pub(crate) const EDITION_PHRASES: &[&[&str]] = &[
    &["directors", "cut"],
    &["final", "cut"],
    &["extended", "cut"],
    &["theatrical", "cut"],
    &["fan", "edit"],
    &["special", "edition"],
    &["collectors", "edition"],
    &["ultimate", "edition"],
    &["extended", "edition"],
    &["anniversary", "edition"],
    &["super", "sized"],
    &["r", "rated", "version"],
];

/// Words for a 3D layout. `3d` alone is not a layout word.
pub(crate) const FORMAT_3D_WORDS: &[(&str, Format3d)] = &[
    ("sbs", Format3d::SideBySide),
    ("sbs3d", Format3d::SideBySide),
    ("hsbs", Format3d::HalfSideBySide),
    ("fsbs", Format3d::FullSideBySide),
    ("tab", Format3d::TopAndBottom),
    ("htab", Format3d::HalfTopAndBottom),
    ("ftab", Format3d::FullTopAndBottom),
    ("mvc", Format3d::Mvc),
    ("3d", Format3d::Unspecified),
];

/// Provider keys recognised in tags, each optionally followed by `id`.
pub(crate) const PROVIDER_KEYS: &[&str] = &[
    "tmdb",
    "imdb",
    "tvdb",
    "tvmaze",
    "anidb",
    "anilist",
    "anisearch",
];

/// Endings that mark a leading bracket as a release group.
pub(crate) const RELEASE_GROUP_ENDINGS: &[&str] = &[
    "subs", "raws", "encodes", "encode", "rips", ".com", ".net", ".org",
];

/// Extensions of video files a title can be made of.
pub(crate) const VIDEO_EXTENSIONS: &[&str] = &[
    "3gp", "avi", "divx", "flv", "m2ts", "m4v", "mk3d", "mkv", "mov", "mp4", "mpeg", "mpg", "mts",
    "ogv", "ts", "webm", "wmv",
];

/// Extensions recognised as video but reported as unsupported rather than admitted.
pub(crate) const UNSUPPORTED_VIDEO_EXTENSIONS: &[&str] = &["vob", "rm", "rmvb"];

/// Disc image extensions: disc structures, reported and never admitted as videos.
pub(crate) const DISC_IMAGE_EXTENSIONS: &[&str] = &["iso", "img"];

/// Folders that hold a disc structure. The files inside are disc clips, not titles.
pub(crate) const DISC_STRUCTURE_FOLDERS: &[&str] = &["bdmv", "video_ts"];

/// Subtitle extensions. `idx` counts only beside its `sub`, and that `sub` is not counted again.
pub(crate) const SUBTITLE_EXTENSIONS: &[&str] = &[
    "srt", "ass", "ssa", "vtt", "sub", "idx", "sup", "smi", "sami", "mks",
];

/// External audio file extensions (S5).
pub(crate) const AUDIO_EXTENSIONS: &[&str] = &[
    "aa", "aac", "ac3", "dts", "eac3", "flac", "m4a", "mka", "mp3", "ogg", "opus", "thd", "wav",
    "wma",
];

/// Image file extensions.
pub(crate) const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp"];

/// The extension of NFO files.
pub(crate) const NFO_EXTENSION: &str = "nfo";

/// NFO stems that refer to the item of their folder rather than to a video (S9).
pub(crate) const NFO_GENERIC_NAMES: &[(&str, NfoRole)] = &[];

/// Suffix that marks an image as a video's still: `Video-thumb.jpg`.
pub(crate) const STILL_IMAGE_SUFFIX: &str = "-thumb";

/// Sidecar words that set the default flag.
pub(crate) const DEFAULT_FLAG_WORDS: &[&str] = &[];

/// Sidecar words that set the forced flag.
pub(crate) const FORCED_FLAG_WORDS: &[&str] = &["foreign"];

/// Sidecar words that set the hearing-impaired flag. `hi` is handled separately because it is
/// also Hindi.
pub(crate) const HEARING_IMPAIRED_FLAG_WORDS: &[&str] = &["cc"];

/// The word after an episode number that marks the last episode of a release run (X20).
pub(crate) const FINAL_EPISODE_WORD: &str = "end";

/// The word that means hearing impaired beside another language, and Hindi otherwise.
pub(crate) const HEARING_IMPAIRED_OR_HINDI_WORD: &str = "hi";

/// Languages as pairs of a BCP 47 tag and its other spellings: ISO 639-2 codes and English
/// names. Codes that are common English words (`cat`, `may`, `mac`) are left out on purpose.
pub(crate) const LANGUAGES: &[(&str, &[&str])] = &[
    ("ar", &["ara", "arabic"]),
    ("bg", &["bul", "bulgarian"]),
    ("bn", &["ben", "bengali"]),
    ("bs", &["bos", "bosnian"]),
    ("ca", &["catalan"]),
    ("cs", &["ces", "cze", "czech"]),
    ("da", &["dan", "danish"]),
    ("de", &["deu", "ger", "german"]),
    ("el", &["ell", "gre", "greek"]),
    ("en", &["eng", "english"]),
    ("es", &["spa", "spanish"]),
    ("et", &["est", "estonian"]),
    ("fa", &["fas", "per", "persian", "farsi"]),
    ("fi", &["fin", "finnish"]),
    ("fil", &["filipino"]),
    ("fr", &["fra", "fre", "french"]),
    ("he", &["heb", "hebrew"]),
    ("hi", &["hin", "hindi"]),
    ("hr", &["hrv", "croatian"]),
    ("hu", &["hun", "hungarian"]),
    ("id", &["ind", "indonesian"]),
    ("is", &["isl", "ice", "icelandic"]),
    ("it", &["ita", "italian"]),
    ("ja", &["jpn", "japanese"]),
    ("ko", &["kor", "korean"]),
    ("lt", &["lit", "lithuanian"]),
    ("lv", &["lav", "latvian"]),
    ("ms", &["msa", "malay"]),
    ("nl", &["nld", "dut", "dutch"]),
    ("no", &["nor", "norwegian"]),
    ("pl", &["pol", "polish"]),
    ("pt", &["por", "portuguese"]),
    ("ro", &["ron", "rum", "romanian"]),
    ("ru", &["rus", "russian"]),
    ("sk", &["slk", "slo", "slovak"]),
    ("sl", &["slv", "slovenian", "slovene"]),
    ("sr", &["srp", "serbian"]),
    ("sv", &["swe", "swedish"]),
    ("ta", &["tam", "tamil"]),
    ("te", &["tel", "telugu"]),
    ("th", &["tha", "thai"]),
    ("tl", &["tgl", "tagalog"]),
    ("tr", &["tur", "turkish"]),
    ("uk", &["ukr", "ukrainian"]),
    ("ur", &["urd", "urdu"]),
    ("vi", &["vie", "vietnamese"]),
    ("yue", &["cantonese"]),
    ("zh", &["zho", "chi", "chinese"]),
];

/// The earliest year that a name can state (M2).
pub(crate) const EARLIEST_PLAUSIBLE_YEAR: u16 = 1900;

/// The default latest year that a name can state, and the largest value of the
/// `latest_plausible_year` preset (M2).
pub(crate) const LATEST_PLAUSIBLE_YEAR: u16 = 2099;

/// Subtitle folder names beside a video, compared without case (S2).
pub(crate) const SUBTITLE_FOLDER_NAMES: &[&str] = &["subs", "subtitles"];

/// Extras folder names, compared without case or spacing, and the kind of extra they hold.
pub(crate) const EXTRAS_FOLDERS: &[(&str, ExtraKind)] =
    &[("teasers", ExtraKind::Trailer), ("extra", ExtraKind::Extra)];

/// Extras suffix words, compared without case or spacing, and the kind of extra they mark.
pub(crate) const EXTRAS_SUFFIXES: &[(&str, ExtraKind)] = &[
    ("trailer", ExtraKind::Trailer),
    ("teaser", ExtraKind::Trailer),
    ("behindthescenes", ExtraKind::BehindTheScenes),
    ("deleted", ExtraKind::DeletedScene),
    ("deletedscene", ExtraKind::DeletedScene),
    ("interview", ExtraKind::Interview),
    ("scene", ExtraKind::Scene),
    ("sample", ExtraKind::Sample),
    ("short", ExtraKind::Short),
    ("featurette", ExtraKind::Featurette),
    ("clip", ExtraKind::Clip),
    ("blooper", ExtraKind::Blooper),
    ("other", ExtraKind::Other),
    ("extra", ExtraKind::Extra),
];

/// Video file stems that are extras on their own.
pub(crate) const EXTRAS_VIDEO_NAMES: &[(&str, ExtraKind)] = &[
    ("trailer", ExtraKind::Trailer),
    ("sample", ExtraKind::Sample),
];

/// Audio file stems that are extras on their own.
pub(crate) const EXTRAS_AUDIO_NAMES: &[(&str, ExtraKind)] = &[];

/// Artwork base names, compared without case, and the artwork role of each.
pub(crate) const ARTWORK_ROLES: &[(&str, ArtworkRole)] = &[
    ("cover", ArtworkRole::Poster),
    ("default", ArtworkRole::Poster),
    ("folder", ArtworkRole::Poster),
    ("backdrop", ArtworkRole::Backdrop),
    ("background", ArtworkRole::Backdrop),
    ("logo", ArtworkRole::Logo),
    ("thumb", ArtworkRole::Thumb),
];

/// NAS, OS and sync-tool entries, excluded wherever they appear. Matched as whole names.
pub(crate) const SYSTEM_JUNK_NAMES: &[&str] = &[
    "@eadir",
    "#recycle",
    "@recycle",
    "#snapshot",
    ".@__thumb",
    "$recycle.bin",
    "system volume information",
    "lost+found",
    "temporary items",
    "network trash folder",
    ".snapshot",
    ".snapshots",
    ".zfs",
    ".grab",
    "temprec",
    "tempsbe",
    "ps3_update",
    "ps3_vprm",
    ".wd_tv",
    "thumbs.db",
    "desktop.ini",
];

/// File endings of sync-tool temporaries, excluded wherever they appear.
pub(crate) const SYSTEM_JUNK_FILE_SUFFIXES: &[&str] = &[".bts", ".sync"];

/// File endings of downloads still in progress.
pub(crate) const PARTIAL_DOWNLOAD_FILE_SUFFIXES: &[&str] = &[".part", ".!qb", ".crdownload"];

/// Folders that other tools generate beside media. These folders are excluded. Content-like
/// folders (`Subs`, `extrafanart`, samples, disc structures) are classified instead.
pub(crate) const GENERATED_METADATA_DIRECTORY_NAMES: &[&str] = &["metadata", ".actors"];

/// The built-in exclusion groups, each switchable by overlays (X13).
pub(crate) const BUILT_IN_IGNORE_GROUPS: &[&str] = &[
    "hidden_entries",
    "system_junk",
    "partial_downloads",
    "generated_metadata",
];

/// File names that mark their folder as containing no media (I3).
pub(crate) const NO_MEDIA_MARKERS: &[&str] = &[".nomedia"];

/// Endings of generated preview-image folders.
pub(crate) const GENERATED_METADATA_DIRECTORY_SUFFIXES: &[&str] = &[".trickplay"];

/// Words naming a season folder, before or after its number (`Season 2`, `3. Staffel`).
pub(crate) const SEASON_WORDS: &[&str] = &[
    "saison",
    "staffel",
    "stagione",
    "sæson",
    "temporada",
    "kausi",
    "säsong",
    "seizoen",
    "seasong",
    "sezon",
    "sezona",
    "sezóna",
    "sezonul",
    "시즌",
    "シーズン",
    "сезон",
];

/// Words that mean a season only directly under a series folder; elsewhere they are
/// ordinary title words (`Series 7 - The Contenders`).
pub(crate) const SERIES_SEASON_WORDS: &[&str] = &["series", "serie", "série", "seria", "séria"];

/// Folder names that hold a show's specials, season 0.
pub(crate) const SPECIALS_FOLDER_NAMES: &[&str] = &["specials", "special"];

/// Endings that make a number an ordinal before a season word (`2nd Season`, `3. Staffel`).
pub(crate) const ORDINAL_SUFFIXES: &[&str] = &["st", "nd", "rd", "th", "."];

/// Video heights: `NNNxHHH` with one of these is a resolution, never a season and episode.
pub(crate) const VIDEO_HEIGHTS: &[&str] = &[
    "240", "288", "360", "480", "540", "576", "720", "768", "800", "900", "1080", "1200", "1440",
    "1600", "2160", "4320",
];

/// Resolution words and the height that each states. An explicit height in the same name
/// takes precedence over them, because `4K` also names a restoration.
pub(crate) const RESOLUTION_WORDS: &[(&str, u32)] = &[("4k", 2160), ("8k", 4320), ("uhd", 2160)];

/// Heights that are a resolution on their own, without a scan type (`1080`).
pub(crate) const BARE_RESOLUTIONS: &[&str] = &["720", "1080", "2160"];

/// Aspect ratios, never a season and episode.
pub(crate) const ASPECT_RATIOS: &[&str] = &[
    "16x9", "4x3", "16x10", "21x9", "1.85x1", "2.35x1", "2.39x1", "2.40x1",
];

/// Audio codec words that a channel layout (`5.1`, `2.0x2`) may follow.
pub(crate) const AUDIO_CODEC_WORDS: &[&str] = &[
    "aac", "ac3", "eac3", "dd", "ddp", "dd+", "dts", "dts-hd", "truehd", "flac", "opus", "atmos",
    "pcm", "lpcm",
];

/// Letters compared as their spelled-out forms (`Æon` matches `Aeon`).
pub(crate) const LETTER_FOLDS: &[(char, &str)] =
    &[('æ', "ae"), ('œ', "oe"), ('ß', "ss"), ('ø', "o")];

/// Articles a title's comparison key drops when they lead it (P4, M1): English and common
/// equivalents, compared case-folded and without accents. Never applied to displayed titles.
pub(crate) const LEADING_ARTICLES: &[&str] = &[
    "the", "a", "an", "le", "la", "les", "l", "un", "une", "der", "die", "das", "ein", "eine",
    "el", "los", "las", "una", "il", "lo", "gli",
];

/// Roman numerals a comparison key reads as their number when they stand as a whole word.
pub(crate) const ROMAN_NUMERALS: &[&str] = &[
    "i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x", "xi", "xii", "xiii", "xiv", "xv",
    "xvi", "xvii", "xviii", "xix", "xx",
];

/// Spelled numbers a comparison key reads as their number.
pub(crate) const SPELLED_NUMBERS: &[&str] = &[
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
    "twenty",
];

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
];

/// The generic NFO name of each role (R19, S9), read as `nfo_names`.
pub(crate) const NFO_OUTPUT_NAMES: &[(NfoRole, &str)] = &[
    (NfoRole::Movie, "movie"),
    (NfoRole::TvShow, "tvshow"),
    (NfoRole::Season, "season"),
];

/// The stem of a theme song or theme video (R19), read as `extras_audio_names`.
pub(crate) const THEME_OUTPUT_STEM: &str = "theme";

/// The word of a season folder (R2), read as `season_words`.
pub(crate) const SEASON_FOLDER_WORD: &str = "Season";

/// The word of a part number (R1), read by the part markers.
pub(crate) const PART_OUTPUT_WORD: &str = "part";

/// The part words, compared without case (M11). They contain `PART_OUTPUT_WORD`.
pub(crate) const PART_WORDS: &[&str] = &["cd", "dvd", "disc", "disk", PART_OUTPUT_WORD, "pt"];

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

/// Release words after a title (X19): the fact, its normalized value, and the spellings
/// that state it. In a spelling, a space, `.`, `-` or `_` matches any one of them or none,
/// so `web-dl` also matches `WEB.DL` and `WEBDL`.
pub(crate) const RELEASE_WORDS: &[(ReleaseFact, &str, &[&str])] = &[
    (
        ReleaseFact::Source,
        "uhd_bluray",
        &[
            "uhd bluray",
            "uhd blu-ray",
            "ultrahd bluray",
            "uhd bdremux",
            "uhd bdrip",
        ],
    ),
    (
        ReleaseFact::Source,
        "bluray",
        &[
            "bluray", "blu-ray", "bdrip", "brrip", "bdremux", "bdscr", "bd25", "bd50", "bdr", "bd",
        ],
    ),
    (ReleaseFact::Source, "hd_dvd", &["hd-dvd", "hddvdrip"]),
    (
        ReleaseFact::Source,
        "dvd",
        &["dvdscr", "dvdrip", "dvd5", "dvd9", "dvd-r", "dvd"],
    ),
    (
        ReleaseFact::Source,
        "web",
        &["web-dl", "webrip", "web-rip", "web-hd", "web"],
    ),
    (ReleaseFact::Source, "ultra_hdtv", &["uhdtv"]),
    (ReleaseFact::Source, "analog_hdtv", &["ahdtv"]),
    (ReleaseFact::Source, "hdtv", &["hdtv", "hdtvrip"]),
    (
        ReleaseFact::Source,
        "digital_tv",
        &["pdtv", "dvbrip", "dvb"],
    ),
    (ReleaseFact::Source, "tv", &["tvrip", "sdtv"]),
    (
        ReleaseFact::Source,
        "satellite",
        &["dsrip", "dsr", "satrip"],
    ),
    (ReleaseFact::Source, "vhs", &["vhsrip", "vhs"]),
    (ReleaseFact::Source, "hd_camera", &["hdcam"]),
    (ReleaseFact::Source, "camera", &["camrip", "cam"]),
    (ReleaseFact::Source, "hd_telesync", &["hdts", "hd-ts"]),
    (ReleaseFact::Source, "telesync", &["telesync", "ts"]),
    (ReleaseFact::Source, "hd_telecine", &["hdtc", "hd-tc"]),
    (ReleaseFact::Source, "telecine", &["telecine", "tc"]),
    (ReleaseFact::Source, "screener", &["screener", "scr"]),
    (ReleaseFact::Source, "workprint", &["workprint"]),
    (ReleaseFact::Source, "laserdisc", &["laserdisc", "ldrip"]),
    (ReleaseFact::Source, "video_on_demand", &["vodrip", "vod"]),
    (
        ReleaseFact::VideoCodec,
        "h264",
        &["x264", "h264", "h.264", "avc"],
    ),
    (
        ReleaseFact::VideoCodec,
        "h265",
        &["x265", "h265", "h.265", "hevc"],
    ),
    (ReleaseFact::VideoCodec, "xvid", &["xvid"]),
    (ReleaseFact::VideoCodec, "divx", &["divx"]),
    (ReleaseFact::VideoCodec, "av1", &["av1"]),
    (ReleaseFact::VideoCodec, "vp9", &["vp9"]),
    (ReleaseFact::VideoCodec, "vc1", &["vc-1"]),
    (ReleaseFact::VideoCodec, "mpeg2", &["mpeg-2"]),
    (
        ReleaseFact::BitDepth,
        "10",
        &["10bit", "10-bit", "hi10p", "hi10"],
    ),
    (ReleaseFact::BitDepth, "8", &["8bit", "8-bit"]),
    (ReleaseFact::BitDepth, "12", &["12bit", "12-bit"]),
    (ReleaseFact::Hdr, "hdr10_plus", &["hdr10+", "hdr10plus"]),
    (ReleaseFact::Hdr, "hdr10", &["hdr10"]),
    (
        ReleaseFact::Hdr,
        "dolby_vision",
        &["dolby vision", "dovi", "dv"],
    ),
    (ReleaseFact::Hdr, "hlg", &["hlg"]),
    (ReleaseFact::Hdr, "hdr", &["hdr"]),
    (
        ReleaseFact::AudioCodec,
        "dolby_digital_plus",
        &["dolby digital plus", "e-ac3", "ddp", "dd+"],
    ),
    (
        ReleaseFact::AudioCodec,
        "dolby_digital",
        &["dolby digital", "ac3", "dd"],
    ),
    (ReleaseFact::AudioCodec, "dolby_truehd", &["truehd"]),
    (ReleaseFact::AudioCodec, "dolby_atmos", &["atmos"]),
    (ReleaseFact::AudioCodec, "dts_x", &["dts:x", "dts-x"]),
    (
        ReleaseFact::AudioCodec,
        "dts_hd",
        &["dts-hd ma", "dts-hd", "dts-ma"],
    ),
    (ReleaseFact::AudioCodec, "dts", &["dts"]),
    (ReleaseFact::AudioCodec, "aac", &["he-aac", "aac-lc", "aac"]),
    (ReleaseFact::AudioCodec, "flac", &["flac"]),
    (ReleaseFact::AudioCodec, "mp3", &["mp3"]),
    (ReleaseFact::AudioCodec, "mp2", &["mp2"]),
    (ReleaseFact::AudioCodec, "opus", &["opus"]),
    (ReleaseFact::AudioCodec, "vorbis", &["vorbis"]),
    (ReleaseFact::AudioCodec, "lpcm", &["lpcm"]),
    (ReleaseFact::AudioCodec, "pcm", &["pcm"]),
    (
        ReleaseFact::AudioLanguage,
        "multi",
        &["multi-audio", "multilang", "multi"],
    ),
    (ReleaseFact::AudioLanguage, "dual", &["dual-audio", "dual"]),
    (
        ReleaseFact::SubtitleLanguage,
        "multi",
        &["multi-subs", "multi-sub", "multisubs", "multisub"],
    ),
    (
        ReleaseFact::SubtitleLanguage,
        "subbed",
        &["hardsubbed", "hardsub", "subbed"],
    ),
    (ReleaseFact::SubtitleLanguage, "fr", &["vostfr"]),
    (ReleaseFact::Revision, "proper", &["proper"]),
    (ReleaseFact::Revision, "repack", &["repack"]),
    (ReleaseFact::Revision, "real", &["real"]),
    (ReleaseFact::Revision, "rerip", &["rerip"]),
    (
        ReleaseFact::StreamingService,
        "netflix",
        &["netflixuhdrip", "netflixuhd", "netflixhd", "netflix", "nf"],
    ),
    (
        ReleaseFact::StreamingService,
        "amazon_prime",
        &["amazonuhd", "amazonhd", "amazon", "amzn"],
    ),
    (
        ReleaseFact::StreamingService,
        "disney_plus",
        &["disney+", "dsnp", "dsny"],
    ),
    (ReleaseFact::StreamingService, "hbo_max", &["hmax"]),
    (ReleaseFact::StreamingService, "hbo", &["hbo"]),
    (ReleaseFact::StreamingService, "apple_tv_plus", &["atvp"]),
    (
        ReleaseFact::StreamingService,
        "itunes",
        &["itunesuhd", "itunesHD", "hditunes", "itunes"],
    ),
    (ReleaseFact::StreamingService, "hulu", &["hulu"]),
    (ReleaseFact::StreamingService, "peacock", &["pcok"]),
    (ReleaseFact::StreamingService, "paramount_plus", &["pmtp"]),
    (
        ReleaseFact::StreamingService,
        "crunchyroll",
        &["crunchyroll"],
    ),
    (ReleaseFact::StreamingService, "starz", &["stz"]),
    (ReleaseFact::StreamingService, "vudu", &["vudu"]),
    (ReleaseFact::StreamingService, "crave", &["crav"]),
    (ReleaseFact::StreamingService, "dc_universe", &["dcu"]),
    (ReleaseFact::StreamingService, "viki", &["viki"]),
    (ReleaseFact::Remux, "remux", &["remux"]),
];

/// Channel spellings that are not `N.N`, and the layout they state.
pub(crate) const AUDIO_CHANNEL_WORDS: &[(&str, &str)] = &[
    ("2ch", "2.0"),
    ("6ch", "5.1"),
    ("8ch", "7.1"),
    ("stereo", "2.0"),
    ("mono", "1.0"),
];

/// Trailing bracketed tags that index sites add after a release name: not a release group.
pub(crate) const RELEASE_SITE_TAGS: &[&str] =
    &["ettv", "eztv", "rarbg", "rartv", "publichd", "vtv"];

/// File names that tracker and release sites add beside a release (X22), compared ignoring
/// case.
pub(crate) const RELEASE_JUNK_NAMES: &[&str] = &[
    "rarbg.txt",
    "rarbg.com.txt",
    "rarbg_do_not_mirror.exe",
    "torrent downloaded from.txt",
];

/// Stem beginnings of release notes and ads (`Downloaded from ....txt`), compared ignoring
/// case. Media files never count.
pub(crate) const RELEASE_JUNK_PREFIXES: &[&str] = &["downloaded from", "torrent downloaded from"];

/// Extensions of shortcuts and executables that releases carry.
pub(crate) const RELEASE_JUNK_EXTENSIONS: &[&str] = &["url", "lnk", "exe", "bat"];

/// Image stems of release screenshots and bitrate graphs, with an optional number after
/// them (`screen1.jpg`, `screenshot_02.png`).
pub(crate) const RELEASE_JUNK_IMAGES: &[&str] = &[
    "screen",
    "screens",
    "screenshot",
    "screenshots",
    "screencap",
    "bitrate",
];

/// Diagnostic message catalogues by BCP 47 tag (R13). English is the fallback.
pub(crate) const MESSAGE_CATALOGS: &[(&str, &str)] =
    &[("en", include_str!("../data/messages/en.yaml"))];
