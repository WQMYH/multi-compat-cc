use super::*;
use axum::{
    extract::Form,
    http::{header, StatusCode},
    routing::{get, post},
    Json, Router,
};
use base64::engine::general_purpose::STANDARD;
use ring::rand::SystemRandom;
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio::{net::TcpListener, task::JoinHandle};

struct Task(JoinHandle<()>);
impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}
pub(super) struct Fixture {
    issuer: String,
    metadata: Arc<Mutex<Value>>,
    pub(super) response: Arc<Mutex<(StatusCode, Value)>>,
    pub(super) fields: Arc<Mutex<Vec<HashMap<String, String>>>>,
    pub(super) gate: Arc<Mutex<Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>>>,
    redirected: Arc<AtomicUsize>,
    key: signature::RsaKeyPair,
    jwks: Value,
    _task: Task,
}
impl Fixture {
    pub(super) async fn new() -> Self {
        let output = std::process::Command::new("node")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../compat/qoder-cn/plan-contract/identity.mjs"
            ))
            .output()
            .expect("Node must run the synthetic identity fixture");
        assert!(output.status.success(), "synthetic key generator failed");
        let generated: Value = serde_json::from_slice(&output.stdout).unwrap();
        let key = signature::RsaKeyPair::from_pkcs8(
            &STANDARD
                .decode(generated["pkcs8"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap();
        let jwks = json!({"keys":[generated["jwk"]]});
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let metadata = Arc::new(Mutex::new(
            json!({"issuer":issuer, "authorization_endpoint":format!("{issuer}/authorize"),
            "token_endpoint":format!("{issuer}/token"), "jwks_uri":format!("{issuer}/jwks"), "revocation_endpoint":format!("{issuer}/revoke")}),
        ));
        let response = Arc::new(Mutex::new((StatusCode::OK, json!({}))));
        let fields = Arc::new(Mutex::new(Vec::new()));
        let meta = metadata.clone();
        let keys = jwks.clone();
        let reply = response.clone();
        let captures = fields.clone();
        let gate = Arc::new(Mutex::new(
            None::<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
        ));
        let token_gate = gate.clone();
        let revoked = fields.clone();
        let redirected = Arc::new(AtomicUsize::new(0));
        let redirect_count = redirected.clone();
        let app = Router::new()
            .route(
                "/.well-known/openid-configuration",
                get(move || {
                    let meta = meta.lock().unwrap().clone();
                    async move { Json(meta) }
                }),
            )
            .route(
                "/jwks",
                get(move || {
                    let keys = keys.clone();
                    async move { Json(keys) }
                }),
            )
            .route(
                "/token",
                post(move |Form(form): Form<HashMap<String, String>>| {
                    captures.lock().unwrap().push(form);
                    let (status, value) = reply.lock().unwrap().clone();
                    let gate = token_gate.lock().unwrap().clone();
                    async move {
                        if let Some((entered, release)) = gate {
                            entered.notify_one();
                            release.notified().await;
                        }
                        (status, Json(value))
                    }
                }),
            )
            .route(
                "/revoke",
                post(move |Form(form): Form<HashMap<String, String>>| {
                    revoked.lock().unwrap().push(form);
                    async { StatusCode::OK }
                }),
            )
            .route(
                "/token-redirect",
                post(move || {
                    redirect_count.fetch_add(1, Ordering::SeqCst);
                    async {
                        (
                            StatusCode::TEMPORARY_REDIRECT,
                            [(header::LOCATION, "/token")],
                            "not-json",
                        )
                    }
                }),
            );
        let task = Task(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
        Self {
            issuer,
            metadata,
            response,
            fields,
            gate,
            redirected,
            key,
            jwks,
            _task: task,
        }
    }
    pub(super) async fn discovery(&self) -> Discovery {
        Discovery::load(&self.issuer).await.unwrap()
    }
    fn claims(&self, nonce: Option<&str>) -> Value {
        let mut claims = json!({"iss":self.issuer, "aud":"issued-test-client", "sub":"synthetic-user",
            "exp":now()+3600, "email":"synthetic@example.test"});
        if let Some(nonce) = nonce {
            claims["nonce"] = json!(nonce);
        }
        claims
    }
    fn sign(&self, claims: &Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","kid":"synthetic-signing-key"}"#);
        let input = format!(
            "{header}.{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).unwrap())
        );
        let mut signed = vec![0; self.key.public().modulus_len()];
        self.key
            .sign(
                &signature::RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                input.as_bytes(),
                &mut signed,
            )
            .unwrap();
        format!("{input}.{}", URL_SAFE_NO_PAD.encode(signed))
    }
    pub(super) fn reply(&self, nonce: Option<&str>, scope: Option<&str>, suffix: &str) {
        let mut value = json!({"access_token":format!("synthetic-access-{suffix}"), "refresh_token":format!("synthetic-refresh-{suffix}"),
            "token_type":"Bearer", "expires_in":3600, "id_token":self.sign(&self.claims(nonce))});
        if let Some(scope) = scope {
            value["scope"] = json!(scope);
        }
        *self.response.lock().unwrap() = (StatusCode::OK, value);
    }
    pub(super) async fn grant(&self, scope: &str) -> Grant {
        self.grant_for_host(scope, "synthetic-host").await
    }

    pub(super) async fn grant_for_host(&self, scope: &str, host: &str) -> Grant {
        let discovery = self.discovery().await;
        let mut attempt = AuthAttempt::new(
            &discovery,
            host,
            "http://127.0.0.1:1455/auth/callback",
            None,
        )
        .unwrap();
        self.reply(Some(&attempt.nonce), Some(scope), "initial");
        let query = callback(&attempt);
        attempt
            .callback(&query)
            .unwrap()
            .exchange(&discovery)
            .await
            .unwrap()
    }
}
fn callback(attempt: &AuthAttempt) -> HashMap<String, String> {
    HashMap::from([
        ("state".into(), attempt.state.clone()),
        ("code".into(), "synthetic-code".into()),
        ("client_id".into(), "issued-test-client".into()),
    ])
}

#[tokio::test]
async fn real_http_registration_exchange_rotation_and_revocation_preserve_the_issued_identity() {
    let fixture = Fixture::new().await;
    let discovery = fixture.discovery().await;
    let mut attempt = AuthAttempt::new(
        &discovery,
        "synthetic-host",
        "http://127.0.0.1:54321/auth/callback",
        None,
    )
    .unwrap();
    let params: HashMap<_, _> = Url::parse(attempt.authorize_url())
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
    assert_eq!(params["client_id"], "dynamic_agent_client");
    assert_eq!(params["agent_name_hint"], "CC Switch");
    assert_eq!(params["resource"], RESOURCE);
    assert!(params["scope"]
        .split(' ')
        .any(|scope| scope == DIRECT_SCOPE));
    let verifier = attempt.verifier.clone();
    assert_eq!(
        params["code_challenge"],
        URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
    );
    fixture.reply(Some(&attempt.nonce), Some(SCOPES), "initial");
    let query = callback(&attempt);
    let grant = attempt
        .callback(&query)
        .unwrap()
        .exchange(&discovery)
        .await
        .unwrap();
    assert!(attempt.callback(&query).is_err());
    assert_eq!(grant.access_token().unwrap(), "synthetic-access-initial");
    let visible = serde_json::to_string(&grant.summary()).unwrap();
    for private in [
        &grant.access_token,
        &grant.id_token,
        grant.refresh_token.as_ref().unwrap(),
    ] {
        assert!(!visible.contains(private));
    }
    {
        let fields = fixture.fields.lock().unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0]["client_id"], "issued-test-client");
        assert_eq!(
            fields[0]["redirect_uri"],
            "http://127.0.0.1:54321/auth/callback"
        );
        assert_eq!(fields[0]["code_verifier"], verifier);
        assert_eq!(fields[0]["resource"], RESOURCE);
    }
    fixture.reply(None, None, "rotated");
    let rotated = grant.refreshed(&discovery).await.unwrap();
    assert_eq!(rotated.access_token().unwrap(), "synthetic-access-rotated");
    assert_eq!(rotated.subject, grant.subject);
    assert_eq!(rotated.scopes, grant.scopes);
    rotated.revoke(&discovery).await.unwrap();
    let fields = fixture.fields.lock().unwrap();
    assert_eq!(fields.len(), 3);
    assert_eq!(fields[1]["client_id"], "issued-test-client");
    assert_eq!(fields[1]["refresh_token"], "synthetic-refresh-initial");
    assert!(!fields[1].contains_key("scope"));
    assert_eq!(fields[2]["token"], "synthetic-refresh-rotated");
    assert_eq!(fields[2]["token_type_hint"], "refresh_token");
}

