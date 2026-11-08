//! Implements the gRPC service. Each call reads its queries through the wire layer,
//! calls the library once per query, and returns the results with the caller's handles,
//! in request order. `RankCandidates` returns its results sorted by handle (P9).

use crate::proto::nomenclator_server::Nomenclator;
use crate::proto::{
    EvaluateIgnoreRequest, EvaluateIgnoreResponse, GetVersionRequest, GetVersionResponse,
    GroupPartsRequest, GroupPartsResponse, InterpretRequest, InterpretResponse,
    InterpretStreamHeader, InterpretStreamRequest, ParseNamesRequest, ParseNamesResponse,
    PlanConversionRequest, PlanImportRequest, PlanImportResponse, PlanRenamesRequest,
    PlanRenamesResponse, RankCandidatesRequest, RankCandidatesResponse, RenderNamesRequest,
    RenderNamesResponse, SearchQueriesRequest, SearchQueriesResponse, SuggestLibraryTypeRequest,
    SuggestLibraryTypeResponse, ValidateRequest, ValidateResponse,
};
use crate::{RULES_VERSION, WIRE_CONTRACT, wire};
use nomenclator::model::{EntryKind, SidecarKind};
use nomenclator::{
    CallerInputs, IgnoreFile, LibraryType, MatchFacts, NamingFacts, OutputProfile, RankQuery,
    ReferenceMetadata, Rules, ScopeGroup,
};
use std::borrow::Cow;
use tonic::Streaming;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use tonic::{Request, Response, Status};

/// The naming service. It keeps no state, because every call contains all the context
/// that it needs.
#[derive(Debug, Default, Clone, Copy)]
pub struct NamingService;

#[derive(Deserialize)]
struct ParseNameQuery {
    handle: String,
    #[serde(flatten)]
    query: NameQuery,
}

/// A query about one name. The variant names are the fields of the contract's `query`
/// oneof.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum NameQuery {
    MovieName {
        name: String,
        kind: EntryKind,
    },
    SeriesName {
        name_or_path: String,
    },
    SeasonFolder {
        name: String,
        parent_name: Option<String>,
        parent_is_series_folder: bool,
    },
    EpisodeName {
        path: String,
        kind: EntryKind,
        season_folder_number: Option<u16>,
    },
    ProviderIds {
        text: String,
    },
    Sidecar {
        name: String,
        video_name: String,
        kind: SidecarKind,
    },
    Extra {
        path: String,
        kind: EntryKind,
        library: Option<LibraryType>,
        context_paths: Vec<String>,
    },
    Artwork {
        path: String,
        video_path: Option<String>,
    },
    Nfo {
        path: String,
        video_paths: Vec<String>,
    },
    VideoFile {
        path: String,
    },
    AudioFile {
        path: String,
    },
    ImageFile {
        path: String,
    },
    SubtitleFile {
        path: String,
        sibling_paths: Vec<String>,
    },
    Release {
        name: String,
        kind: EntryKind,
    },
    ReleaseJunk {
        path: String,
    },
}

impl NameQuery {
    /// Answers the query. Returns the name of the field in the contract's `result` oneof
    /// and the result for that field.
    fn answer(self, rules: &Rules) -> Result<(&'static str, Json), Status> {
        Ok(match self {
            NameQuery::MovieName { name, kind } => {
                ("movie_name", to_json(rules.parse_movie_name(&name, kind))?)
            }
            NameQuery::SeriesName { name_or_path } => (
                "series_name",
                to_json(rules.parse_series_name(&name_or_path))?,
            ),
            NameQuery::SeasonFolder {
                name,
                parent_name,
                parent_is_series_folder,
            } => (
                "season_folder",
                to_json(rules.parse_season_folder(
                    &name,
                    parent_name.as_deref(),
                    parent_is_series_folder,
                ))?,
            ),
            NameQuery::EpisodeName {
                path,
                kind,
                season_folder_number,
            } => (
                "episode_name",
                to_json(rules.parse_episode_name(&path, kind, season_folder_number))?,
            ),
            NameQuery::ProviderIds { text } => {
                ("provider_ids", to_json(rules.parse_provider_ids(&text))?)
            }
            NameQuery::Sidecar {
                name,
                video_name,
                kind,
            } => (
                "sidecar",
                to_json(rules.parse_sidecar(&name, &video_name, kind))?,
            ),
            NameQuery::Extra {
                path,
                kind,
                library,
                context_paths,
            } => (
                "extra",
                to_json(rules.classify_extra(&path, kind, library, &context_paths))?,
            ),
            NameQuery::Artwork { path, video_path } => (
                "artwork",
                to_json(rules.classify_artwork(&path, video_path.as_deref()))?,
            ),
            NameQuery::Nfo { path, video_paths } => {
                ("nfo", to_json(rules.classify_nfo(&path, &video_paths))?)
            }
            NameQuery::VideoFile { path } => ("video_file", to_json(rules.is_video_file(&path))?),
            NameQuery::AudioFile { path } => ("audio_file", to_json(rules.is_audio_file(&path))?),
            NameQuery::ImageFile { path } => ("image_file", to_json(rules.is_image_file(&path))?),
            NameQuery::SubtitleFile {
                path,
                sibling_paths,
            } => (
                "subtitle_file",
                to_json(rules.is_subtitle_file(&path, &sibling_paths))?,
            ),
            NameQuery::ReleaseJunk { path } => {
                ("release_junk", to_json(rules.is_release_junk(&path))?)
            }
            NameQuery::Release { name, kind } => {
                ("release", to_json(rules.parse_release(&name, kind))?)
            }
        })
    }
}

