//! Public ChatGPT-plan OAuth protocol. No Codex cache adoption or native configuration writes.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::signature;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use url::Url;

const ISSUER: &str = "https://auth.openai.com";
const RESOURCE: &str = "https://api.openai.com/v1";
const DIRECT_SCOPE: &str = "chatgpt.tokens.use.direct";
const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum PlanAuthError {
    #[error("chatgpt_plan_invalid:{0}")]
    Invalid(&'static str),
    #[error("chatgpt_plan_response_unknown:{0}")]
    Unknown(&'static str),
    #[error("chatgpt_plan_oauth:{status}:{code}")]
    OAuth { status: u16, code: &'static str },
}
type Result<T> = std::result::Result<T, PlanAuthError>;

#[derive(Clone, Deserialize)]
pub struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    revocation_endpoint: String,
}
impl Discovery {
    pub async fn official() -> Result<Self> {
        Self::load(ISSUER).await
    }

    async fn load(issuer: &str) -> Result<Self> {
        let base = Url::parse(issuer).map_err(|_| PlanAuthError::Invalid("issuer"))?;
        let allowed = issuer == ISSUER;
        #[cfg(test)]
        let allowed = allowed || (base.scheme() == "http" && base.host_str() == Some("127.0.0.1"));
        if !allowed {
            return Err(PlanAuthError::Invalid("issuer"));
        }
        let address = format!("{issuer}/.well-known/openid-configuration");
        let response = client(&address)?
            .get(&address)
            .timeout(HTTP_TIMEOUT)
            .send()
            .await
            .map_err(|_| PlanAuthError::Unknown("discovery"))?;
        let value = read_json(response).await?;
        let discovery: Self =
            serde_json::from_value(value).map_err(|_| PlanAuthError::Invalid("discovery"))?;
        if discovery.issuer != issuer {
            return Err(PlanAuthError::Invalid("issuer_mismatch"));
        }
        for endpoint in [
            &discovery.authorization_endpoint,
            &discovery.token_endpoint,
            &discovery.jwks_uri,
            &discovery.revocation_endpoint,
        ] {
            let url = Url::parse(endpoint).map_err(|_| PlanAuthError::Invalid("endpoint"))?;
            if url.origin() != base.origin()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(PlanAuthError::Invalid("endpoint_origin"));
            }
        }
        Ok(discovery)
    }

    async fn keys(&self) -> Result<Value> {
        let response = client(&self.jwks_uri)?
            .get(&self.jwks_uri)
            .timeout(HTTP_TIMEOUT)
            .send()
            .await
            .map_err(|_| PlanAuthError::Unknown("jwks"))?;
        read_json(response).await
    }
}

// Persistable before token exchange; this is registration metadata, not proof of consent.
#[derive(Clone, Serialize, Deserialize)]
pub struct Registration {
    issuer: String,
    client_id: String,
    host_id: String,
    subject: Option<String>,
}

pub struct AuthAttempt {
    issuer: String,
    state: String,
    nonce: String,
    verifier: String,
    host_id: String,
    redirect_uri: String,
    returning: Option<Registration>,
    authorize_url: Url,
    expires_at: i64,
    consumed: bool,
}

// Not Clone: one validated callback can authorize only one code-exchange attempt.
pub struct AuthCode {
    issuer: String,
    code: String,
    client_id: String,
    nonce: String,
    verifier: String,
    host_id: String,
    redirect_uri: String,
    expected_subject: Option<String>,
    expires_at: i64,
}

impl AuthAttempt {
    pub fn new(
        discovery: &Discovery,
        host_id: &str,
        redirect_uri: &str,
        returning: Option<&Grant>,
    ) -> Result<Self> {
        Self::start(
            discovery,
            host_id,
            redirect_uri,
            returning.map(Grant::registration).as_ref(),
            returning,
        )
    }

    /// Reauthorize an issued client after a failed exchange, with fresh PKCE/state/nonce.
    pub fn resume(
        discovery: &Discovery,
        registration: &Registration,
        redirect_uri: &str,
    ) -> Result<Self> {
        Self::start(
            discovery,
            &registration.host_id,
            redirect_uri,
            Some(registration),
            None,
        )
    }

    fn start(
        discovery: &Discovery,
        host_id: &str,
        redirect_uri: &str,
        registration: Option<&Registration>,
        returning: Option<&Grant>,
    ) -> Result<Self> {
        let redirect =
            Url::parse(redirect_uri).map_err(|_| PlanAuthError::Invalid("redirect_uri"))?;
        if host_id.trim().is_empty()
            || redirect.scheme() != "http"
            || redirect.host_str() != Some("127.0.0.1")
            || redirect.path() != "/auth/callback"
            || !redirect.username().is_empty()
            || redirect.password().is_some()
            || redirect.query().is_some()
            || redirect.fragment().is_some()
        {
            return Err(PlanAuthError::Invalid("redirect_uri_or_host"));
        }
        if registration.is_some_and(|value| {
            value.issuer != discovery.issuer
                || value.host_id != host_id
                || value.client_id.trim().is_empty()
                || value.client_id == "dynamic_agent_client"
        }) {
            return Err(PlanAuthError::Invalid("registration_host"));
        }
        let state = random_secret();
        let nonce = random_secret();
        let verifier = random_secret();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut url = Url::parse(&discovery.authorization_endpoint)
            .map_err(|_| PlanAuthError::Invalid("authorization_endpoint"))?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair(
                    "client_id",
                    registration.map_or("dynamic_agent_client", |value| &value.client_id),
                )
                .append_pair("ext_agent_host_id", host_id)
                .append_pair("response_type", "code")
                .append_pair("redirect_uri", redirect_uri)
                .append_pair("scope", SCOPES)
                .append_pair("resource", RESOURCE)
                .append_pair("state", &state)
                .append_pair("nonce", &nonce)
                .append_pair("code_challenge_method", "S256")
                .append_pair("code_challenge", &challenge);
            if let Some(grant) = returning {
                if !grant.id_token.is_empty() {
                    query.append_pair("id_token_hint", &grant.id_token);
                }
                if let Some(email) = &grant.email {
                    query.append_pair("login_hint", email);
                }
            } else if registration.is_none() {
                query.append_pair("agent_name_hint", "CC Switch");
            }
        }
        Ok(Self {
            issuer: discovery.issuer.clone(),
            state,
            nonce,
            verifier,
            host_id: host_id.into(),
            redirect_uri: redirect_uri.into(),
            returning: registration.cloned(),
            authorize_url: url,
            expires_at: now() + 600,
            consumed: false,
        })
    }

    /// Contains transaction secrets and possibly an ID-token hint; open it, never log it.
    pub fn authorize_url(&self) -> &str {
        self.authorize_url.as_str()
    }

    pub fn callback(&mut self, query: &HashMap<String, String>) -> Result<AuthCode> {
        if self.consumed || self.expires_at <= now() {
            return Err(PlanAuthError::Invalid("attempt_closed"));
        }
        if query.get("state") != Some(&self.state) {
            return Err(PlanAuthError::Invalid("state"));
        }
        self.consumed = true;
        if query.contains_key("error") {
            return Err(PlanAuthError::Invalid("consent_denied"));
        }
        let supplied_client = query.get("client_id").filter(|id| !id.trim().is_empty());
        let client_id = match &self.returning {
            Some(value) if supplied_client.is_none_or(|supplied| supplied == &value.client_id) => {
                value.client_id.clone()
            }
            Some(_) => return Err(PlanAuthError::Invalid("client_mismatch")),
            None => supplied_client
                .filter(|id| id.as_str() != "dynamic_agent_client")
                .cloned()
                .ok_or(PlanAuthError::Invalid("issued_client_missing"))?,
        };
        let code = query
            .get("code")
            .filter(|code| !code.trim().is_empty())
            .cloned()
            .ok_or(PlanAuthError::Invalid("code_missing"))?;
        Ok(AuthCode {
            issuer: self.issuer.clone(),
            code,
            client_id,
            nonce: self.nonce.clone(),
            verifier: self.verifier.clone(),
            host_id: self.host_id.clone(),
            redirect_uri: self.redirect_uri.clone(),
            expected_subject: self
                .returning
                .as_ref()
                .and_then(|value| value.subject.clone()),
            expires_at: self.expires_at,
        })
    }
}

