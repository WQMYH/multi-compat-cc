//! Isolated BYOK routes in the existing CC HTTP server. No native product state or retries.
use super::providers::chatgpt_plan_auth::inference;
use super::{hyper_client, server::ProxyState};
use crate::qoder_gateway::{self, ApiFormat};
use axum::{
    body::{to_bytes, Body},
    extract::{Path, Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use bytes::Bytes;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub(super) fn routes() -> Router<ProxyState> {
    Router::new()
        .route("/qoder/v1/models", get(models))
        .route("/qoder/v1/chat/completions", post(chat))
        .route("/qoder/v1/responses", post(responses))
        .route("/qoder/:provider_id/v1/chat/completions", post(qoder_pinned_chat))
        .route("/qoder/:provider_id/v1/responses", post(qoder_pinned_responses))
}

struct GatewayError {
    status: StatusCode,
    code: &'static str,
    request_id: String,
    dispatch_started: bool,
}
impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        log::warn!(
            "[QoderGateway] request_id={} dispatch_started={} error={}",
            self.request_id,
            self.dispatch_started,
            self.code
        );
        (
            self.status,
            Json(json!({"error": {
                "type":"qoder_gateway_error", "code":self.code, "message":self.code,
                "request_id":self.request_id, "dispatchStarted":self.dispatch_started,
                "retrySafe":!self.dispatch_started,
            }})),
        )
            .into_response()
    }
}
fn failure(
    status: StatusCode,
    code: &'static str,
    request_id: &str,
    dispatch_started: bool,
) -> GatewayError {
    GatewayError {
        status,
        code,
        request_id: request_id.into(),
        dispatch_started,
    }
}

fn plan_failure(error: String, id: &str) -> GatewayError {
    if error == "CHATGPT_PLAN_AUTHORIZATION_CHANGED" {
        failure(
            StatusCode::CONFLICT,
            "QODER_GATEWAY_PLAN_AUTHORIZATION_CHANGED",
            id,
            false,
        )
    } else {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "QODER_GATEWAY_PLAN_ACCOUNT_OR_CATALOG_UNAVAILABLE",
            id,
            false,
        )
    }
}

async fn models(
    State(state): State<ProxyState>,
    headers: HeaderMap,
) -> Result<Json<Value>, GatewayError> {
    let id = uuid::Uuid::new_v4().to_string();
    qoder_gateway::authorize(&state.db, &headers).map_err(|_| {
        failure(
            StatusCode::UNAUTHORIZED,
            "QODER_GATEWAY_UNAUTHORIZED",
            &id,
            false,
        )
    })?;
    let provider = qoder_gateway::selected(&state.db).map_err(|_| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "QODER_GATEWAY_NO_PROVIDER",
            &id,
            false,
        )
    })?;
    let mut listing = qoder_gateway::model_list(&provider).map_err(|_| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "QODER_GATEWAY_PROVIDER_INVALID",
            &id,
            false,
        )
    })?;
    if let Some(account) = qoder_gateway::codex_account_id(&provider).map_err(|_| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "QODER_GATEWAY_PROVIDER_INVALID",
            &id,
            false,
        )
    })? {
        let (_, catalog) = codex_catalog(&state, &account).await.map_err(|_| {
            failure(
                StatusCode::SERVICE_UNAVAILABLE,
                "QODER_GATEWAY_CODEX_ACCOUNT_OR_CATALOG_UNAVAILABLE",
                &id,
                false,
            )
        })?;
        let allowed = listing["data"].as_array().expect("generated model list");
        listing = json!({"object":"list", "data":catalog.into_iter()
            .filter(|model| allowed.iter().any(|value| value["id"] == model.id))
            .map(|model| json!({"id":model.id, "object":"model", "owned_by":"cc-codex-oauth"}))
            .collect::<Vec<_>>()});
    }
    if let Some(account) = qoder_gateway::plan_account_id(&provider).map_err(|_| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "QODER_GATEWAY_PROVIDER_INVALID",
            &id,
            false,
        )
    })? {
        let catalog = plan_catalog(&state, &account)
            .await
            .map_err(|error| plan_failure(error, &id))?;
        let allowed = listing["data"].as_array().expect("generated model list");
        listing = json!({"object":"list", "data":catalog.models.into_iter()
            .filter(|model| allowed.iter().any(|value| value["id"] == model.id))
            .map(|model| json!({"id":model.id, "object":"model", "display_name":model.display_name,
                "owned_by":format!("qoder-gateway:{}", provider.id)})).collect::<Vec<_>>()});
    }
    Ok(Json(listing))
}

async fn codex_catalog(
    state: &ProxyState,
    id: &str,
) -> Result<
    (
        super::providers::codex_oauth_auth::ManagedTokenBundle,
        Vec<crate::services::model_fetch::FetchedModel>,
    ),
    String,
> {
    use tauri::Manager;
    let app = state
        .app_handle
        .as_ref()
        .ok_or("CODEX_OAUTH_HOST_UNAVAILABLE")?;
    let auth = app.state::<crate::commands::CodexOAuthState>();
    let bundle = auth
        .0
        .get_valid_token_bundle_for_account(id)
        .await
        .map_err(|_| "CODEX_OAUTH_ACCOUNT_UNAVAILABLE")?;
    let catalog = crate::services::codex_oauth_models::fetch_models_with_token(
        &bundle.access_token,
        &bundle.chatgpt_account_id,
    )
    .await?;
    Ok((bundle, catalog))
}

