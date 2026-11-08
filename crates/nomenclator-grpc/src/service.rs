//! Implements the gRPC service. Each call reads its queries through the wire layer, runs
//! the library operation of the call once per query with [`Rules::run`], and returns the
//! results with the caller's handles, in request order. `RankCandidates` returns its
//! results sorted by handle (P9).

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
use nomenclator::{OutputProfile, Rules};
use serde_json::{Map, Value as Json};
use std::borrow::Cow;
use tonic::{Request, Response, Status, Streaming};

/// The naming service. It keeps no state, because every call contains all the context
/// that it needs.
#[derive(Debug, Default, Clone, Copy)]
pub struct NamingService;

/// A call of the contract that answers queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rpc {
    /// `ParseNames`.
    ParseNames,
    /// `GroupParts`.
    GroupParts,
    /// `EvaluateIgnore`.
    EvaluateIgnore,
    /// `Interpret` and `InterpretStream`.
    Interpret,
    /// `RankCandidates`.
    RankCandidates,
    /// `SearchQueries`.
    SearchQueries,
    /// `RenderNames`.
    RenderNames,
    /// `PlanRenames`.
    PlanRenames,
    /// `PlanImport`.
    PlanImport,
    /// `PlanConversion`.
    PlanConversion,
    /// `Validate`.
    Validate,
    /// `SuggestLibraryType`.
    SuggestLibraryType,
}

/// How one library operation is called over the wire.
#[derive(Debug, Clone, Copy)]
pub struct WireCall {
    /// The library operation that [`Rules::run`] runs for each query.
    pub operation: &'static str,
    /// The call that carries the operation.
    pub rpc: Rpc,
    /// The field of a `ParseNames` query and result that selects the operation.
    pub parse_field: Option<&'static str>,
    /// The input fields that a request states once for all its queries.
    pub request_fields: &'static [&'static str],
}

const fn parse(operation: &'static str, field: &'static str) -> WireCall {
    WireCall {
        operation,
        rpc: Rpc::ParseNames,
        parse_field: Some(field),
        request_fields: &[],
    }
}

const fn call(
    operation: &'static str,
    rpc: Rpc,
    request_fields: &'static [&'static str],
) -> WireCall {
    WireCall {
        operation,
        rpc,
        parse_field: None,
        request_fields,
    }
}

/// Every library operation that the contract carries.
pub const WIRE_CALLS: &[WireCall] = &[
    parse("parse_movie_name", "movie_name"),
    parse("parse_series_name", "series_name"),
    parse("parse_season_folder", "season_folder"),
    parse("parse_episode_name", "episode_name"),
    parse("parse_provider_ids", "provider_ids"),
    parse("parse_sidecar", "sidecar"),
    parse("classify_extra", "extra"),
    parse("classify_artwork", "artwork"),
    parse("classify_nfo", "nfo"),
    parse("is_video_file", "video_file"),
    parse("is_audio_file", "audio_file"),
    parse("is_image_file", "image_file"),
    parse("is_subtitle_file", "subtitle_file"),
    parse("parse_release", "release"),
    parse("is_release_junk", "release_junk"),
    call("group_parts", Rpc::GroupParts, &[]),
    call("evaluate_ignore", Rpc::EvaluateIgnore, &["ignore_files"]),
    call("interpret", Rpc::Interpret, &[]),
    call("rank_candidates", Rpc::RankCandidates, &[]),
    call("search_queries", Rpc::SearchQueries, &[]),
    call("render_names", Rpc::RenderNames, &["profile"]),
    call("plan_renames", Rpc::PlanRenames, &["profile"]),
    call("plan_import", Rpc::PlanImport, &["profile"]),
    call(
        "convert_layout",
        Rpc::PlanConversion,
        &["from_profile", "to_profile"],
    ),
    call("validate", Rpc::Validate, &["profile"]),
    call("suggest_library_type", Rpc::SuggestLibraryType, &[]),
];

/// Returns how the library operation `operation` is called over the wire.
/// Returns `None` if the contract has no call for the operation.
#[must_use]
pub fn wire_call(operation: &str) -> Option<&'static WireCall> {
    WIRE_CALLS.iter().find(|call| call.operation == operation)
}

#[tonic::async_trait]
impl Nomenclator for NamingService {
    async fn get_version(
        &self,
        _request: Request<GetVersionRequest>,
    ) -> Result<Response<GetVersionResponse>, Status> {
        let identity = without_nulls(to_json(Rules::builtin().identity()));
        Ok(Response::new(GetVersionResponse {
            wire_contract: WIRE_CONTRACT.to_owned(),
            rules_version: RULES_VERSION.to_owned(),
            rules: Some(
                wire::json_to_message(&identity).expect("an identity fits the wire contract"),
            ),
            implementation_version: env!("CARGO_PKG_VERSION").to_owned(),
        }))
    }

