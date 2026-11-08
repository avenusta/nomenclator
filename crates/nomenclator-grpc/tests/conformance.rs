//! Replays every non-pending case in `corpus/` through an in-process gRPC server, and
//! compares the wire results with the cases' expectations under the same subset rules as
//! the library's corpus runner. If both runners pass, direct and wire calls return equal
//! results.

use nomenclator_grpc::proto::nomenclator_client::NomenclatorClient;
use nomenclator_grpc::proto::{
    EvaluateIgnoreRequest, GetVersionRequest, GroupPartsRequest, InterpretRequest,
    InterpretStreamHeader, InterpretStreamRequest, LibraryType as WireLibraryType, ParseNameQuery,
    ParseNamesRequest, PlanConversionRequest, PlanImportRequest, PlanRenamesRequest,
    RankCandidatesRequest, RenderNamesRequest, SearchQueriesRequest, SuggestLibraryTypeRequest,
    ValidateRequest,
};
use nomenclator_grpc::service::{Rpc, wire_call};
use nomenclator_grpc::wire::{json_to_message, message_to_json};
use nomenclator_grpc::{RULES_VERSION, WIRE_CONTRACT, serve_tcp};
#[path = "../../nomenclator/tests/support/cases.rs"]
mod cases;
#[path = "../../nomenclator/tests/support/expectations.rs"]
mod expectations;

use cases::{CorpusCase, collect_yaml_files, corpus_directory, load_cases};

use expectations::expect_subset;
use serde_json::Value;
use serde_json::{Value as Json, json};
use std::future::Future;
use tonic::transport::Channel;
use tonic::{Response, Status};

type Client = NomenclatorClient<Channel>;

/// Calls `method` with the request built from its JSON shape, and returns the response's
/// JSON shape.
macro_rules! rpc {
    ($replay:expr, $method:ident, $request_type:ty, $request:expr) => {{
        let mut client = $replay.client.clone();
        call(
            $replay.with_rules($request),
            |request: $request_type| async move { client.$method(request).await },
        )
        .await
    }};
}

/// The context of a case's calls: the client, and the rules that every request
/// contains.
struct Replay {
    client: Client,
    overlays: Vec<String>,
    languages: Option<Vec<String>>,
}

impl Replay {
    fn with_rules(&self, mut request: Json) -> Json {
        if !self.overlays.is_empty() || self.languages.is_some() {
            request["rules"] = json!({ "overlays": self.overlays, "languages": self.languages });
        }
        request
    }
}

/// The handle of every replayed query. Each call contains one query.
const CASE_HANDLE: &str = "case";

