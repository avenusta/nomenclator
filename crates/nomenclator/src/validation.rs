//! Validates an interpretation under an output profile and reports diagnostics (R7-R9).
//! Validation renames nothing. A profile violation includes the path that the profile
//! would give the file.

use crate::model::AmbiguityReason;
use crate::model::serde_name;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::inputs::{
    EpisodeOrder, InputDiagnostic, InputDiagnosticReason, InputKind, ReferenceMetadata,
    SeriesInputs, normalized, series_matches,
};
use crate::interpret::{Artwork, Episode, Extra, Interpretation, Item, Movie, Series, Version};
use crate::lexical;
use crate::model::{EntryKind, ExtraKind, Finding};
use crate::parsers::file_types::{file_name, parent_folder};
use crate::parsers::movie_name::MovieName;
use crate::rendering::episode_token;
use crate::rendering::targets::file_targets;
use crate::rendering::{AssetItemKind, AssetRole};
use crate::rendering::{BlockReason, OutputProfile};
use crate::rules::{Rules, RulesIdentity};

/// The result of `validate`: the diagnostics that it found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Validation {
    /// The diagnostics, sorted by first path, then rule, then reason.
    pub diagnostics: Vec<Diagnostic>,
    /// The reference entries that no item matched (X18).
    pub input_diagnostics: Vec<InputDiagnostic>,
    /// The counts of the diagnostics per rule and severity (R13).
    pub summary: ValidationSummary,
    /// The rules this result was produced under.
    pub rules: RulesIdentity,
}

/// A problem, with the rule that found it and the entries involved.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Diagnostic {
    /// The rule that found the problem.
    pub rule: DiagnosticRule,
    /// The rule's severity, or the profile's override of that severity.
    pub severity: Severity,
    /// The entries involved.
    pub paths: Vec<String>,
    /// The interpreter's ambiguity reason, the provider, the block reason, or the fact
    /// that differs in a folder mismatch.
    pub reason: Option<DiagnosticReason>,
    /// The path that the profile gives the file of a profile violation.
    pub proposed_path: Option<String>,
    /// The rule's stable code (R13).
    pub code: &'static str,
    /// The message (R13), from the template of the rule in `data/messages/en.yaml`.
    pub message: String,
}

/// The counts of a validation's diagnostics (R13). It reports counts, never a score.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ValidationSummary {
    /// The diagnostics at the `error` severity.
    pub errors: usize,
    /// The diagnostics at the `warning` severity.
    pub warnings: usize,
    /// The diagnostics at the `info` severity.
    pub infos: usize,
    /// The diagnostics that a suppression left out.
    pub suppressed: usize,
    /// The count per rule and severity, the most severe first, then in rule order.
    pub rules: Vec<RuleCount>,
}

/// The number of diagnostics of one rule at one severity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleCount {
    /// The rule.
    pub rule: DiagnosticRule,
    /// The rule's stable code.
    pub code: &'static str,
    /// The severity.
    pub severity: Severity,
    /// The number of diagnostics.
    pub count: usize,
}

/// A validation rule, identified by its ID (R7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticRule {
    /// An item whose identity the interpreter could not resolve, with the reason.
    AmbiguousIdentity,
    /// Names of one item that contain different IDs of one provider, with the provider.
    ConflictingProviderIds,
    /// Series content in a Movies library's season folder (M15).
    MisplacedSeries,
    /// An extra that no item owns.
    OrphanExtra,
    /// An unclaimed subtitle or audio file.
    OrphanSidecar,
    /// An unclaimed image.
    OrphanArtwork,
    /// Any other unclaimed file.
    UnclaimedFile,
    /// A gap in a stacked item's parts.
    MissingPart,
    /// Two files that claim the same part of a stacked item.
    DuplicatePart,
    /// Versions of one item whose labels and editions are equal ignoring case.
    ClashingVersionLabels,
    /// A missing fact that the profile requires (R3).
    InsufficientFacts,
    /// A name under the profile that is a reserved name or has a component that is too
    /// long.
    InvalidNameUnderProfile,
    /// A file whose name states a fact differently from its folder (R8).
    FolderMismatch,
    /// Episode items of one series whose episode ranges overlap but are not equal (R8).
    OverlappingEpisodes,
    /// A movie without a year (R9).
    MissingYear,
    /// A series without a year (R9).
    MissingSeriesYear,
    /// A movie or series without a provider ID (R9).
    MissingProviderId,
    /// An episode without a title (R9).
    MissingEpisodeTitle,
    /// A reference episode that is missing from the library, with its `SxxEyy` (X18).
    MissingEpisode,
    /// An episode that the reference does not list, with its `SxxEyy` (X18).
    UnexpectedEpisode,
    /// A movie whose year differs from the reference, with the reference year (X18).
    ReferenceYearMismatch,
    /// One work in two folders: the same provider ID, or the same title and year (R11).
    DuplicateWork,
    /// Two episode items of one series with the same season and episode (R11).
    DuplicateEpisode,
    /// An asset role that the profile requires and an item lacks, with the role (R12).
    MissingAsset,
    /// A subtitle that the profile wants and a feature video lacks, with the language and
    /// flags (R12).
    MissingSubtitle,
    /// An unclaimed file that is release junk: a tracker note, an ad, a shortcut or a
    /// screenshot (X22).
    ReleaseJunk,
    /// A file whose name differs from its name under the profile, with the proposed path.
    ProfileViolation,
}