async fn plan_catalog(state: &ProxyState, id: &str) -> Result<inference::Catalog, String> {
    use tauri::Manager;
    let accounts = state
        .chatgpt_plan_accounts
        .clone()
        .or_else(|| {
            state.app_handle.as_ref().and_then(|app| {
                app.try_state::<crate::commands::ChatgptPlanAuthState>()
                    .and_then(|state| state.manager().ok())
            })
        })
        .ok_or("CHATGPT_PLAN_STORE_UNAVAILABLE")?;
    #[cfg(test)]
    if let Some(base) = &state.chatgpt_plan_fixture_base {
        return inference::catalog_at(&accounts, id, base).await;
    }
    inference::catalog(&accounts, id).await
}

async fn chat(State(state): State<ProxyState>, request: Request) -> Result<Response, GatewayError> {
    forward(state, request, ApiFormat::OpenaiChat, "qoder", None).await
}
async fn responses(
    State(state): State<ProxyState>,
    request: Request,
) -> Result<Response, GatewayError> {
    forward(state, request, ApiFormat::OpenaiResponses, "qoder", None).await
}

async fn qoder_pinned_chat(State(state): State<ProxyState>, Path(id): Path<String>, request: Request) -> Result<Response, GatewayError> {
    forward(state, request, ApiFormat::OpenaiChat, "qoder", Some(&id)).await
}
async fn qoder_pinned_responses(State(state): State<ProxyState>, Path(id): Path<String>, request: Request) -> Result<Response, GatewayError> {
    forward(state, request, ApiFormat::OpenaiResponses, "qoder", Some(&id)).await
}

