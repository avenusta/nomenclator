//! Reads media names and limited folder context and returns structured naming facts.
//!
//! The crate is pure: it does not access the filesystem, network, clock or locale. Each
//! module is usable on its own, and no module is a mandatory stage of another.
//!
//! # In and out
//!
//! Inputs are root-relative paths with `/` separators, the library type
//! ([`LibraryType::Movies`] or [`LibraryType::Tv`]), the contents of any ignore files,
//! and an [`OutputProfile`] if names are rendered. The caller scans, probes and stores.
//! This crate reads only the strings that it receives. Every fact is returned as
//! resolved, ambiguous or unknown ([`model::Finding`]). Every entry appears in the
//! result: as an item, as an excluded, undecided or invalid entry, or as an unclaimed
//! file.
//!
//! # Modules
//!
//! - **Parsers** read one name at a time: [`parse_movie_name`], [`parse_series_name`],
//!   [`parse_season_folder`], [`parse_episode_name`], [`parse_sidecar`],
//!   [`parse_provider_ids`], [`classify_extra`], [`classify_artwork`],
//!   [`classify_nfo`] and the file-type checks such as [`is_video_file`].
//! - **Interpretation** ([`interpret`]) reads a whole library listing: movies with their
//!   versions, parts and extras, or series with seasons and episodes, plus the subtitles
//!   and artwork each owns. [`group_parts`] groups multi-part files on its own.
//! - **Ignore rules** ([`evaluate_ignore`], [`IgnoreRules`]) evaluate which entries are
//!   excluded, from rule files that the caller reads.
//! - **Matching** ([`rank_candidates`]) scores caller-supplied provider candidates
//!   against the facts of a name. It does not use any provider's API.
//! - **Rendering and planning** ([`render_names`], [`plan_renames`]) render facts as
//!   names under a profile, and turn a listing into a rename plan that the caller may
//!   apply.
//! - **Validation** ([`validate`]) reports the entries of an interpreted library that
//!   do not follow the conventions or the profile.
//!
//! # Rules
//!
//! Every vocabulary that defines the meaning of a name (release words, extras and season
//! words, languages, extensions, built-in exclusions) is stored in one [`Rules`] value.
//! Each operation is a method on it. The free functions above use [`Rules::builtin`].
//! Whole-library results contain the [`RulesIdentity`] that they were produced under, so
//! a caller can check a stored result against the rules in use.
//!
//! # Examples
//!
//! Parse one movie name:
//!
//! ```
//! use nomenclator::model::{EntryKind, Finding};
//! use nomenclator::parse_movie_name;
//!
//! let name = parse_movie_name("Northern Passage (2021).mkv", EntryKind::File);
//! assert!(matches!(name.title, Finding::Resolved { ref value, .. } if value == "Northern Passage"));
//! assert!(matches!(name.year, Finding::Resolved { value: 2021, .. }));
//! assert_eq!(name.container.as_deref(), Some("mkv"));
//! ```
//!
//! Interpret a listing, then validate it:
//!
//! ```
//! use nomenclator::{
//!     DiagnosticRule, Item, LibraryType, OutputProfile, interpret, validate,
//! };
//!
//! let paths = vec!["Northern.Passage.2021.1080p.BluRay.x264.mkv".to_owned()];
//! let interpretation = interpret(LibraryType::Movies, &paths, &[]);
//! let Item::Movie(movie) = &interpretation.items[0] else {
//!     panic!("expected a movie");
//! };
//! assert_eq!(movie.title.as_deref(), Some("Northern Passage"));
//! assert_eq!(movie.year, Some(2021));
//!
//! let validation = validate(&interpretation, &OutputProfile::default());
//! let violation = validation
//!     .diagnostics
//!     .iter()
//!     .find(|d| d.rule == DiagnosticRule::ProfileViolation)
//!     .unwrap();
//! assert_eq!(
//!     violation.proposed_path.as_deref(),
//!     Some("Northern Passage (2021)/Northern Passage (2021).mkv"),
//! );
//! ```
//!
//! Plan renames for a listing:
//!
//! ```
//! use nomenclator::{LibraryType, OutputProfile, interpret, plan_renames};
//!
//! let paths = vec![
//!     "Northern Passage 2021/Northern.Passage.2021.1080p.mkv".to_owned(),
//!     "Northern Passage 2021/Northern.Passage.2021.1080p.en.srt".to_owned(),
//! ];
//! let interpretation = interpret(LibraryType::Movies, &paths, &[]);
//! let plan = plan_renames(&interpretation, &paths, &OutputProfile::default());
//!
//! let targets: Vec<&str> = plan.proposals.iter().map(|p| p.to.as_str()).collect();
//! assert_eq!(
//!     targets,
//!     [
//!         "Northern Passage (2021)/Northern Passage (2021).en.srt",
//!         "Northern Passage (2021)/Northern Passage (2021).mkv",
//!     ],
//! );
//! assert!(plan.blocked.is_empty());
//! ```
//!
//! Run an operation under an explicit rules value:
//!
//! ```
//! use nomenclator::{LibraryType, Rules};
//!
//! let rules = Rules::default();
//! let paths = vec!["Northern Passage (2021)/Northern Passage (2021).mkv".to_owned()];
//! let interpretation = rules.interpret(LibraryType::Movies, &paths, &[]);
//! assert_eq!(&interpretation.rules, rules.identity());
//! ```