impl DiagnosticRule {
    /// Every rule.
    pub const ALL: &[DiagnosticRule] = &[
        DiagnosticRule::AmbiguousIdentity,
        DiagnosticRule::ConflictingProviderIds,
        DiagnosticRule::MisplacedSeries,
        DiagnosticRule::OrphanExtra,
        DiagnosticRule::OrphanSidecar,
        DiagnosticRule::OrphanArtwork,
        DiagnosticRule::UnclaimedFile,
        DiagnosticRule::MissingPart,
        DiagnosticRule::DuplicatePart,
        DiagnosticRule::ClashingVersionLabels,
        DiagnosticRule::InsufficientFacts,
        DiagnosticRule::InvalidNameUnderProfile,
        DiagnosticRule::FolderMismatch,
        DiagnosticRule::OverlappingEpisodes,
        DiagnosticRule::MissingYear,
        DiagnosticRule::MissingSeriesYear,
        DiagnosticRule::MissingProviderId,
        DiagnosticRule::MissingEpisodeTitle,
        DiagnosticRule::MissingEpisode,
        DiagnosticRule::UnexpectedEpisode,
        DiagnosticRule::ReferenceYearMismatch,
        DiagnosticRule::DuplicateWork,
        DiagnosticRule::DuplicateEpisode,
        DiagnosticRule::MissingAsset,
        DiagnosticRule::MissingSubtitle,
        DiagnosticRule::ReleaseJunk,
        DiagnosticRule::ProfileViolation,
    ];

    /// Returns the rule's stable code (R13). The first digit groups the rules: 1 identity,
    /// 2 parts and versions, 3 files that no item claims, 4 names under the profile,
    /// 5 missing facts and assets, 6 the reference.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            DiagnosticRule::AmbiguousIdentity => "N101",
            DiagnosticRule::ConflictingProviderIds => "N102",
            DiagnosticRule::MisplacedSeries => "N103",
            DiagnosticRule::OrphanExtra => "N301",
            DiagnosticRule::OrphanSidecar => "N302",
            DiagnosticRule::OrphanArtwork => "N303",
            DiagnosticRule::UnclaimedFile => "N304",
            DiagnosticRule::MissingPart => "N201",
            DiagnosticRule::DuplicatePart => "N202",
            DiagnosticRule::ClashingVersionLabels => "N203",
            DiagnosticRule::InsufficientFacts => "N401",
            DiagnosticRule::InvalidNameUnderProfile => "N402",
            DiagnosticRule::FolderMismatch => "N107",
            DiagnosticRule::OverlappingEpisodes => "N106",
            DiagnosticRule::MissingYear => "N501",
            DiagnosticRule::MissingSeriesYear => "N502",
            DiagnosticRule::MissingProviderId => "N503",
            DiagnosticRule::MissingEpisodeTitle => "N504",
            DiagnosticRule::MissingEpisode => "N601",
            DiagnosticRule::UnexpectedEpisode => "N602",
            DiagnosticRule::ReferenceYearMismatch => "N603",
            DiagnosticRule::DuplicateWork => "N104",
            DiagnosticRule::DuplicateEpisode => "N105",
            DiagnosticRule::MissingAsset => "N505",
            DiagnosticRule::MissingSubtitle => "N506",
            DiagnosticRule::ReleaseJunk => "N305",
            DiagnosticRule::ProfileViolation => "N403",
        }
    }

    /// Returns the severity that the rule reports at if the profile does not override it
    /// (R7).
    #[must_use]
    pub fn default_severity(self) -> Severity {
        match self {
            DiagnosticRule::InsufficientFacts
            | DiagnosticRule::InvalidNameUnderProfile
            | DiagnosticRule::AmbiguousIdentity
            | DiagnosticRule::ConflictingProviderIds
            | DiagnosticRule::MisplacedSeries
            | DiagnosticRule::MissingPart
            | DiagnosticRule::DuplicatePart => Severity::Error,
            DiagnosticRule::MissingProviderId
            | DiagnosticRule::MissingEpisodeTitle
            | DiagnosticRule::MissingEpisode
            | DiagnosticRule::ReleaseJunk => Severity::Info,
            _ => Severity::Warning,
        }
    }

    /// Returns `true` if the rule reports an optional fact that a name leaves out (R9).
    fn is_missing_fact(self) -> bool {
        matches!(
            self,
            DiagnosticRule::MissingYear
                | DiagnosticRule::MissingSeriesYear
                | DiagnosticRule::MissingProviderId
                | DiagnosticRule::MissingEpisodeTitle
        )
    }
}

/// The severity of a diagnostic. Variants are ordered from most to least serious.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// An entry that cannot be named safely.
    Error,
    /// A name that does not follow the profile, or a likely error.
    Warning,
    /// A missing optional fact.
    Info,
}

/// A profile's override of a rule's severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeverityOverride {
    /// The error severity.
    Error,
    /// The warning severity.
    Warning,
    /// The info severity.
    Info,
    /// No report: the rule is turned off.
    Off,
}

/// The detail of a diagnostic beyond its rule.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(untagged)]
pub enum DiagnosticReason {
    /// The reason that the profile cannot name the entry.
    Blocked(BlockReason),
    /// The fact that differs in a folder mismatch (R8).
    Mismatch(MismatchedFact),
    /// The interpreter's ambiguity reason.
    Ambiguity(AmbiguityReason),
    /// The provider whose IDs conflict, a reference year or episode, an asset role, or a
    /// subtitle language.
    Named(String),
}

/// A fact that a file states differently from its folder (R8). Variants are in reporting
/// order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MismatchedFact {
    /// A movie's title, compared with its owning folder.
    Title,
    /// A movie's year, compared with its owning folder.
    Year,
    /// A movie's `{edition-X}` tag, compared with its owning folder.
    Edition,
    /// An episode's series title, compared with the series folder.
    SeriesTitle,
    /// An episode's series year, compared with the series folder.
    SeriesYear,
    /// An episode's season, compared with the season folder.
    Season,
}