#[derive(Deserialize)]
struct GroupPartsQuery {
    handle: String,
    paths: Vec<String>,
}

#[derive(Deserialize)]
struct SuggestLibraryTypeQuery {
    handle: String,
    paths: Vec<String>,
}

#[derive(Deserialize)]
struct EvaluateIgnoreQuery {
    handle: String,
    path: String,
    kind: EntryKind,
}

#[derive(Deserialize)]
struct LibraryTreeQuery {
    handle: String,
    library: LibraryType,
    paths: Vec<String>,
    ignore_files: Vec<IgnoreFile>,
    #[serde(default)]
    inputs: CallerInputs,
    #[serde(default)]
    reference: ReferenceMetadata,
    #[serde(default)]
    scope_group: Option<WireScopeGroup>,
}

/// Interprets a tree, or only its scope group if the query names one (X23).
fn interpret_tree(
    rules: &Rules,
    library: LibraryType,
    scope_group: Option<&WireScopeGroup>,
    paths: &[String],
    ignore_files: &[IgnoreFile],
    inputs: &CallerInputs,
) -> nomenclator::Interpretation {
    match scope_group {
        Some(group) => {
            rules.interpret_scopes(library, &group.scopes(), paths, ignore_files, inputs)
        }
        None => rules.interpret_with(library, paths, ignore_files, inputs),
    }
}

/// The header of an `InterpretStream` call.
#[derive(Deserialize)]
struct StreamHeader {
    handle: String,
    library: LibraryType,
    #[serde(default)]
    ignore_files: Vec<IgnoreFile>,
    #[serde(default)]
    inputs: CallerInputs,
    #[serde(default)]
    scope_group: Option<WireScopeGroup>,
}

/// The wire form of a scope group (X23).
#[derive(Deserialize)]
struct WireScopeGroup {
    #[serde(default)]
    root: bool,
    #[serde(default)]
    folders: Vec<String>,
}

impl WireScopeGroup {
    fn scopes(&self) -> ScopeGroup {
        let mut scopes: ScopeGroup = self.folders.iter().cloned().map(Some).collect();
        if self.root {
            scopes.insert(None);
        }
        scopes
    }
}

#[derive(Deserialize)]
struct ImportQuery {
    handle: String,
    library: LibraryType,
    paths: Vec<String>,
    ignore_files: Vec<IgnoreFile>,
    #[serde(default)]
    inputs: CallerInputs,
    library_paths: Vec<String>,
    library_ignore_files: Vec<IgnoreFile>,
}

#[derive(Deserialize)]
struct RenderNamesQuery {
    handle: String,
    facts: NamingFacts,
}

#[derive(Deserialize)]
struct SearchQueriesQuery {
    handle: String,
    facts: MatchFacts,
}

#[tonic::async_trait]
impl Nomenclator for NamingService {
    async fn get_version(
        &self,
        _request: Request<GetVersionRequest>,
    ) -> Result<Response<GetVersionResponse>, Status> {
        Ok(Response::new(GetVersionResponse {
            wire_contract: WIRE_CONTRACT.to_owned(),
            rules_version: RULES_VERSION.to_owned(),
            rules: Some(rules_identity(Rules::builtin())),
            implementation_version: env!("CARGO_PKG_VERSION").to_owned(),
        }))
    }

    async fn parse_names(
        &self,
        request: Request<ParseNamesRequest>,
    ) -> Result<Response<ParseNamesResponse>, Status> {
        let request = read_request(request.get_ref())?;
        let rules = request_rules(&request)?;
        let rules = rules.as_ref();
        let mut results = Vec::new();
        for ParseNameQuery { handle, query } in read_queries(&request)? {
            let (operation, result) = query.answer(rules)?;
            results.push(serde_json::json!({ "handle": handle, operation: result }));
        }
        respond(rules, results)
    }

