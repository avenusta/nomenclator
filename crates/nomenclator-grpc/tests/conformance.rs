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
        // Skips `build_rules` and `diff_rules`, because the service has no such calls. A
        // request's rules are built with the request, and a diff is two calls that the
        // caller compares. Skips templates and `match_facts`, which are library helpers.
        let replayable = |case: &CorpusCase| {
            !case.pending
                && ![
                    "build_rules",
                    "check_profile",
                    "diff_rules",
                    "render_template",
                    "read_template",
                    "match_facts",
                ]
                .contains(&case.operation.as_str())
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
async fn interpret_reads_only_the_scope_group() {
    let mut client = start_server().await;
    let request: InterpretRequest = json_to_message(&json!({
        "queries": [{
            "handle": "one",
            "library": "movies",
            "paths": ["Arrival (2016)/Arrival (2016).mkv", "Heat (1995)/Heat (1995).mkv", "Loose (2001).mkv"],
            "scope_group": { "folders": ["Heat (1995)"] },
        }]
    }))
    .expect("a valid interpret request");
    let response = client
        .interpret(request)
        .await
        .expect("Interpret answers")
        .into_inner();
    let response = message_to_json(&response).expect("a valid response");
    let titles: Vec<&Json> = response["results"][0]["items"]
        .as_array()
        .expect("a list of items")
        .iter()
        .map(|item| &item["title"])
        .collect();
    assert_eq!(titles, [&json!("Heat")]);
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
async fn replay_case(client: &Replay, case: &CorpusCase) -> Result<Value, String> {
    let input = serde_json::to_value(&case.input).map_err(|error| error.to_string())?;
    let field = |name: &str| input.get(name).cloned().unwrap_or(Json::Null);
    let either = |first: &str, second: &str| match field(first) {
        Json::Null => field(second),
        value => value,
    };
    let result = match case.operation.as_str() {
        "parse_movie_name" => {
            let query = json!({ "name": field("name"), "kind": field("kind") });
            parse_name(client, "movie_name", query).await?
        }
        "parse_series_name" => {
            let query = json!({ "name_or_path": either("name", "path") });
            parse_name(client, "series_name", query).await?
        }
        "parse_season_folder" => {
            let query = json!({
                "name": field("name"),
                "parent_name": field("parent_name"),
                "parent_is_series_folder": field("parent_is_series_folder"),
            });
            parse_name(client, "season_folder", query).await?
        }
        "parse_episode_name" => {
            let kind = match field("kind") {
                Json::Null => json!("file"),
                kind => kind,
            };
            let query = json!({
                "path": field("path"),
                "kind": kind,
                "season_folder_number": field("season_folder_number"),
            });
            parse_name(client, "episode_name", query).await?
        }
        "suggest_library_type" => {
            let request =
                json!({ "queries": [query_with_handle(json!({ "paths": field("paths") }))] });
            only_result(rpc!(
                client,
                suggest_library_type,
                SuggestLibraryTypeRequest,
                request
            )?)?
        }
        "parse_release" => {
            let query = json!({ "name": field("name"), "kind": field("kind") });
            parse_name(client, "release", query).await?
        }
        "parse_provider_ids" => {
            parse_name(client, "provider_ids", json!({ "text": field("text") })).await?
        }
        "parse_sidecar" => {
            let query = json!({
                "name": field("name"),
                "video_name": field("video_name"),
                "kind": field("kind"),
            });
            parse_name(client, "sidecar", query).await?
        }
        "classify_extra" => {
            // Keeps only the entries that the ignore rules keep, as a caller would.
            let context_paths =
                paths_kept_by_ignore_rules(client, field("context_paths"), field("ignore_files"))
                    .await?;
            let query = json!({
                "path": field("path"),
                "kind": field("kind"),
                "library": field("library"),
                "context_paths": context_paths,
            });
            parse_name(client, "extra", query).await?
        }
        "classify_artwork" => {
            let query =
                json!({ "path": either("path", "name"), "video_path": field("video_path") });
            parse_name(client, "artwork", query).await?
        }
        "classify_nfo" => {
            let query = json!({ "path": field("path"), "video_paths": field("video_paths") });
            parse_name(client, "nfo", query).await?
        }
        "is_video_file" => {
            parse_name(client, "video_file", json!({ "path": field("path") })).await?
        }
        "is_release_junk" => {
            parse_name(client, "release_junk", json!({ "path": field("path") })).await?
        }
        "is_audio_file" => {
            parse_name(client, "audio_file", json!({ "path": field("path") })).await?
        }
        "is_image_file" => {
            parse_name(client, "image_file", json!({ "path": field("path") })).await?
        }
        "is_subtitle_file" => {
            let query = json!({ "path": field("path"), "sibling_paths": field("sibling_paths") });
            parse_name(client, "subtitle_file", query).await?
        }
        "group_parts" => {
            let request =
                json!({ "queries": [query_with_handle(json!({ "paths": field("paths") }))] });
            only_result(rpc!(client, group_parts, GroupPartsRequest, request)?)?
        }
        "evaluate_ignore" => {
            let query = json!({ "path": field("path"), "kind": field("kind") });
            let request = json!({ "ignore_files": field("ignore_files"), "queries": [query_with_handle(query)] });
            only_result(rpc!(
                client,
                evaluate_ignore,
                EvaluateIgnoreRequest,
                request
            )?)?
        }
        "interpret" => {
            let request = json!({ "queries": [library_tree(&input)] });
            only_result(rpc!(client, interpret, InterpretRequest, request)?)?
        }
        "plan_renames" => {
            let request = json!({ "profile": field("profile"), "queries": [library_tree(&input)] });
            only_result(rpc!(client, plan_renames, PlanRenamesRequest, request)?)?
        }
        "convert_layout" => {
            let request = json!({
                "from_profile": field("from_profile"),
                "to_profile": field("to_profile"),
                "queries": [library_tree(&input)],
            });
            only_result(rpc!(
                client,
                plan_conversion,
                PlanConversionRequest,
                request
            )?)?
        }
        "plan_import" => {
            let mut query = library_tree(&input);
            query["library_paths"] = field("library_paths");
            if let Json::Object(fields) = &mut query {
                fields.remove("reference");
            }
            let request = json!({ "profile": field("profile"), "queries": [query] });
            only_result(rpc!(client, plan_import, PlanImportRequest, request)?)?
        }
        "validate" => {
            let request = json!({ "profile": field("profile"), "queries": [library_tree(&input)] });
            only_result(rpc!(client, validate, ValidateRequest, request)?)?
        }
        "end_to_end" => {
            let tree = json!({ "queries": [library_tree(&input)] });
            let profiled =
                json!({ "profile": field("profile"), "queries": [library_tree(&input)] });
            json!({
                "interpretation": only_result(rpc!(client, interpret, InterpretRequest, tree)?)?,
                "plan": only_result(rpc!(client, plan_renames, PlanRenamesRequest, profiled.clone())?)?,
                "validation": only_result(rpc!(client, validate, ValidateRequest, profiled)?)?,
            })
        }
        "render_names" => {
            let query = query_with_handle(json!({ "facts": field("facts") }));
            let request = json!({ "profile": field("profile"), "queries": [query] });
            only_result(rpc!(client, render_names, RenderNamesRequest, request)?)?
        }
        "rank_batch" => {
            // Each query keeps its own handle, and each result keeps the ranking fields
            // beside its handle.
            let request = json!({ "queries": field("queries") });
            let response = rpc!(client, rank_candidates, RankCandidatesRequest, request)?;
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
            json!({ "results": results })
        }
        "search_queries" => {
            let query = json!({ "facts": field("facts") });
            let request = json!({ "queries": [query_with_handle(query)] });
            only_result(rpc!(client, search_queries, SearchQueriesRequest, request)?)?
        }
        "rank_candidates" => {
            let query = json!({ "facts": field("facts"), "candidates": field("candidates") });
            let request = json!({ "queries": [query_with_handle(query)] });
            only_result(rpc!(
                client,
                rank_candidates,
                RankCandidatesRequest,
                request
            )?)?
        }
        unknown_operation => return Err(format!("unknown operation `{unknown_operation}`")),
    };
    serde_json::to_value(result).map_err(|error| error.to_string())
}

/// Sends one `ParseNames` query that selects `operation`, and returns the result under
/// the same name.
async fn parse_name(client: &Replay, operation: &str, query: Json) -> Result<Json, String> {
    let request = json!({ "queries": [query_with_handle(json!({ operation: query }))] });
    let mut result = only_result(rpc!(client, parse_names, ParseNamesRequest, request)?)?;
    result
        .get_mut(operation)
        .map(Json::take)
        .ok_or_else(|| format!("no `{operation}` result in {result}"))
}

async fn paths_kept_by_ignore_rules(
    client: &Replay,
    paths: Json,
    ignore_files: Json,
) -> Result<Vec<Json>, String> {
    let paths = paths.as_array().cloned().unwrap_or_default();
    if paths.is_empty() {
        return Ok(paths);
    }
    let queries: Vec<Json> = paths
        .iter()
        .enumerate()
        .map(|(index, path)| json!({ "handle": index.to_string(), "path": path, "kind": "file" }))
        .collect();
    let request = json!({ "ignore_files": ignore_files, "queries": queries });
    let response = rpc!(client, evaluate_ignore, EvaluateIgnoreRequest, request)?;
    let results = response["results"].as_array().cloned().unwrap_or_default();
    if results.len() != paths.len() {
        return Err(format!(
            "{} paths, {} ignore results",
            paths.len(),
            results.len()
        ));
    }
    Ok(paths
        .into_iter()
        .zip(results)
        .filter(|(_, result)| result["excluded"] == json!(false))
        .map(|(path, _)| path)
        .collect())
}

fn library_tree(input: &Json) -> Json {
    let field = |name: &str| input.get(name).cloned().unwrap_or(Json::Null);
    query_with_handle(json!({
        "library": field("library"),
        "paths": field("paths"),
        "inputs": field("inputs"),
        "reference": field("reference"),
        "ignore_files": field("ignore_files"),
    }))
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