    async fn parse_names(
        &self,
        request: Request<ParseNamesRequest>,
    ) -> Result<Response<ParseNamesResponse>, Status> {
        answer(request.get_ref(), Rpc::ParseNames)
    }

    async fn group_parts(
        &self,
        request: Request<GroupPartsRequest>,
    ) -> Result<Response<GroupPartsResponse>, Status> {
        answer(request.get_ref(), Rpc::GroupParts)
    }

    async fn evaluate_ignore(
        &self,
        request: Request<EvaluateIgnoreRequest>,
    ) -> Result<Response<EvaluateIgnoreResponse>, Status> {
        answer(request.get_ref(), Rpc::EvaluateIgnore)
    }

    async fn interpret(
        &self,
        request: Request<InterpretRequest>,
    ) -> Result<Response<InterpretResponse>, Status> {
        answer(request.get_ref(), Rpc::Interpret)
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
        let mut query = read_request(&header)?;
        let rules = request_rules(&mut query)?;
        query["paths"] = Json::from(paths);
        let result = run_query(rules.as_ref(), &Json::Null, Rpc::Interpret, query)?;
        respond(to_json(rules.identity()), vec![result])
    }

    async fn rank_candidates(
        &self,
        request: Request<RankCandidatesRequest>,
    ) -> Result<Response<RankCandidatesResponse>, Status> {
        let mut request = read_request(request.get_ref())?;
        let rules = request_rules(&mut request)?;
        let input = serde_json::json!({ "queries": request["queries"].take() });
        let ranking = rules.run("rank_batch", input).map_err(invalid)?;
        let results = ranking["results"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|result| with_handle(&result["handle"], result["ranking"].clone()))
            .collect();
        respond(to_json(rules.identity()), results)
    }

    async fn search_queries(
        &self,
        request: Request<SearchQueriesRequest>,
    ) -> Result<Response<SearchQueriesResponse>, Status> {
        answer(request.get_ref(), Rpc::SearchQueries)
    }

    async fn render_names(
        &self,
        request: Request<RenderNamesRequest>,
    ) -> Result<Response<RenderNamesResponse>, Status> {
        answer(request.get_ref(), Rpc::RenderNames)
    }

    async fn plan_renames(
        &self,
        request: Request<PlanRenamesRequest>,
    ) -> Result<Response<PlanRenamesResponse>, Status> {
        answer(request.get_ref(), Rpc::PlanRenames)
    }

    async fn plan_conversion(
        &self,
        request: Request<PlanConversionRequest>,
    ) -> Result<Response<PlanRenamesResponse>, Status> {
        answer(request.get_ref(), Rpc::PlanConversion)
    }

    async fn plan_import(
        &self,
        request: Request<PlanImportRequest>,
    ) -> Result<Response<PlanImportResponse>, Status> {
        answer(request.get_ref(), Rpc::PlanImport)
    }

    async fn suggest_library_type(
        &self,
        request: Request<SuggestLibraryTypeRequest>,
    ) -> Result<Response<SuggestLibraryTypeResponse>, Status> {
        answer(request.get_ref(), Rpc::SuggestLibraryType)
    }

    async fn validate(
        &self,
        request: Request<ValidateRequest>,
    ) -> Result<Response<ValidateResponse>, Status> {
        answer(request.get_ref(), Rpc::Validate)
    }
}

/// Answers every query of a request to `rpc`, under the rules of the request. Fails if
/// the rules or a profile of the request are rejected, or if a query cannot be answered.
fn answer<M, R>(request: &M, rpc: Rpc) -> Result<Response<R>, Status>
where
    M: prost::Message + prost::Name,
    R: prost::Message + prost::Name + Default,
{
    let mut request = read_request(request)?;
    let rules = request_rules(&mut request)?;
    check_profiles(&rules, &request)?;
    let queries = match request["queries"].take() {
        Json::Array(queries) => queries,
        _ => Vec::new(),
    };
    let results: Vec<Json> = queries
        .into_iter()
        .map(|query| run_query(&rules, &request, rpc, query))
        .collect::<Result<_, _>>()?;
    // A conversion states the rules that read the source profile's names, which every
    // result shares.
    let identity = results
        .first()
        .and_then(|result| result.get("rules"))
        .cloned()
        .unwrap_or_else(|| to_json(rules.identity()));
    respond(identity, results)
}