#[tokio::test]
async fn invalid_callbacks_and_signed_identity_mismatches_cannot_produce_a_grant() {
    let fixture = Fixture::new().await;
    let discovery = fixture.discovery().await;
    let mut attempt = AuthAttempt::new(
        &discovery,
        "synthetic-host",
        "http://127.0.0.1:1455/auth/callback",
        None,
    )
    .unwrap();
    let mut query = callback(&attempt);
    query.insert("state".into(), "foreign-state".into());
    assert!(attempt.callback(&query).is_err());
    assert!(!attempt.consumed);
    query.insert("state".into(), attempt.state.clone());
    query.remove("client_id");
    assert!(attempt.callback(&query).is_err());
    assert!(fixture.fields.lock().unwrap().is_empty());
    let claims = fixture.claims(Some("expected-nonce"));
    for (field, bad) in [
        ("iss", json!("https://foreign.invalid")),
        ("aud", json!("foreign-client")),
        ("sub", json!("foreign-subject")),
        ("exp", json!(now() - 1)),
        ("nonce", json!("foreign-nonce")),
    ] {
        let mut wrong = claims.clone();
        wrong[field] = bad;
        assert!(
            verify_identity(
                &fixture.sign(&wrong),
                &fixture.jwks,
                &fixture.issuer,
                "issued-test-client",
                Some("expected-nonce"),
                Some("synthetic-user"),
                now(),
            )
            .is_err(),
            "must reject {field}"
        );
    }
    let signed = fixture.sign(&claims);
    let mut damaged = signed.into_bytes();
    let last = damaged.len() - 4;
    damaged[last] = if damaged[last] == b'A' { b'B' } else { b'A' };
    assert!(verify_identity(
        std::str::from_utf8(&damaged).unwrap(),
        &fixture.jwks,
        &fixture.issuer,
        "issued-test-client",
        Some("expected-nonce"),
        None,
        now(),
    )
    .is_err());
    assert!(AuthAttempt::new(
        &discovery,
        "synthetic-host",
        "http://localhost:1455/auth/callback",
        None
    )
    .is_err());
    assert!(AuthAttempt::new(
        &discovery,
        "synthetic-host",
        "http://127.0.0.1:1455/callback",
        None
    )
    .is_err());
}