impl Rules {
    /// Validates the names of an interpreted library under `profile`.
    pub fn validate(&self, interpretation: &Interpretation, profile: &OutputProfile) -> Validation {
        self.validate_with(interpretation, profile, &ReferenceMetadata::default())
    }

    /// Validates like [`Rules::validate`] and also compares the interpretation with the
    /// caller's reference metadata: movie years and episode lists (X18). The reference
    /// never changes the interpretation.
    pub fn validate_with(
        &self,
        interpretation: &Interpretation,
        profile: &OutputProfile,
        reference: &ReferenceMetadata,
    ) -> Validation {
        let rules = self;
        let mut diagnostics = Vec::new();
        let input_diagnostics =
            diagnose_reference(rules, interpretation, reference, &mut diagnostics);
        for item in &interpretation.items {
            diagnose_item(rules, item, &mut diagnostics);
        }
        diagnose_series(rules, interpretation, &mut diagnostics);
        diagnose_overlapping_episodes(interpretation, &mut diagnostics);
        diagnose_duplicate_works(rules, interpretation, &mut diagnostics);
        diagnose_coverage(interpretation, profile, &mut diagnostics);
        diagnose_duplicate_episodes(interpretation, &mut diagnostics);
        diagnose_unclaimed_files(rules, interpretation, &mut diagnostics);
        for target in file_targets(rules, interpretation, profile, None) {
            match target.destination {
                Ok(destination) if destination != target.path => diagnostics.push(Diagnostic {
                    proposed_path: Some(destination),
                    ..diagnostic(DiagnosticRule::ProfileViolation, vec![target.path], None)
                }),
                Ok(_) => {}
                Err(reasons) => {
                    for reason in reasons {
                        let rule = if reason.is_missing_fact() {
                            DiagnosticRule::InsufficientFacts
                        } else if reason.is_invalid_name() {
                            DiagnosticRule::InvalidNameUnderProfile
                        } else {
                            continue;
                        };
                        let reason = Some(DiagnosticReason::Blocked(reason));
                        diagnostics.push(diagnostic(rule, vec![target.path.clone()], reason));
                    }
                }
            }
        }
        drop_missing_facts_of_errors(&mut diagnostics);
        apply_severity_overrides(&mut diagnostics, profile);
        diagnostics.sort_by(|left, right| {
            (left.paths.first(), left.rule, left).cmp(&(right.paths.first(), right.rule, right))
        });
        diagnostics.dedup();
        let before = diagnostics.len();
        diagnostics.retain(|diagnostic| !is_suppressed(diagnostic, profile));
        let suppressed = before - diagnostics.len();
        for diagnostic in &mut diagnostics {
            diagnostic.message = render_message(diagnostic);
        }
        Validation {
            summary: summarize(&diagnostics, suppressed),
            diagnostics,
            input_diagnostics,
            rules: rules.identity().clone(),
        }
    }
}

/// Compares movie years and episode lists with the reference (X18). Skips episodes in an
/// order other than aired. Reports no missing episodes for a series that contains such an
/// episode. Returns the reference entries that no item matched.
fn diagnose_reference(
    rules: &Rules,
    interpretation: &Interpretation,
    reference: &ReferenceMetadata,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<InputDiagnostic> {
    let mut unused = Vec::new();
    let not_applicable = |input: InputKind, path: &str| InputDiagnostic {
        input,
        path: normalized(path),
        reason: InputDiagnosticReason::NotApplicable,
        fact: None,
    };
    for movie_reference in &reference.movies {
        let mut taken = false;
        for item in &interpretation.items {
            let Item::Movie(movie) = item else { continue };
            let video_paths: Vec<&String> = movie
                .versions
                .iter()
                .flat_map(|version| &version.paths)
                .collect();
            let by_path = movie_reference.path.as_deref().is_some_and(|path| {
                let path = normalized(path);
                movie.path.as_deref() == Some(path.as_str())
                    || video_paths.iter().any(|video| **video == path)
            });
            let by_provider = movie_reference.provider_ids.iter().any(|(provider, id)| {
                movie
                    .provider_ids
                    .get(&lexical::vocabulary_form(provider))
                    .and_then(Finding::resolved_value)
                    == Some(id)
            });
            if !(by_path || by_provider) {
                continue;
            }
            taken = true;
            if let Some(year) = movie.year
                && year != movie_reference.year
            {
                diagnostics.push(diagnostic(
                    DiagnosticRule::ReferenceYearMismatch,
                    video_paths.into_iter().cloned().collect(),
                    named(&movie_reference.year.to_string()),
                ));
            }
        }
        if !taken {
            let path = movie_reference.path.clone().unwrap_or_else(|| {
                let ids: Vec<String> = movie_reference
                    .provider_ids
                    .iter()
                    .map(|(provider, id)| format!("{provider}-{id}"))
                    .collect();
                ids.join(", ")
            });
            unused.push(not_applicable(InputKind::ReferenceMovie, &path));
        }
    }
    for series_reference in &reference.series {
        let wanted = SeriesInputs {
            series: series_reference.series.clone(),
            ..SeriesInputs::default()
        };
        let series = interpretation.items.iter().find_map(|item| match item {
            Item::Series(series)
                if series_matches(
                    rules,
                    &wanted,
                    series.path.as_deref(),
                    series.title.as_deref(),
                ) =>
            {
                Some(series)
            }
            _ => None,
        });
        let Some(series) = series else {
            unused.push(not_applicable(
                InputKind::ReferenceSeries,
                &series_reference.series,
            ));
            continue;
        };
        let expected: BTreeSet<(u16, u32)> = series_reference.episodes.iter().copied().collect();
        let mut present: BTreeSet<(u16, u32)> = BTreeSet::new();
        let mut holds_other_orders = false;
        for item in &interpretation.items {
            let Item::Episode(episode) = item else {
                continue;
            };
            if episode.series_path != series.path {
                continue;
            }
            if !matches!(episode.order, None | Some(EpisodeOrder::Aired)) {
                holds_other_orders = true;
                continue;
            }
            let (Some(&season), Some(numbers)) =
                (episode.season.resolved_value(), episode.episodes.as_ref())
            else {
                continue;
            };
            for &number in numbers {
                present.insert((season, number));
                if !expected.contains(&(season, number)) {
                    let videos = episode
                        .versions
                        .iter()
                        .flat_map(|version| version.paths.iter().cloned())
                        .collect();
                    diagnostics.push(diagnostic(
                        DiagnosticRule::UnexpectedEpisode,
                        videos,
                        named(&episode_token(season, &[number])),
                    ));
                }
            }
        }
        let series_path = series
            .path
            .clone()
            .unwrap_or_else(|| normalized(&series_reference.series));
        let missing = expected
            .difference(&present)
            .filter(|_| !holds_other_orders);
        for &(season, number) in missing {
            diagnostics.push(diagnostic(
                DiagnosticRule::MissingEpisode,
                vec![series_path.clone()],
                named(&episode_token(season, &[number])),
            ));
        }
    }
    unused.sort();
    unused
}

/// Returns `true` if a suppression of the profile matches the diagnostic: its rule, or
/// every rule, and a glob that matches one of its paths (R13).
fn is_suppressed(diagnostic: &Diagnostic, profile: &OutputProfile) -> bool {
    profile.suppressions.iter().any(|suppression| {
        suppression.rule.is_none_or(|rule| rule == diagnostic.rule)
            && suppression.paths.iter().any(|glob| {
                diagnostic
                    .paths
                    .iter()
                    .any(|path| crate::ignore::glob_matches(glob, path))
            })
    })
}

fn summarize(diagnostics: &[Diagnostic], suppressed: usize) -> ValidationSummary {
    let mut counts: BTreeMap<(Severity, DiagnosticRule), usize> = BTreeMap::new();
    for diagnostic in diagnostics {
        *counts
            .entry((diagnostic.severity, diagnostic.rule))
            .or_default() += 1;
    }
    let total = |severity: Severity| {
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == severity)
            .count()
    };
    ValidationSummary {
        errors: total(Severity::Error),
        warnings: total(Severity::Warning),
        infos: total(Severity::Info),
        suppressed,
        rules: counts
            .into_iter()
            .map(|((severity, rule), count)| RuleCount {
                rule,
                code: rule.code(),
                severity,
                count,
            })
            .collect(),
    }
}

