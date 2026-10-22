//! CC-managed ChatGPT plan registrations, separate from native Codex OAuth accounts.

use super::{AuthAttempt, Discovery, Grant, GrantSummary, PlanAuthError, Registration};
use axum::{extract::Query, http::StatusCode, routing::get, Router};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs::File, io::Write, path::PathBuf, sync::Arc};
use tokio::{
    net::TcpListener,
    sync::{oneshot, Mutex},
};

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Phase {
    SignedOut,
    Connected,
    ReauthRequired,
    ExchangeUnknown,
    Refreshing,
    RefreshUnknown,
}

#[derive(Clone, Serialize, Deserialize)]
struct Profile {
    label: String,
    revision: u64,
    registration: Option<Registration>,
    grant: Option<Grant>,
    phase: Phase,
    last_error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Store {
    version: u32,
    host_id: String,
    active_id: Option<String>,
    accounts: BTreeMap<String, Profile>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginView {
    pub attempt_id: String,
    pub account_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub id: String,
    pub label: String,
    phase: Phase,
    pub identity: Option<GrantSummary>,
    pub last_error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountsView {
    pub accounts: Vec<AccountView>,
    pub active_id: Option<String>,
    pub pending: Option<LoginView>,
}

// Never serialize or log this: the authorize URL can contain a retained ID-token hint.
pub struct LoginLaunch {
    pub view: LoginView,
    pub authorize_url: String,
    pub callback_url: String,
}

struct Pending {
    view: LoginView,
    revision: u64,
    attempt: Option<AuthAttempt>,
    _cancel: oneshot::Sender<()>,
}

struct Inner {
    store: Store,
    pending: Option<Pending>,
}

pub struct PlanAccounts {
    path: PathBuf,
    inner: Mutex<Inner>,
    refresh_locks: Mutex<BTreeMap<String, Arc<Mutex<()>>>>,
    // A second process must not race a rotating refresh token or overwrite this store.
    _file_lock: File,
}

// In-memory dispatch authority only; never serialize credentials or accept a caller-made lease.
pub(crate) struct AccessLease {
    owner: Arc<PlanAccounts>,
    account_id: String,
    revision: u64,
    token: String,
}

impl AccessLease {
    pub(crate) fn token(&self) -> &str {
        &self.token
    }

    fn matches(&self, profile: &Profile) -> bool {
        profile.phase == Phase::Connected
            && profile.revision == self.revision
            && profile.grant.as_ref().is_some_and(|grant| {
                grant
                    .access_token()
                    .is_ok_and(|token| token == self.token.as_str())
            })
    }

    pub(crate) async fn ensure_current(&self) -> Result<()> {
        let inner = self.owner.inner.lock().await;
        if !inner
            .store
            .accounts
            .get(&self.account_id)
            .is_some_and(|profile| self.matches(profile))
        {
            return Err("CHATGPT_PLAN_AUTHORIZATION_CHANGED".into());
        }
        Ok(())
    }
}

impl PlanAccounts {
    pub fn open(path: PathBuf) -> Result<Arc<Self>> {
        let directory = path.parent().ok_or("CHATGPT_PLAN_STORE_PATH")?;
        std::fs::create_dir_all(directory).map_err(|_| "CHATGPT_PLAN_STORE_IO")?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("lock"))
            .map_err(|_| "CHATGPT_PLAN_STORE_IO")?;
        lock.try_lock().map_err(|_| "CHATGPT_PLAN_STORE_BUSY")?;
        let mut store: Store = match std::fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|_| "CHATGPT_PLAN_STORE_INVALID")?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Store {
                version: 1,
                host_id: format!("urn:uuid:{}", uuid::Uuid::new_v4()),
                active_id: None,
                accounts: BTreeMap::new(),
            },
            Err(_) => return Err("CHATGPT_PLAN_STORE_IO".into()),
        };
        if store.version != 1
            || !store.host_id.starts_with("urn:uuid:")
            || uuid::Uuid::parse_str(&store.host_id[9..]).is_err()
        {
            return Err("CHATGPT_PLAN_STORE_INVALID".into());
        }
        for account in store.accounts.values_mut() {
            if account.phase == Phase::Refreshing {
                account.phase = Phase::RefreshUnknown;
                account.last_error = Some("refresh_interrupted_unknown".into());
            }
        }
        Ok(Arc::new(Self {
            path,
            inner: Mutex::new(Inner {
                store,
                pending: None,
            }),
            refresh_locks: Mutex::new(BTreeMap::new()),
            _file_lock: lock,
        }))
    }