#[tokio::test]
async fn identity_only_consent_and_returning_account_selection_never_infer_plan_permission_or_switch_users(
) {
    let fixture = Fixture::new().await;
    let grant = fixture.grant("openid email profile").await;
    assert!(!grant.summary().inference_authorized);
    assert!(grant.access_token().is_err());
    let discovery = fixture.discovery().await;
    assert!(AuthAttempt::new(
        &discovery,
        "another-host",
        "http://127.0.0.1:1455/auth/callback",
        Some(&grant)
    )
    .is_err());
    let mut attempt = AuthAttempt::new(
        &discovery,
        "synthetic-host",
        "http://127.0.0.1:1455/auth/callback",
        Some(&grant),
    )
    .unwrap();
    let params: HashMap<_, _> = Url::parse(attempt.authorize_url())
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
    assert_eq!(params["client_id"], "issued-test-client");
    assert!(!params.contains_key("agent_name_hint"));
    assert_eq!(params["id_token_hint"], grant.id_token);
    let mut query = callback(&attempt);
    query.insert("client_id".into(), "foreign-client".into());
    assert!(attempt.callback(&query).is_err());
    assert_eq!(fixture.fields.lock().unwrap().len(), 1);
    let mut attempt = AuthAttempt::new(
        &discovery,
        "synthetic-host",
        "http://127.0.0.1:1455/auth/callback",
        Some(&grant),
    )
    .unwrap();
    let mut query = callback(&attempt);
    query.remove("client_id");
    let mut changed = fixture.claims(Some(&attempt.nonce));
    changed["sub"] = json!("another-user");
    fixture.reply(Some(&attempt.nonce), Some(SCOPES), "changed");
    fixture.response.lock().unwrap().1["id_token"] = json!(fixture.sign(&changed));
    assert!(attempt
        .callback(&query)
        .unwrap()
        .exchange(&discovery)
        .await
        .is_err());
    assert_eq!(grant.subject, "synthetic-user");
    assert!(!grant.summary().inference_authorized);
}