/// A message catalogue: per rule name, the template and the templates per reason.
type MessageCatalog = BTreeMap<String, MessageTemplate>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageTemplate {
    text: String,
    #[serde(default)]
    reasons: BTreeMap<String, String>,
}

/// The diagnostic messages (R13).
static MESSAGES: std::sync::LazyLock<MessageCatalog> = std::sync::LazyLock::new(|| {
    crate::parse_document(crate::data::MESSAGES).expect("the message catalogue is valid YAML")
});

/// Renders the message of `diagnostic` from its rule's template.
fn render_message(diagnostic: &Diagnostic) -> String {
    let rule = serde_name(diagnostic.rule);
    let reason = diagnostic
        .reason
        .as_ref()
        .map(reason_text)
        .unwrap_or_default();
    let Some(template) = MESSAGES.get(&rule) else {
        return String::new();
    };
    let text = template.reasons.get(&reason).unwrap_or(&template.text);
    let paths: Vec<String> = diagnostic
        .paths
        .iter()
        .map(|path| format!("`{path}`"))
        .collect();
    text.replace(
        "{path}",
        diagnostic.paths.first().map_or("", String::as_str),
    )
    .replace("{paths}", &paths.join(", "))
    .replace("{reason}", &reason)
    .replace(
        "{proposed_path}",
        diagnostic.proposed_path.as_deref().unwrap_or(""),
    )
    .replace("{code}", diagnostic.code)
}

/// The reason as a message shows it: a block reason or a mismatched fact by its name.
fn reason_text(reason: &DiagnosticReason) -> String {
    match reason {
        DiagnosticReason::Named(name) => name.clone(),
        DiagnosticReason::Ambiguity(reason) => serde_name(reason),
        DiagnosticReason::Blocked(blocked) => serde_name(blocked),
        DiagnosticReason::Mismatch(fact) => serde_name(fact),
    }
}

fn diagnostic(
    rule: DiagnosticRule,
    paths: Vec<String>,
    reason: Option<DiagnosticReason>,
) -> Diagnostic {
    Diagnostic {
        rule,
        severity: rule.default_severity(),
        paths,
        reason,
        proposed_path: None,
        code: rule.code(),
        message: String::new(),
    }
}

fn named(reason: &str) -> Option<DiagnosticReason> {
    Some(DiagnosticReason::Named(reason.to_owned()))
}

fn finding_value<T>(finding: &Finding<T>) -> Option<&T> {
    match finding {
        Finding::Resolved { value, .. } => Some(value),
        _ => None,
    }
}