    async fn group_parts(
        &self,
        request: Request<GroupPartsRequest>,
    ) -> Result<Response<GroupPartsResponse>, Status> {
        let request = read_request(request.get_ref())?;
        let rules = request_rules(&request)?;
        let rules = rules.as_ref();
        let results = read_queries(&request)?
            .into_iter()
            .map(|GroupPartsQuery { handle, paths }| with_handle(handle, rules.group_parts(&paths)))
            .collect::<Result<_, _>>()?;
        respond(rules, results)
    }

    async fn evaluate_ignore(
        &self,
        request: Request<EvaluateIgnoreRequest>,
    ) -> Result<Response<EvaluateIgnoreResponse>, Status> {
        let request = read_request(request.get_ref())?;
        let rules = request_rules(&request)?;
        let rules = rules.as_ref();
        let ignore_files: Vec<IgnoreFile> = read_field(&request, "ignore_files")?;
        let ignore_rules = rules.ignore_rules(&ignore_files);
        let results = read_queries(&request)?
            .into_iter()
            .map(|EvaluateIgnoreQuery { handle, path, kind }| {
                with_handle(handle, ignore_rules.evaluate(&path, kind))
            })
            .collect::<Result<_, _>>()?;
        respond(rules, results)
    }

    async fn interpret(
        &self,
        request: Request<InterpretRequest>,
    ) -> Result<Response<InterpretResponse>, Status> {
        let request = read_request(request.get_ref())?;
        let rules = request_rules(&request)?;
        let rules = rules.as_ref();
        let results = read_queries(&request)?
            .into_iter()
            .map(|tree: LibraryTreeQuery| {
                let interpretation = interpret_tree(
                    rules,
                    tree.library,
                    tree.scope_group.as_ref(),
                    &tree.paths,
                    &tree.ignore_files,
                    &tree.inputs,
                );
                with_handle(tree.handle, interpretation)
            })
            .collect::<Result<_, _>>()?;
        respond(rules, results)
    }

    async fn interpret_stream(
        &self,
        request: Request<Streaming<InterpretStreamRequest>>,
    ) -> Result<Response<InterpretResponse>, Status> {
        let mut stream = request.into_inner();
        let mut header: Option<InterpretStreamHeader> = None;
        let mut paths: Vec<String> = Vec::new();
        while let Some(mut message) = stream.message().await? {
            match (message.header.take(), &header) {
                (Some(_), Some(_)) => {
                    return Err(Status::invalid_argument(
                        "header: a later message has a header. Send the header in the first message only",
                    ));
                }
                (Some(first), None) => header = Some(first),
                (None, None) => {
                    return Err(Status::invalid_argument(
                        "header: the first message has no header. Send the header first",
                    ));
                }
                (None, Some(_)) => {}
            }
            // Moves the batch's strings into the listing, so the message is not kept.
            paths.append(&mut message.paths);
        }
        let header = header.ok_or_else(|| {
            Status::invalid_argument("header: the stream has no message. Send the header first")
        })?;
        let header = read_request(&header)?;
        let rules = request_rules(&header)?;
        let rules = rules.as_ref();
        let tree: StreamHeader = serde_json::from_value(header).map_err(|error| {
            Status::invalid_argument(format!(
                "header: cannot read the header: {error}. Check the wire contract"
            ))
        })?;
        let interpretation = interpret_tree(
            rules,
            tree.library,
            tree.scope_group.as_ref(),
            &paths,
            &tree.ignore_files,
            &tree.inputs,
        );
        respond(rules, vec![with_handle(tree.handle, interpretation)?])
    }

    async fn rank_candidates(
        &self,
        request: Request<RankCandidatesRequest>,
    ) -> Result<Response<RankCandidatesResponse>, Status> {
        let request = read_request(request.get_ref())?;
        let rules = request_rules(&request)?;
        let rules = rules.as_ref();
        let queries: Vec<RankQuery> = read_queries(&request)?;
        let results = rules
            .rank_batch(&queries)
            .results
            .into_iter()
            .map(|result| with_handle(result.handle, result.ranking))
            .collect::<Result<_, _>>()?;
        respond(rules, results)
    }