#[tokio::test]
async fn foreign_metadata_and_token_redirects_fail_without_forwarding_or_replaying_the_code() {
    let fixture = Fixture::new().await;
    fixture.metadata.lock().unwrap()["jwks_uri"] = json!("https://foreign.invalid/jwks");
    assert!(Discovery::load(&fixture.issuer).await.is_err());
    assert!(fixture.fields.lock().unwrap().is_empty());
    fixture.metadata.lock().unwrap()["jwks_uri"] = json!(format!("{}/jwks", fixture.issuer));
    fixture.metadata.lock().unwrap()["token_endpoint"] =
        json!(format!("{}/token-redirect", fixture.issuer));
    let discovery = fixture.discovery().await;
    let mut attempt = AuthAttempt::new(
        &discovery,
        "synthetic-host",
        "http://127.0.0.1:1455/auth/callback",
        None,
    )
    .unwrap();
    let query = callback(&attempt);
    assert!(matches!(
        attempt.callback(&query).unwrap().exchange(&discovery).await,
        Err(PlanAuthError::OAuth { status: 307, .. })
    ));
    assert_eq!(fixture.redirected.load(Ordering::SeqCst), 1);
    assert!(fixture.fields.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failed_or_malformed_refresh_is_single_attempt_and_cannot_mutate_the_original_grant() {
    let fixture = Fixture::new().await;
    let grant = fixture.grant(SCOPES).await;
    let discovery = fixture.discovery().await;
    *fixture.response.lock().unwrap() = (
        StatusCode::SERVICE_UNAVAILABLE,
        json!({"error":"synthetic-secret-must-not-be-a-diagnostic"}),
    );
    let failure = grant.refreshed(&discovery).await;
    assert!(matches!(
        failure,
        Err(PlanAuthError::OAuth {
            status: 503,
            code: "oauth_error"
        })
    ));
    assert_eq!(fixture.fields.lock().unwrap().len(), 2);
    assert_eq!(grant.access_token().unwrap(), "synthetic-access-initial");
    fixture.reply(None, None, "no-replacement");
    fixture
        .response
        .lock()
        .unwrap()
        .1
        .as_object_mut()
        .unwrap()
        .remove("refresh_token");
    assert!(grant.refreshed(&discovery).await.is_err());
    assert_eq!(
        grant.refresh_token.as_deref(),
        Some("synthetic-refresh-initial")
    );
    assert_eq!(grant.access_token().unwrap(), "synthetic-access-initial");
}

#[tokio::test]
async fn failed_initial_exchange_retains_registration_for_fresh_consent_without_replaying_the_code()
{
    let fixture = Fixture::new().await;
    let discovery = fixture.discovery().await;
    let mut first = AuthAttempt::new(
        &discovery,
        "synthetic-host",
        "http://127.0.0.1:1455/auth/callback",
        None,
    )
    .unwrap();
    let code = first.callback(&callback(&first)).unwrap();
    let saved = serde_json::to_string(&code.registration()).unwrap();
    for secret in [&code.code, &code.nonce, &code.verifier] {
        assert!(!saved.contains(secret));
    }
    *fixture.response.lock().unwrap() = (StatusCode::BAD_REQUEST, json!({"error":"invalid_grant"}));
    assert!(matches!(
        code.exchange(&discovery).await,
        Err(PlanAuthError::OAuth {
            status: 400,
            code: "invalid_grant"
        })
    ));

    let registration: Registration = serde_json::from_str(&saved).unwrap();
    assert!(registration.subject.is_none());
    let mut retry = AuthAttempt::resume(
        &discovery,
        &registration,
        "http://127.0.0.1:54321/auth/callback",
    )
    .unwrap();
    let params: HashMap<_, _> = Url::parse(retry.authorize_url())
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
    assert_eq!(params["client_id"], "issued-test-client");
    assert_eq!(params["ext_agent_host_id"], "synthetic-host");
    assert!(!params.contains_key("agent_name_hint"));
    assert!(!params.contains_key("id_token_hint"));
    assert_ne!(retry.state, first.state);
    assert_ne!(retry.nonce, first.nonce);
    assert_ne!(retry.verifier, first.verifier);
    fixture.reply(Some(&retry.nonce), Some(SCOPES), "fresh-consent");
    let mut query = callback(&retry);
    query.insert("code".into(), "synthetic-fresh-code".into());
    let grant = retry
        .callback(&query)
        .unwrap()
        .exchange(&discovery)
        .await
        .unwrap();
    assert_eq!(
        grant.access_token().unwrap(),
        "synthetic-access-fresh-consent"
    );
    {
        let fields = fixture.fields.lock().unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0]["code"], "synthetic-code");
        assert_eq!(fields[1]["code"], "synthetic-fresh-code");
        assert_eq!(fields[0]["client_id"], fields[1]["client_id"]);
    }

    let verified = grant.registration();
    let mut returning =
        AuthAttempt::resume(&discovery, &verified, "http://127.0.0.1:1455/auth/callback").unwrap();
    fixture.reply(Some(&returning.nonce), Some(SCOPES), "foreign-subject");
    let mut claims = fixture.claims(Some(&returning.nonce));
    claims["sub"] = json!("foreign-user");
    fixture.response.lock().unwrap().1["id_token"] = json!(fixture.sign(&claims));
    assert!(matches!(
        returning
            .callback(&callback(&returning))
            .unwrap()
            .exchange(&discovery)
            .await,
        Err(PlanAuthError::Invalid("id_token_identity"))
    ));
    assert_eq!(grant.subject, "synthetic-user");
}