#[tokio::test(flavor = "multi_thread")]
async fn corpus_cases_match_expected_facts_over_the_wire() {
    let client = start_server().await;
    let mut failures = Vec::new();
    let mut replayed = 0;
    for file in collect_yaml_files(&corpus_directory()) {
        // Skips the operations that the contract has no call for. A request's rules are
        // built with the request, a diff is two calls that the caller compares, and
        // templates and match facts are library helpers.
        let replayable = |case: &CorpusCase| {
            !case.pending
                && (wire_call(&case.operation).is_some()
                    || ["end_to_end", "rank_batch"].contains(&case.operation.as_str()))
        };
        for case in load_cases(&file).into_iter().filter(replayable) {
            replayed += 1;
            let replay = Replay {
                client: client.clone(),
                overlays: case.overlays.clone(),
                languages: case.languages.clone(),
            };
            let outcome = match replay_case(&replay, &case).await {
                Ok(actual) => expect_subset(&case.expect, &actual, "expect"),
                Err(reason) => Err(reason),
            };
            if let Err(reason) = outcome {
                failures.push(format!("{} › {}: {reason}", file.display(), case.name));
            }
        }
    }
    assert!(replayed > 0, "the corpus holds non-pending cases");
    assert!(
        failures.is_empty(),
        "{} of {replayed} cases fail over the wire:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[tokio::test]
async fn get_version_reports_wire_contract_and_rules_versions_separately() {
    let mut client = start_server().await;
    let version = client
        .get_version(GetVersionRequest {})
        .await
        .expect("GetVersion answers")
        .into_inner();
    assert_eq!(version.wire_contract, WIRE_CONTRACT);
    assert_eq!(version.rules_version, RULES_VERSION);
    assert_eq!(
        version.rules.expect("GetVersion names the rules").version,
        RULES_VERSION
    );
    assert!(!version.implementation_version.is_empty());
}

#[tokio::test]
async fn interpret_rejects_conflicting_overlays_as_invalid_argument() {
    let mut client = start_server().await;
    let request: InterpretRequest = json_to_message(&json!({
        "rules": { "overlays": [
            "name: keeps-end\ntitle_like_release_words:\n  remove: [end]\n",
            "name: cuts-end\ntitle_like_release_words:\n  add: [end]\n",
        ] },
        "queries": [{ "handle": "one", "library": "movies", "paths": ["The End 2160p.mkv"] }]
    }))
    .expect("a valid interpret request");
    let status = client
        .interpret(request)
        .await
        .expect_err("conflicting overlays build no rules");
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    assert!(
        status.message().contains("conflicting_change"),
        "{status:?}"
    );
}

#[tokio::test]
async fn interpret_response_contains_overlays_of_its_rules() {
    let mut client = start_server().await;
    let request: InterpretRequest = json_to_message(&json!({
        "rules": { "overlays": ["name: keeps-end\nversion: '3'\ntitle_like_release_words:\n  remove: [end]\n"] },
        "queries": [{ "handle": "one", "library": "movies", "paths": ["The End 2160p.mkv"] }]
    }))
    .expect("a valid interpret request");
    let response = client
        .interpret(request)
        .await
        .expect("Interpret answers")
        .into_inner();
    let overlays = response
        .rules
        .expect("the response names its rules")
        .overlays;
    assert_eq!(overlays.len(), 1);
    assert_eq!(overlays[0].name, "keeps-end");
    assert_eq!(overlays[0].version.as_deref(), Some("3"));
}

#[tokio::test]
async fn interpret_stream_equals_interpret_of_the_joined_batches() {
    let mut client = start_server().await;
    let paths: Vec<String> = (0..30)
        .flat_map(|number| {
            let folder = format!("Movie {number} ({})", 1990 + number);
            [
                format!("{folder}/{folder}.mkv"),
                format!("{folder}/{folder}.en.srt"),
            ]
        })
        .collect();
    let unary: InterpretRequest = json_to_message(&json!({
        "queries": [{ "handle": "tree", "library": "movies", "paths": paths }]
    }))
    .expect("a valid interpret request");
    let unary = client
        .interpret(unary)
        .await
        .expect("Interpret answers")
        .into_inner();
    let header = InterpretStreamHeader {
        handle: "tree".to_owned(),
        library: WireLibraryType::Movies as i32,
        ..InterpretStreamHeader::default()
    };
    let mut messages = vec![InterpretStreamRequest {
        header: Some(header),
        paths: Vec::new(),
    }];
    messages.extend(paths.chunks(25).map(|batch| InterpretStreamRequest {
        header: None,
        paths: batch.to_vec(),
    }));
    let streamed = client
        .interpret_stream(tokio_stream::iter(messages))
        .await
        .expect("InterpretStream answers")
        .into_inner();
    assert_eq!(
        message_to_json(&streamed).expect("a valid response"),
        message_to_json(&unary).expect("a valid response")
    );
}

#[tokio::test]
async fn interpret_stream_rejects_a_first_message_without_header() {
    let mut client = start_server().await;
    let messages = vec![InterpretStreamRequest {
        header: None,
        paths: vec!["Movie (2020).mkv".to_owned()],
    }];
    let status = client
        .interpret_stream(tokio_stream::iter(messages))
        .await
        .expect_err("a stream without a header is rejected");
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
}

#[tokio::test]
async fn interpret_response_contains_rules_version() {
    let mut client = start_server().await;
    let request: InterpretRequest = json_to_message(&json!({
        "queries": [{ "handle": "one", "library": "movies", "paths": ["Movie (2020).mkv"] }]
    }))
    .expect("a valid interpret request");
    let response = client
        .interpret(request)
        .await
        .expect("Interpret answers")
        .into_inner();
    assert_eq!(
        response
            .rules
            .expect("the response names its rules")
            .version,
        RULES_VERSION
    );
}

#[tokio::test]
async fn parse_names_rejects_query_without_operation_as_invalid_argument() {
    let mut client = start_server().await;
    let request = ParseNamesRequest {
        queries: vec![ParseNameQuery {
            handle: "no operation".to_owned(),
            query: None,
        }],
        rules: None,
    };
    let status = client
        .parse_names(request)
        .await
        .expect_err("a query must choose an operation");
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    assert!(status.message().contains("no operation"), "{status:?}");
}

async fn start_server() -> Client {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a local port is free");
    let address = listener.local_addr().expect("the listener has an address");
    tokio::spawn(serve_tcp(listener, std::future::pending()));
    NomenclatorClient::connect(format!("http://{address}"))
        .await
        .expect("the server accepts connections")
}

/// Sends the case's input as the operation's wire query and returns the one result in
/// the library's shape.
async fn replay_case(replay: &Replay, case: &CorpusCase) -> Result<Value, String> {
    let input = case.input.clone();
    match case.operation.as_str() {
        "end_to_end" => {
            let without = |fields: &[&str]| {
                let mut input = input.clone();
                if let Json::Object(object) = &mut input {
                    object.retain(|key, _| !fields.contains(&key.as_str()));
                }
                input
            };
            Ok(json!({
                "interpretation": replay_operation(replay, "interpret", without(&["profile", "reference"])).await?,
                "plan": replay_operation(replay, "plan_renames", without(&["reference"])).await?,
                "validation": replay_operation(replay, "validate", input).await?,
            }))
        }
        "rank_batch" => {
            // Each query keeps its own handle, and each result keeps the ranking fields
            // beside its handle.
            let request = json!({ "queries": input["queries"] });
            let response = send(replay, Rpc::RankCandidates, request).await?;
            let results: Vec<Json> = response["results"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|mut result| {
                    let handle = result
                        .as_object_mut()
                        .and_then(|fields| fields.remove("handle"));
                    json!({ "handle": handle, "ranking": result })
                })
                .collect();
            Ok(json!({ "results": results }))
        }
        operation => replay_operation(replay, operation, input).await,
    }
}

/// Sends `input` as one query of the call that carries `operation`: the call's request
/// fields go into the request, and the other fields into the query.
async fn replay_operation(replay: &Replay, operation: &str, input: Json) -> Result<Json, String> {
    let call = wire_call(operation).ok_or_else(|| format!("no call carries `{operation}`"))?;
    let Json::Object(mut query) = input else {
        return Err("the input is not a mapping".to_owned());
    };
    let mut request = serde_json::Map::new();
    for field in call.request_fields {
        if let Some(value) = query.remove(*field) {
            request.insert((*field).to_owned(), value);
        }
    }
    let query = match call.parse_field {
        Some(field) => json!({ field: query }),
        None => Json::Object(query),
    };
    request.insert("queries".to_owned(), json!([query_with_handle(query)]));
    let mut result = only_result(send(replay, call.rpc, Json::Object(request)).await?)?;
    match call.parse_field {
        Some(field) => result
            .get_mut(field)
            .map(Json::take)
            .ok_or_else(|| format!("no `{field}` result in {result}")),
        None => Ok(result),
    }
}

/// Sends `request` to `rpc` with the rules of the case, and returns the response's JSON
/// shape.
async fn send(replay: &Replay, rpc: Rpc, request: Json) -> Result<Json, String> {
    match rpc {
        Rpc::ParseNames => rpc!(replay, parse_names, ParseNamesRequest, request),
        Rpc::GroupParts => rpc!(replay, group_parts, GroupPartsRequest, request),
        Rpc::EvaluateIgnore => rpc!(replay, evaluate_ignore, EvaluateIgnoreRequest, request),
        Rpc::Interpret => rpc!(replay, interpret, InterpretRequest, request),
        Rpc::RankCandidates => rpc!(replay, rank_candidates, RankCandidatesRequest, request),
        Rpc::SearchQueries => rpc!(replay, search_queries, SearchQueriesRequest, request),
        Rpc::RenderNames => rpc!(replay, render_names, RenderNamesRequest, request),
        Rpc::PlanRenames => rpc!(replay, plan_renames, PlanRenamesRequest, request),
        Rpc::PlanImport => rpc!(replay, plan_import, PlanImportRequest, request),
        Rpc::PlanConversion => rpc!(replay, plan_conversion, PlanConversionRequest, request),
        Rpc::Validate => rpc!(replay, validate, ValidateRequest, request),
        Rpc::SuggestLibraryType => {
            rpc!(
                replay,
                suggest_library_type,
                SuggestLibraryTypeRequest,
                request
            )
        }
    }
}

fn query_with_handle(mut query: Json) -> Json {
    query["handle"] = json!(CASE_HANDLE);
    query
}

/// Builds the request message from its JSON shape, sends the call, and returns the
/// response's JSON shape.
async fn call<Request, Answer, Rpc, Pending>(request: Json, rpc: Rpc) -> Result<Json, String>
where
    Request: prost::Message + prost::Name + Default,
    Answer: prost::Message + prost::Name,
    Rpc: FnOnce(Request) -> Pending,
    Pending: Future<Output = Result<Response<Answer>, Status>>,
{
    let request =
        json_to_message::<Request>(&request).map_err(|error| format!("request: {error}"))?;
    let response = rpc(request)
        .await
        .map_err(|status| format!("{:?}: {}", status.code(), status.message()))?;
    message_to_json(response.get_ref()).map_err(|error| format!("response: {error}"))
}

/// Returns the single result of a one-query call, without its handle. Fails if the
/// response does not contain exactly one result or if the handle is not `CASE_HANDLE`.
fn only_result(response: Json) -> Result<Json, String> {
    let mut results = response["results"].as_array().cloned().unwrap_or_default();
    let (Some(mut result), true) = (results.pop(), results.is_empty()) else {
        return Err(format!("expected one result, got {response}"));
    };
    let handle = result
        .as_object_mut()
        .and_then(|fields| fields.remove("handle"));
    if handle != Some(json!(CASE_HANDLE)) {
        return Err(format!("the result's handle is {handle:?}"));
    }
    Ok(result)
}
