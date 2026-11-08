//! Runs an operation by its name, with its input and its result as JSON values. This is
//! the one dispatch that the corpus, the CLI and the server use: an operation's name and
//! its input fields are defined once, here, and `corpus/README.md` documents them.

use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::model::{EntryKind, SidecarKind};
use crate::rendering::Template;
use crate::{
    CallerInputs, IgnoreFile, Interpretation, LibraryType, MatchCandidate, MatchFacts, NamingFacts,
    OutputProfile, OverlayDiagnostic, ProfileDiagnostic, RankQuery, ReferenceMetadata, Rules,
    RulesIdentity, ScopeGroup, TemplateError, TemplateFacts, diff_rules, match_facts,
};

/// The reason an operation returns no result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationError {
    /// No operation has the name.
    UnknownOperation(String),
    /// The input does not have the fields of the operation.
    InvalidInput(String),
    /// An overlay or a declared language of the input is rejected.
    RejectedRules(Vec<OverlayDiagnostic>),
    /// An output profile of the input writes names that the rules do not read back.
    RejectedProfile(Vec<OverlayDiagnostic>),
}

impl fmt::Display for OperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OperationError::UnknownOperation(name) => write!(
                formatter,
                "unknown operation `{name}`. Use an operation listed in corpus/README.md"
            ),
            OperationError::InvalidInput(message) => write!(formatter, "input: {message}"),
            OperationError::RejectedRules(diagnostics) => {
                write!(formatter, "the rules are rejected: {}", joined(diagnostics))
            }
            OperationError::RejectedProfile(diagnostics) => write!(
                formatter,
                "the rules cannot read the names of the source profile: {}",
                joined(diagnostics)
            ),
        }
    }
}

impl std::error::Error for OperationError {}

fn joined(diagnostics: &[OverlayDiagnostic]) -> String {
    let described: Vec<String> = diagnostics.iter().map(ToString::to_string).collect();
    described.join("; ")
}

type Handler = fn(&Rules, Value) -> Result<Value, OperationError>;