// The crate is pure (CODING_STANDARDS.md): it never prints, exits or unwraps.
#![warn(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::exit,
    clippy::unwrap_used
)]

/// The version of the naming rules. It changes whenever naming behaviour changes.
pub const RULES_VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod model;
mod parsers;

mod data;
mod diff;
mod document;
mod ignore;
mod inputs;
mod interpret;
mod lexical;
mod matching;
mod rendering;
mod rules;
mod suggestion;
mod validation;

pub use diff::{FactDifference, PathDifference, RulesDiff, diff_interpretations, diff_rules};
pub use document::{DocumentError, parse_document};
pub use ignore::{
    Exclusion, ExclusionReason, IgnoreDialect, IgnoreEvaluation, IgnoreFile, IgnoreRules,
    RuleDiagnostic, RuleDiagnosticReason, evaluate_ignore,
};
pub use inputs::{
    AbsoluteRange, AppliedHint, CallerInputs, EpisodeOrder, ExtraOverride, Hint, HintOutcome,
    InputDiagnostic, InputDiagnosticReason, MappedNumbering, NumberingMap, Overrides,
    ReferenceMetadata, ReferenceMovie, ReferenceSeries, SceneMapping, SeriesInputs,
};
pub use interpret::parts::{Stack, Stacks, group_parts};
pub use interpret::{
    Artwork, Episode, ExcludedEntry, Extra, Interpretation, InvalidEntry, InvalidEntryReason, Item,
    Movie, OrphanExtra, ScopeGroup, Season, Series, SidecarFile, UndecidedEntry, Version,
    interpret, interpret_with,
};
pub use matching::{
    BatchRanking, CandidateRanking, Contradiction, EntityKind, EpisodeCoordinates, HandledRanking,
    LocalizedTitle, MatchCandidate, MatchComponent, MatchFacts, MatchOutcome, QueryReason,
    RankQuery, ReleaseDate, ScoreComponent, ScoredCandidate, SearchQueries, SearchQuery,
    match_facts, rank_candidates, search_queries,
};
pub use parsers::artwork::{ArtworkClassification, classify_artwork};
pub use parsers::episode_name::{
    AlternateIdentity, EpisodeName, PresentationKind, Special, SpecialKind, parse_episode_name,
};
pub use parsers::extras::{ExtraClassification, LibraryType, classify_extra};
pub use parsers::file_types::{
    FileTypeCheck, UnsupportedVideoReason, VideoFileCheck, is_audio_file, is_image_file,
    is_release_junk, is_subtitle_file, is_video_file,
};
pub use parsers::movie_name::{AlternateTitle, MovieName, parse_movie_name};
pub use parsers::nfo::{NfoClassification, classify_nfo};
pub use parsers::provider_ids::{ProviderIds, parse_provider_ids};
pub use parsers::release::{ReleaseFacts, ReleaseMarker, parse_release};
pub use parsers::season_folder::{SeasonFolder, parse_season_folder};
pub use parsers::series_name::{SeriesName, parse_series_name};
pub use parsers::sidecar::{Sidecar, parse_sidecar};
pub use rendering::{
    AssetFacts, AssetItemKind, AssetRole, BlockReason, BlockedRename, ExtensionCase, ExtraFacts,
    ImportPlan, ImportProposal, LeftFile, LeftReason, MovieLayout, NamingFacts, OutputProfile,
    ProfileDiagnostic, ProfileDiagnosticReason, ProposedRemoval, ProviderTagStyle, RemovalReason,
    RenamePlan, RenameProposal, RenderedName, RenderedTemplate, SeasonFolderStyle, SidecarFacts,
    Suppression, TargetFilesystem, Template, TemplateError, TemplateErrorReason, TemplateFacts,
    TemplateSlot, VersionLabelStyle, WantedSubtitle, plan_renames, render_names,
};
pub use rules::{
    Overlay, OverlayDiagnostic, OverlayDiagnosticReason, OverlayIdentity, Rules, RulesIdentity,
};
pub use suggestion::{LibraryTypeSuggestion, suggest_library_type};
pub use validation::{
    Diagnostic, DiagnosticReason, DiagnosticRule, MismatchedFact, RuleCount, Severity,
    SeverityOverride, Validation, ValidationSummary, diagnostic_message, validate, validate_with,
};