/// Runs the operation of one query, with the request fields that the operation reads.
/// Returns the result with the query's handle.
fn run_query(rules: &Rules, request: &Json, rpc: Rpc, query: Json) -> Result<Json, Status> {
    let Json::Object(mut input) = query else {
        return Err(Status::invalid_argument(
            "query: a query is a message. Check the wire contract",
        ));
    };
    let handle = input.remove("handle").unwrap_or(Json::Null);
    let call = match rpc {
        Rpc::ParseNames => {
            let chosen = input.keys().next().cloned().unwrap_or_default();
            let call = WIRE_CALLS
                .iter()
                .find(|call| call.parse_field == Some(chosen.as_str()))
                .ok_or_else(|| {
                    Status::invalid_argument(format!(
                        "query {handle}: the query chooses no operation. Set one field of `query`"
                    ))
                })?;
            input = match input.remove(&chosen) {
                Some(Json::Object(fields)) => fields,
                _ => Map::new(),
            };
            call
        }
        _ => WIRE_CALLS
            .iter()
            .find(|call| call.rpc == rpc)
            .expect("every rpc that answers queries has a call"),
    };
    for field in call.request_fields {
        if let Some(value) = request.get(field) {
            input.insert((*field).to_owned(), value.clone());
        }
    }
    let result = rules
        .run(call.operation, Json::Object(input))
        .map_err(|error| Status::invalid_argument(format!("query {handle}: {error}")))?;
    Ok(match call.parse_field {
        Some(field) => serde_json::json!({ "handle": handle, field: result }),
        None => with_handle(&handle, result),
    })
}

/// Converts the request to the library's serialized shape. Leaves out unset fields so
/// that their defaults apply.
fn read_request<M: prost::Message + prost::Name>(request: &M) -> Result<Json, Status> {
    let json =
        wire::message_to_json(request).map_err(|error| Status::internal(error.to_string()))?;
    Ok(without_nulls(json))
}

/// Builds the rules that a request is answered under, and removes the request's `rules`
/// field: the built-in rules, or the rules of its declared languages, with its overlays
/// applied. Fails with every overlay diagnostic if the rules cannot be built.
fn request_rules(request: &mut Json) -> Result<Cow<'static, Rules>, Status> {
    let selection = request
        .as_object_mut()
        .and_then(|fields| fields.remove("rules"))
        .unwrap_or(Json::Null);
    let strings = |field: &str| -> Vec<String> {
        serde_json::from_value(selection[field].clone()).unwrap_or_default()
    };
    let (overlays, languages) = (strings("overlays"), strings("languages"));
    if overlays.is_empty() && languages.is_empty() {
        return Ok(Cow::Borrowed(Rules::builtin()));
    }
    Rules::from_documents(&languages, &overlays)
        .map(Cow::Owned)
        .map_err(|diagnostics| {
            let listed = serde_json::to_string(&diagnostics).unwrap_or_default();
            Status::invalid_argument(format!(
                "rules: cannot build the rules: {listed}. Correct the listed overlays or languages"
            ))
        })
}

/// Fails with `INVALID_ARGUMENT` if the reader does not read back a name that a profile of
/// the request writes (X33).
fn check_profiles(rules: &Rules, request: &Json) -> Result<(), Status> {
    for field in ["profile", "from_profile", "to_profile"] {
        let Some(profile) = request.get(field) else {
            continue;
        };
        let profile: OutputProfile = serde_json::from_value(profile.clone()).map_err(|error| {
            Status::invalid_argument(format!("{field}: cannot read the profile: {error}"))
        })?;
        let diagnostics = rules.check_profile(&profile);
        if !diagnostics.is_empty() {
            return Err(Status::invalid_argument(format!(
                "{field}: the output profile is rejected: {diagnostics:?}. Change the listed templates"
            )));
        }
    }
    Ok(())
}

fn invalid(error: nomenclator::OperationError) -> Status {
    Status::invalid_argument(error.to_string())
}

/// Returns `result` with `handle`.
fn with_handle(handle: &Json, mut result: Json) -> Json {
    result["handle"] = handle.clone();
    result
}

fn without_rules(mut result: Json) -> Json {
    if let Json::Object(fields) = &mut result {
        fields.remove("rules");
    }
    result
}

fn to_json(result: impl serde::Serialize) -> Json {
    serde_json::to_value(result).expect("every result serializes")
}

/// Returns the response with `results` and the rules `identity`. Removes the identity of
/// each result, because the response states it once.
fn respond<M: prost::Message + prost::Name + Default>(
    identity: Json,
    results: Vec<Json>,
) -> Result<Response<M>, Status> {
    let results: Vec<Json> = results.into_iter().map(without_rules).collect();
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
