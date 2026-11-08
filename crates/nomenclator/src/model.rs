//! Provides the facts that the library returns, each with the evidence behind it.

use serde::{Deserialize, Serialize};

/// Byte range `[start, end)` into the original UTF-8 name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Span(pub usize, pub usize);

/// The naming rule that produced a fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    /// The whole cleaned name, with nothing after the title.
    WholeName,
    /// The text before the year (M2).
    TextBeforeYear,
    /// The text before the first release word (M4).
    TextBeforeReleaseWords,
    /// A year in brackets or parentheses (M3).
    BracketedYear,
    /// An unbracketed year followed by release words or nothing (M3).
    BareYear,
    /// A language token in a sidecar name (S4).
    LanguageToken,
    /// Sidecar tokens that are neither language nor flag.
    SidecarTitleTokens,
    /// An `{edition-X}` or `[edition-X]` tag (M9).
    EditionTag,
    /// Edition words after the title (M9).
    EditionWords,
    /// A 3D layout word (M12).
    Format3dWord,
    /// A file inside a folder named from the extras vocabulary, owned by a title.
    ExtrasFolder,
    /// A separator, an extras word and an optional index ending the stem.
    ExtrasSuffix,
    /// A whole stem that is an extra on its own (`trailer`, `sample`, `theme`).
    ExtrasName,
    /// `Season 2`, `S02`, `3. Staffel` in a folder or series name.
    SeasonMarker,
    /// A specials folder, season 0 (T9).
    SpecialsFolder,
    /// A folder named only a number, directly under a series folder.
    BareNumberSeasonFolder,
    /// The season of the folder that contains the entry.
    SeasonFolder,
    /// `S01E02`, `1x02`, `E02` and bare episode numbers.
    EpisodeToken,
    /// `301` read as season 3, episode 1 because the season folder agrees.
    CompactNumber,
    /// An air date in the name (T13).
    Date,
    /// A release word, release group or checksum after the title (X19).
    ReleaseWord,
    /// A special marker after the series title: `OVA`, `OAD`, `ONA`, or a decimal number
    /// such as `12.5` (X20).
    SpecialMarker,
}

/// The source of a fact: the rule and the part of the name that the rule read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Evidence {
    /// The rule that produced the fact.
    pub rule: Rule,
    /// The text the rule read.
    pub span: Span,
}

/// The outcome of one fact. A missing fact is `Unknown`, never an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Finding<T> {
    /// One value, with its evidence when a name span supports it.
    Resolved {
        /// The fact.
        value: T,
        /// The rule and span behind the value.
        #[serde(skip_serializing_if = "Option::is_none")]
        evidence: Option<Evidence>,
    },
    /// Several values compete and none is chosen (X6).
    Ambiguous {
        /// The competing values.
        candidates: Vec<T>,
        /// Why they compete.
        reason: &'static str,
    },
    /// No evidence for the fact.
    Unknown,
}

/// The kind of entry that a path names: a file or a folder. A file name ends in its extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// A file.
    File,
    /// A folder.
    Directory,
}

/// The role of a sidecar file beside its video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidecarKind {
    /// A subtitle track.
    Subtitle,
    /// An external audio track.
    Audio,
    /// A still image of the video.
    Image,
}

/// A track flag read from a sidecar name, in canonical order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidecarFlag {
    /// The default track.
    Default,
    /// A forced track, for foreign dialogue only.
    Forced,
    /// A track for the deaf and hard of hearing (`sdh`, `cc`, `hi`).
    HearingImpaired,
}

/// A 3D layout named in a file or folder name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Format3d {
    /// `sbs`.
    SideBySide,
    /// `hsbs`.
    HalfSideBySide,
    /// `fsbs`.
    FullSideBySide,
    /// `tab`.
    TopAndBottom,
    /// `htab`.
    HalfTopAndBottom,
    /// `ftab`.
    FullTopAndBottom,
    /// `mvc`, frame-packed multiview.
    Mvc,
    /// `3d` without a layout word.
    Unspecified,
}

/// The kind of extra that a file or folder is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtraKind {
    /// A trailer.
    Trailer,
    /// Behind-the-scenes footage.
    BehindTheScenes,
    /// A deleted scene.
    DeletedScene,
    /// A featurette.
    Featurette,
    /// An interview.
    Interview,
    /// A scene from the feature.
    Scene,
    /// A short film.
    Short,
    /// A clip.
    Clip,
    /// A blooper.
    Blooper,
    /// A sample of the feature (E6).
    Sample,
    /// A theme song, an audio file.
    ThemeSong,
    /// A theme video.
    ThemeVideo,
    /// An extra from an `other` folder or suffix.
    Other,
    /// An extra of no stated kind, from an `extras` folder or suffix.
    Extra,
}

/// The item that a local NFO file describes, by its name (S9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NfoRole {
    /// The item of the video it is named after.
    Video,
    /// The movie of its folder, or of the stack it is named after.
    Movie,
    /// The series of its folder.
    #[serde(rename = "tvshow")]
    TvShow,
    /// The season of its folder.
    Season,
}

/// The role a local artwork image plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtworkRole {
    /// `poster`, `cover`, `default`, `folder`.
    Poster,
    /// `backdrop`, `fanart`, `background`.
    Backdrop,
    /// `banner`.
    Banner,
    /// `logo`, `clearlogo`.
    Logo,
    /// `thumb`, `landscape`.
    Thumb,
    /// A video's still, `<video stem>-thumb` (S6).
    EpisodeStill,
    /// An image whose name gives no role.
    Other,
}

/// The ambiguity reason of an item with two values of one provider's ID.
pub(crate) const CONFLICTING_PROVIDER_ID: &str = "conflicting_provider_id";
/// The ambiguity reason of parts with a gap (M11).
pub(crate) const MISSING_PART: &str = "missing_part";
/// The ambiguity reason of parts with a repeated number (M11).
pub(crate) const DUPLICATE_PART: &str = "duplicate_part";

/// Returns `true` if any finding of `findings` is ambiguous, such as a provider with two
/// IDs.
pub(crate) fn has_ambiguous_value<K, T>(
    findings: &std::collections::BTreeMap<K, Finding<T>>,
) -> bool {
    findings
        .values()
        .any(|finding| matches!(finding, Finding::Ambiguous { .. }))
}

/// Returns the resolved value of each finding of `findings`, by key. Leaves out the
/// unknown and ambiguous findings.
#[must_use]
pub fn resolved_values<K: Clone + Ord, T: Clone>(
    findings: &std::collections::BTreeMap<K, Finding<T>>,
) -> std::collections::BTreeMap<K, T> {
    findings
        .iter()
        .filter_map(|(key, finding)| Some((key.clone(), finding.resolved_value()?.clone())))
        .collect()
}

impl<T> Finding<T> {
    /// Returns `true` if the fact has one value.
    pub fn is_resolved(&self) -> bool {
        matches!(self, Finding::Resolved { .. })
    }

    /// Returns the value of a resolved fact.
    /// Returns `None` if the fact is not resolved.
    pub fn resolved_value(&self) -> Option<&T> {
        match self {
            Finding::Resolved { value, .. } => Some(value),
            _ => None,
        }
    }
}