async fn forward(
    state: ProxyState,
    request: Request,
    api: ApiFormat,
    app: &str,
    provider_id: Option<&str>,
) -> Result<Response, GatewayError> {
    let started = Instant::now();
    let request_id = uuid::Uuid::new_v4().to_string();
    qoder_gateway::authorize(&state.db, request.headers()).map_err(|_| {
        failure(
            StatusCode::UNAUTHORIZED,
            "QODER_GATEWAY_UNAUTHORIZED",
            &request_id,
            false,
        )
    })?;
    // Capture identity and credentials before the first await; later selection/edit cannot retarget it.
    let provider = provider_id.map(|id| qoder_gateway::provider_for(&state.db, app, id))
        .unwrap_or_else(|| qoder_gateway::selected_for(&state.db, app)).map_err(|_| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "QODER_GATEWAY_NO_PROVIDER",
            &request_id,
            false,
        )
    })?;
    let mut body = to_bytes(request.into_body(), hyper_client::MAX_RESPONSE_BODY_BYTES)
        .await
        .map_err(|_| {
            failure(
                StatusCode::PAYLOAD_TOO_LARGE,
                "QODER_GATEWAY_BODY_TOO_LARGE",
                &request_id,
                false,
            )
        })?;
    let parsed: Value = serde_json::from_slice(&body).map_err(|_| {
        failure(
            StatusCode::BAD_REQUEST,
            "QODER_GATEWAY_JSON_INVALID",
            &request_id,
            false,
        )
    })?;
    let model = parsed.get("model").and_then(Value::as_str).ok_or_else(|| {
        failure(
            StatusCode::BAD_REQUEST,
            "QODER_GATEWAY_MODEL_REQUIRED",
            &request_id,
            false,
        )
    })?;
    let mut target = qoder_gateway::request_target(&provider, model, api).map_err(|_| {
        failure(
            StatusCode::BAD_REQUEST,
            "QODER_GATEWAY_MODEL_OR_FORMAT_NOT_ALLOWED",
            &request_id,
            false,
        )
    })?;
    let mut codex_workspace = None;
    if let Some(account) = &target.codex_account_id {
        if let Some(prepared) = inference::prepare_request(&parsed, &body)
            .map_err(|code| failure(StatusCode::BAD_REQUEST, code, &request_id, false))?
        {
            body = prepared.into();
        }
        let (bundle, catalog) = codex_catalog(&state, account).await.map_err(|_| {
            failure(
                StatusCode::SERVICE_UNAVAILABLE,
                "QODER_GATEWAY_CODEX_ACCOUNT_OR_CATALOG_UNAVAILABLE",
                &request_id,
                false,
            )
        })?;
        if !catalog.iter().any(|available| available.id == model) {
            return Err(failure(
                StatusCode::BAD_REQUEST,
                "QODER_GATEWAY_MODEL_NOT_AVAILABLE_TO_ACCOUNT",
                &request_id,
                false,
            ));
        }
        target.api_key = bundle.access_token;
        codex_workspace = Some(bundle.chatgpt_account_id);
    }
    let mut plan_lease = None;
    if let Some(account) = &target.plan_account_id {
        if let Some(prepared) = inference::prepare_request(&parsed, &body)
            .map_err(|code| failure(StatusCode::BAD_REQUEST, code, &request_id, false))?
        {
            body = prepared.into();
        }
        let catalog = plan_catalog(&state, account)
            .await
            .map_err(|error| plan_failure(error, &request_id))?;
        if !catalog.models.iter().any(|available| available.id == model) {
            return Err(failure(
                StatusCode::BAD_REQUEST,
                "QODER_GATEWAY_MODEL_NOT_AVAILABLE_TO_ACCOUNT",
                &request_id,
                false,
            ));
        }
        target.api_key = catalog.lease.token().to_owned();
        plan_lease = Some(catalog.lease);
        #[cfg(test)]
        if let Some(base) = &state.chatgpt_plan_fixture_base {
            target.url = format!("{base}/responses");
        }
    }
    let uri: Uri = target.url.parse().map_err(|_| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "QODER_GATEWAY_BASE_URL_INVALID",
            &request_id,
            false,
        )
    })?;
    let mut headers = HeaderMap::new();
    if let Some(workspace) = codex_workspace {
        headers.insert(
            "chatgpt-account-id",
            HeaderValue::from_str(&workspace).map_err(|_| {
                failure(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "QODER_GATEWAY_CODEX_IDENTITY_INVALID",
                    &request_id,
                    false,
                )
            })?,
        );
        headers.insert("originator", HeaderValue::from_static("codex_cli_rs"));
    }
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", target.api_key)).map_err(|_| {
            failure(
                StatusCode::SERVICE_UNAVAILABLE,
                "QODER_GATEWAY_CREDENTIAL_INVALID",
                &request_id,
                false,
            )
        })?,
    );
    headers.insert(
        header::HOST,
        HeaderValue::from_str(
            uri.authority()
                .ok_or_else(|| {
                    failure(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "QODER_GATEWAY_BASE_URL_INVALID",
                        &request_id,
                        false,
                    )
                })?
                .as_str(),
        )
        .map_err(|_| {
            failure(
                StatusCode::SERVICE_UNAVAILABLE,
                "QODER_GATEWAY_BASE_URL_INVALID",
                &request_id,
                false,
            )
        })?,
    );
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(
        header::ACCEPT,
        HeaderValue::from_static("application/json, text/event-stream"),
    );
    headers.insert(
        header::ACCEPT_ENCODING,
        HeaderValue::from_static("identity"),
    );
    headers.insert(
        "x-request-id",
        HeaderValue::from_str(&request_id).expect("generated UUID"),
    );
    let proxy = super::http_client::proxy_url_for(&target.url);
    if let Some(lease) = &plan_lease {
        lease
            .ensure_current()
            .await
            .map_err(|error| plan_failure(error, &request_id))?;
    }
    log::info!(
        "[QoderGateway] request_id={} provider_id={} api={} phase=dispatch",
        request_id,
        provider.id,
        api.as_str()
    );
    let response = tokio::time::timeout(
        Duration::from_secs(600),
        send_once(&uri, &headers, body, proxy.as_deref()),
    )
    .await
    .map_err(|_| {
        failure(
            StatusCode::GATEWAY_TIMEOUT,
            "QODER_GATEWAY_UPSTREAM_TIMEOUT",
            &request_id,
            true,
        )
    })?
    .map_err(|_| {
        failure(
            StatusCode::BAD_GATEWAY,
            "QODER_GATEWAY_UPSTREAM_UNKNOWN",
            &request_id,
            true,
        )
    })?;
    let status = response.status();
    log::info!(
        "[QoderGateway] request_id={} provider_id={} api={} status={} header_latency_ms={}",
        request_id,
        provider.id,
        api.as_str(),
        status.as_u16(),
        started.elapsed().as_millis()
    );
    state
        .current_providers
        .write()
        .await
        .insert(app.into(), (provider.id, provider.name));
    let mut builder = Response::builder()
        .status(status)
        .header("x-cc-switch-request-id", &request_id);
    // Only protocol headers; never reflect upstream cookies, credentials, redirects or hop-by-hop headers.
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_ENCODING,
        header::CACHE_CONTROL,
        header::RETRY_AFTER,
    ] {
        if let Some(value) = response.headers().get(&name) {
            builder = builder.header(name, value);
        }
    }
    builder
        .body(Body::from_stream(response.bytes_stream()))
        .map_err(|_| {
            failure(
                StatusCode::BAD_GATEWAY,
                "QODER_GATEWAY_RESPONSE_FAILED",
                &request_id,
                true,
            )
        })
}