impl AuthCode {
    /// Save this before consuming the code; a failed exchange must not lose its issued client.
    pub fn registration(&self) -> Registration {
        Registration {
            issuer: self.issuer.clone(),
            client_id: self.client_id.clone(),
            host_id: self.host_id.clone(),
            subject: self.expected_subject.clone(),
        }
    }

    pub async fn exchange(self, discovery: &Discovery) -> Result<Grant> {
        if self.issuer != discovery.issuer {
            return Err(PlanAuthError::Invalid("issuer_mismatch"));
        }
        if self.expires_at <= now() {
            return Err(PlanAuthError::Invalid("attempt_closed"));
        }
        let response = client(&discovery.token_endpoint)?
            .post(&discovery.token_endpoint)
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", self.client_id.as_str()),
                ("code", &self.code),
                ("code_verifier", &self.verifier),
                ("redirect_uri", &self.redirect_uri),
                ("resource", RESOURCE),
            ])
            .timeout(HTTP_TIMEOUT)
            .send()
            .await
            .map_err(|_| PlanAuthError::Unknown("code_exchange"))?;
        let received_at = now();
        let tokens: TokenResponse = serde_json::from_value(read_json(response).await?)
            .map_err(|_| PlanAuthError::Invalid("token_response"))?;
        let keys = discovery.keys().await?;
        self.finish(tokens, &keys, received_at, now())
    }

    fn finish(
        self,
        tokens: TokenResponse,
        keys: &Value,
        received_at: i64,
        verified_at: i64,
    ) -> Result<Grant> {
        if self.expires_at <= verified_at {
            return Err(PlanAuthError::Invalid("attempt_closed"));
        }
        let id_token = tokens
            .id_token
            .as_deref()
            .filter(|token| !token.is_empty())
            .ok_or(PlanAuthError::Invalid("id_token_missing"))?;
        let identity = verify_identity(
            id_token,
            keys,
            &self.issuer,
            &self.client_id,
            Some(&self.nonce),
            self.expected_subject.as_deref(),
            verified_at,
        )?;
        let mut grant = Grant {
            issuer: self.issuer,
            client_id: self.client_id,
            host_id: self.host_id,
            subject: identity.sub,
            email: identity.email,
            id_token: id_token.into(),
            access_token: String::new(),
            refresh_token: None,
            scopes: Vec::new(),
            expires_at: 0,
        };
        grant.apply_tokens(tokens, false, received_at)?;
        Ok(grant)
    }
}