    async fn search_queries(
        &self,
        request: Request<SearchQueriesRequest>,
    ) -> Result<Response<SearchQueriesResponse>, Status> {
        let request = read_request(request.get_ref())?;
        let rules = request_rules(&request)?;
        let rules = rules.as_ref();
        let results = read_queries(&request)?
            .into_iter()
            .map(|query: SearchQueriesQuery| {
                with_handle(query.handle, rules.search_queries(&query.facts))
            })
            .collect::<Result<_, _>>()?;
        respond(rules, results)
    }

    async fn render_names(
        &self,
        request: Request<RenderNamesRequest>,
    ) -> Result<Response<RenderNamesResponse>, Status> {
        let request = read_request(request.get_ref())?;
        let rules = request_rules(&request)?;
        let rules = rules.as_ref();
        let profile = read_profile(&request)?;
        let results = read_queries(&request)?
            .into_iter()
            .map(|RenderNamesQuery { handle, facts }| {
                with_handle(handle, rules.render_names(&facts, &profile))
            })
            .collect::<Result<_, _>>()?;
        respond(rules, results)
    }

    async fn plan_renames(
        &self,
        request: Request<PlanRenamesRequest>,
    ) -> Result<Response<PlanRenamesResponse>, Status> {
        let request = read_request(request.get_ref())?;
        let rules = request_rules(&request)?;
        let rules = rules.as_ref();
        let profile = read_profile(&request)?;
        let results = read_queries(&request)?
            .into_iter()
            .map(|tree: LibraryTreeQuery| {
                let interpretation = rules.interpret_with(
                    tree.library,
                    &tree.paths,
                    &tree.ignore_files,
                    &tree.inputs,
                );
                let plan = rules.plan_renames(&interpretation, &tree.paths, &profile);
                with_handle(tree.handle, plan)
            })
            .collect::<Result<_, _>>()?;
        respond(rules, results)
    }

    async fn plan_conversion(
        &self,
        request: Request<PlanConversionRequest>,
    ) -> Result<Response<PlanRenamesResponse>, Status> {
        let request = read_request(request.get_ref())?;
        let rules = request_rules(&request)?;
        let rules = rules.as_ref();
        let from: OutputProfile = read_field(&request, "from_profile")?;
        let to: OutputProfile = read_field(&request, "to_profile")?;
        let mut identity = rules.identity().clone();
        let mut results = Vec::new();
        for tree in read_queries::<LibraryTreeQuery>(&request)? {
            let plan = rules
                .plan_conversion(
                    tree.library,
                    &tree.paths,
                    &tree.ignore_files,
                    &tree.inputs,
                    &from,
                    &to,
                )
                .map_err(|diagnostics| {
                    Status::invalid_argument(format!(
                        "from_profile: the rules cannot read the profile's folder names: {}. Use other folder names",
                        serde_json::to_string(&diagnostics).unwrap_or_default()
                    ))
                })?;
            // The plan states the rules that read the source profile's names.
            identity = plan.rules.clone();
            results.push(with_handle(tree.handle, plan)?);
        }
        respond_with_identity(&identity, results)
    }

    async fn plan_import(
        &self,
        request: Request<PlanImportRequest>,
    ) -> Result<Response<PlanImportResponse>, Status> {
        let request = read_request(request.get_ref())?;
        let rules = request_rules(&request)?;
        let rules = rules.as_ref();
        let profile = read_profile(&request)?;
        let results = read_queries(&request)?
            .into_iter()
            .map(|query: ImportQuery| {
                let download = rules.interpret_with(
                    query.library,
                    &query.paths,
                    &query.ignore_files,
                    &query.inputs,
                );
                let library = rules.interpret(
                    query.library,
                    &query.library_paths,
                    &query.library_ignore_files,
                );
                let plan = rules.plan_import(&download, &library, &query.library_paths, &profile);
                with_handle(query.handle, plan)
            })
            .collect::<Result<_, _>>()?;
        respond(rules, results)
    }

    async fn suggest_library_type(
        &self,
        request: Request<SuggestLibraryTypeRequest>,
    ) -> Result<Response<SuggestLibraryTypeResponse>, Status> {
        let request = read_request(request.get_ref())?;
        let rules = request_rules(&request)?;
        let rules = rules.as_ref();
        let results = read_queries(&request)?
            .into_iter()
            .map(|SuggestLibraryTypeQuery { handle, paths }| {
                with_handle(handle, rules.suggest_library_type(&paths))
            })
            .collect::<Result<_, _>>()?;
        respond(rules, results)
    }

