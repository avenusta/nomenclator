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
//! Every operation is a method on [`Rules`]. [`Rules::builtin`] returns the built-in rules.
//!
//! - **Parsers** read one name at a time: [`Rules::parse_movie_name`],
//!   [`Rules::parse_series_name`], [`Rules::parse_season_folder`],
//!   [`Rules::parse_episode_name`], [`Rules::parse_sidecar`], [`Rules::parse_provider_ids`],
//!   [`Rules::parse_release`], [`Rules::classify_extra`], [`Rules::classify_artwork`],
//!   [`Rules::classify_nfo`] and the file-type checks such as [`Rules::is_video_file`].
//! - **Interpretation** ([`Rules::interpret`]) reads a whole library listing: movies with
//!   their versions, parts and extras, or series with seasons and episodes, plus the
//!   subtitles and artwork each owns. [`Rules::group_parts`] groups multi-part files on its
//!   own.
//! - **Ignore rules** ([`Rules::evaluate_ignore`], [`IgnoreRules`]) evaluate which entries
//!   are excluded, from rule files that the caller reads.
//! - **Matching** ([`Rules::rank_candidates`]) scores caller-supplied provider candidates
//!   against the facts of a name. It does not use any provider's API.
//! - **Rendering and planning** ([`Rules::render_names`], [`Rules::plan_renames`]) render
//!   facts as names under a profile, and turn a listing into a rename plan that the caller
//!   may apply.
//! - **Validation** ([`Rules::validate`]) reports the entries of an interpreted library that
//!   do not follow the conventions or the profile.
//!
//! # Rules
//!
//! Every vocabulary that defines the meaning of a name (release words, extras and season
//! words, languages, extensions, built-in exclusions) is stored in one [`Rules`] value.
//! [`Rules::from_documents`] builds rules from overlay documents and declared languages.
//! Whole-library results contain the [`RulesIdentity`] that they were produced under, so
//! a caller can check a stored result against the rules in use.
//!
//! # Examples
//!
//! Parse one movie name:
//!
//! ```
//! use nomenclator::Rules;
//! use nomenclator::model::{EntryKind, Finding};
//!
//! let name = Rules::builtin().parse_movie_name("Northern Passage (2021).mkv", EntryKind::File);
//! assert!(matches!(name.title, Finding::Resolved { ref value, .. } if value == "Northern Passage"));
//! assert!(matches!(name.year, Finding::Resolved { value: 2021, .. }));
//! assert_eq!(name.container.as_deref(), Some("mkv"));
//! ```
//!
//! Interpret a listing, then validate it:
//!
//! ```
//! use nomenclator::{DiagnosticRule, Item, LibraryType, OutputProfile, Rules};
//!
//! let rules = Rules::builtin();
//! let paths = vec!["Northern.Passage.2021.1080p.BluRay.x264.mkv".to_owned()];
//! let interpretation = rules.interpret(LibraryType::Movies, &paths, &[]);
//! let Item::Movie(movie) = &interpretation.items[0] else {
//!     panic!("expected a movie");
//! };
//! assert_eq!(movie.title.as_deref(), Some("Northern Passage"));
//! assert_eq!(movie.year, Some(2021));
//!
//! let validation = rules.validate(&interpretation, &OutputProfile::default());
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
//! use nomenclator::{LibraryType, OutputProfile, Rules};
//!
//! let rules = Rules::builtin();
//! let paths = vec![
//!     "Northern Passage 2021/Northern.Passage.2021.1080p.mkv".to_owned(),
//!     "Northern Passage 2021/Northern.Passage.2021.1080p.en.srt".to_owned(),
//! ];
//! let interpretation = rules.interpret(LibraryType::Movies, &paths, &[]);
//! let plan = rules.plan_renames(&interpretation, &paths, &OutputProfile::default());
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
//! Run an operation under rules built from an overlay:
//!
//! ```
//! use nomenclator::{LibraryType, Rules};
//!
//! let overlay = "name: mine\nextras_folders:\n  add: { Making Of: behind_the_scenes }\n";
//! let rules = Rules::from_documents(&[], &[overlay.to_owned()]).expect("a valid overlay");
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
mod operations;
mod rendering;
mod rules;
mod suggestion;
mod validation;

pub use diff::{FactDifference, PathDifference, RulesDiff, diff_interpretations, diff_rules};
pub use document::{DocumentError, parse_document};
pub use ignore::{
    Exclusion, ExclusionReason, IgnoreDialect, IgnoreEvaluation, IgnoreFile, IgnoreRules,
    RuleDiagnostic, RuleDiagnosticReason,
};
pub use inputs::{
    AbsoluteRange, AppliedHint, CallerInputs, EpisodeOrder, ExtraOverride, Hint, HintOutcome,
    InputDiagnostic, InputDiagnosticReason, MappedNumbering, NumberingMap, Overrides,
    ReferenceMetadata, ReferenceMovie, ReferenceSeries, SceneMapping, SeriesInputs,
};
pub use interpret::parts::{Stack, Stacks};
pub use interpret::{
    Artwork, Episode, ExcludedEntry, Extra, Interpretation, InvalidEntry, InvalidEntryReason, Item,
    Movie, OrphanExtra, ScopeGroup, Season, Series, SidecarFile, UndecidedEntry, Version,
};
pub use matching::{
    BatchRanking, CandidateRanking, Contradiction, EntityKind, EpisodeCoordinates, HandledRanking,
    LocalizedTitle, MatchCandidate, MatchComponent, MatchFacts, MatchOutcome, QueryReason,
    RankQuery, ReleaseDate, ScoreComponent, ScoredCandidate, SearchQueries, SearchQuery,
    match_facts,
};
pub use operations::OperationError;
pub use parsers::artwork::ArtworkClassification;
pub use parsers::episode_name::{
    AlternateIdentity, EpisodeName, PresentationKind, Special, SpecialKind,
};
pub use parsers::extras::{ExtraClassification, LibraryType};
pub use parsers::file_types::{FileTypeCheck, UnsupportedVideoReason, VideoFileCheck};
pub use parsers::movie_name::{AlternateTitle, MovieName};
pub use parsers::nfo::NfoClassification;
pub use parsers::provider_ids::ProviderIds;
pub use parsers::release::{ReleaseFacts, ReleaseMarker};
pub use parsers::season_folder::SeasonFolder;
pub use parsers::series_name::SeriesName;
pub use parsers::sidecar::Sidecar;
pub use rendering::{
    AssetFacts, AssetItemKind, AssetRole, BlockReason, BlockedRename, ExtensionCase, ExtraFacts,
    ImportPlan, ImportProposal, LeftFile, LeftReason, MovieLayout, NamingFacts, OutputProfile,
    ProfileDiagnostic, ProfileDiagnosticReason, ProposedRemoval, ProviderTagStyle, RemovalReason,
    RenamePlan, RenameProposal, RenderedName, RenderedTemplate, SeasonFolderStyle, SidecarFacts,
    Suppression, TargetFilesystem, Template, TemplateError, TemplateErrorReason, TemplateFacts,
    TemplateSlot, VersionLabelStyle, WantedSubtitle,
};
pub use rules::{
    Overlay, OverlayDiagnostic, OverlayDiagnosticReason, OverlayIdentity, Rules, RulesIdentity,
};
pub use suggestion::LibraryTypeSuggestion;
pub use validation::{
    Diagnostic, DiagnosticReason, DiagnosticRule, MismatchedFact, RuleCount, Severity,
    SeverityOverride, Validation, ValidationSummary,
};