    fn commit(&self, inner: &mut Inner, store: Store) -> Result<()> {
        let directory = self.path.parent().ok_or("CHATGPT_PLAN_STORE_PATH")?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(directory).map_err(|_| "CHATGPT_PLAN_STORE_IO")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temporary
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|_| "CHATGPT_PLAN_STORE_IO")?;
        }
        serde_json::to_writer(&mut temporary, &store).map_err(|_| "CHATGPT_PLAN_STORE_IO")?;
        temporary.flush().map_err(|_| "CHATGPT_PLAN_STORE_IO")?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| "CHATGPT_PLAN_STORE_IO")?;
        temporary
            .persist(&self.path)
            .map_err(|_| "CHATGPT_PLAN_STORE_IO")?;
        inner.store = store;
        Ok(())
    }

    pub async fn status(&self) -> AccountsView {
        let inner = self.inner.lock().await;
        AccountsView {
            active_id: inner.store.active_id.clone(),
            pending: inner.pending.as_ref().map(|pending| pending.view.clone()),
            accounts: inner
                .store
                .accounts
                .iter()
                .map(|(id, profile)| AccountView {
                    id: id.clone(),
                    label: profile.label.clone(),
                    phase: profile.phase.clone(),
                    identity: profile.grant.as_ref().map(Grant::summary),
                    last_error: profile.last_error.clone(),
                })
                .collect(),
        }
    }

    pub async fn begin(
        self: &Arc<Self>,
        discovery: Discovery,
        account_id: Option<&str>,
        label: &str,
    ) -> Result<LoginLaunch> {
        if label.trim().is_empty() || label.len() > 256 {
            return Err("CHATGPT_PLAN_LABEL_INVALID".into());
        }
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| "CHATGPT_PLAN_CALLBACK_UNAVAILABLE")?;
        let callback_url = format!(
            "http://{}/auth/callback",
            listener
                .local_addr()
                .map_err(|_| "CHATGPT_PLAN_CALLBACK_UNAVAILABLE")?
        );
        let mut inner = self.inner.lock().await;
        if inner.pending.is_some() {
            return Err("CHATGPT_PLAN_LOGIN_IN_PROGRESS".into());
        }
        let mut store = inner.store.clone();
        let id = account_id
            .map(str::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        if account_id.is_some() && !store.accounts.contains_key(&id) {
            return Err("CHATGPT_PLAN_ACCOUNT_NOT_FOUND".into());
        }
        let profile = store.accounts.entry(id.clone()).or_insert_with(|| Profile {
            label: label.trim().into(),
            revision: 0,
            registration: None,
            grant: None,
            phase: Phase::SignedOut,
            last_error: None,
        });
        let attempt = if let Some(grant) = &profile.grant {
            AuthAttempt::new(&discovery, &store.host_id, &callback_url, Some(grant))
        } else if let Some(registration) = &profile.registration {
            AuthAttempt::resume(&discovery, registration, &callback_url)
        } else {
            AuthAttempt::new(&discovery, &store.host_id, &callback_url, None)
        }
        .map_err(|error| error.to_string())?;
        if profile.phase == Phase::Refreshing {
            profile.phase = Phase::RefreshUnknown;
            profile.last_error = Some("refresh_superseded_unknown".into());
        }
        profile.revision = profile
            .revision
            .checked_add(1)
            .ok_or("CHATGPT_PLAN_REVISION")?;
        let revision = profile.revision;
        let view = LoginView {
            attempt_id: uuid::Uuid::new_v4().to_string(),
            account_id: id,
        };
        let authorize_url = attempt.authorize_url().to_owned();
        // Host and registration identity must be durable before opening a browser.
        self.commit(&mut inner, store)?;
        let (cancel, stopped) = oneshot::channel();
        inner.pending = Some(Pending {
            view: view.clone(),
            revision,
            attempt: Some(attempt),
            _cancel: cancel,
        });
        drop(inner);
        let weak = Arc::downgrade(self);
        let callback_owner = weak.clone();
        let attempt_id = view.attempt_id.clone();
        let callback_attempt = attempt_id.clone();
        let app = Router::new().route(
            "/auth/callback",
            get(
                move |Query(query): Query<std::collections::HashMap<String, String>>| {
                    let owner = callback_owner.clone();
                    let attempt_id = callback_attempt.clone();
                    let discovery = discovery.clone();
                    async move {
                        let result = match owner.upgrade() {
                            Some(owner) => owner.complete(&attempt_id, &query, &discovery).await,
                            None => Err("CHATGPT_PLAN_LOGIN_CLOSED".into()),
                        };
                        let (status, body) = if result.is_ok() {
                            (StatusCode::OK, "Sign-in completed. Return to CC Switch.")
                        } else {
                            (
                                StatusCode::BAD_REQUEST,
                                "Sign-in was not completed. See CC Switch for details.",
                            )
                        };
                        (
                            status,
                            [
                                ("Cache-Control", "no-store"),
                                ("Referrer-Policy", "no-referrer"),
                            ],
                            body,
                        )
                    }
                },
            ),
        );
        tokio::spawn(async move {
            let timeout_owner = weak.clone();
            let timeout_attempt = attempt_id.clone();
            let outcome = axum::serve(listener, app).with_graceful_shutdown(async move {
                tokio::select! {
                    _ = stopped => {},
                    _ = tokio::time::sleep(std::time::Duration::from_secs(600)) => {
                        if let Some(owner) = timeout_owner.upgrade() {
                            let _ = owner.cancel_reason(&timeout_attempt, "login_expired").await;
                        }
                    }
                }
            }).await;
            if outcome.is_err() {
                if let Some(owner) = weak.upgrade() {
                    let _ = owner
                        .cancel_reason(&attempt_id, "callback_listener_failed")
                        .await;
                }
            }
        });
        Ok(LoginLaunch {
            view,
            authorize_url,
            callback_url,
        })
    }

    async fn complete(
        &self,
        attempt_id: &str,
        query: &std::collections::HashMap<String, String>,
        discovery: &Discovery,
    ) -> Result<()> {
        let (code, id, revision) = {
            let mut inner = self.inner.lock().await;
            let pending = inner
                .pending
                .as_mut()
                .filter(|pending| pending.view.attempt_id == attempt_id)
                .ok_or("CHATGPT_PLAN_LOGIN_CLOSED")?;
            let attempt = pending
                .attempt
                .as_mut()
                .ok_or("CHATGPT_PLAN_LOGIN_DISPATCHED")?;
            let code = match attempt.callback(query) {
                Ok(code) => code,
                Err(error) => {
                    let closed = attempt.consumed;
                    drop(inner);
                    if closed {
                        self.cancel_reason(attempt_id, "login_callback_rejected")
                            .await?;
                    }
                    return Err(error.to_string());
                }
            };
            pending.attempt = None;
            let id = pending.view.account_id.clone();
            let revision = pending.revision;
            let mut store = inner.store.clone();
            let profile = store
                .accounts
                .get_mut(&id)
                .ok_or("CHATGPT_PLAN_ACCOUNT_NOT_FOUND")?;
            if profile.revision != revision {
                return Err("CHATGPT_PLAN_LOGIN_STALE".into());
            }
            profile.registration = Some(code.registration());
            profile.phase = Phase::ExchangeUnknown;
            profile.last_error = Some("authorization_exchange_unknown".into());
            if let Err(error) = self.commit(&mut inner, store) {
                inner.pending = None;
                return Err(error);
            }
            (code, id, revision)
        };
        let result = code.exchange(discovery).await;
        let mut inner = self.inner.lock().await;
        if !inner
            .pending
            .as_ref()
            .is_some_and(|pending| pending.view.attempt_id == attempt_id)
            || !inner
                .store
                .accounts
                .get(&id)
                .is_some_and(|profile| profile.revision == revision)
        {
            return Err("CHATGPT_PLAN_LOGIN_STALE".into());
        }
        let mut store = inner.store.clone();
        let profile = store
            .accounts
            .get_mut(&id)
            .ok_or("CHATGPT_PLAN_ACCOUNT_NOT_FOUND")?;
        let outcome = match result {
            Ok(grant) => {
                profile.registration = Some(grant.registration());
                profile.grant = Some(grant);
                profile.phase = Phase::Connected;
                profile.last_error = None;
                store.active_id = Some(id);
                Ok(())
            }
            Err(error) => {
                profile.phase = Phase::ReauthRequired;
                profile.last_error = Some(error.to_string());
                Err(error.to_string())
            }
        };
        inner.pending = None;
        self.commit(&mut inner, store)?;
        outcome
    }

    pub async fn cancel(&self, attempt_id: &str) -> Result<AccountsView> {
        self.cancel_reason(attempt_id, "login_cancelled").await?;
        Ok(self.status().await)
    }

    async fn cancel_reason(&self, attempt_id: &str, reason: &str) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let Some(pending) = inner.pending.take() else {
            return Ok(());
        };
        if pending.view.attempt_id != attempt_id {
            inner.pending = Some(pending);
            return Err("CHATGPT_PLAN_LOGIN_STALE".into());
        }
        let mut store = inner.store.clone();
        if let Some(profile) = store.accounts.get_mut(&pending.view.account_id) {
            profile.revision = profile
                .revision
                .checked_add(1)
                .ok_or("CHATGPT_PLAN_REVISION")?;
            profile.last_error = Some(reason.into());
            if profile.phase == Phase::ExchangeUnknown {
                profile.phase = Phase::ReauthRequired;
            }
        }
        self.commit(&mut inner, store)
    }

    pub async fn select(&self, id: Option<String>) -> Result<AccountsView> {
        let mut inner = self.inner.lock().await;
        let mut store = inner.store.clone();
        if let Some(id) = &id {
            let profile = store
                .accounts
                .get(id)
                .ok_or("CHATGPT_PLAN_ACCOUNT_NOT_FOUND")?;
            if profile.phase != Phase::Connected || profile.grant.is_none() {
                return Err("CHATGPT_PLAN_SIGN_IN_REQUIRED".into());
            }
        }
        store.active_id = id;
        self.commit(&mut inner, store)?;
        drop(inner);
        Ok(self.status().await)
    }

    pub(crate) async fn lease_for(self: &Arc<Self>, id: &str) -> Result<AccessLease> {
        let token = self.token_for(id).await?;
        let inner = self.inner.lock().await;
        let profile = inner
            .store
            .accounts
            .get(id)
            .ok_or("CHATGPT_PLAN_ACCOUNT_NOT_FOUND")?;
        let lease = AccessLease {
            owner: self.clone(),
            account_id: id.into(),
            revision: profile.revision,
            token,
        };
        if !lease.matches(profile) {
            return Err("CHATGPT_PLAN_AUTHORIZATION_CHANGED".into());
        }
        Ok(lease)
    }

    pub async fn token_for(self: &Arc<Self>, id: &str) -> Result<String> {
        self.owned_token(id, None).await
    }

    async fn owned_token(
        self: &Arc<Self>,
        id: &str,
        discovery: Option<Discovery>,
    ) -> Result<String> {
        let manager = self.clone();
        let id = id.to_owned();
        // A disconnected consumer must not abandon a rotating refresh before its durable result.
        tokio::spawn(async move { manager.token(&id, discovery.as_ref()).await })
            .await
            .map_err(|_| "CHATGPT_PLAN_REFRESH_TASK_INTERRUPTED")?
    }

    async fn token(&self, id: &str, discovery: Option<&Discovery>) -> Result<String> {
        let lock = self
            .refresh_locks
            .lock()
            .await
            .entry(id.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let _refresh = lock.lock().await;
        let (grant, prior_revision) = {
            let inner = self.inner.lock().await;
            let profile = inner
                .store
                .accounts
                .get(id)
                .ok_or("CHATGPT_PLAN_ACCOUNT_NOT_FOUND")?;
            if profile.phase != Phase::Connected {
                return Err("CHATGPT_PLAN_SIGN_IN_REQUIRED".into());
            }
            let grant = profile
                .grant
                .as_ref()
                .ok_or("CHATGPT_PLAN_SIGN_IN_REQUIRED")?;
            if !grant.summary().inference_authorized {
                return Err("CHATGPT_PLAN_NOT_AUTHORIZED".into());
            }
            if let Ok(token) = grant.access_token() {
                return Ok(token.into());
            }
            (grant.clone(), profile.revision)
        };
        // Discovery is read-only; a failure here must not poison a never-dispatched refresh.
        let metadata = match discovery {
            Some(value) => value.clone(),
            None => Discovery::official()
                .await
                .map_err(|error| error.to_string())?,
        };
        let revision = {
            let mut inner = self.inner.lock().await;
            let mut store = inner.store.clone();
            let profile = store
                .accounts
                .get_mut(id)
                .ok_or("CHATGPT_PLAN_ACCOUNT_NOT_FOUND")?;
            if profile.revision != prior_revision || profile.phase != Phase::Connected {
                return Err("CHATGPT_PLAN_REFRESH_STALE".into());
            }
            profile.revision = profile
                .revision
                .checked_add(1)
                .ok_or("CHATGPT_PLAN_REVISION")?;
            let revision = profile.revision;
            // Crash or lost response must never automatically replay this rotating token.
            profile.phase = Phase::Refreshing;
            profile.last_error = Some("refresh_pending_or_unknown".into());
            self.commit(&mut inner, store)?;
            revision
        };
        let result = grant.refreshed(&metadata).await;
        let mut inner = self.inner.lock().await;
        if !inner
            .store
            .accounts
            .get(id)
            .is_some_and(|profile| profile.revision == revision)
        {
            return Err("CHATGPT_PLAN_REFRESH_STALE".into());
        }
        let mut store = inner.store.clone();
        let profile = store
            .accounts
            .get_mut(id)
            .ok_or("CHATGPT_PLAN_ACCOUNT_NOT_FOUND")?;
        let outcome = match result {
            Ok(grant) => {
                let token = grant
                    .access_token()
                    .map(str::to_owned)
                    .map_err(|error| error.to_string());
                profile.grant = Some(grant);
                profile.phase = Phase::Connected;
                profile.last_error = None;
                token
            }
            Err(error) => {
                profile.phase = if matches!(
                    &error,
                    PlanAuthError::OAuth {
                        code: "invalid_grant",
                        ..
                    }
                ) {
                    Phase::ReauthRequired
                } else {
                    Phase::RefreshUnknown
                };
                profile.last_error = Some(error.to_string());
                Err(error.to_string())
            }
        };
        if let Err(error) = self.commit(&mut inner, store) {
            if let Some(profile) = inner.store.accounts.get_mut(id) {
                profile.phase = Phase::RefreshUnknown;
                profile.last_error = Some("refresh_result_save_failed".into());
            }
            return Err(error);
        }
        outcome
    }

    pub async fn sign_out(&self, id: &str) -> Result<AccountsView> {
        self.sign_out_with(id, None).await
    }

    pub(super) async fn sign_out_with(
        &self,
        id: &str,
        discovery: Option<&Discovery>,
    ) -> Result<AccountsView> {
        let lock = self
            .refresh_locks
            .lock()
            .await
            .entry(id.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let _refresh = lock.lock().await;
        let (grant, revision, uncertain) = {
            let mut inner = self.inner.lock().await;
            let mut store = inner.store.clone();
            let profile = store
                .accounts
                .get_mut(id)
                .ok_or("CHATGPT_PLAN_ACCOUNT_NOT_FOUND")?;
            let uncertain = matches!(
                profile.phase,
                Phase::ExchangeUnknown | Phase::RefreshUnknown | Phase::Refreshing
            );
            let grant = profile.grant.take();
            profile.revision = profile
                .revision
                .checked_add(1)
                .ok_or("CHATGPT_PLAN_REVISION")?;
            let revision = profile.revision;
            profile.phase = Phase::SignedOut;
            if grant.is_some() || uncertain {
                profile.last_error = Some("remote_revocation_unconfirmed".into());
            }
            if store.active_id.as_deref() == Some(id) {
                store.active_id = None;
            }
            self.commit(&mut inner, store)?;
            if inner
                .pending
                .as_ref()
                .is_some_and(|pending| pending.view.account_id == id)
            {
                inner.pending = None;
            }
            (grant, revision, uncertain)
        };
        if let Some(grant) = grant {
            let result = async {
                let metadata = match discovery {
                    Some(value) => value.clone(),
                    None => Discovery::official().await?,
                };
                grant.revoke(&metadata).await
            }
            .await;
            if result.is_ok() && !uncertain {
                let mut inner = self.inner.lock().await;
                let mut store = inner.store.clone();
                if let Some(profile) = store
                    .accounts
                    .get_mut(id)
                    .filter(|profile| profile.revision == revision)
                {
                    profile.last_error = None;
                    self.commit(&mut inner, store)?;
                }
            }
        }
        Ok(self.status().await)
    }
}

#[cfg(test)]
#[path = "chatgpt_plan_accounts_tests.rs"]
mod tests;
