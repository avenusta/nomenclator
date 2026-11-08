//! Runs the YAML cases in `corpus/` against the library. See `corpus/README.md` for the format.

use nomenclator::model::{EntryKind, SidecarKind};
use nomenclator::{
    CallerInputs, IgnoreFile, LibraryType, MatchCandidate, MatchFacts, NamingFacts, OutputProfile,
    OverlayDiagnostic, RankQuery, ReferenceMetadata, Rules, RulesIdentity, diff_rules,
};
#[path = "support/cases.rs"]
mod cases;
#[path = "support/expectations.rs"]
mod expectations;

use cases::{CorpusCase, collect_yaml_files, corpus_directory, load_cases};
use expectations::expect_subset;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct ProfileInput {
    profile: Value,
}

#[derive(serde::Serialize)]
struct CheckedProfile {
    invalid: bool,
    diagnostics: Vec<nomenclator::ProfileDiagnostic>,
}

#[derive(Deserialize)]
struct Overlays {
    #[serde(default)]
    overlays: Vec<String>,
    languages: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct RulesInput {
    #[serde(default)]
    overlays: Vec<String>,
    languages: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct DiffInput {
    library: LibraryType,
    paths: Vec<String>,
    #[serde(default)]
    ignore_files: Vec<IgnoreFile>,
    #[serde(default)]
    inputs: CallerInputs,
    a: RulesInput,
    b: RulesInput,
}

#[derive(Deserialize)]
struct TemplateInput {
    template: String,
    #[serde(default)]
    facts: nomenclator::TemplateFacts,
    name: Option<String>,
}

#[derive(serde::Serialize)]
struct TemplateOutcome {
    error: Option<nomenclator::TemplateError>,
    facts: Option<nomenclator::model::Finding<nomenclator::TemplateFacts>>,
}

#[derive(serde::Serialize)]
struct BuiltRules {
    identity: Option<RulesIdentity>,
    diagnostics: Vec<OverlayDiagnostic>,
}

#[derive(Deserialize)]
struct Text {
    text: String,
}

#[derive(Deserialize)]
struct Paths {
    paths: Vec<String>,
}

#[derive(Deserialize)]
struct NameAndKind {
    name: String,
    kind: EntryKind,
}

#[derive(Deserialize)]
struct SidecarInput {
    name: String,
    video_name: String,
    kind: SidecarKind,
}

#[derive(Deserialize)]
struct ExtraInput {
    path: String,
    kind: EntryKind,
    library: Option<LibraryType>,
    #[serde(default)]
    context_paths: Vec<String>,
    #[serde(default)]
    ignore_files: Vec<IgnoreFile>,
}

#[derive(Deserialize)]
struct InterpretInput {
    library: LibraryType,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    ignore_files: Vec<IgnoreFile>,
    #[serde(default)]
    inputs: CallerInputs,
}

#[derive(Deserialize)]
struct ArtworkInput {
    name: Option<String>,
    path: Option<String>,
    video_path: Option<String>,
}

#[derive(Deserialize)]
struct NfoInput {
    path: String,
    #[serde(default)]
    video_paths: Vec<String>,
}

#[derive(Deserialize)]
struct FileTypeInput {
    path: String,
    #[serde(default)]
    sibling_paths: Vec<String>,
}

#[derive(Deserialize)]
struct IgnoreInput {
    path: String,
    kind: EntryKind,
    #[serde(default)]
    ignore_files: Vec<IgnoreFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SeriesNameInput {
    name: Option<String>,
    path: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SeasonFolderInput {
    name: String,
    parent_name: Option<String>,
    #[serde(default)]
    parent_is_series_folder: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EpisodeNameInput {
    path: String,
    #[serde(default = "file_kind")]
    kind: EntryKind,
    season_folder_number: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenderInput {
    facts: NamingFacts,
    #[serde(default)]
    profile: OutputProfile,
}

/// The input of a case that interprets a tree, then plans or validates it under a
/// profile.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfiledTreeInput {
    library: LibraryType,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    ignore_files: Vec<IgnoreFile>,
    #[serde(default)]
    profile: OutputProfile,
    #[serde(default)]
    inputs: CallerInputs,
    #[serde(default)]
    reference: ReferenceMetadata,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConversionInput {
    library: LibraryType,
    paths: Vec<String>,
    #[serde(default)]
    ignore_files: Vec<IgnoreFile>,
    #[serde(default)]
    from_profile: OutputProfile,
    #[serde(default)]
    to_profile: OutputProfile,
    #[serde(default)]
    inputs: CallerInputs,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportInput {
    library: LibraryType,
    /// The download folder's paths.
    paths: Vec<String>,
    #[serde(default)]
    ignore_files: Vec<IgnoreFile>,
    library_paths: Vec<String>,
    #[serde(default)]
    profile: OutputProfile,
    #[serde(default)]
    inputs: CallerInputs,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RankBatchInput {
    queries: Vec<RankQuery>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchQueriesInput {
    facts: MatchFacts,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RankCandidatesInput {
    facts: MatchFacts,
    #[serde(default)]
    candidates: Vec<MatchCandidate>,
}

fn file_kind() -> EntryKind {
    EntryKind::File
}

/// Fails if a non-pending case fails or if a pending case now passes. `just promote`
/// removes the pending mark from cases that pass. Prints the pending progress per file.
#[test]
fn corpus_cases_match_expected_facts() {
    let promote = std::env::var_os("PROMOTE").is_some();
    let show_pending_in = std::env::var("PENDING").ok();
    let mut failures = Vec::new();
    let mut now_passing: BTreeMap<PathBuf, Vec<usize>> = BTreeMap::new();
    let mut pending_passed_and_total: BTreeMap<PathBuf, (usize, usize)> = BTreeMap::new();
    let rules = Rules::builtin();

    for file in collect_yaml_files(&corpus_directory()) {
        for (index, case) in load_cases(&file).into_iter().enumerate() {
            let outcome = case_rules(&case)
                .and_then(|case_rules| run_case(case_rules.as_ref().unwrap_or(rules), &case));
            if !case.pending {
                if let Err(reason) = outcome {
                    failures.push(format!("{} › {}: {reason}", file.display(), case.name));
                }
                continue;
            }
            let counts = pending_passed_and_total.entry(file.clone()).or_default();
            counts.1 += 1;
            match outcome {
                Ok(()) => {
                    counts.0 += 1;
                    now_passing.entry(file.clone()).or_default().push(index);
                }
                Err(reason)
                    if show_pending_in
                        .as_deref()
                        .is_some_and(|wanted| file.to_string_lossy().contains(wanted)) =>
                {
                    println!("PENDING {} › {}: {reason}", file.display(), case.name);
                }
                Err(_) => {}
            }
        }
    }

    for (file, (passed, total)) in &pending_passed_and_total {
        let relative = file.strip_prefix(corpus_directory()).unwrap_or(file);
        println!(
            "{passed:>5}/{total:<5} pending now pass  {}",
            relative.display()
        );
    }
    if promote {
        for (file, indexes) in &now_passing {
            remove_pending_marks(file, indexes);
        }
        now_passing.clear();
    }
    let promotable: usize = now_passing.values().map(Vec::len).sum();
    assert!(
        failures.is_empty(),
        "{} failing cases:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        promotable == 0,
        "{promotable} pending cases now pass; run `just promote` to remove their mark"
    );
}

/// Builds the rules from a case's overlays and languages. Returns `None` if the case
/// uses the built-in rules.
fn case_rules(case: &CorpusCase) -> Result<Option<Rules>, String> {
    if case.uses_builtin_rules() {
        return Ok(None);
    }
    build_rules(case.languages.as_deref(), &case.overlays)
        .map(Some)
        .map_err(|diagnostics| format!("overlays rejected: {diagnostics:?}"))
}

/// Builds the rules of a case's languages and overlay documents. A case without
/// `languages` declares none, which selects every language.
fn build_rules(
    languages: Option<&[String]>,
    documents: &[String],
) -> Result<Rules, Vec<OverlayDiagnostic>> {
    Rules::from_documents(languages.unwrap_or_default(), documents)
}

fn run_case(rules: &Rules, case: &CorpusCase) -> Result<(), String> {
    let actual = match case.operation.as_str() {
        "parse_movie_name" => {
            let input: NameAndKind = parse_input(&case.input)?;
            to_value(rules.parse_movie_name(&input.name, input.kind))
        }
        "check_profile" => {
            let input: ProfileInput = parse_input(&case.input)?;
            match serde_json::from_value::<OutputProfile>(input.profile) {
                Ok(profile) => to_value(CheckedProfile {
                    invalid: false,
                    diagnostics: rules.check_profile(&profile),
                }),
                Err(_) => to_value(CheckedProfile {
                    invalid: true,
                    diagnostics: Vec::new(),
                }),
            }
        }
        "render_template" => {
            let input: TemplateInput = parse_input(&case.input)?;
            match nomenclator::Template::parse(&input.template) {
                Ok(template) => to_value(template.render(&input.facts)),
                Err(error) => to_value(TemplateOutcome {
                    error: Some(error),
                    facts: None,
                }),
            }
        }
        "read_template" => {
            let input: TemplateInput = parse_input(&case.input)?;
            match nomenclator::Template::parse(&input.template) {
                Ok(template) => to_value(TemplateOutcome {
                    error: None,
                    facts: Some(template.read(input.name.as_deref().unwrap_or_default())),
                }),
                Err(error) => to_value(TemplateOutcome {
                    error: Some(error),
                    facts: None,
                }),
            }
        }
        "suggest_library_type" => {
            let input: Paths = parse_input(&case.input)?;
            to_value(rules.suggest_library_type(&input.paths))
        }
        "parse_release" => {
            let input: NameAndKind = parse_input(&case.input)?;
            to_value(rules.parse_release(&input.name, input.kind))
        }
        "parse_provider_ids" => {
            let input: Text = parse_input(&case.input)?;
            to_value(rules.parse_provider_ids(&input.text))
        }
        "parse_sidecar" => {
            let input: SidecarInput = parse_input(&case.input)?;
            to_value(rules.parse_sidecar(&input.name, &input.video_name, input.kind))
        }
        "group_parts" => {
            let input: Paths = parse_input(&case.input)?;
            to_value(rules.group_parts(&input.paths))
        }
        "classify_extra" => {
            let input: ExtraInput = parse_input(&case.input)?;
            // Keeps only the entries that the ignore rules keep, as a caller would.
            let in_scope_context: Vec<String> = input
                .context_paths
                .into_iter()
                .filter(|path| {
                    rules
                        .evaluate_ignore(path, EntryKind::File, &input.ignore_files)
                        .excluded
                        == Some(false)
                })
                .collect();
            to_value(rules.classify_extra(
                &input.path,
                input.kind,
                input.library,
                &in_scope_context,
            ))
        }
        "classify_artwork" => {
            let input: ArtworkInput = parse_input(&case.input)?;
            let path = input
                .path
                .or(input.name)
                .ok_or("input: needs `name` or `path`")?;
            to_value(rules.classify_artwork(&path, input.video_path.as_deref()))
        }
        "classify_nfo" => {
            let input: NfoInput = parse_input(&case.input)?;
            to_value(rules.classify_nfo(&input.path, &input.video_paths))
        }
        "is_video_file" => {
            to_value(rules.is_video_file(&parse_input::<FileTypeInput>(&case.input)?.path))
        }
        "is_release_junk" => {
            let input: FileTypeInput = parse_input(&case.input)?;
            to_value(rules.is_release_junk(&input.path))
        }
        "is_audio_file" => {
            to_value(rules.is_audio_file(&parse_input::<FileTypeInput>(&case.input)?.path))
        }
        "is_image_file" => {
            to_value(rules.is_image_file(&parse_input::<FileTypeInput>(&case.input)?.path))
        }
        "is_subtitle_file" => {
            let input: FileTypeInput = parse_input(&case.input)?;
            to_value(rules.is_subtitle_file(&input.path, &input.sibling_paths))
        }
        "evaluate_ignore" => {
            let input: IgnoreInput = parse_input(&case.input)?;
            to_value(rules.evaluate_ignore(&input.path, input.kind, &input.ignore_files))
        }
        "parse_series_name" => {
            let input: SeriesNameInput = parse_input(&case.input)?;
            let name_or_path = input.name.or(input.path).ok_or("input: name or path")?;
            to_value(rules.parse_series_name(&name_or_path))
        }
        "parse_season_folder" => {
            let input: SeasonFolderInput = parse_input(&case.input)?;
            to_value(rules.parse_season_folder(
                &input.name,
                input.parent_name.as_deref(),
                input.parent_is_series_folder,
            ))
        }
        "parse_episode_name" => {
            let input: EpisodeNameInput = parse_input(&case.input)?;
            to_value(rules.parse_episode_name(&input.path, input.kind, input.season_folder_number))
        }
        "interpret" => {
            let input: InterpretInput = parse_input(&case.input)?;
            to_value(rules.interpret_with(
                input.library,
                &input.paths,
                &input.ignore_files,
                &input.inputs,
            ))
        }
        "render_names" => {
            let input: RenderInput = parse_input(&case.input)?;
            to_value(rules.render_names(&input.facts, &input.profile))
        }
        "plan_renames" => {
            let input: ProfiledTreeInput = parse_input(&case.input)?;
            let interpretation = rules.interpret_with(
                input.library,
                &input.paths,
                &input.ignore_files,
                &input.inputs,
            );
            to_value(rules.plan_renames(&interpretation, &input.paths, &input.profile))
        }
        "convert_layout" => {
            let input: ConversionInput = parse_input(&case.input)?;
            let plan = rules
                .plan_conversion(
                    input.library,
                    &input.paths,
                    &input.ignore_files,
                    &input.inputs,
                    &input.from_profile,
                    &input.to_profile,
                )
                .map_err(|diagnostics| format!("profile rejected: {diagnostics:?}"))?;
            to_value(plan)
        }
        "plan_import" => {
            let input: ImportInput = parse_input(&case.input)?;
            let download = rules.interpret_with(
                input.library,
                &input.paths,
                &input.ignore_files,
                &input.inputs,
            );
            let library = rules.interpret(input.library, &input.library_paths, &[]);
            to_value(rules.plan_import(&download, &library, &input.library_paths, &input.profile))
        }
        "validate" => {
            let input: ProfiledTreeInput = parse_input(&case.input)?;
            let interpretation = rules.interpret_with(
                input.library,
                &input.paths,
                &input.ignore_files,
                &input.inputs,
            );
            to_value(rules.validate_with(&interpretation, &input.profile, &input.reference))
        }
        "end_to_end" => {
            let input: ProfiledTreeInput = parse_input(&case.input)?;
            let interpretation = rules.interpret_with(
                input.library,
                &input.paths,
                &input.ignore_files,
                &input.inputs,
            );
            let plan = rules.plan_renames(&interpretation, &input.paths, &input.profile);
            let validation = rules.validate_with(&interpretation, &input.profile, &input.reference);
            let mut stages = serde_json::Map::new();
            stages.insert("interpretation".into(), to_value(&interpretation));
            stages.insert("plan".into(), to_value(&plan));
            stages.insert("validation".into(), to_value(&validation));
            Value::Object(stages)
        }
        "diff_rules" => {
            let input: DiffInput = parse_input(&case.input)?;
            let side = |side: &RulesInput| {
                build_rules(side.languages.as_deref(), &side.overlays)
                    .map_err(|diagnostics| format!("rules rejected: {diagnostics:?}"))
            };
            let (a, b) = (side(&input.a)?, side(&input.b)?);
            to_value(diff_rules(
                &a,
                &b,
                input.library,
                &input.paths,
                &input.ignore_files,
                &input.inputs,
            ))
        }
        "build_rules" => {
            let input: Overlays = parse_input(&case.input)?;
            let (identity, diagnostics) =
                match build_rules(input.languages.as_deref(), &input.overlays) {
                    Ok(rules) => (Some(rules.identity().clone()), Vec::new()),
                    Err(diagnostics) => (None, diagnostics),
                };
            to_value(BuiltRules {
                identity,
                diagnostics,
            })
        }
        "rank_candidates" => {
            let input: RankCandidatesInput = parse_input(&case.input)?;
            to_value(rules.rank_candidates(&input.facts, &input.candidates))
        }
        "rank_batch" => {
            let input: RankBatchInput = parse_input(&case.input)?;
            to_value(rules.rank_batch(&input.queries))
        }
        "search_queries" => {
            let input: SearchQueriesInput = parse_input(&case.input)?;
            to_value(rules.search_queries(&input.facts))
        }
        "match_facts" => {
            let input: ProfiledTreeInput = parse_input(&case.input)?;
            let interpretation = rules.interpret_with(
                input.library,
                &input.paths,
                &input.ignore_files,
                &input.inputs,
            );
            let mut facts = serde_json::Map::new();
            facts.insert(
                "facts".into(),
                to_value(nomenclator::match_facts(&interpretation.items)),
            );
            Value::Object(facts)
        }
        unknown_operation => return Err(format!("unknown operation `{unknown_operation}`")),
    };
    expect_subset(&case.expect, &actual, "expect")
}

fn parse_input<T: DeserializeOwned>(input: &Value) -> Result<T, String> {
    serde_json::from_value(input.clone()).map_err(|error| format!("input: {error}"))
}

fn to_value(result: impl serde::Serialize) -> Value {
    serde_json::to_value(result).expect("results serialize to YAML")
}

/// Drops `pending: true` from the cases at `indexes`. Each case starts with `- ` at column 0.
fn remove_pending_marks(file: &Path, indexes: &[usize]) {
    let text = std::fs::read_to_string(file).expect("corpus file is readable");
    let mut case_index = None;
    let kept: Vec<&str> = text
        .split_inclusive('\n')
        .filter(|line| {
            if line.starts_with("- ") {
                case_index = Some(case_index.map_or(0, |index| index + 1));
            }
            let is_mark = line.trim_end() == "  pending: true";
            !(is_mark && case_index.is_some_and(|index| indexes.contains(&index)))
        })
        .collect();
    std::fs::write(file, kept.concat()).expect("corpus file is writable");
}