    async fn validate(
        &self,
        request: Request<ValidateRequest>,
    ) -> Result<Response<ValidateResponse>, Status> {
        let request = read_request(request.get_ref())?;
        let rules = request_rules(&request)?;
        let rules = rules.as_ref();
        let profile = read_profile(&request)?;
        let results = read_queries(&request)?
            .into_iter()
            .map(|tree: LibraryTreeQuery| {
                let interpretation = rules.interpret_with(
                    tree.library,
                    &tree.paths,
                    &tree.ignore_files,
                    &tree.inputs,
                );
                with_handle(
                    tree.handle,
                    rules.validate_with(&interpretation, &profile, &tree.reference),
                )
            })
            .collect::<Result<_, _>>()?;
        respond(rules, results)
    }
}

/// Converts the request to the library's serialized shape. Leaves out unset fields so
/// that their defaults apply.
fn read_request<M: prost::Message + prost::Name>(request: &M) -> Result<Json, Status> {
    let json =
        wire::message_to_json(request).map_err(|error| Status::internal(error.to_string()))?;
    Ok(without_nulls(json))
}

#[derive(Deserialize, Default)]
struct RulesSelection {
    #[serde(default)]
    overlays: Vec<String>,
    #[serde(default)]
    languages: Vec<String>,
}

/// Builds the rules that a request is answered under: the built-in rules, or the rules
/// of its declared languages, with its overlays applied. Fails with every overlay
/// diagnostic if the rules cannot be built.
fn request_rules(request: &Json) -> Result<Cow<'static, Rules>, Status> {
    let selection: RulesSelection = read_field(request, "rules")?;
    if selection.overlays.is_empty() && selection.languages.is_empty() {
        return Ok(Cow::Borrowed(Rules::builtin()));
    }
    let built = Rules::from_documents(&selection.languages, &selection.overlays);
    built.map(Cow::Owned).map_err(|diagnostics| {
        let listed = serde_json::to_string(&diagnostics).unwrap_or_default();
        Status::invalid_argument(format!(
            "rules: cannot build the rules: {listed}. Correct the listed overlays or languages"
        ))
    })
}

fn rules_identity(rules: &Rules) -> crate::proto::RulesIdentity {
    let identity = without_nulls(to_json(rules.identity()).expect("an identity serializes"));
    wire::json_to_message(&identity).expect("an identity fits the wire contract")
}

fn read_queries<Q: DeserializeOwned>(request: &Json) -> Result<Vec<Q>, Status> {
    let queries = request["queries"].as_array().cloned().unwrap_or_default();
    queries
        .into_iter()
        .map(|query| {
            let handle = query["handle"].as_str().unwrap_or_default().to_owned();
            serde_json::from_value(query).map_err(|error| {
                Status::invalid_argument(format!(
                    "query {handle:?}: cannot read the query: {error}. Check the wire contract"
                ))
            })
        })
        .collect()
}

fn read_field<T: DeserializeOwned + Default>(request: &Json, field: &str) -> Result<T, Status> {
    match request.get(field) {
        None => Ok(T::default()),
        Some(value) => serde_json::from_value(value.clone()).map_err(|error| {
            Status::invalid_argument(format!("{field}: cannot read the field: {error}"))
        }),
    }
}

fn read_profile(request: &Json) -> Result<OutputProfile, Status> {
    read_field(request, "profile")
}

/// Returns `result` with its handle. Removes the rules identity, because the response
/// states it once per call.
fn with_handle(handle: String, result: impl Serialize) -> Result<Json, Status> {
    let mut json = to_json(result)?;
    if let Json::Object(fields) = &mut json {
        fields.remove("rules");
    }
    json["handle"] = Json::String(handle);
    Ok(json)
}

fn to_json(result: impl Serialize) -> Result<Json, Status> {
    serde_json::to_value(result).map_err(|error| Status::internal(error.to_string()))
}

fn respond<M: prost::Message + prost::Name + Default>(
    rules: &Rules,
    results: Vec<Json>,
) -> Result<Response<M>, Status> {
    respond_with_identity(rules.identity(), results)
}

fn respond_with_identity<M: prost::Message + prost::Name + Default>(
    identity: &nomenclator::RulesIdentity,
    results: Vec<Json>,
) -> Result<Response<M>, Status> {
    let response = serde_json::json!({ "results": results, "rules": identity });
    wire::json_to_message(&response)
        .map(Response::new)
        .map_err(|error| {
            Status::internal(format!(
                "cannot encode the response: the result is outside the wire contract: {error}"
            ))
        })
}

fn without_nulls(json: Json) -> Json {
    match json {
        Json::Object(fields) => Json::Object(
            fields
                .into_iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(name, value)| (name, without_nulls(value)))
                .collect(),
        ),
        Json::Array(items) => Json::Array(items.into_iter().map(without_nulls).collect()),
        other => other,
    }
}