// Serialized only by the protected auth store; never return this type to the webview.
#[derive(Clone, Serialize, Deserialize)]
pub struct Grant {
    issuer: String,
    client_id: String,
    host_id: String,
    subject: String,
    email: Option<String>,
    id_token: String,
    access_token: String,
    refresh_token: Option<String>,
    scopes: Vec<String>,
    expires_at: i64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantSummary {
    pub issuer: String,
    pub client_id: String,
    pub subject: String,
    pub email: Option<String>,
    pub inference_authorized: bool,
    pub expires_at: i64,
}
impl Grant {
    pub fn registration(&self) -> Registration {
        Registration {
            issuer: self.issuer.clone(),
            client_id: self.client_id.clone(),
            host_id: self.host_id.clone(),
            subject: Some(self.subject.clone()),
        }
    }

    pub fn summary(&self) -> GrantSummary {
        GrantSummary {
            issuer: self.issuer.clone(),
            client_id: self.client_id.clone(),
            subject: self.subject.clone(),
            email: self.email.clone(),
            inference_authorized: self.scopes.iter().any(|scope| scope == DIRECT_SCOPE),
            expires_at: self.expires_at,
        }
    }

    pub fn access_token(&self) -> Result<&str> {
        if self.expires_at <= now() {
            return Err(PlanAuthError::Invalid("access_expired"));
        }
        if !self.summary().inference_authorized {
            return Err(PlanAuthError::Invalid("plan_not_authorized"));
        }
        Ok(&self.access_token)
    }