async fn send_once(
    uri: &Uri,
    headers: &HeaderMap,
    body: Bytes,
    proxy: Option<&str>,
) -> Result<hyper_client::ProxyResponse, String> {
    let socks_proxy = proxy.filter(|url| {
        let scheme = url.split(':').next().unwrap_or_default();
        scheme.eq_ignore_ascii_case("socks5") || scheme.eq_ignore_ascii_case("socks5h")
    });
    if let Some(proxy) = socks_proxy {
        // shortcut: one client per SOCKS dispatch; cache by config revision if setup cost becomes material.
        let client = super::http_client::single_dispatch_proxy_client(proxy)?;
        return client
            .post(uri.to_string())
            .headers(headers.clone())
            .body(body)
            .send()
            .await
            .map(hyper_client::ProxyResponse::Reqwest)
            .map_err(|error| error.to_string());
    }
    // No raw->hyper fallback after a POST whose external effect may be unknown.
    hyper_client::send_raw_request(
        uri,
        &Method::POST,
        headers,
        &hyper_client::OriginalHeaderCases::default(),
        &body,
        proxy,
    )
    .await
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        proxy::{server::ProxyServer, ProxyConfig},
        qoder_gateway::ProviderInput,
        Database,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::{oneshot, Mutex},
        task::JoinHandle,
    };
    use tower::Service;

    struct TaskGuard(JoinHandle<()>);
    impl Drop for TaskGuard {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    struct Upstream {
        base: String,
        received: Arc<Mutex<Vec<(HeaderMap, Value)>>>,
        _task: TaskGuard,
    }
    async fn upstream(owner: &'static str) -> Upstream {
        let received = Arc::new(Mutex::new(Vec::new()));
        let capture = received.clone();
        let app = Router::new().fallback_service(post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let capture = capture.clone();
            async move {
                capture.lock().await.push((headers, body.clone()));
                if body.get("fixture_redirect") == Some(&json!(true)) {
                    return (StatusCode::TEMPORARY_REDIRECT, [(header::LOCATION, "/repeat")]).into_response();
                }
                if body.get("stream") == Some(&json!(true)) {
                    let chunks = if body.get("input").is_some() {
                        vec!["event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n", "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"r-1\",\"status\":\"completed\"}}\n\n"]
                    } else {
                        vec!["data: {\"id\":\"s-1\",\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n", "\ndata: [DONE]\n\n"]
                    };
                    return Response::builder().header(header::CONTENT_TYPE, "text/event-stream")
                        .body(Body::from_stream(futures::stream::iter(chunks.into_iter().map(|bytes| Ok::<_, std::io::Error>(Bytes::from_static(bytes.as_bytes())))))).unwrap();
                }
                if body.pointer("/messages/0/content") == Some(&json!("tool-fixture"))
                    && body.get("messages").and_then(Value::as_array).and_then(|messages| messages.last()).and_then(|message| message.get("role")) != Some(&json!("tool")) {
                    return Json(json!({"id":"c-1", "object":"chat.completion", "choices":[{"index":0,
                        "message":{"role":"assistant", "content":null, "tool_calls":[{"id":"call-1", "type":"function", "function":{"name":"fixture_tool", "arguments":"{\"n\":1}"}}]}, "finish_reason":"tool_calls"}]})).into_response();
                }
                Json(json!({"id":"c-2", "fixture_owner":owner, "choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]})).into_response()
            }
        }));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Upstream {
            base,
            received,
            _task: TaskGuard(task),
        }
    }
    async fn dispatch(
        mut router: Router,
        request: Request,
    ) -> Result<Response, std::convert::Infallible> {
        std::future::poll_fn(|cx| <Router as Service<Request>>::poll_ready(&mut router, cx))
            .await?;
        router.call(request).await
    }
    fn gateway(db: Arc<Database>) -> Router {
        ProxyServer::new(ProxyConfig::default(), db, None).build_router()
    }
    fn add(db: &Database, upstream: &Upstream, format: ApiFormat) -> String {
        let value = qoder_gateway::save(
            db,
            ProviderInput {
                id: None,
                name: "Synthetic provider".into(),
                plan_account_id: None,
                codex_account_id: None,
                base_url: upstream.base.clone(),
                api_key: Some("synthetic-upstream-key".into()),
                api_format: format,
                models: vec!["fixture-model".into()],
            },
        )
        .unwrap();
        value
            .providers
            .iter()
            .find(|provider| provider.base_url == upstream.base)
            .unwrap()
            .id
            .clone()
    }
    fn request(path: &str, key: &str, body: &Value) -> Request {
        Request::builder()
            .method("POST")
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {key}"))
            .header(header::CONTENT_TYPE, "application/json")
            .header("cookie", "must-not-reach-upstream")
            .body(Body::from(body.to_string()))
            .unwrap()
    }
    async fn json_body(response: Response) -> Value {
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
    }

    async fn socks_proxy(
        destination: std::net::SocketAddr,
    ) -> (String, Arc<AtomicUsize>, TaskGuard) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let task = TaskGuard(tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                count.fetch_add(1, Ordering::SeqCst);
                let mut greeting = [0; 2];
                socket.read_exact(&mut greeting).await.unwrap();
                assert_eq!(greeting[0], 5, "must use SOCKS, not HTTP CONNECT");
                let mut methods = vec![0; greeting[1] as usize];
                socket.read_exact(&mut methods).await.unwrap();
                assert!(methods.contains(&0));
                socket.write_all(&[5, 0]).await.unwrap();
                let mut connect = [0; 4];
                socket.read_exact(&mut connect).await.unwrap();
                assert_eq!(&connect[..3], &[5, 1, 0]);
                let length = match connect[3] {
                    1 => 4,
                    3 => socket.read_u8().await.unwrap() as usize,
                    other => panic!("unexpected address type: {other}"),
                };
                let mut host = vec![0; length];
                socket.read_exact(&mut host).await.unwrap();
                match connect[3] {
                    1 => assert_eq!(host, [127, 0, 0, 1]),
                    3 => assert_eq!(host, b"qoder-fixture.invalid"),
                    _ => unreachable!(),
                }
                assert_eq!(socket.read_u16().await.unwrap(), destination.port());
                let mut upstream = TcpStream::connect(destination).await.unwrap();
                socket
                    .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                    .await
                    .unwrap();
                tokio::io::copy_bidirectional(&mut socket, &mut upstream)
                    .await
                    .ok();
            }
        }));
        (address, calls, task)
    }

    #[tokio::test]
    async fn socks_transport_preserves_protocol_and_never_follows_redirects() {
        for scheme in ["SOCKS5", "socks5h"] {
            let upstream = upstream("SOCKS").await;
            let destination: std::net::SocketAddr = upstream
                .base
                .trim_start_matches("http://")
                .trim_end_matches("/v1")
                .parse()
                .unwrap();
            for redirect in [false, true] {
                let (proxy, calls, _task) = socks_proxy(destination).await;
                let host = if scheme == "socks5h" {
                    "qoder-fixture.invalid"
                } else {
                    "127.0.0.1"
                };
                let uri = format!("http://{host}:{}/v1/chat/completions", destination.port())
                    .parse()
                    .unwrap();
                let mut headers = HeaderMap::new();
                headers.insert(
                    header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer synthetic-upstream-key"),
                );
                headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                );
                let payload =
                    json!({"model":"fixture-model", "stream":true, "fixture_redirect":redirect});
                // Call the production transport seam explicitly; public loopback targets bypass proxies.
                let response = tokio::time::timeout(
                    Duration::from_secs(3),
                    send_once(
                        &uri,
                        &headers,
                        Bytes::from(payload.to_string()),
                        Some(&format!("{scheme}://{proxy}")),
                    ),
                )
                .await
                .unwrap()
                .unwrap();
                if redirect {
                    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
                } else {
                    use futures::TryStreamExt;
                    let chunks: Vec<Bytes> = response.bytes_stream().try_collect().await.unwrap();
                    let content = chunks.concat();
                    assert_eq!(content, b"data: {\"id\":\"s-1\",\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n");
                }
                assert_eq!(calls.load(Ordering::SeqCst), 1);
                let received = upstream.received.lock().await;
                assert_eq!(received.len(), if redirect { 2 } else { 1 });
                assert_eq!(received.last().unwrap().1, payload);
                assert_eq!(
                    received.last().unwrap().0[header::AUTHORIZATION],
                    "Bearer synthetic-upstream-key"
                );
            }
        }
    }

    async fn read_post(socket: &mut TcpStream) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let length = socket.read(&mut buffer).await.unwrap();
            assert_ne!(
                length, 0,
                "must receive the complete POST before disconnecting"
            );
            bytes.extend_from_slice(&buffer[..length]);
            assert!(bytes.len() < 1024 * 1024);
            if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                let length: usize = headers
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .unwrap()
                    .1
                    .trim()
                    .parse()
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    return bytes[end + 4..end + 4 + length].to_vec();
                }
            }
        }
    }

    #[tokio::test]
    async fn socks_disconnect_after_complete_post_does_not_retry() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let destination = listener.local_addr().unwrap();
        let (proxy, calls, _proxy_task) = socks_proxy(destination).await;
        let (observed_tx, observed_rx) = oneshot::channel();
        let count = Arc::new(AtomicUsize::new(0));
        let received_count = count.clone();
        let _upstream_task = TaskGuard(tokio::spawn(async move {
            let mut receipt = Some(observed_tx);
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                received_count.fetch_add(1, Ordering::SeqCst);
                let body = read_post(&mut socket).await;
                if let Some(receipt) = receipt.take() {
                    receipt.send(body).unwrap();
                }
                drop(socket);
            }
        }));
        let payload = Bytes::from_static(br#"{"model":"fixture-model","input":[]}"#);
        let uri = format!(
            "http://qoder-fixture.invalid:{}/v1/responses",
            destination.port()
        )
        .parse()
        .unwrap();
        let response = tokio::time::timeout(
            Duration::from_secs(3),
            send_once(
                &uri,
                &HeaderMap::new(),
                payload.clone(),
                Some(&format!("socks5h://{proxy}")),
            ),
        )
        .await
        .unwrap();
        assert!(response.is_err());
        assert_eq!(observed_rx.await.unwrap(), payload);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn actual_router_enforces_qoder_namespace_auth_and_model_format_before_dispatch() {
        let db = Arc::new(Database::memory().unwrap());
        let upstream = upstream("A").await;
        let router = gateway(db.clone());
        let rejected = dispatch(
            router.clone(),
            Request::builder()
                .uri("/qoder/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
        assert!(!qoder_gateway::status(&db).unwrap().key_configured);
        let key = qoder_gateway::prepare_key(&db).unwrap();
        let missing = dispatch(
            router.clone(),
            request(
                "/qoder/v1/chat/completions",
                &key,
                &json!({"model":"fixture-model"}),
            ),
        )
        .await
        .unwrap();
        assert_eq!(missing.status(), StatusCode::SERVICE_UNAVAILABLE);
        let id = add(&db, &upstream, ApiFormat::OpenaiChat);
        qoder_gateway::select(&db, &id).unwrap();
        for (path, token, body) in [
            (
                "/qoder/v1/chat/completions",
                "wrong",
                json!({"model":"fixture-model"}),
            ),
            (
                "/qoder/v1/chat/completions",
                key.as_str(),
                json!({"model":"codex-only-model"}),
            ),
            (
                "/qoder/v1/responses",
                key.as_str(),
                json!({"model":"fixture-model"}),
            ),
        ] {
            assert!(!dispatch(router.clone(), request(path, token, &body))
                .await
                .unwrap()
                .status()
                .is_success());
        }
        assert!(upstream.received.lock().await.is_empty());
        let models = dispatch(
            router.clone(),
            Request::builder()
                .uri("/qoder/v1/models")
                .header(header::AUTHORIZATION, format!("Bearer {key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(json_body(models).await["data"][0]["id"], "fixture-model");
        let health = dispatch(
            router,
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(health.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn actual_router_preserves_tool_roundtrip_messages_stream_and_credential_attribution() {
        let db = Arc::new(Database::memory().unwrap());
        let upstream = upstream("A").await;
        let id = add(&db, &upstream, ApiFormat::OpenaiChat);
        qoder_gateway::select(&db, &id).unwrap();
        let key = qoder_gateway::prepare_key(&db).unwrap();
        let router = gateway(db);
        let first = json!({"model":"fixture-model", "messages":[{"role":"user","content":"tool-fixture"}],
            "tools":[{"type":"function","function":{"name":"fixture_tool","parameters":{"type":"object","properties":{"n":{"type":"integer"}}}}}]});
        let reply = json_body(
            dispatch(
                router.clone(),
                request("/qoder/v1/chat/completions", &key, &first),
            )
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(reply["choices"][0]["finish_reason"], "tool_calls");
        let next = json!({"model":"fixture-model", "messages":[first["messages"][0].clone(),reply["choices"][0]["message"].clone(),
            {"role":"tool","tool_call_id":"call-1","content":"{\"result\":2}"}]});
        let final_reply = json_body(
            dispatch(
                router.clone(),
                request("/qoder/v1/chat/completions", &key, &next),
            )
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(final_reply["choices"][0]["message"]["content"], "ok");
        let mut streaming = next.clone();
        streaming["stream"] = json!(true);
        let response = dispatch(
            router,
            request("/qoder/v1/chat/completions", &key, &streaming),
        )
        .await
        .unwrap();
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/event-stream"
        );
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        assert!(std::str::from_utf8(&bytes)
            .unwrap()
            .ends_with("data: [DONE]\n\n"));
        let received = upstream.received.lock().await;
        assert_eq!(received.len(), 3);
        assert_eq!(received[0].1, first);
        assert_eq!(received[1].1, next);
        assert_eq!(received[2].1, streaming);
        for (headers, _) in received.iter() {
            assert_eq!(
                headers[header::AUTHORIZATION],
                "Bearer synthetic-upstream-key"
            );
            assert!(!headers.contains_key("cookie"));
            assert_ne!(headers[header::AUTHORIZATION], format!("Bearer {key}"));
        }
    }

    #[tokio::test]
    async fn source10_dsh_messages_enters_cc_common_forwarder() {
        let db = Arc::new(Database::memory().unwrap());
        let received = Arc::new(Mutex::new(Vec::new()));
        let capture = received.clone();
        let completed = json!({"id":"dsh-response", "object":"response", "model":"fixture-model", "status":"completed",
            "output":[{"id":"dsh-message", "type":"message", "role":"assistant", "status":"completed",
                "content":[{"type":"output_text", "text":"ok", "annotations":[]}]}],
            "usage":{"input_tokens":19,"output_tokens":3,"total_tokens":22,"input_tokens_details":{"cached_tokens":4}}});
        let app = Router::new().route("/v1/responses", post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let capture = capture.clone();
            let completed = completed.clone();
            async move {
                capture.lock().await.push((headers, body.clone()));
                let mut completed = completed;
                completed["id"] = json!(uuid::Uuid::new_v4().to_string());
                completed["output"][0]["id"] = json!(uuid::Uuid::new_v4().to_string());
                if body["stream"] == true {
                    let events = [
                        json!({"type":"response.created", "response":{"id":completed["id"],"model":"fixture-model","output":[]}}),
                        json!({"type":"response.output_item.added","output_index":0,"item":{"id":completed["output"][0]["id"],"type":"message","role":"assistant","content":[]}}),
                        json!({"type":"response.content_part.added","output_index":0,"content_index":0,"part":{"type":"output_text","text":""}}),
                        json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"ok"}),
                        json!({"type":"response.output_item.done","output_index":0,"item":completed["output"][0]}),
                        json!({"type":"response.completed","response":completed}),
                    ];
                    let chunks: Vec<_> = events.into_iter().map(|event| Ok::<_, std::io::Error>(Bytes::from(format!("event: {}\ndata: {}\n\n",event["type"].as_str().unwrap(),event)))).collect();
                    Response::builder().header(header::CONTENT_TYPE,"text/event-stream").body(Body::from_stream(futures::stream::iter(chunks))).unwrap()
                } else { Json(completed).into_response() }
            }
        }));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream = Upstream {
            base:format!("http://{}/v1",listener.local_addr().unwrap()), received,
            _task:TaskGuard(tokio::spawn(async move { axum::serve(listener,app).await.unwrap(); })),
        };
        let provider = crate::provider::Provider::with_id("dsh-common".into(), "DSH own row".into(), json!({
            "auth":{"OPENAI_API_KEY":"synthetic-upstream-key"},
            "config":format!("model = \"fixture-model\"\nmodel_provider = \"custom\"\n[model_providers.custom]\nbase_url = \"{}\"\nwire_api = \"responses\"", upstream.base)
        }), None);
        db.save_provider("dsh", &provider).unwrap();
        let key = qoder_gateway::prepare_key(&db).unwrap();
        let payload = json!({"model":"fixture-model", "max_tokens":32, "stream":true,
            "messages":[{"role":"user", "content":"synthetic-only"}]});
        let mut req = request("/dsh/dsh-common/v1/messages", &key, &payload);
        req.headers_mut().remove(header::AUTHORIZATION);
        req.headers_mut().insert("x-api-key", HeaderValue::from_str(&key).unwrap());
        let server = ProxyServer::new(ProxyConfig {enable_logging:true, ..Default::default()}, db.clone(), None);
        let router = server.build_router();
        let response = dispatch(router.clone(), req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let sse = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(sse.contains("content_block_delta"), "{sse}");
        assert!(sse.contains("message_stop"), "{sse}");
        let mut non_stream = payload.clone();
        non_stream["stream"] = json!(false);
        let reply = json_body(dispatch(router, request("/dsh/dsh-common/v1/messages", &key, &non_stream)).await.unwrap()).await;
        assert_eq!(reply["content"][0]["text"], "ok");
        let requests = upstream.received.lock().await;
        assert_eq!(requests.len(), 2);
        for (headers, body) in requests.iter() {
            assert!(body.get("input").is_some());
            assert_eq!(body["max_output_tokens"], 32);
            assert_eq!(headers[header::AUTHORIZATION], "Bearer synthetic-upstream-key");
            assert!(!headers.contains_key("x-api-key"));
        }
        drop(requests);
        let mut logs = db.get_request_logs(&Default::default(), 0, 10).unwrap();
        for _ in 0..50 {
            if logs.total == 2 { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
            logs = db.get_request_logs(&Default::default(), 0, 10).unwrap();
        }
        assert_eq!(logs.total, 2, "both response styles must use CC's usage logger");
        assert_ne!(logs.data[0].request_id, logs.data[1].request_id);
        for row in &logs.data {
            assert_eq!(row.app_type, "dsh");
            assert_eq!(row.provider_id, provider.id);
            assert_eq!(row.model, "fixture-model");
            assert_eq!(row.output_tokens, 3);
            assert_eq!(row.cache_read_tokens, 4);
        }
        assert!(server.get_status().await.active_targets.is_empty());
    }

    #[tokio::test]
    async fn source10_dsh_pinned_route_keeps_its_own_provider_and_global_routing_unchanged() {
        let db = Arc::new(Database::memory().unwrap());
        let a = upstream("Qoder").await;
        let b = upstream("DSH").await;
        let a_id = add(&db, &a, ApiFormat::OpenaiChat);
        qoder_gateway::select(&db, &a_id).unwrap();
        let provider = crate::provider::Provider::with_id("dsh-explicit".into(), "DSH managed row".into(), json!({
            "auth":{"OPENAI_API_KEY":"synthetic-upstream-key"},
            "config":format!("model = \"fixture-model\"\nmodel_provider = \"custom\"\n[model_providers.custom]\nbase_url = \"{}\"\nwire_api = \"chat\"", b.base)
        }), None);
        db.save_provider("dsh", &provider).unwrap();
        db.set_current_provider("dsh", &provider.id).unwrap();
        let key = qoder_gateway::prepare_key(&db).unwrap();
        let payload = json!({"model":"fixture-model","max_tokens":32,"messages":[{"role":"user","content":"synthetic-only"}]});
        let server = ProxyServer::new(ProxyConfig::default(), db.clone(), None);
        let router = server.build_router();
        let reply = json_body(dispatch(router.clone(), request("/dsh/dsh-explicit/v1/messages", &key, &payload)).await.unwrap()).await;
        assert_eq!(reply["content"][0]["text"], "ok");
        assert_eq!(b.received.lock().await.len(), 1);
        assert_eq!(b.received.lock().await[0].1["model"], "fixture-model");
        assert!(a.received.lock().await.is_empty());
        assert_eq!(db.get_current_provider("qoder").unwrap(), Some(a_id));
        assert!(db.get_current_provider("codex").unwrap().is_none());
        let other = crate::provider::Provider::with_id("other-dsh".into(), "Other".into(), json!({"auth":{"OPENAI_API_KEY":"synthetic-upstream-key"},
            "config":format!("model = \"fixture-model\"\nmodel_provider = \"custom\"\n[model_providers.custom]\nbase_url = \"{}\"\nwire_api = \"chat\"", a.base)}), None);
        db.save_provider("dsh", &other).unwrap();
        db.set_current_provider("dsh", &other.id).unwrap();
        let pinned = json_body(dispatch(router.clone(), request("/dsh/dsh-explicit/v1/messages", &key, &payload)).await.unwrap()).await;
        assert_eq!(pinned["content"][0]["text"], "ok");
        assert_eq!(b.received.lock().await.len(), 2);
        assert!(a.received.lock().await.is_empty());
        assert_eq!(db.get_current_provider("dsh").unwrap().as_deref(), Some("other-dsh"));
        let status = server.get_status().await;
        assert!(status.active_targets.is_empty());
        let invalid_model = json!({"model":"unconfigured", "messages":[]});
        let rejected = dispatch(router.clone(), request("/dsh/dsh-explicit/v1/messages", &key, &invalid_model)).await.unwrap();
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
        let missing = dispatch(router.clone(), request("/dsh/missing/v1/messages", &key, &payload)).await.unwrap();
        assert_eq!(missing.status(), StatusCode::BAD_REQUEST);
        let legacy = dispatch(router, request("/dsh/dsh-explicit/v1/responses", &key, &payload)).await.unwrap();
        assert_eq!(legacy.status(), StatusCode::CONFLICT);
        assert_eq!(b.received.lock().await.len(), 2, "invalid targets and legacy calls must never dispatch");
    }

    #[tokio::test]
    async fn actual_router_preserves_responses_function_output_and_terminal_event() {
        let db = Arc::new(Database::memory().unwrap());
        let upstream = upstream("Responses").await;
        let id = add(&db, &upstream, ApiFormat::OpenaiResponses);
        qoder_gateway::select(&db, &id).unwrap();
        let key = qoder_gateway::prepare_key(&db).unwrap();
        let payload = json!({"model":"fixture-model", "store":false, "stream":true,
            "previous_response_id":"r-before", "input":[{"type":"function_call_output","call_id":"call-1","output":"2"}]});
        let response = dispatch(gateway(db), request("/qoder/v1/responses", &key, &payload))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        assert!(std::str::from_utf8(&bytes)
            .unwrap()
            .contains("\"type\":\"response.completed\""));
        assert_eq!(upstream.received.lock().await[0].1, payload);
    }

    #[tokio::test]
    async fn changing_selection_during_body_read_cannot_retarget_the_started_request() {
        let db = Arc::new(Database::memory().unwrap());
        let a = upstream("A").await;
        let b = upstream("B").await;
        let a_id = add(&db, &a, ApiFormat::OpenaiChat);
        let b_id = add(&db, &b, ApiFormat::OpenaiChat);
        qoder_gateway::select(&db, &a_id).unwrap();
        let key = qoder_gateway::prepare_key(&db).unwrap();
        let router = gateway(db.clone());
        let body =
            json!({"model":"fixture-model", "messages":[{"role":"user","content":"snapshot"}]});
        let (ready_tx, ready_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let held_body = body.to_string();
        let mut held = request("/qoder/v1/chat/completions", &key, &body);
        *held.body_mut() = Body::from_stream(futures::stream::once(async move {
            let _ = ready_tx.send(());
            release_rx.await.unwrap();
            Ok::<_, std::io::Error>(Bytes::from(held_body))
        }));
        let in_flight = tokio::spawn(dispatch(router.clone(), held));
        tokio::time::timeout(Duration::from_secs(5), ready_rx)
            .await
            .unwrap()
            .unwrap();
        qoder_gateway::select(&db, &b_id).unwrap();
        release_tx.send(()).unwrap();
        let reply = json_body(in_flight.await.unwrap().unwrap()).await;
        assert_eq!(reply["fixture_owner"], "A");
        assert_eq!(a.received.lock().await.len(), 1);
        assert!(b.received.lock().await.is_empty());
        let next = json_body(
            dispatch(router, request("/qoder/v1/chat/completions", &key, &body))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(next["fixture_owner"], "B");
        assert_eq!(
            db.get_current_provider(qoder_gateway::APP).unwrap(),
            Some(b_id)
        );
    }

    #[tokio::test]
    async fn dropped_upstream_after_full_post_returns_unknown_without_a_second_attempt() {
        let db = Arc::new(Database::memory().unwrap());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let (observed_tx, observed_rx) = oneshot::channel();
        let task = TaskGuard(tokio::spawn(async move {
            let mut receipt = Some(observed_tx);
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                count.fetch_add(1, Ordering::SeqCst);
                let body = read_post(&mut socket).await;
                if let Some(receipt) = receipt.take() {
                    receipt.send(body).unwrap();
                }
                drop(socket); // The external effect is unknown: do not answer or retry it.
            }
        }));
        let created = qoder_gateway::save(
            &db,
            ProviderInput {
                id: None,
                name: "Disconnect fixture".into(),
                plan_account_id: None,
                codex_account_id: None,
                base_url: base,
                api_key: Some("synthetic-upstream-key".into()),
                api_format: ApiFormat::OpenaiChat,
                models: vec!["fixture-model".into()],
            },
        )
        .unwrap();
        qoder_gateway::select(&db, &created.providers[0].id).unwrap();
        let key = qoder_gateway::prepare_key(&db).unwrap();
        let payload = json!({"model":"fixture-model", "messages":[{"role":"user","content":"unknown-effect-fixture"}]});
        let response = dispatch(
            gateway(db),
            request("/qoder/v1/chat/completions", &key, &payload),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let failure = json_body(response).await;
        assert_eq!(failure["error"]["dispatchStarted"], true);
        assert_eq!(failure["error"]["retrySafe"], false);
        assert!(failure["error"]["request_id"].as_str().is_some());
        let observed = observed_rx.await.unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&observed).unwrap(), payload);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        drop(task);
    }
}
