use super::{accounts::PlanAccounts, inference, tests::Fixture, SCOPES};
use crate::{
    proxy::{server::ProxyServer, ProxyConfig},
    qoder_gateway::{self, ApiFormat, ProviderInput},
    Database,
};
use axum::{
    body::{to_bytes, Body, Bytes},
    extract::Request,
    http::{header, HeaderMap, StatusCode},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::{net::TcpListener, sync::Notify, task::JoinHandle};
use tower::Service;

struct Task(JoinHandle<()>);
impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}
struct Upstream {
    base: String,
    requests: Arc<Mutex<Vec<(HeaderMap, Value)>>>,
    raw_requests: Arc<Mutex<Vec<Vec<u8>>>>,
    reads: Arc<Mutex<Vec<HeaderMap>>>,
    gate: Arc<Mutex<Option<(Arc<Notify>, Arc<Notify>)>>>,
    _task: Task,
}
async fn upstream() -> Upstream {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let raw_requests = Arc::new(Mutex::new(Vec::new()));
    let reads = Arc::new(Mutex::new(Vec::new()));
    let gate = Arc::new(Mutex::new(None::<(Arc<Notify>, Arc<Notify>)>));
    let capture = requests.clone();
    let raw_capture = raw_requests.clone();
    let catalogs = reads.clone();
    let catalog_gate = gate.clone();
    let app = Router::new()
        .route("/v1/models", get(move |headers: HeaderMap| {
            catalogs.lock().unwrap().push(headers);
            let gate = catalog_gate.lock().unwrap().clone();
            async move {
                if let Some((entered, release)) = gate {
                    entered.notify_one();
                    release.notified().await;
                }
                Json(json!({"models":[
                    {"slug":"second-model","display_name":"Second display","visibility":"list"},
                    {"slug":"hidden-model","display_name":"Not displayed","visibility":"hidden"},
                    {"slug":"fixture-model","display_name":"Fixture display","visibility":"list"}
                ]}))
            }
        }))
        .route("/v1/responses", post(move |headers: HeaderMap, raw: Bytes| {
            raw_capture.lock().unwrap().push(raw.to_vec());
            let body: Value = serde_json::from_slice(&raw).unwrap();
            capture.lock().unwrap().push((headers, body.clone()));
            async move {
                let input = body["input"].as_array().unwrap();
                let added_tool = input.iter()
                    .filter(|item| item["type"] == "additional_tools")
                    .find_map(|item| item["tools"].as_array().and_then(|tools| tools.first()));
                let has_output = input.iter().any(|item| matches!(item["type"].as_str(), Some("function_call_output" | "custom_tool_call_output")));
                let event = if body.pointer("/input/0/content").and_then(Value::as_str) == Some("simulate-failure") {
                    json!({"type":"response.failed","response":{"status":"failed","error":{"code":"subscription_sharing_usage_limit_exceeded"}}})
                } else if (body.get("tools").is_some() || added_tool.is_some()) && !has_output {
                    let call = match added_tool {
                        Some(tool) if tool["type"] == "custom" => json!({"type":"custom_tool_call","call_id":"call-fixture","name":tool["name"],"input":"n=1"}),
                        Some(tool) => json!({"type":"function_call","call_id":"call-fixture","name":tool["name"],"arguments":"{\"n\":1}"}),
                        None => json!({"type":"function_call","call_id":"call-fixture","namespace":"functions","name":"fixture_tool","arguments":"{\"n\":1}"}),
                    };
                    json!({"type":"response.completed","response":{"id":"r-tool","status":"completed","output":[call]}})
                } else {
                    json!({"type":"response.completed","response":{"id":"r-final","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}]}})
                };
                let content = format!("event: {}\ndata: {event}\n\n", event["type"].as_str().unwrap());
                Response::builder().header(header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from(content)).unwrap()
            }
        }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = Task(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    }));
    Upstream {
        base,
        requests,
        raw_requests,
        reads,
        gate,
        _task: task,
    }
}

async fn sign_in(
    accounts: &Arc<PlanAccounts>,
    fixture: &Fixture,
    scope: &str,
    suffix: &str,
) -> String {
    let launch = accounts
        .begin(fixture.discovery().await, None, suffix)
        .await
        .unwrap();
    let params: std::collections::HashMap<String, String> = url::Url::parse(&launch.authorize_url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
    fixture.reply(Some(&params["nonce"]), Some(scope), suffix);
    let mut callback = url::Url::parse(&launch.callback_url).unwrap();
    callback
        .query_pairs_mut()
        .append_pair("state", &params["state"])
        .append_pair("code", "synthetic-code")
        .append_pair("client_id", "issued-test-client");
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(callback)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    launch.view.account_id
}
fn add(db: &Database, account: &str) -> String {
    let status = qoder_gateway::save(
        db,
        ProviderInput {
            id: None,
            name: account.into(),
            base_url: inference::API_BASE.into(),
            api_key: None,
            api_format: ApiFormat::OpenaiResponses,
            models: vec![
                "fixture-model".into(),
                "second-model".into(),
                "hidden-model".into(),
            ],
            plan_account_id: Some(account.into()),
            codex_account_id: None,
        },
    )
    .unwrap();
    let id = status
        .providers
        .iter()
        .find(|provider| provider.plan_account_id.as_deref() == Some(account))
        .unwrap()
        .id
        .clone();
    qoder_gateway::select(db, &id).unwrap();
    id
}
fn router(db: Arc<Database>, accounts: Arc<PlanAccounts>, upstream: &Upstream) -> Router {
    ProxyServer::new(ProxyConfig::default(), db, None)
        .with_plan_fixture(accounts, &upstream.base)
        .build_router()
}
fn request(key: &str, body: &Value) -> Request {
    Request::builder()
        .method("POST")
        .uri("/qoder/v1/responses")
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("cookie", "must-not-leak")
        .body(Body::from(body.to_string()))
        .unwrap()
}
async fn dispatch(mut router: Router, request: Request) -> Response {
    std::future::poll_fn(|cx| <Router as Service<Request>>::poll_ready(&mut router, cx))
        .await
        .unwrap();
    router.call(request).await.unwrap()
}
async fn bytes(response: Response) -> Vec<u8> {
    to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap()
        .to_vec()
}
fn payload() -> Value {
    json!({"model":"fixture-model", "input":[{"role":"user", "content":"hello"}],"store":false,"stream":true})
}

#[tokio::test]
async fn plan_router_catalog_uses_explicit_account_visible_slugs_and_server_order() {
    let directory = tempfile::tempdir().unwrap();
    let accounts = PlanAccounts::open(directory.path().join("plan.json")).unwrap();
    let fixture = Fixture::new().await;
    let a = sign_in(&accounts, &fixture, SCOPES, "account-a").await;
    let _active_b = sign_in(&accounts, &fixture, SCOPES, "account-b").await;
    let db = Arc::new(Database::memory().unwrap());
    let provider = add(&db, &a);
    let key = qoder_gateway::prepare_key(&db).unwrap();
    let upstream = upstream().await;
    let response = dispatch(
        router(db.clone(), accounts, &upstream),
        Request::builder()
            .uri("/qoder/v1/models")
            .header(header::AUTHORIZATION, format!("Bearer {key}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let value: Value = serde_json::from_slice(&bytes(response).await).unwrap();
    assert_eq!(
        value["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|model| model["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["second-model", "fixture-model"]
    );
    assert_eq!(value["data"][0]["display_name"], "Second display");
    assert_eq!(
        value["data"][0]["owned_by"],
        format!("qoder-gateway:{provider}")
    );
    assert_eq!(
        upstream.reads.lock().unwrap()[0][header::AUTHORIZATION],
        "Bearer synthetic-access-account-a"
    );
    assert!(upstream.requests.lock().unwrap().is_empty());
    assert!(db.get_current_provider("codex").unwrap().is_none());
    assert!(!serde_json::to_string(&qoder_gateway::status(&db).unwrap())
        .unwrap()
        .contains("synthetic-access"));
}

#[tokio::test]
async fn plan_router_preserves_namespaced_tool_round_trip_and_failed_terminal_without_retry() {
    let directory = tempfile::tempdir().unwrap();
    let accounts = PlanAccounts::open(directory.path().join("plan.json")).unwrap();
    let fixture = Fixture::new().await;
    let id = sign_in(&accounts, &fixture, SCOPES, "tools").await;
    let db = Arc::new(Database::memory().unwrap());
    add(&db, &id);
    let key = qoder_gateway::prepare_key(&db).unwrap();
    let upstream = upstream().await;
    let router = router(db, accounts, &upstream);
    let mut first = payload();
    first["tools"] = json!([{"type":"namespace","name":"functions","tools":[{"type":"function","name":"fixture_tool","parameters":{"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}}]}]);
    let response = dispatch(router.clone(), request(&key, &first)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let first_bytes = bytes(response).await;
    let data = std::str::from_utf8(&first_bytes)
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .unwrap();
    let completed: Value = serde_json::from_str(data).unwrap();
    assert_eq!(completed["type"], "response.completed");
    let call = completed["response"]["output"][0].clone();
    let mut second = first.clone();
    second["input"].as_array_mut().unwrap().extend([
        call.clone(),
        json!({"type":"function_call_output","call_id":call["call_id"],"output":"{\"n\":2}"}),
    ]);
    let response = dispatch(router.clone(), request(&key, &second)).await;
    let text = String::from_utf8(bytes(response).await).unwrap();
    assert!(text.contains("response.completed") && text.contains("r-final"));
    let mut failure = payload();
    failure["input"][0]["content"] = json!("simulate-failure");
    let response = dispatch(router, request(&key, &failure)).await;
    let text = String::from_utf8(bytes(response).await).unwrap();
    assert!(text.contains("response.failed") && !text.contains("response.completed"));
    let received = upstream.requests.lock().unwrap();
    assert_eq!(
        received.len(),
        3,
        "one dispatch per caller request, no failed-stream retry"
    );
    assert_eq!(received[0].1, first);
    assert_eq!(received[1].1, second);
    assert_eq!(received[2].1, failure);
    for (headers, _) in received.iter() {
        assert_eq!(
            headers[header::AUTHORIZATION],
            "Bearer synthetic-access-tools"
        );
        assert!(!headers.contains_key(header::COOKIE));
        assert_ne!(headers[header::AUTHORIZATION], format!("Bearer {key}"));
    }
}

#[tokio::test]
async fn consumer_gap_flat_client_tools_round_trip_without_losing_history() {
    let directory = tempfile::tempdir().unwrap();
    let accounts = PlanAccounts::open(directory.path().join("plan.json")).unwrap();
    let fixture = Fixture::new().await;
    let id = sign_in(&accounts, &fixture, SCOPES, "flat-client").await;
    let db = Arc::new(Database::memory().unwrap());
    add(&db, &id);
    let key = qoder_gateway::prepare_key(&db).unwrap();
    let upstream = upstream().await;
    let router = router(db, accounts, &upstream);
    let retained = json!([
        {"type":"web_search"},
        {"type":"namespace","name":"existing","tools":[{"type":"function","name":"existing_function","parameters":{"type":"object","properties":{}}}]}
    ]);
    for (tool, call_type, output_type) in [
        (
            json!({"type":"function","name":"client_function","description":"Caller-owned function","strict":true,"parameters":{"type":"object","properties":{"n":{"type":"integer"}},"required":["n"],"additionalProperties":false}}),
            "function_call",
            "function_call_output",
        ),
        (
            json!({"type":"custom","name":"client_custom","description":"Caller-owned custom tool","format":{"type":"text"}}),
            "custom_tool_call",
            "custom_tool_call_output",
        ),
    ] {
        let mut first = payload();
        first["tools"] = json!([tool.clone(), retained[0].clone(), retained[1].clone()]);
        first["input"] = json!([
            {"type":"message","role":"developer","content":[{"type":"input_text","text":"Preserve this instruction"}]},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"Original question"}]},
            {"type":"message","role":"assistant","content":[{"type":"output_text","text":"Earlier answer"}]},
            {"type":"additional_tools","role":"developer","tools":[{"type":"function","name":"later_function","parameters":{"type":"object","properties":{}}}]},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"Use the caller tool"}]}
        ]);
        let response = dispatch(router.clone(), request(&key, &first)).await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "accept ordinary client tool definitions through the official plan format"
        );
        let response_bytes = bytes(response).await;
        let event: Value = serde_json::from_str(
            std::str::from_utf8(&response_bytes)
                .unwrap()
                .lines()
                .find_map(|line| line.strip_prefix("data: "))
                .unwrap(),
        )
        .unwrap();
        let call = event["response"]["output"][0].clone();
        assert_eq!(call["type"], call_type);
        assert_eq!(call["name"], tool["name"]);
        assert!(
            call.get("namespace").is_none(),
            "do not rename a caller tool"
        );
        let mut second = first.clone();
        second["input"].as_array_mut().unwrap().extend([
            call.clone(),
            json!({"type":output_type,"call_id":call["call_id"],"output":"caller-result"}),
        ]);
        let response = dispatch(router.clone(), request(&key, &second)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let text = String::from_utf8(bytes(response).await).unwrap();
        assert!(text.contains("response.completed") && text.contains("r-final"));
        let received = upstream.requests.lock().unwrap();
        for (sent, original) in received.iter().rev().take(2).zip([&second, &first]) {
            let mut expected = original.clone();
            expected["tools"] = retained.clone();
            expected["input"].as_array_mut().unwrap().insert(
                0,
                json!({"type":"additional_tools","role":"developer","tools":[tool.clone()]}),
            );
            assert_eq!(
                sent.1, expected,
                "preserve schemas, ordered history and the caller's exact tool result"
            );
            assert_eq!(
                sent.0[header::AUTHORIZATION],
                "Bearer synthetic-access-flat-client"
            );
        }
    }
    let mut forbidden = payload();
    forbidden["tools"] = json!([
        {"type":"function","name":"valid_but_mixed","parameters":{"type":"object","properties":{}}},
        {"type":"mcp","server_url":"https://must-not-contact.invalid"}
    ]);
    let response = dispatch(router, request(&key, &forbidden)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: Value = serde_json::from_slice(&bytes(response).await).unwrap();
    assert_eq!(error["error"]["dispatchStarted"], false);
    assert_eq!(
        upstream.requests.lock().unwrap().len(),
        4,
        "one POST per admitted caller request, none for rejected tools"
    );
    assert_eq!(
        upstream.reads.lock().unwrap().len(),
        4,
        "reject the mixed unsupported container before catalog access"
    );
}

#[tokio::test]
async fn consumer_gap_plan_tool_preparation_preserves_raw_schema_numbers() {
    let directory = tempfile::tempdir().unwrap();
    let accounts = PlanAccounts::open(directory.path().join("plan.json")).unwrap();
    let fixture = Fixture::new().await;
    let id = sign_in(&accounts, &fixture, SCOPES, "raw-schema").await;
    let db = Arc::new(Database::memory().unwrap());
    add(&db, &id);
    let key = qoder_gateway::prepare_key(&db).unwrap();
    let upstream = upstream().await;
    let router = router(db, accounts, &upstream);
    const TOOL: &str = r#"{"type":"function","name":"fixture_tool","parameters":{"type":"object","properties":{"integer":{"type":"integer","const":18446744073709551617},"fraction":{"type":"number","const":0.12345678901234567890123456789}},"required":["integer","fraction"],"additionalProperties":false}}"#;
    let mut statuses = Vec::new();
    let mut original_bodies = Vec::new();
    for tools in [
        format!("[{TOOL}]"),
        format!(r#"[{{"type":"namespace","name":"functions","tools":[{TOOL}]}}]"#),
    ] {
        let raw = format!(
            r#"{{"model":"fixture-model","store":false,"stream":true,"input":[{{"role":"user","content":"Keep the exact schema"}}],"tools":{tools}}}"#
        );
        let request = Request::builder()
            .method("POST")
            .uri("/qoder/v1/responses")
            .header(header::AUTHORIZATION, format!("Bearer {key}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(raw.clone()))
            .unwrap();
        let response = dispatch(router.clone(), request).await;
        statuses.push(response.status());
        bytes(response).await;
        original_bodies.push(raw);
    }
    assert_eq!(statuses, vec![StatusCode::OK; 2]);
    let captured = upstream.raw_requests.lock().unwrap();
    let exact: Vec<bool> = captured
        .iter()
        .map(|raw| {
            let text = std::str::from_utf8(raw).unwrap();
            text.contains(TOOL)
                && text.contains("18446744073709551617")
                && text.contains("0.12345678901234567890123456789")
        })
        .collect();
    assert_eq!(
        exact,
        vec![true; 2],
        "raw HTTP schemas must not round-trip through lossy Value numbers"
    );
    assert_eq!(
        captured[1],
        original_bodies[1].as_bytes(),
        "a request already in the plan format needs no rewriting"
    );
    assert_eq!(captured.len(), 2, "no inference retries");
}

#[tokio::test]
async fn plan_router_rejects_incompatible_requests_and_missing_scope_before_inference() {
    let directory = tempfile::tempdir().unwrap();
    let accounts = PlanAccounts::open(directory.path().join("plan.json")).unwrap();
    let fixture = Fixture::new().await;
    let id = sign_in(&accounts, &fixture, SCOPES, "validation").await;
    let db = Arc::new(Database::memory().unwrap());
    add(&db, &id);
    let key = qoder_gateway::prepare_key(&db).unwrap();
    let upstream = upstream().await;
    let router = router(db.clone(), accounts.clone(), &upstream);
    for (field, value) in [
        ("store", json!(true)),
        ("stream", json!(false)),
        ("input", json!("lost-history")),
        ("previous_response_id", json!("old")),
        ("temperature", json!(0.4)),
        (
            "tools",
            json!([{"type":"mcp","server_url":"https://must-not-contact.invalid"}]),
        ),
        (
            "tools",
            json!([{"type":"file_search","vector_store_ids":["unsupported"]}]),
        ),
        (
            "input",
            json!([{"type":"message","role":"system","content":"unsupported"}]),
        ),
        ("model", json!("hidden-model")),
    ] {
        let mut body = payload();
        body[field] = value;
        let response = dispatch(router.clone(), request(&key, &body)).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value: Value = serde_json::from_slice(&bytes(response).await).unwrap();
        assert_eq!(value["error"]["dispatchStarted"], false);
    }
    let reads_before = upstream.reads.lock().unwrap().len();
    let identity_only = sign_in(&accounts, &fixture, "openid profile email", "identity-only").await;
    add(&db, &identity_only);
    let response = dispatch(router, request(&key, &payload())).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        upstream.reads.lock().unwrap().len(),
        reads_before,
        "identity-only never exports a bearer to models"
    );
    assert!(upstream.requests.lock().unwrap().is_empty());
    assert_eq!(
        fixture.fields.lock().unwrap().len(),
        2,
        "only the two explicit logins, no automatic reconsent or refresh"
    );
}

#[tokio::test]
async fn plan_router_selection_during_catalog_wait_cannot_retarget_admitted_request() {
    let directory = tempfile::tempdir().unwrap();
    let accounts = PlanAccounts::open(directory.path().join("plan.json")).unwrap();
    let fixture = Fixture::new().await;
    let a = sign_in(&accounts, &fixture, SCOPES, "admitted-a").await;
    let b = sign_in(&accounts, &fixture, SCOPES, "later-b").await;
    let db = Arc::new(Database::memory().unwrap());
    let a_provider = add(&db, &a);
    let b_provider = add(&db, &b);
    qoder_gateway::select(&db, &a_provider).unwrap();
    let key = qoder_gateway::prepare_key(&db).unwrap();
    let upstream = upstream().await;
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    *upstream.gate.lock().unwrap() = Some((entered.clone(), release.clone()));
    let router = router(db.clone(), accounts, &upstream);
    let request = request(&key, &payload());
    let pending = tokio::spawn(async move { dispatch(router, request).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    qoder_gateway::select(&db, &b_provider).unwrap();
    release.notify_one();
    let response = pending.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    bytes(response).await;
    let received = upstream.requests.lock().unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(
        received[0].0[header::AUTHORIZATION],
        "Bearer synthetic-access-admitted-a"
    );
    assert_eq!(
        upstream.reads.lock().unwrap()[0][header::AUTHORIZATION],
        received[0].0[header::AUTHORIZATION]
    );
    assert_eq!(
        db.get_current_provider(qoder_gateway::APP)
            .unwrap()
            .as_deref(),
        Some(b_provider.as_str())
    );
}

#[test]
fn plan_configuration_cannot_attach_oauth_to_an_arbitrary_url_or_mix_api_keys() {
    for (base, key, format) in [
        (
            "https://foreign.invalid/v1",
            None,
            ApiFormat::OpenaiResponses,
        ),
        (
            inference::API_BASE,
            Some("must-not-mix".into()),
            ApiFormat::OpenaiResponses,
        ),
        (inference::API_BASE, None, ApiFormat::OpenaiChat),
    ] {
        let db = Database::memory().unwrap();
        assert!(qoder_gateway::save(
            &db,
            ProviderInput {
                id: None,
                name: "rejected".into(),
                base_url: base.into(),
                api_key: key,
                api_format: format,
                models: vec!["fixture-model".into()],
                plan_account_id: Some(uuid::Uuid::new_v4().to_string()),
                codex_account_id: None,
            }
        )
        .is_err());
        assert!(qoder_gateway::status(&db).unwrap().providers.is_empty());
    }
}

async fn changed_authorization_during_catalog_cannot_start_inference(sign_out: bool) {
    let directory = tempfile::tempdir().unwrap();
    let accounts = PlanAccounts::open(directory.path().join("plan.json")).unwrap();
    let fixture = Fixture::new().await;
    let id = sign_in(&accounts, &fixture, SCOPES, "lease-a").await;
    let db = Arc::new(Database::memory().unwrap());
    add(&db, &id);
    let key = qoder_gateway::prepare_key(&db).unwrap();
    let upstream = upstream().await;
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    *upstream.gate.lock().unwrap() = Some((entered.clone(), release.clone()));
    let router = router(db, accounts.clone(), &upstream);
    let request = request(&key, &payload());
    let pending = tokio::spawn(async move { dispatch(router, request).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    let discovery = fixture.discovery().await;
    let reauthorization = if sign_out {
        accounts.sign_out_with(&id, Some(&discovery)).await.unwrap();
        None
    } else {
        Some(
            accounts
                .begin(discovery, Some(&id), "same verified account")
                .await
                .unwrap(),
        )
    };
    release.notify_one();
    let response = pending.await.unwrap();
    let status = response.status();
    let body: Value = serde_json::from_slice(&bytes(response).await).unwrap_or(Value::Null);
    if let Some(launch) = reauthorization {
        accounts.cancel(&launch.view.attempt_id).await.unwrap();
    }
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "authorization changed before inference dispatch"
    );
    assert_eq!(body["error"]["dispatchStarted"], false);
    assert_eq!(
        body["error"]["code"],
        "QODER_GATEWAY_PLAN_AUTHORIZATION_CHANGED"
    );
    assert!(
        upstream.requests.lock().unwrap().is_empty(),
        "never POST the old or a replacement token after the catalog wait"
    );
    assert_eq!(
        upstream.reads.lock().unwrap().len(),
        1,
        "do not retry with another catalog or account"
    );
}

#[tokio::test]
async fn consumer_gap_sign_out_after_catalog_request_prevents_a_new_inference_post() {
    changed_authorization_during_catalog_cannot_start_inference(true).await;
}

#[tokio::test]
async fn consumer_gap_reauthorization_after_catalog_request_prevents_a_new_inference_post() {
    changed_authorization_during_catalog_cannot_start_inference(false).await;
}

#[tokio::test]
async fn consumer_gap_additional_tools_reject_hosted_tools_without_losing_supported_definitions() {
    let directory = tempfile::tempdir().unwrap();
    let accounts = PlanAccounts::open(directory.path().join("plan.json")).unwrap();
    let fixture = Fixture::new().await;
    let id = sign_in(&accounts, &fixture, SCOPES, "additional").await;
    let db = Arc::new(Database::memory().unwrap());
    add(&db, &id);
    let key = qoder_gateway::prepare_key(&db).unwrap();
    let upstream = upstream().await;
    let router = router(db, accounts, &upstream);
    let mut body = payload();
    body["input"].as_array_mut().unwrap().push(json!({"type":"additional_tools","role":"developer","tools":[
        {"type":"function","name":"later_function","parameters":{"type":"object","properties":{}}},
        {"type":"custom","name":"later_custom","format":{"type":"text"}}
    ]}));
    let response = dispatch(router.clone(), request(&key, &body)).await;
    assert_eq!(response.status(), StatusCode::OK);
    bytes(response).await;
    assert_eq!(
        upstream.requests.lock().unwrap()[0].1,
        body,
        "preserve additional_tools at its original position and schema"
    );
    let mut statuses = Vec::new();
    for kind in [
        "mcp",
        "tool_search",
        "file_search",
        "code_interpreter",
        "computer",
        "image_generation",
        "programmatic_tool_calling",
    ] {
        let mut forbidden = payload();
        forbidden["input"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"additional_tools","role":"developer","tools":[{"type":kind}]}));
        let response = dispatch(router.clone(), request(&key, &forbidden)).await;
        statuses.push(response.status());
        bytes(response).await;
    }
    assert_eq!(
        statuses,
        vec![StatusCode::BAD_REQUEST; 7],
        "all tool containers share the same pre-dispatch policy"
    );
    assert_eq!(upstream.requests.lock().unwrap().len(), 1);
    assert_eq!(upstream.reads.lock().unwrap().len(), 1);
}