fn diagnose_item(rules: &Rules, item: &Item, diagnostics: &mut Vec<Diagnostic>) {
    match item {
        Item::Movie(movie) => {
            let videos = video_paths(&movie.versions);
            if movie.misplaced_series {
                diagnostics.push(diagnostic(
                    DiagnosticRule::MisplacedSeries,
                    videos.clone(),
                    None,
                ));
            }
            let ambiguity_reasons = movie.ambiguity_reasons.iter().filter(|reason| {
                !(movie.misplaced_series && **reason == AmbiguityReason::EpisodeTokenInMovieLibrary)
            });
            diagnose_ambiguity(ambiguity_reasons, &videos, diagnostics);
            diagnose_provider_conflicts(&movie.provider_ids, &videos, diagnostics);
            diagnose_versions(&movie.versions, diagnostics);
            diagnose_movie_folder(rules, movie, diagnostics);
            if movie.year.is_none() {
                diagnostics.push(diagnostic(
                    DiagnosticRule::MissingYear,
                    videos.clone(),
                    None,
                ));
            }
            if !has_provider_id(&movie.provider_ids) {
                diagnostics.push(diagnostic(DiagnosticRule::MissingProviderId, videos, None));
            }
        }
        Item::Episode(episode) => {
            let videos = video_paths(&episode.versions);
            diagnose_ambiguity(episode.ambiguity_reasons.iter(), &videos, diagnostics);
            if let Finding::Ambiguous { reason, .. } = &episode.season {
                diagnostics.push(diagnostic(
                    DiagnosticRule::AmbiguousIdentity,
                    videos.clone(),
                    Some(DiagnosticReason::Ambiguity(*reason)),
                ));
            }
            diagnose_provider_conflicts(&episode.provider_ids, &videos, diagnostics);
            diagnose_versions(&episode.versions, diagnostics);
            if episode.episode_title.is_none() {
                diagnostics.push(diagnostic(
                    DiagnosticRule::MissingEpisodeTitle,
                    videos,
                    None,
                ));
            }
        }
        Item::Series(series) => {
            let folder: Vec<String> = series.path.iter().cloned().collect();
            diagnose_provider_conflicts(&series.provider_ids, &folder, diagnostics);
        }
        Item::Season(season) => {
            let folder: Vec<String> = season.path.iter().cloned().collect();
            diagnose_provider_conflicts(&season.provider_ids, &folder, diagnostics);
        }
        Item::Extra(orphan) => {
            let reason = orphan
                .ambiguous
                .then_some(DiagnosticReason::Ambiguity(AmbiguityReason::SeveralOwners));
            diagnostics.push(diagnostic(
                DiagnosticRule::OrphanExtra,
                orphan.paths.clone(),
                reason,
            ));
        }
    }
}

fn video_paths(versions: &[Version]) -> Vec<String> {
    versions
        .iter()
        .flat_map(|version| version.paths.clone())
        .collect()
}

fn has_provider_id(provider_ids: &BTreeMap<String, Finding<String>>) -> bool {
    provider_ids
        .values()
        .any(|finding| finding_value(finding).is_some())
}

/// Reports the ambiguity reasons of an item, except the reasons that a rule of their own
/// reports: conflicting provider IDs and part numbering.
fn diagnose_ambiguity<'a>(
    reasons: impl Iterator<Item = &'a AmbiguityReason>,
    videos: &[String],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for reason in reasons {
        if matches!(
            reason,
            AmbiguityReason::ConflictingProviderId
                | AmbiguityReason::MissingPart
                | AmbiguityReason::DuplicatePart
        ) {
            continue;
        }
        diagnostics.push(diagnostic(
            DiagnosticRule::AmbiguousIdentity,
            videos.to_vec(),
            Some(DiagnosticReason::Ambiguity(*reason)),
        ));
    }
}

fn diagnose_provider_conflicts(
    provider_ids: &BTreeMap<String, Finding<String>>,
    paths: &[String],
    diagnostics: &mut Vec<Diagnostic>,
) {
    if paths.is_empty() {
        return;
    }
    for (provider, finding) in provider_ids {
        if matches!(finding, Finding::Ambiguous { .. }) {
            diagnostics.push(diagnostic(
                DiagnosticRule::ConflictingProviderIds,
                paths.to_vec(),
                named(provider),
            ));
        }
    }
}

/// Reports parts that are not numbered 1, 2, 3 and so on, and versions that have the same
/// edition and label.
fn diagnose_versions(versions: &[Version], diagnostics: &mut Vec<Diagnostic>) {
    for version in versions {
        let rule = match version.ambiguity_reason {
            Some(AmbiguityReason::MissingPart) => DiagnosticRule::MissingPart,
            Some(AmbiguityReason::DuplicatePart) => DiagnosticRule::DuplicatePart,
            _ => continue,
        };
        diagnostics.push(diagnostic(rule, version.paths.clone(), None));
    }
    let mut versions_by_label: BTreeMap<(String, String), Vec<&Version>> = BTreeMap::new();
    for version in versions {
        let fold =
            |text: &Option<String>| lexical::fold_case_text(text.as_deref().unwrap_or_default());
        versions_by_label
            .entry((fold(&version.edition), fold(&version.label)))
            .or_default()
            .push(version);
    }
    for clashing in versions_by_label.values().filter(|group| group.len() > 1) {
        let mut paths: Vec<String> = clashing
            .iter()
            .flat_map(|version| version.paths.clone())
            .collect();
        paths.sort();
        diagnostics.push(diagnostic(
            DiagnosticRule::ClashingVersionLabels,
            paths,
            None,
        ));
    }
}

fn mismatch(path: &str, fact: MismatchedFact) -> Diagnostic {
    diagnostic(
        DiagnosticRule::FolderMismatch,
        vec![path.to_owned()],
        Some(DiagnosticReason::Mismatch(fact)),
    )
}