#[tokio::test]
async fn exchange_completion_rechecks_the_deadline_and_keeps_the_token_receipt_time_anchor() {
    let fixture = Fixture::new().await;
    let discovery = fixture.discovery().await;
    let received_at = now() - 10;
    let mut attempt = AuthAttempt::new(
        &discovery,
        "synthetic-host",
        "http://127.0.0.1:1455/auth/callback",
        None,
    )
    .unwrap();
    fixture.reply(Some(&attempt.nonce), Some(SCOPES), "short-lived");
    let mut tokens: TokenResponse =
        serde_json::from_value(fixture.response.lock().unwrap().1.clone()).unwrap();
    tokens.expires_in = 1;
    let grant = attempt
        .callback(&callback(&attempt))
        .unwrap()
        .finish(tokens, &fixture.jwks, received_at, received_at + 5)
        .unwrap();
    assert_eq!(grant.summary().expires_at, received_at + 1);
    assert!(matches!(
        grant.access_token(),
        Err(PlanAuthError::Invalid("access_expired"))
    ));

    let mut attempt = AuthAttempt::new(
        &discovery,
        "synthetic-host",
        "http://127.0.0.1:1455/auth/callback",
        None,
    )
    .unwrap();
    fixture.reply(Some(&attempt.nonce), Some(SCOPES), "late-completion");
    let tokens: TokenResponse =
        serde_json::from_value(fixture.response.lock().unwrap().1.clone()).unwrap();
    let mut code = attempt.callback(&callback(&attempt)).unwrap();
    code.expires_at = received_at + 5;
    assert!(matches!(
        code.finish(tokens, &fixture.jwks, received_at, received_at + 5),
        Err(PlanAuthError::Invalid("attempt_closed"))
    ));
    assert!(fixture.fields.lock().unwrap().is_empty());
}

#[tokio::test]
async fn refresh_scope_narrowing_and_expiry_never_preserve_inference_permission_or_extend_validity()
{
    let fixture = Fixture::new().await;
    let grant = fixture.grant(SCOPES).await;
    let discovery = fixture.discovery().await;
    fixture.reply(None, Some("openid profile email"), "narrowed");
    let mut narrowed = grant.refreshed(&discovery).await.unwrap();
    assert!(!narrowed.summary().inference_authorized);
    assert!(matches!(
        narrowed.access_token(),
        Err(PlanAuthError::Invalid("plan_not_authorized"))
    ));
    assert!(grant.summary().inference_authorized);

    let mut tokens: TokenResponse =
        serde_json::from_value(fixture.response.lock().unwrap().1.clone()).unwrap();
    tokens.expires_in = 1;
    let received_at = now() - 10;
    narrowed.apply_tokens(tokens, true, received_at).unwrap();
    assert_eq!(narrowed.summary().expires_at, received_at + 1);
    assert!(matches!(
        narrowed.access_token(),
        Err(PlanAuthError::Invalid("access_expired"))
    ));
    assert_eq!(fixture.fields.lock().unwrap().len(), 2);
}