/// Every operation, by name.
const OPERATIONS: &[(&str, Handler)] = &[
    ("parse_movie_name", |rules, input| {
        let input: NameAndKind = read(input)?;
        Ok(json(rules.parse_movie_name(&input.name, input.kind)))
    }),
    ("parse_series_name", |rules, input| {
        let input: SeriesNameInput = read(input)?;
        Ok(json(rules.parse_series_name(&input.name_or_path)))
    }),
    ("parse_season_folder", |rules, input| {
        let input: SeasonFolderInput = read(input)?;
        Ok(json(rules.parse_season_folder(
            &input.name,
            input.parent_name.as_deref(),
            input.parent_is_series_folder,
        )))
    }),
    ("parse_episode_name", |rules, input| {
        let input: EpisodeNameInput = read(input)?;
        Ok(json(rules.parse_episode_name(
            &input.path,
            input.kind,
            input.season_folder_number,
        )))
    }),
    ("parse_release", |rules, input| {
        let input: NameAndKind = read(input)?;
        Ok(json(rules.parse_release(&input.name, input.kind)))
    }),
    ("parse_provider_ids", |rules, input| {
        let input: Text = read(input)?;
        Ok(json(rules.parse_provider_ids(&input.text)))
    }),
    ("parse_sidecar", |rules, input| {
        let input: SidecarInput = read(input)?;
        Ok(json(rules.parse_sidecar(
            &input.name,
            &input.video_name,
            input.kind,
        )))
    }),
    ("suggest_library_type", |rules, input| {
        let input: Paths = read(input)?;
        Ok(json(rules.suggest_library_type(&input.paths)))
    }),
    ("group_parts", |rules, input| {
        let input: Paths = read(input)?;
        Ok(json(rules.group_parts(&input.paths)))
    }),
    ("classify_extra", |rules, input| {
        let input: ExtraInput = read(input)?;
        // Keeps only the entries that the ignore rules keep, as a caller does.
        let in_scope: Vec<String> = input
            .context_paths
            .into_iter()
            .filter(|path| {
                rules
                    .evaluate_ignore(path, EntryKind::File, &input.ignore_files)
                    .excluded
                    == Some(false)
            })
            .collect();
        Ok(json(rules.classify_extra(
            &input.path,
            input.kind,
            input.library,
            &in_scope,
        )))
    }),
    ("classify_artwork", |rules, input| {
        let input: ArtworkInput = read(input)?;
        Ok(json(rules.classify_artwork(
            &input.path,
            input.video_path.as_deref(),
        )))
    }),
    ("classify_nfo", |rules, input| {
        let input: NfoInput = read(input)?;
        Ok(json(rules.classify_nfo(&input.path, &input.video_paths)))
    }),
    ("is_video_file", |rules, input| {
        let input: FileTypeInput = read(input)?;
        Ok(json(rules.is_video_file(&input.path)))
    }),
    ("is_audio_file", |rules, input| {
        let input: FileTypeInput = read(input)?;
        Ok(json(rules.is_audio_file(&input.path)))
    }),
    ("is_image_file", |rules, input| {
        let input: FileTypeInput = read(input)?;
        Ok(json(rules.is_image_file(&input.path)))
    }),
    ("is_subtitle_file", |rules, input| {
        let input: FileTypeInput = read(input)?;
        Ok(json(
            rules.is_subtitle_file(&input.path, &input.sibling_paths),
        ))
    }),
    ("is_release_junk", |rules, input| {
        let input: FileTypeInput = read(input)?;
        Ok(json(rules.is_release_junk(&input.path)))
    }),
    ("evaluate_ignore", |rules, input| {
        let input: IgnoreInput = read(input)?;
        Ok(json(rules.evaluate_ignore(
            &input.path,
            input.kind,
            &input.ignore_files,
        )))
    }),
    ("interpret", |rules, input| {
        let tree: Tree = read(input)?;
        Ok(json(tree.interpret(rules)))
    }),
    ("render_names", |rules, input| {
        let input: RenderInput = read(input)?;
        Ok(json(rules.render_names(&input.facts, &input.profile)))
    }),
    ("render_template", |_, input| {
        let input: TemplateInput = read(input)?;
        Ok(match Template::parse(&input.template) {
            Ok(template) => json(template.render(&input.facts)),
            Err(error) => json(TemplateOutcome {
                error: Some(error),
                facts: None,
            }),
        })
    }),
    ("read_template", |_, input| {
        let input: TemplateInput = read(input)?;
        Ok(match Template::parse(&input.template) {
            Ok(template) => json(TemplateOutcome {
                error: None,
                facts: Some(template.read(input.name.as_deref().unwrap_or_default())),
            }),
            Err(error) => json(TemplateOutcome {
                error: Some(error),
                facts: None,
            }),
        })
    }),
    ("check_profile", |rules, input| {
        let input: ProfileInput = read(input)?;
        Ok(json(
            match serde_json::from_value::<OutputProfile>(input.profile) {
                Ok(profile) => CheckedProfile {
                    invalid: false,
                    diagnostics: rules.check_profile(&profile),
                },
                Err(_) => CheckedProfile {
                    invalid: true,
                    diagnostics: Vec::new(),
                },
            },
        ))
    }),
    ("plan_renames", |rules, input| {
        let tree: Tree = read(input)?;
        let interpretation = tree.interpret(rules);
        Ok(json(rules.plan_renames(
            &interpretation,
            &tree.paths,
            &tree.profile,
        )))
    }),
    ("convert_layout", |rules, input| {
        let input: ConversionInput = read(input)?;
        rules
            .plan_conversion(
                input.library,
                &input.paths,
                &input.ignore_files,
                &input.inputs,
                &input.from_profile,
                &input.to_profile,
            )
            .map(json)
            .map_err(OperationError::RejectedProfile)
    }),
    ("plan_import", |rules, input| {
        let input: ImportInput = read(input)?;
        let download = rules.interpret_with(
            input.library,
            &input.paths,
            &input.ignore_files,
            &input.inputs,
        );
        let library = rules.interpret(
            input.library,
            &input.library_paths,
            &input.library_ignore_files,
        );
        Ok(json(rules.plan_import(
            &download,
            &library,
            &input.library_paths,
            &input.profile,
        )))
    }),
    ("validate", |rules, input| {
        let tree: Tree = read(input)?;
        let interpretation = tree.interpret(rules);
        Ok(json(rules.validate_with(
            &interpretation,
            &tree.profile,
            &tree.reference,
        )))
    }),
    ("end_to_end", |rules, input| {
        let tree: Tree = read(input)?;
        let interpretation = tree.interpret(rules);
        let plan = rules.plan_renames(&interpretation, &tree.paths, &tree.profile);
        let validation = rules.validate_with(&interpretation, &tree.profile, &tree.reference);
        Ok(serde_json::json!({
            "interpretation": json(&interpretation),
            "plan": json(&plan),
            "validation": json(&validation),
        }))
    }),
    ("match_facts", |rules, input| {
        let tree: Tree = read(input)?;
        let interpretation = tree.interpret(rules);
        Ok(serde_json::json!({ "facts": json(match_facts(&interpretation.items)) }))
    }),
    ("rank_candidates", |rules, input| {
        let input: RankCandidatesInput = read(input)?;
        Ok(json(rules.rank_candidates(&input.facts, &input.candidates)))
    }),
    ("rank_batch", |rules, input| {
        let input: RankBatchInput = read(input)?;
        Ok(json(rules.rank_batch(&input.queries)))
    }),
    ("search_queries", |rules, input| {
        let input: SearchQueriesInput = read(input)?;
        Ok(json(rules.search_queries(&input.facts)))
    }),
    ("build_rules", |_, input| {
        let input: RulesInput = read(input)?;
        Ok(json(match input.build() {
            Ok(rules) => BuiltRules {
                identity: Some(rules.identity().clone()),
                diagnostics: Vec::new(),
            },
            Err(OperationError::RejectedRules(diagnostics)) => BuiltRules {
                identity: None,
                diagnostics,
            },
            Err(error) => return Err(error),
        }))
    }),
    ("diff_rules", |_, input| {
        let input: DiffInput = read(input)?;
        let (a, b) = (input.a.build()?, input.b.build()?);
        Ok(json(diff_rules(
            &a,
            &b,
            input.library,
            &input.paths,
            &input.ignore_files,
            &input.inputs,
        )))
    }),
];