/// Reports videos whose title, year or edition tag differs from the movie folder that
/// contains them (R8). Skips a folder without a year or provider ID, because only such a
/// folder identifies one work (M1).
fn diagnose_movie_folder(rules: &Rules, movie: &Movie, diagnostics: &mut Vec<Diagnostic>) {
    let Some(folder_path) = movie.path.as_deref() else {
        return;
    };
    let folder = rules.parse_movie_name(file_name(folder_path), EntryKind::Directory);
    if finding_value(&folder.year).is_none() && !has_provider_id(&folder.provider_ids) {
        return;
    }
    for path in movie.versions.iter().flat_map(|version| &version.paths) {
        if path.ends_with('/') || parent_folder(path) != Some(folder_path) {
            continue;
        }
        let file = rules.parse_movie_name(file_name(path), EntryKind::File);
        for fact in movie_mismatches(rules, &folder, &file) {
            diagnostics.push(mismatch(path, fact));
        }
    }
}

fn movie_mismatches(rules: &Rules, folder: &MovieName, file: &MovieName) -> Vec<MismatchedFact> {
    let mut facts = Vec::new();
    let folder_year = finding_value(&folder.year).copied();
    if let (Some(folder_title), Some(file_title)) =
        (finding_value(&folder.title), finding_value(&file.title))
    {
        let repeats_folder_year = folder_year.is_some_and(|year| {
            finding_value(&file.year) == Some(&year)
                && file_title
                    .strip_suffix(&year.to_string())
                    .is_some_and(|rest| lexical::titles_agree(rules, folder_title, rest.trim_end()))
        });
        if !lexical::titles_agree(rules, folder_title, file_title) && !repeats_folder_year {
            facts.push(MismatchedFact::Title);
        }
    }
    if matches!(
        (folder_year, finding_value(&file.year)),
        (Some(folder), Some(own)) if folder != *own
    ) {
        facts.push(MismatchedFact::Year);
    }
    if let (Some(folder_edition), Some(file_edition)) =
        (finding_value(&folder.edition), finding_value(&file.edition))
        && lexical::fold_case_text(folder_edition) != lexical::fold_case_text(file_edition)
    {
        facts.push(MismatchedFact::Edition);
    }
    facts
}

/// Reports each series' missing facts (R9) and the episodes that differ from its folders
/// (R8).
fn diagnose_series(
    rules: &Rules,
    interpretation: &Interpretation,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let episodes: Vec<&Episode> = interpretation
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Episode(episode) => Some(&**episode),
            _ => None,
        })
        .collect();
    for item in &interpretation.items {
        let Item::Series(series) = item else {
            continue;
        };
        let own_episodes: Vec<&Episode> = episodes
            .iter()
            .copied()
            .filter(|episode| {
                episode.series_path == series.path && episode.series_name == series.title
            })
            .collect();
        let paths = match &series.path {
            Some(folder) => vec![folder.clone()],
            None => own_episodes
                .iter()
                .flat_map(|episode| video_paths(&episode.versions))
                .collect(),
        };
        if !paths.is_empty() {
            if series.year.is_none() {
                diagnostics.push(diagnostic(
                    DiagnosticRule::MissingSeriesYear,
                    paths.clone(),
                    None,
                ));
            }
            if !has_provider_id(&series.provider_ids) {
                diagnostics.push(diagnostic(DiagnosticRule::MissingProviderId, paths, None));
            }
        }
        if let Some(folder) = &series.path {
            for episode in &own_episodes {
                diagnose_episode_folders(rules, series, folder, episode, diagnostics);
            }
        }
    }
}

/// Compares an episode's series title, series year and season with its series folder and
/// the season folder directly inside it (R8). The series title is compared only if it is
/// the series' alternate identity (T2). Otherwise the text before the episode code is the
/// episode title.
fn diagnose_episode_folders(
    rules: &Rules,
    series: &Series,
    series_folder: &str,
    episode: &Episode,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for path in episode.versions.iter().flat_map(|version| &version.paths) {
        if path.ends_with('/') {
            continue;
        }
        let file = rules.parse_episode_name(file_name(path), EntryKind::File, None);
        let file_series = finding_value(&file.series_name);
        let agrees_with_series = file_series
            .zip(series.title.as_deref())
            .is_some_and(|(own, title)| lexical::titles_agree(rules, title, own));
        let is_alternate = file_series
            .zip(series.alternate_identity.as_ref())
            .is_some_and(|(own, alternate)| lexical::titles_agree(rules, &alternate.title, own));
        if is_alternate && !agrees_with_series {
            diagnostics.push(mismatch(path, MismatchedFact::SeriesTitle));
        }
        if agrees_with_series
            && matches!(
                (series.year, finding_value(&file.series_year)),
                (Some(folder), Some(own)) if folder != *own
            )
        {
            diagnostics.push(mismatch(path, MismatchedFact::SeriesYear));
        }
        let folder_season = parent_folder(path)
            .filter(|folder| parent_folder(folder) == Some(series_folder))
            .map(|folder| {
                rules
                    .parse_season_folder(file_name(folder), Some(file_name(series_folder)), true)
                    .season
            });
        if let (Some(folder), Some(own)) = (
            folder_season.as_ref().and_then(finding_value),
            finding_value(&file.season),
        ) && folder != own
        {
            diagnostics.push(mismatch(path, MismatchedFact::Season));
        }
    }
}

/// A season key: the series folder, the series name and the season number.
type SeasonKey<'a> = (&'a Option<String>, &'a Option<String>, u16);