    /// Caller serializes refresh for this registration and commits only if its generation is unchanged.
    pub async fn refreshed(&self, discovery: &Discovery) -> Result<Self> {
        if self.issuer != discovery.issuer {
            return Err(PlanAuthError::Invalid("issuer_mismatch"));
        }
        let refresh = self
            .refresh_token
            .as_deref()
            .ok_or(PlanAuthError::Invalid("sign_in_required"))?;
        let response = client(&discovery.token_endpoint)?
            .post(&discovery.token_endpoint)
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", self.client_id.as_str()),
                ("refresh_token", refresh),
                ("resource", RESOURCE),
            ])
            .timeout(HTTP_TIMEOUT)
            .send()
            .await
            .map_err(|_| PlanAuthError::Unknown("refresh"))?;
        let received_at = now();
        let tokens: TokenResponse = serde_json::from_value(read_json(response).await?)
            .map_err(|_| PlanAuthError::Invalid("token_response"))?;
        if let Some(id_token) = tokens.id_token.as_deref() {
            // Refresh is bound to the verified registration, not a new browser nonce.
            let keys = discovery.keys().await?;
            verify_identity(
                id_token,
                &keys,
                &self.issuer,
                &self.client_id,
                None,
                Some(&self.subject),
                now(),
            )?;
        }
        let mut updated = self.clone();
        updated.apply_tokens(tokens, true, received_at)?;
        Ok(updated)
    }

    /// A failed revocation is not confirmation. The owner clears local credentials and retains the mapping.
    pub async fn revoke(&self, discovery: &Discovery) -> Result<()> {
        if self.issuer != discovery.issuer {
            return Err(PlanAuthError::Invalid("issuer_mismatch"));
        }
        let refresh = self
            .refresh_token
            .as_deref()
            .ok_or(PlanAuthError::Invalid("refresh_missing"))?;
        let response = client(&discovery.revocation_endpoint)?
            .post(&discovery.revocation_endpoint)
            .form(&[
                ("token", refresh),
                ("token_type_hint", "refresh_token"),
                ("client_id", self.client_id.as_str()),
            ])
            .timeout(HTTP_TIMEOUT)
            .send()
            .await
            .map_err(|_| PlanAuthError::Unknown("revocation"))?;
        if response.status().as_u16() != 200 {
            return Err(PlanAuthError::OAuth {
                status: response.status().as_u16(),
                code: "revocation_unconfirmed",
            });
        }
        Ok(())
    }

    fn apply_tokens(
        &mut self,
        tokens: TokenResponse,
        refresh: bool,
        received_at: i64,
    ) -> Result<()> {
        if !tokens.token_type.eq_ignore_ascii_case("bearer")
            || tokens.access_token.is_empty()
            || tokens.access_token.chars().any(char::is_control)
            || tokens.expires_in <= 0
        {
            return Err(PlanAuthError::Invalid("token_response"));
        }
        if let Some(client_id) = &tokens.client_id {
            if client_id != &self.client_id {
                return Err(PlanAuthError::Invalid("client_mismatch"));
            }
        }
        if refresh && tokens.refresh_token.as_deref().is_none_or(str::is_empty) {
            return Err(PlanAuthError::Invalid("replacement_refresh_missing"));
        }
        self.expires_at = received_at
            .checked_add(tokens.expires_in)
            .ok_or(PlanAuthError::Invalid("expiry"))?;
        self.access_token = tokens.access_token;
        if let Some(token) = tokens.refresh_token.filter(|token| !token.is_empty()) {
            self.refresh_token = Some(token);
        }
        if let Some(scope) = tokens.scope {
            self.scopes = scope.split_ascii_whitespace().map(str::to_owned).collect();
        }
        if let Some(token) = tokens.id_token {
            self.id_token = token;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: i64,
    refresh_token: Option<String>,
    id_token: Option<String>,
    scope: Option<String>,
    client_id: Option<String>,
}
#[derive(Deserialize)]
struct Identity {
    iss: String,
    aud: Value,
    sub: String,
    exp: i64,
    nonce: Option<String>,
    email: Option<String>,
    azp: Option<String>,
    nbf: Option<i64>,
}

fn verify_identity(
    token: &str,
    keys: &Value,
    issuer: &str,
    client_id: &str,
    nonce: Option<&str>,
    subject: Option<&str>,
    verified_at: i64,
) -> Result<Identity> {
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3 || token.len() > MAX_RESPONSE_BYTES {
        return Err(PlanAuthError::Invalid("id_token"));
    }
    let decode = |part: &str| {
        URL_SAFE_NO_PAD
            .decode(part)
            .map_err(|_| PlanAuthError::Invalid("id_token_encoding"))
    };
    let header: Value = serde_json::from_slice(&decode(parts[0])?)
        .map_err(|_| PlanAuthError::Invalid("id_token_header"))?;
    if header["alg"] != "RS256" {
        return Err(PlanAuthError::Invalid("id_token_algorithm"));
    }
    let key = keys
        .get("keys")
        .and_then(Value::as_array)
        .and_then(|keys| {
            keys.iter().find(|key| {
                key["kid"].as_str().is_some()
                    && key["kid"] == header["kid"]
                    && key["kty"] == "RSA"
                    && key.get("use").is_none_or(|value| value == "sig")
                    && key.get("alg").is_none_or(|value| value == "RS256")
            })
        })
        .ok_or(PlanAuthError::Invalid("signing_key"))?;
    let n = decode(
        key["n"]
            .as_str()
            .ok_or(PlanAuthError::Invalid("signing_key"))?,
    )?;
    let e = decode(
        key["e"]
            .as_str()
            .ok_or(PlanAuthError::Invalid("signing_key"))?,
    )?;
    signature::RsaPublicKeyComponents { n: &n, e: &e }
        .verify(
            &signature::RSA_PKCS1_2048_8192_SHA256,
            format!("{}.{}", parts[0], parts[1]).as_bytes(),
            &decode(parts[2])?,
        )
        .map_err(|_| PlanAuthError::Invalid("id_token_signature"))?;
    let identity: Identity = serde_json::from_slice(&decode(parts[1])?)
        .map_err(|_| PlanAuthError::Invalid("id_token_claims"))?;
    let audience = identity.aud.as_str().is_some_and(|aud| aud == client_id)
        || identity.aud.as_array().is_some_and(|aud| {
            aud.iter().any(|value| value == client_id)
                && (aud.len() == 1 || identity.azp.as_deref() == Some(client_id))
        });
    if identity.iss != issuer
        || !audience
        || identity.azp.as_deref().is_some_and(|azp| azp != client_id)
        || identity.sub.trim().is_empty()
        || identity.exp <= verified_at
        || identity.nbf.is_some_and(|nbf| nbf > verified_at)
        || nonce.is_some_and(|nonce| identity.nonce.as_deref() != Some(nonce))
        || subject.is_some_and(|subject| identity.sub != subject)
    {
        return Err(PlanAuthError::Invalid("id_token_identity"));
    }
    Ok(identity)
}

fn client(url: &str) -> Result<reqwest::Client> {
    crate::proxy::http_client::single_dispatch_client_for(url)
        .map_err(|_| PlanAuthError::Invalid("http_client"))
}
async fn read_json(mut response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| PlanAuthError::Unknown("response_body"))?
    {
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(PlanAuthError::Invalid("response_size"));
        }
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let code = match value.get("error").and_then(Value::as_str) {
            Some("invalid_grant") => "invalid_grant",
            Some("invalid_client") => "invalid_client",
            Some("invalid_scope") => "invalid_scope",
            Some("access_denied") => "access_denied",
            _ => "oauth_error",
        };
        return Err(PlanAuthError::OAuth {
            status: status.as_u16(),
            code,
        });
    }
    serde_json::from_slice(&bytes).map_err(|_| PlanAuthError::Invalid("response_json"))
}
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
fn random_secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

#[cfg(test)]
#[path = "chatgpt_plan_auth_tests.rs"]
mod tests;

#[path = "chatgpt_plan_accounts.rs"]
pub mod accounts;

#[path = "chatgpt_plan_inference.rs"]
pub mod inference;

#[cfg(test)]
#[path = "chatgpt_plan_gateway_tests.rs"]
mod gateway_tests;
