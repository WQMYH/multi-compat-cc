use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::{
    extract::{Path, Request, State},
    http::{header, HeaderMap},
    response::Response,
    routing::post,
    Extension, Router,
};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::app_config::AppType;
use crate::database::Database;
use crate::provider::Provider;
use crate::services::provider::dsh_oauth;

use super::{handlers, server::ProxyState, ProxyError};

const KEY_PREFIX: &str = "ccsw-dsh-v1-";

pub(crate) fn new_dsh_oauth_route_key() -> String {
    format!(
        "{KEY_PREFIX}{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

#[derive(Clone)]
pub(crate) struct DshOAuthRouteBinding {
    provider_id: String,
    account_id: String,
    key_digest: [u8; 32],
    draft_digest: [u8; 32],
    generation: Uuid,
}

#[derive(Clone, Copy)]
pub(super) struct DshPeer {
    pub address: SocketAddr,
    pub generation: Uuid,
}

#[derive(Clone)]
pub(super) struct AuthorizedDshProvider(pub Provider);

#[derive(Default)]
struct ListenerState {
    address: Option<SocketAddr>,
    generation: Uuid,
    bindings: HashMap<String, DshOAuthRouteBinding>,
}

#[derive(Default)]
pub(super) struct DshOAuthRoutes {
    listener: Mutex<ListenerState>,
}

impl DshOAuthRoutes {
    fn lock(&self) -> std::sync::MutexGuard<'_, ListenerState> {
        self.listener.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn bind(&self, address: SocketAddr) -> Uuid {
        let mut listener = self.lock();
        listener.generation = Uuid::new_v4();
        listener.address = Some(address);
        listener.bindings.clear();
        listener.generation
    }

    pub(super) fn close(&self, generation: Option<Uuid>) {
        let mut listener = self.lock();
        if generation.is_some_and(|generation| generation != listener.generation) {
            return;
        }
        listener.address = None;
        listener.bindings.clear();
    }

    pub(super) fn base_url(&self, provider_id: &str) -> Result<String, String> {
        listener_url(self.lock().address, provider_id)
    }

    pub(super) fn prepare(
        &self,
        draft: &Provider,
        projected: &serde_json::Value,
    ) -> Result<DshOAuthRouteBinding, String> {
        let account_id = dsh_oauth::account_id(draft)
            .map_err(|_| "DSH OAuth requires an explicit managed account".to_string())?;
        let listener = self.lock();
        let base_url = listener_url(listener.address, &draft.id)?;
        let key = projected
            .pointer("/options/apiKey")
            .and_then(|key| key.as_str())
            .filter(|key| valid_key(key))
            .ok_or_else(|| "DSH OAuth requires a generated local route key".to_string())?;
        let expected = dsh_oauth::project(draft, &base_url, key)
            .map_err(|_| "Invalid DSH OAuth native projection".to_string())?;
        if expected != *projected {
            return Err("DSH OAuth projection does not match the current card and listener".into());
        }
        Ok(DshOAuthRouteBinding {
            provider_id: draft.id.clone(),
            account_id: account_id.to_owned(),
            key_digest: Sha256::digest(key.as_bytes()).into(),
            draft_digest: draft_digest(draft, account_id),
            generation: listener.generation,
        })
    }

    pub(super) fn activate(
        &self,
        db: &Database,
        binding: DshOAuthRouteBinding,
    ) -> Result<(), String> {
        current_provider(db, &binding)
            .map_err(|_| "DSH OAuth card changed before activation".to_string())?;
        let mut listener = self.lock();
        if !listener.address.is_some_and(|addr| addr.ip().is_loopback())
            || binding.generation != listener.generation
        {
            return Err("DSH OAuth listener changed; apply the card again".into());
        }
        listener
            .bindings
            .insert(binding.provider_id.clone(), binding);
        Ok(())
    }

    pub(super) fn deactivate(&self, provider_id: &str) {
        self.lock().bindings.remove(provider_id);
    }

    pub(super) fn has_active(&self, db: &Database) -> bool {
        let bindings: Vec<_> = {
            let listener = self.lock();
            if !listener.address.is_some_and(|addr| addr.ip().is_loopback()) {
                return false;
            }
            listener
                .bindings
                .values()
                .filter(|binding| binding.generation == listener.generation)
                .cloned()
                .collect()
        };
        bindings
            .iter()
            .any(|binding| current_provider(db, binding).is_ok())
    }

    pub(super) fn is_active(&self, db: &Database, provider_id: &str) -> bool {
        self.binding(provider_id)
            .is_some_and(|binding| current_provider(db, &binding).is_ok())
    }

    fn binding(&self, provider_id: &str) -> Option<DshOAuthRouteBinding> {
        let listener = self.lock();
        if !listener.address.is_some_and(|addr| addr.ip().is_loopback()) {
            return None;
        }
        listener
            .bindings
            .get(provider_id)
            .filter(|binding| binding.generation == listener.generation)
            .cloned()
    }

    fn authorize(
        &self,
        db: &Database,
        provider_id: &str,
        peer: DshPeer,
        headers: &HeaderMap,
    ) -> Result<Provider, ProxyError> {
        if !valid_provider_id(provider_id) || !peer.address.ip().is_loopback() {
            return Err(auth_error());
        }
        let binding = self.binding(provider_id).ok_or_else(auth_error)?;
        if binding.generation != peer.generation {
            return Err(auth_error());
        }
        let authorization = headers.get_all(header::AUTHORIZATION);
        let mut values = authorization.iter();
        let value = values
            .next()
            .and_then(|value| value.to_str().ok())
            .ok_or_else(auth_error)?;
        if values.next().is_some() {
            return Err(auth_error());
        }
        let (scheme, key) = value.split_once(' ').ok_or_else(auth_error)?;
        if !scheme.eq_ignore_ascii_case("Bearer") || !valid_key(key) {
            return Err(auth_error());
        }
        let presented: [u8; 32] = Sha256::digest(key.as_bytes()).into();
        if !bool::from(binding.key_digest.ct_eq(&presented)) {
            return Err(auth_error());
        }
        current_provider(db, &binding)
    }
}

fn listener_url(address: Option<SocketAddr>, provider_id: &str) -> Result<String, String> {
    if !valid_provider_id(provider_id) {
        return Err("Invalid DSH OAuth provider ID".into());
    }
    let address = address
        .filter(|address| address.port() != 0)
        .ok_or_else(|| "DSH_OAUTH_PROXY_UNAVAILABLE".to_string())?;
    if !address.ip().is_loopback() {
        return Err("DSH_OAUTH_NON_LOOPBACK".into());
    }
    Ok(format!("http://{address}/dsh/oauth/{provider_id}/v1"))
}

fn valid_provider_id(id: &str) -> bool {
    !id.is_empty()
        && id != "deepseek-official"
        && !id.starts_with('-')
        && !id.ends_with('-')
        && !id.contains("--")
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_key(key: &str) -> bool {
    key.strip_prefix(KEY_PREFIX).is_some_and(|key| {
        key.len() == 64
            && key
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

pub(super) fn bound_account(provider: &Provider) -> Option<&str> {
    let meta = provider.meta.as_ref()?;
    if meta.live_config_managed != Some(true)
        || provider.settings_config["api"].as_str() != Some("openai-responses")
    {
        return None;
    }
    dsh_oauth::account_id(provider).ok()
}

fn draft_digest(provider: &Provider, account_id: &str) -> [u8; 32] {
    let descriptor = serde_json::json!({
        "id": provider.id,
        "name": provider.name,
        "accountId": account_id,
        "settings": provider.settings_config,
    });
    let canonical = super::json_canonical::canonicalize_value(descriptor).to_string();
    Sha256::digest(canonical.as_bytes()).into()
}

fn current_provider(db: &Database, binding: &DshOAuthRouteBinding) -> Result<Provider, ProxyError> {
    let provider = db
        .get_provider_by_id(&binding.provider_id, AppType::Dsh.as_str())
        .map_err(|_| auth_error())?
        .ok_or_else(auth_error)?;
    if bound_account(&provider) != Some(binding.account_id.as_str())
        || dsh_oauth::model_ids(&provider).is_err()
        || draft_digest(&provider, &binding.account_id) != binding.draft_digest
    {
        return Err(auth_error());
    }
    Ok(provider)
}

pub(super) fn auth_error() -> ProxyError {
    ProxyError::AuthError("DSH OAuth card is unavailable or unauthorized".into())
}

pub(super) fn routes(bindings: Arc<DshOAuthRoutes>) -> Router<ProxyState> {
    Router::new()
        .route("/dsh/oauth/:provider_id/v1/responses", post(responses))
        .layer(Extension(bindings))
}

async fn responses(
    State(state): State<ProxyState>,
    Extension(bindings): Extension<Arc<DshOAuthRoutes>>,
    Path(provider_id): Path<String>,
    peer: Option<Extension<DshPeer>>,
    mut request: Request,
) -> Result<Response, ProxyError> {
    let Extension(peer) = peer.ok_or_else(auth_error)?;
    let provider = bindings.authorize(&state.db, &provider_id, peer, request.headers())?;

    // The local capability and any client identity headers must stay local.
    let mut headers = HeaderMap::new();
    for name in [
        header::ACCEPT,
        header::CONTENT_TYPE,
        header::CONTENT_ENCODING,
    ] {
        if let Some(value) = request.headers().get(&name) {
            headers.insert(name, value.clone());
        }
    }
    *request.headers_mut() = headers;
    request
        .extensions_mut()
        .insert(AuthorizedDshProvider(provider));
    handlers::handle_responses_for_app(
        state,
        request,
        AppType::Dsh,
        "DSH",
        "dsh",
        handlers::ResponsesEndpoint::Responses,
    )
    .await
}

#[cfg(test)]
#[derive(Clone)]
pub(super) struct DshForwardTestFixture {
    pub manager: Arc<super::providers::codex_oauth_auth::CodexOAuthManager>,
    pub upstream: SocketAddr,
    pub client: reqwest::Client,
}

#[cfg(test)]
impl DshForwardTestFixture {
    pub(super) fn loopback_url(&self, url: &str) -> Result<String, ProxyError> {
        let expected = format!("{}/responses", super::providers::CHATGPT_CODEX_BASE_URL);
        if !self.upstream.ip().is_loopback() || self.upstream.port() == 0 || url != expected {
            return Err(ProxyError::ConfigError(
                "DSH forwarding fixture requires the canonical Responses target and a loopback upstream".into(),
            ));
        }
        Ok(format!(
            "http://{}/backend-api/codex/responses",
            self.upstream
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{AuthBinding, AuthBindingSource, ProviderMeta};
    use axum::{body::Body, http::StatusCode};
    use serde_json::json;

    fn draft(id: &str, account: &str) -> Provider {
        let mut provider = Provider::with_id(
            id.into(),
            "Synthetic account card".into(),
            json!({
                "api": "openai-responses",
                "models": {"fixture-one": {}, "fixture-two": {"reasoning": true}}
            }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            provider_type: Some("codex_oauth".into()),
            auth_binding: Some(AuthBinding {
                source: AuthBindingSource::ManagedAccount,
                auth_provider: Some("codex_oauth".into()),
                account_id: Some(account.into()),
            }),
            live_config_managed: Some(false),
            ..ProviderMeta::default()
        });
        provider
    }

    fn publish(
        db: &Database,
        bindings: &DshOAuthRoutes,
        provider: &mut Provider,
        key: &str,
    ) -> DshOAuthRouteBinding {
        let url = bindings.base_url(&provider.id).unwrap();
        let projection = dsh_oauth::project(provider, &url, key).unwrap();
        let binding = bindings.prepare(provider, &projection).unwrap();
        provider.meta.as_mut().unwrap().live_config_managed = Some(true);
        db.save_provider("dsh", provider).unwrap();
        bindings.activate(db, binding.clone()).unwrap();
        binding
    }

    async fn request(
        app: &mut Router,
        path: &str,
        key: &str,
        peer: DshPeer,
        model: &str,
    ) -> StatusCode {
        let mut request = axum::http::Request::builder()
            .method("POST")
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {key}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({"model": model, "input": "synthetic"}).to_string(),
            ))
            .unwrap();
        request.extensions_mut().insert(peer);
        tower::Service::call(app, request).await.unwrap().status()
    }

    #[tokio::test]
    async fn route_requires_local_listener_current_card_key_and_model() {
        let db = Arc::new(Database::memory().unwrap());
        let bindings = Arc::new(DshOAuthRoutes::default());
        let address = "127.0.0.1:15721".parse().unwrap();
        let generation = bindings.bind(address);
        let peer = DshPeer {
            address,
            generation,
        };
        let mut app = routes(bindings.clone()).with_state(ProxyState::for_test(db.clone()));
        let path = "/dsh/oauth/first-card/v1/responses";
        let key = new_dsh_oauth_route_key();
        let mut provider = draft("first-card", "fixture-account-one");
        db.save_provider("dsh", &provider).unwrap();
        assert_eq!(
            request(&mut app, path, &key, peer, "fixture-one").await,
            StatusCode::UNAUTHORIZED
        );

        publish(&db, &bindings, &mut provider, &key);
        assert!(bindings.is_active(&db, &provider.id));
        for invalid_key in ["PROXY_MANAGED".to_string(), new_dsh_oauth_route_key()] {
            assert_eq!(
                request(&mut app, path, &invalid_key, peer, "fixture-one").await,
                StatusCode::UNAUTHORIZED
            );
        }
        let remote_peer = DshPeer {
            address: "192.0.2.1:40000".parse().unwrap(),
            generation,
        };
        assert_eq!(
            request(&mut app, path, &key, remote_peer, "fixture-one").await,
            StatusCode::UNAUTHORIZED
        );
        // A valid grant reaches model validation, without an OAuth manager or an upstream socket.
        assert_eq!(
            request(&mut app, path, &key, peer, "unlisted").await,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            request(
                &mut app,
                "/dsh/oauth/second-card/v1/responses",
                &key,
                peer,
                "fixture-one"
            )
            .await,
            StatusCode::UNAUTHORIZED
        );

        provider.settings_config["models"]["fixture-two"]["reasoning"] = json!(false);
        db.save_provider("dsh", &provider).unwrap();
        assert!(!bindings.has_active(&db));
        assert!(!bindings.is_active(&db, &provider.id));
        assert_eq!(
            request(&mut app, path, &key, peer, "fixture-one").await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn bindings_expire_on_restart_or_card_removal() {
        let db = Arc::new(Database::memory().unwrap());
        let bindings = Arc::new(DshOAuthRoutes::default());
        let address = "127.0.0.1:15721".parse().unwrap();
        let first_generation = bindings.bind(address);
        let key = new_dsh_oauth_route_key();
        let mut provider = draft("first-card", "fixture-account-one");
        let old_binding = publish(&db, &bindings, &mut provider, &key);
        let path = "/dsh/oauth/first-card/v1/responses";
        let mut app = routes(bindings.clone()).with_state(ProxyState::for_test(db.clone()));

        bindings.close(Some(first_generation));
        assert!(!bindings.has_active(&db));
        let generation = bindings.bind(address);
        assert!(bindings.activate(&db, old_binding).is_err());
        let peer = DshPeer {
            address,
            generation,
        };
        assert_eq!(
            request(&mut app, path, &key, peer, "unlisted").await,
            StatusCode::UNAUTHORIZED
        );
        let new_key = new_dsh_oauth_route_key();
        publish(&db, &bindings, &mut provider, &new_key);
        let old_peer = DshPeer {
            address,
            generation: first_generation,
        };
        assert_eq!(
            request(&mut app, path, &new_key, old_peer, "unlisted").await,
            StatusCode::UNAUTHORIZED
        );
        db.delete_provider("dsh", &provider.id).unwrap();
        assert!(!bindings.has_active(&db));
        assert_eq!(
            request(&mut app, path, &new_key, peer, "unlisted").await,
            StatusCode::UNAUTHORIZED
        );

        bindings.bind("0.0.0.0:15721".parse().unwrap());
        assert!(bindings.base_url(&provider.id).is_err());
    }

    #[tokio::test]
    async fn forwards_selected_account_for_streaming_and_json_responses() {
        use std::time::Duration;

        type CapturedRequests = Arc<Mutex<Vec<(HeaderMap, serde_json::Value)>>>;
        async fn capture(
            State(captured): State<CapturedRequests>,
            request: Request,
        ) -> ([(axum::http::HeaderName, &'static str); 1], &'static str) {
            let (parts, body) = request.into_parts();
            let bytes = axum::body::to_bytes(body, 64 * 1024).await.unwrap();
            let body = serde_json::from_slice(&bytes).unwrap();
            captured.lock().unwrap().push((parts.headers, body));
            (
                [(header::CONTENT_TYPE, "text/event-stream")],
                concat!(
                    "event: response.completed\n",
                    "data: {\"type\":\"response.completed\",\"response\":{",
                    "\"id\":\"resp_fixture\",\"object\":\"response\",\"status\":\"completed\",",
                    "\"model\":\"fixture-two\",\"output\":[{\"type\":\"message\",",
                    "\"id\":\"msg_fixture\",\"role\":\"assistant\",\"content\":[{",
                    "\"type\":\"output_text\",\"text\":\"fixture answer\",\"annotations\":[]}]}],",
                    "\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n"
                ),
            )
        }

        let captured = CapturedRequests::default();
        let upstream = Router::new()
            .route("/backend-api/codex/responses", post(capture))
            .with_state(captured.clone());
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let upstream_addr = listener.local_addr().unwrap();
        let upstream_task =
            tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });

        let temp = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let manager = Arc::new(
            super::super::providers::codex_oauth_auth::CodexOAuthManager::new(
                temp.path().to_path_buf(),
            ),
        );
        for (id, workspace, token) in [
            (
                "default-account",
                "default-workspace",
                "fixture-default-access",
            ),
            (
                "selected-account",
                "selected-workspace",
                "fixture-selected-access",
            ),
        ] {
            let identity = crate::codex_config::test_codex_id_token(id);
            manager
                .add_test_account_with_workspace_and_access_token(
                    id,
                    workspace,
                    token,
                    Some(&identity),
                )
                .await
                .unwrap();
        }
        manager
            .set_default_account("default-account")
            .await
            .unwrap();
        let fixture = DshForwardTestFixture {
            manager,
            upstream: upstream_addr,
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
        };

        let db = Arc::new(Database::memory().unwrap());
        let state = ProxyState::for_test(db.clone());
        state.current_providers.write().await.insert(
            "codex".into(),
            ("codex-current".into(), "Fixture Codex selection".into()),
        );
        let bindings = Arc::new(DshOAuthRoutes::default());
        let address = "127.0.0.1:15721".parse().unwrap();
        let peer = DshPeer {
            address,
            generation: bindings.bind(address),
        };
        let mut app = routes(bindings.clone()).with_state(state.clone());
        let local_key = new_dsh_oauth_route_key();
        let mut provider = draft("bound-card", "selected-account");
        publish(&db, &bindings, &mut provider, &local_key);

        for wants_stream in [true, false] {
            let mut request = axum::http::Request::builder()
                .method("POST")
                .uri("/dsh/oauth/bound-card/v1/responses")
                .header(header::AUTHORIZATION, format!("Bearer {local_key}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("chatgpt-account-id", "spoofed-workspace")
                .header("x-api-key", local_key.as_str())
                .body(Body::from(
                    json!({
                        "model": "fixture-two",
                        "input": "synthetic prompt",
                        "stream": wants_stream,
                        "store": true,
                    })
                    .to_string(),
                ))
                .unwrap();
            request.extensions_mut().insert(peer);
            request.extensions_mut().insert(fixture.clone());

            let response = tokio::time::timeout(
                Duration::from_secs(5),
                tower::Service::call(&mut app, request),
            )
            .await
            .unwrap()
            .unwrap();
            let (parts, body) = response.into_parts();
            let bytes = tokio::time::timeout(
                Duration::from_secs(5),
                axum::body::to_bytes(body, 64 * 1024),
            )
            .await
            .unwrap()
            .unwrap();
            let text = std::str::from_utf8(&bytes).unwrap();
            assert_eq!(parts.status, StatusCode::OK, "{text}");
            assert!(!text.contains(&local_key));
            assert!(!text.contains("fixture-selected-access"));
            if wants_stream {
                assert_eq!(parts.headers[header::CONTENT_TYPE], "text/event-stream");
                assert!(text.contains("event: response.completed"));
                assert!(text.contains("fixture answer"));
            } else {
                assert_eq!(parts.headers[header::CONTENT_TYPE], "application/json");
                let response: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(response["id"], "resp_fixture");
                assert_eq!(
                    response["output"][0]["content"][0]["text"],
                    "fixture answer"
                );
            }
        }

        upstream_task.abort();
        assert!(upstream_task.await.unwrap_err().is_cancelled());
        let current = state.current_providers.read().await;
        assert_eq!(current.get("codex").unwrap().0, "codex-current");
        assert!(!current.contains_key("dsh"));
        let captured = captured.lock().unwrap();
        assert_eq!(captured.len(), 2);
        for (headers, body) in captured.iter() {
            assert_eq!(
                headers[header::AUTHORIZATION],
                "Bearer fixture-selected-access"
            );
            assert_eq!(headers["chatgpt-account-id"], "selected-workspace");
            assert!(!headers.contains_key("x-api-key"));
            assert!(headers.values().all(|value| {
                let value = value.to_str().unwrap_or_default();
                !value.contains(&local_key)
                    && !value.contains("spoofed-workspace")
                    && !value.contains("fixture-default-access")
            }));
            assert_eq!(body["model"], "fixture-two");
            assert_eq!(body["stream"], true);
            assert_eq!(body["store"], false);
            let body_text = body.to_string();
            for credential in [
                local_key.as_str(),
                "fixture-default-access",
                "fixture-selected-access",
                "spoofed-workspace",
            ] {
                assert!(!body_text.contains(credential));
            }
        }
    }
}