/// Reports episode items of one series and season whose episode ranges overlap but are
/// not equal (T17, R8).
fn diagnose_overlapping_episodes(
    interpretation: &Interpretation,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut by_season: BTreeMap<SeasonKey, Vec<&Episode>> = BTreeMap::new();
    for item in &interpretation.items {
        if let Item::Episode(episode) = item
            && let Some(season) = finding_value(&episode.season)
            && episode.episodes.is_some()
        {
            by_season
                .entry((&episode.series_path, &episode.series_name, *season))
                .or_default()
                .push(episode);
        }
    }
    for episodes in by_season.values() {
        for (index, left) in episodes.iter().enumerate() {
            for right in &episodes[index + 1..] {
                let covers = |episode: &Episode| -> BTreeSet<u32> {
                    episode.episodes.iter().flatten().copied().collect()
                };
                let (left_covers, right_covers) = (covers(left), covers(right));
                if left_covers != right_covers && !left_covers.is_disjoint(&right_covers) {
                    let mut paths = video_paths(&left.versions);
                    paths.extend(video_paths(&right.versions));
                    paths.sort();
                    diagnostics.push(diagnostic(DiagnosticRule::OverlappingEpisodes, paths, None));
                }
            }
        }
    }
}

/// Reports unclaimed files by kind. An NFO named after a claimed video is attached to its
/// version, and a generic NFO is attached to its item (S9). An unclaimed NFO therefore
/// matches no item.
/// Reports movies, and series, that are one work in two folders (R11): the same resolved
/// provider ID, else the same title in comparison form and the same year. The caller
/// merges them (M10).
fn diagnose_duplicate_works(
    rules: &Rules,
    interpretation: &Interpretation,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Each work: its location, title, year and provider IDs.
    type Work<'a> = (
        String,
        Option<&'a String>,
        Option<u16>,
        &'a BTreeMap<String, Finding<String>>,
    );
    let works: Vec<Work> = interpretation
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Movie(movie) => movie
                .versions
                .first()
                .and_then(|version| version.paths.first())
                .map(|path| {
                    (
                        path.clone(),
                        movie.title.as_ref(),
                        movie.year,
                        &movie.provider_ids,
                    )
                }),
            Item::Series(series) => series.path.clone().map(|path| {
                (
                    path,
                    series.title.as_ref(),
                    series.year,
                    &series.provider_ids,
                )
            }),
            _ => None,
        })
        .collect();
    let mut groups: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for (location, title, year, provider_ids) in &works {
        for (provider, finding) in provider_ids.iter() {
            if let Some(id) = finding.resolved_value() {
                groups
                    .entry((provider.clone(), id.clone()))
                    .or_default()
                    .push(location.clone());
            }
        }
        if let (Some(title), Some(year)) = (title, year) {
            let key = format!("{}|{year}", lexical::title_comparison_key(rules, title));
            groups
                .entry(("title_and_year".to_owned(), key))
                .or_default()
                .push(location.clone());
        }
    }
    let mut reported: BTreeSet<Vec<String>> = BTreeSet::new();
    // A provider ID group is reported before a title-and-year group with the same
    // locations, so that the stronger reason is the one reported.
    let ordered = groups
        .iter()
        .filter(|((kind, _), _)| kind != "title_and_year")
        .chain(
            groups
                .iter()
                .filter(|((kind, _), _)| kind == "title_and_year"),
        );
    for ((kind, _), locations) in ordered {
        let mut locations = locations.clone();
        locations.sort();
        locations.dedup();
        if locations.len() > 1 && reported.insert(locations.clone()) {
            diagnostics.push(diagnostic(
                DiagnosticRule::DuplicateWork,
                locations,
                named(kind),
            ));
        }
    }
}

/// Reports episode items of one series that cover the same season and episodes in the
/// aired order (R11). Items whose coverages overlap without being equal are
/// `overlapping_episodes` (R8). Specials without episode numbers and other orders are not
/// compared.
fn diagnose_duplicate_episodes(interpretation: &Interpretation, diagnostics: &mut Vec<Diagnostic>) {
    type Coordinate = (Option<String>, Option<String>, u16, Vec<u32>);
    let mut by_coordinate: BTreeMap<Coordinate, Vec<Vec<String>>> = BTreeMap::new();
    for item in &interpretation.items {
        let Item::Episode(episode) = item else {
            continue;
        };
        if !matches!(episode.order, None | Some(EpisodeOrder::Aired)) {
            continue;
        }
        let (Some(&season), Some(numbers)) =
            (episode.season.resolved_value(), episode.episodes.as_ref())
        else {
            continue;
        };
        let videos: Vec<String> = episode
            .versions
            .iter()
            .flat_map(|version| version.paths.iter().cloned())
            .collect();
        by_coordinate
            .entry((
                episode.series_path.clone(),
                episode.series_name.clone(),
                season,
                numbers.clone(),
            ))
            .or_default()
            .push(videos);
    }
    for ((_, _, season, numbers), items) in by_coordinate {
        if items.len() > 1 {
            let mut paths: Vec<String> = items.into_iter().flatten().collect();
            paths.sort();
            diagnostics.push(diagnostic(
                DiagnosticRule::DuplicateEpisode,
                paths,
                named(&episode_token(season, &numbers)),
            ));
        }
    }
}