impl Rules {
    /// Runs the operation `name` with `input` under these rules, and returns its result.
    /// The operations, their input fields and their results are listed in
    /// `corpus/README.md`.
    /// Fails if no operation has the name, if the input does not have the operation's
    /// fields, or if rules or a profile in the input are rejected.
    pub fn run(&self, name: &str, input: Value) -> Result<Value, OperationError> {
        let (_, handler) = OPERATIONS
            .iter()
            .find(|(known, _)| *known == name)
            .ok_or_else(|| OperationError::UnknownOperation(name.to_owned()))?;
        handler(self, input)
    }

    /// Returns the name of every operation that [`Rules::run`] runs.
    pub fn operations() -> impl Iterator<Item = &'static str> {
        OPERATIONS.iter().map(|(name, _)| *name)
    }
}

fn read<T: DeserializeOwned>(input: Value) -> Result<T, OperationError> {
    serde_json::from_value(input).map_err(|error| OperationError::InvalidInput(error.to_string()))
}

fn json(result: impl Serialize) -> Value {
    serde_json::to_value(result).expect("every result serializes")
}

fn file_kind() -> EntryKind {
    EntryKind::File
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NameAndKind {
    name: String,
    kind: EntryKind,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SeriesNameInput {
    name_or_path: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SeasonFolderInput {
    name: String,
    parent_name: Option<String>,
    #[serde(default)]
    parent_is_series_folder: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EpisodeNameInput {
    path: String,
    #[serde(default = "file_kind")]
    kind: EntryKind,
    season_folder_number: Option<u16>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Text {
    text: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Paths {
    paths: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SidecarInput {
    name: String,
    video_name: String,
    kind: SidecarKind,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtraInput {
    path: String,
    kind: EntryKind,
    library: Option<LibraryType>,
    #[serde(default)]
    context_paths: Vec<String>,
    #[serde(default)]
    ignore_files: Vec<IgnoreFile>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtworkInput {
    path: String,
    video_path: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NfoInput {
    path: String,
    #[serde(default)]
    video_paths: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FileTypeInput {
    path: String,
    #[serde(default)]
    sibling_paths: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct IgnoreInput {
    path: String,
    kind: EntryKind,
    #[serde(default)]
    ignore_files: Vec<IgnoreFile>,
}

/// A library listing with its ignore files and caller inputs, and the profile, reference
/// metadata and scope group of the operations that read them.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Tree {
    library: LibraryType,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    ignore_files: Vec<IgnoreFile>,
    #[serde(default)]
    inputs: CallerInputs,
    #[serde(default)]
    profile: OutputProfile,
    #[serde(default)]
    reference: ReferenceMetadata,
    /// The scope group to interpret (X23). Unset for the whole listing.
    scope_group: Option<ScopeGroupInput>,
}

impl Tree {
    /// Interprets the listing, or only its scope group if the input names one.
    fn interpret(&self, rules: &Rules) -> Interpretation {
        match &self.scope_group {
            Some(group) => rules.interpret_scopes(
                self.library,
                &group.scopes(),
                &self.paths,
                &self.ignore_files,
                &self.inputs,
            ),
            None => {
                rules.interpret_with(self.library, &self.paths, &self.ignore_files, &self.inputs)
            }
        }
    }
}

/// A scope group: the files at the library root, the folders at the root, or both.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeGroupInput {
    #[serde(default)]
    root: bool,
    #[serde(default)]
    folders: Vec<String>,
}

impl ScopeGroupInput {
    fn scopes(&self) -> ScopeGroup {
        let mut scopes: ScopeGroup = self.folders.iter().cloned().map(Some).collect();
        if self.root {
            scopes.insert(None);
        }
        scopes
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RenderInput {
    facts: NamingFacts,
    #[serde(default)]
    profile: OutputProfile,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateInput {
    template: String,
    #[serde(default)]
    facts: TemplateFacts,
    name: Option<String>,
}

#[derive(Serialize)]
struct TemplateOutcome {
    error: Option<TemplateError>,
    facts: Option<crate::model::Finding<TemplateFacts>>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileInput {
    profile: Value,
}

#[derive(Serialize)]
struct CheckedProfile {
    invalid: bool,
    diagnostics: Vec<ProfileDiagnostic>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ConversionInput {
    library: LibraryType,
    paths: Vec<String>,
    #[serde(default)]
    ignore_files: Vec<IgnoreFile>,
    #[serde(default)]
    inputs: CallerInputs,
    #[serde(default)]
    from_profile: OutputProfile,
    #[serde(default)]
    to_profile: OutputProfile,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportInput {
    library: LibraryType,
    /// The download folder's paths.
    paths: Vec<String>,
    #[serde(default)]
    ignore_files: Vec<IgnoreFile>,
    #[serde(default)]
    inputs: CallerInputs,
    library_paths: Vec<String>,
    #[serde(default)]
    library_ignore_files: Vec<IgnoreFile>,
    #[serde(default)]
    profile: OutputProfile,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RankCandidatesInput {
    facts: MatchFacts,
    #[serde(default)]
    candidates: Vec<MatchCandidate>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RankBatchInput {
    queries: Vec<RankQuery>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchQueriesInput {
    facts: MatchFacts,
}

/// Overlay documents and declared languages. No declared languages select every language.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RulesInput {
    #[serde(default)]
    overlays: Vec<String>,
    #[serde(default)]
    languages: Vec<String>,
}

impl RulesInput {
    fn build(&self) -> Result<Rules, OperationError> {
        Rules::from_documents(&self.languages, &self.overlays)
            .map_err(OperationError::RejectedRules)
    }
}

#[derive(Serialize)]
struct BuiltRules {
    identity: Option<RulesIdentity>,
    diagnostics: Vec<OverlayDiagnostic>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DiffInput {
    library: LibraryType,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    ignore_files: Vec<IgnoreFile>,
    #[serde(default)]
    inputs: CallerInputs,
    a: RulesInput,
    b: RulesInput,
}