/// Reports the asset roles and subtitles that the profile requires and an item or a feature
/// video lacks (R12). Only files that the interpretation lists count: embedded subtitle
/// tracks are not visible to it.
fn diagnose_coverage(
    interpretation: &Interpretation,
    profile: &OutputProfile,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for item in &interpretation.items {
        let (kind, location, artwork, nfo, extras): (_, Option<String>, &[Artwork], _, &[Extra]) =
            match item {
                Item::Movie(movie) => (
                    AssetItemKind::Movie,
                    movie.path.clone().or_else(|| first_video(&movie.versions)),
                    &movie.artwork,
                    has_nfo(&movie.nfo, &movie.versions),
                    &movie.extras,
                ),
                Item::Series(series) => (
                    AssetItemKind::Series,
                    series.path.clone(),
                    &series.artwork,
                    series.nfo.is_some(),
                    &series.extras,
                ),
                Item::Season(season) => (
                    AssetItemKind::Season,
                    season.path.clone().or_else(|| season.series_path.clone()),
                    &season.artwork,
                    season.nfo.is_some(),
                    &season.extras,
                ),
                Item::Episode(episode) => (
                    AssetItemKind::Episode,
                    first_video(&episode.versions),
                    &episode.artwork,
                    has_nfo(&episode.nfo, &episode.versions),
                    &episode.extras,
                ),
                Item::Extra(_) => continue,
            };
        let (Some(required), Some(location)) = (profile.required_assets.get(&kind), location)
        else {
            continue;
        };
        let has = |role: AssetRole| match role {
            AssetRole::Nfo => nfo,
            AssetRole::ThemeSong => extras
                .iter()
                .any(|extra| extra.extra == ExtraKind::ThemeSong),
            AssetRole::ThemeVideo => extras
                .iter()
                .any(|extra| extra.extra == ExtraKind::ThemeVideo),
            _ => artwork
                .iter()
                .any(|image| AssetRole::of_artwork(image.role) == Some(role)),
        };
        let mut missing: Vec<AssetRole> = required
            .iter()
            .copied()
            .filter(|role| !has(*role))
            .collect();
        missing.sort();
        missing.dedup();
        for role in missing {
            diagnostics.push(diagnostic(
                DiagnosticRule::MissingAsset,
                vec![location.clone()],
                named(&serde_name(role)),
            ));
        }
    }
    if profile.wanted_subtitles.is_empty() {
        return;
    }
    let feature_videos = interpretation.items.iter().flat_map(|item| {
        let versions: &[Version] = match item {
            Item::Movie(movie) => &movie.versions,
            Item::Episode(episode) => &episode.versions,
            _ => &[],
        };
        versions.iter().filter_map(|version| version.paths.first())
    });
    for video in feature_videos {
        let subtitles = interpretation.video_subtitles.get(video);
        for wanted in &profile.wanted_subtitles {
            let wanted_language = lexical::vocabulary_form(&wanted.language);
            let present = subtitles.into_iter().flatten().any(|subtitle| {
                let language_matches = subtitle.language.resolved_value().is_some_and(|tag| {
                    let tag = lexical::vocabulary_form(tag);
                    tag == wanted_language
                        || tag
                            .strip_prefix(&wanted_language)
                            .is_some_and(|rest| rest.starts_with('-'))
                });
                language_matches
                    && wanted
                        .flags
                        .iter()
                        .all(|flag| subtitle.flags.contains(flag))
            });
            if !present {
                let flags = wanted
                    .flags
                    .iter()
                    .map(|flag| format!(".{}", serde_name(*flag)));
                let reason = format!("{}{}", wanted.language, flags.collect::<String>());
                diagnostics.push(diagnostic(
                    DiagnosticRule::MissingSubtitle,
                    vec![video.clone()],
                    named(&reason),
                ));
            }
        }
    }
}

/// Returns `true` if an item has an NFO: its own, or the NFO of any of its versions
/// (R12, S9). The item's own NFO is only its primary version's, so a later version's NFO
/// counts here too.
fn has_nfo(nfo: &Option<String>, versions: &[Version]) -> bool {
    nfo.is_some() || versions.iter().any(|version| version.nfo.is_some())
}

fn first_video(versions: &[Version]) -> Option<String> {
    versions
        .first()
        .and_then(|version| version.paths.first())
        .cloned()
}

fn diagnose_unclaimed_files(
    rules: &Rules,
    interpretation: &Interpretation,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let unclaimed = &interpretation.unclaimed_files;
    for path in unclaimed {
        let rule =
            if rules.is_subtitle_file(path, unclaimed).result || rules.is_audio_file(path).result {
                DiagnosticRule::OrphanSidecar
            } else if rules.is_release_junk(path).result {
                DiagnosticRule::ReleaseJunk
            } else if rules.is_image_file(path).result {
                DiagnosticRule::OrphanArtwork
            } else {
                DiagnosticRule::UnclaimedFile
            };
        diagnostics.push(diagnostic(rule, vec![path.clone()], None));
    }
}

/// Drops the missing-fact diagnostics of paths that have an error diagnostic. An item
/// that cannot be named safely reports only the error (R9).
fn drop_missing_facts_of_errors(diagnostics: &mut Vec<Diagnostic>) {
    let error_paths: BTreeSet<String> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .flat_map(|diagnostic| diagnostic.paths.clone())
        .collect();
    diagnostics.retain(|diagnostic| {
        !diagnostic.rule.is_missing_fact()
            || diagnostic
                .paths
                .iter()
                .all(|path| !error_paths.contains(path))
    });
}

fn apply_severity_overrides(diagnostics: &mut Vec<Diagnostic>, profile: &OutputProfile) {
    diagnostics.retain_mut(|diagnostic| match profile.severity.get(&diagnostic.rule) {
        None => true,
        Some(SeverityOverride::Off) => false,
        Some(level) => {
            diagnostic.severity = match level {
                SeverityOverride::Error => Severity::Error,
                SeverityOverride::Warning => Severity::Warning,
                SeverityOverride::Info | SeverityOverride::Off => Severity::Info,
            };
            true
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_rule_codes_are_unique() {
        let codes: BTreeSet<&str> = DiagnosticRule::ALL.iter().map(|rule| rule.code()).collect();
        assert_eq!(codes.len(), DiagnosticRule::ALL.len());
    }

    #[test]
    fn messages_have_a_template_for_every_rule() {
        for rule in DiagnosticRule::ALL {
            assert!(
                MESSAGES.contains_key(&serde_name(*rule)),
                "no English message for `{}`",
                serde_name(*rule)
            );
        }
        assert_eq!(MESSAGES.len(), DiagnosticRule::ALL.len());
    }
}
