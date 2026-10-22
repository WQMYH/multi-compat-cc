use super::super::{now, tests::Fixture, SCOPES};
use super::*;
use serde_json::json;
use tokio::sync::Notify;
use url::Url;

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

fn parameters(launch: &LoginLaunch) -> BTreeMap<String, String> {
    Url::parse(&launch.authorize_url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

async fn callback(launch: &LoginLaunch, code: &str) -> StatusCode {
    let mut url = Url::parse(&launch.callback_url).unwrap();
    url.query_pairs_mut()
        .append_pair("state", &parameters(launch)["state"])
        .append_pair("code", code)
        .append_pair("client_id", "issued-test-client");
    let response = client().get(url).send().await.unwrap();
    let status = response.status();
    assert!(!response.text().await.unwrap().contains(code));
    status
}

async fn seed(manager: &PlanAccounts, grant: Grant) -> String {
    let mut inner = manager.inner.lock().await;
    let mut store = inner.store.clone();
    let id = uuid::Uuid::new_v4().to_string();
    store.accounts.insert(
        id.clone(),
        Profile {
            label: "synthetic saved account".into(),
            revision: 1,
            registration: Some(grant.registration()),
            grant: Some(grant),
            phase: Phase::Connected,
            last_error: None,
        },
    );
    store.active_id = Some(id.clone());
    manager.commit(&mut inner, store).unwrap();
    id
}

async fn expired_grant(manager: &PlanAccounts, fixture: &Fixture) -> Grant {
    let host = manager.inner.lock().await.store.host_id.clone();
    let mut grant = fixture.grant_for_host(SCOPES, &host).await;
    grant.expires_at = now() - 1;
    grant
}

#[tokio::test]
async fn dispatch_lease_rechecks_expiry_without_refreshing_or_changing_its_token() {
    let fixture = Fixture::new().await;
    let directory = tempfile::tempdir().unwrap();
    let manager = PlanAccounts::open(directory.path().join("plan.json")).unwrap();
    let host = manager.inner.lock().await.store.host_id.clone();
    let id = seed(&manager, fixture.grant_for_host(SCOPES, &host).await).await;
    let lease = manager.lease_for(&id).await.unwrap();
    lease.ensure_current().await.unwrap();
    manager
        .inner
        .lock()
        .await
        .store
        .accounts
        .get_mut(&id)
        .unwrap()
        .grant
        .as_mut()
        .unwrap()
        .expires_at = now() - 1;
    assert_eq!(
        lease.ensure_current().await.unwrap_err(),
        "CHATGPT_PLAN_AUTHORIZATION_CHANGED"
    );
    assert_eq!(lease.token(), "synthetic-access-initial");
    assert_eq!(
        fixture.fields.lock().unwrap().len(),
        1,
        "lease verification must not fetch a replacement token"
    );
}

#[tokio::test]
async fn disconnected_consumer_does_not_abandon_a_dispatched_rotating_refresh() {
    for owned in [false, true] {
        let fixture = Fixture::new().await;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("plan.json");
        let manager = PlanAccounts::open(path.clone()).unwrap();
        let id = seed(&manager, expired_grant(&manager, &fixture).await).await;
        fixture.reply(None, Some(SCOPES), "consumer-finished");
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        *fixture.gate.lock().unwrap() = Some((entered.clone(), release.clone()));
        let owner = manager.clone();
        let selected = id.clone();
        let discovery = fixture.discovery().await;
        let consumer = tokio::spawn(async move {
            if owned {
                owner.owned_token(&selected, Some(discovery)).await
            } else {
                owner.token(&selected, Some(&discovery)).await
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        let lock = manager.refresh_locks.lock().await[&id].clone();
        consumer.abort();
        assert!(consumer.await.unwrap_err().is_cancelled());
        release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let _finished = lock.lock().await;
        })
        .await
        .unwrap();
        let phase = manager.inner.lock().await.store.accounts[&id].phase.clone();
        // Negative control: the former unowned path leaves no writer to finish Refreshing.
        assert!(
            phase
                == if owned {
                    Phase::Connected
                } else {
                    Phase::Refreshing
                }
        );
        if owned {
            assert_eq!(
                manager.token_for(&id).await.unwrap(),
                "synthetic-access-consumer-finished"
            );
            assert!(std::fs::read_to_string(&path)
                .unwrap()
                .contains("synthetic-refresh-consumer-finished"));
        } else {
            drop(manager);
            assert!(
                PlanAccounts::open(path)
                    .unwrap()
                    .inner
                    .lock()
                    .await
                    .store
                    .accounts[&id]
                    .phase
                    == Phase::RefreshUnknown
            );
        }
        assert_eq!(
            fixture
                .fields
                .lock()
                .unwrap()
                .iter()
                .filter(|form| form.get("grant_type").map(String::as_str) == Some("refresh_token"))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn loopback_sign_in_persists_host_and_verified_identity_without_exposing_tokens_in_views() {
    let fixture = Fixture::new().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plan.json");
    let manager = PlanAccounts::open(path.clone()).unwrap();
    assert!(
        PlanAccounts::open(path.clone()).is_err(),
        "only one process may own the store"
    );
    assert!(
        !path.exists(),
        "status does not enroll or write credentials"
    );
    assert!(manager.status().await.accounts.is_empty());
    let discovery = fixture.discovery().await;
    let launch = manager
        .begin(discovery.clone(), None, "Personal plan")
        .await
        .unwrap();
    let host = parameters(&launch)["ext_agent_host_id"].clone();
    assert!(host.starts_with("urn:uuid:"));
    let pending: Store = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(pending.host_id, host);
    assert!(pending.accounts[&launch.view.account_id].grant.is_none());
    fixture.reply(
        Some(&parameters(&launch)["nonce"]),
        Some(SCOPES),
        "signed-in",
    );
    assert_eq!(callback(&launch, "synthetic-code").await, StatusCode::OK);
    let status = manager.status().await;
    assert!(status.pending.is_none());
    assert_eq!(
        status.active_id.as_deref(),
        Some(launch.view.account_id.as_str())
    );
    assert_eq!(
        status.accounts[0].identity.as_ref().unwrap().subject,
        "synthetic-user"
    );
    let before = std::fs::read(&path).unwrap();
    let view = serde_json::to_string(&manager.status().await).unwrap();
    for secret in [
        "synthetic-access-signed-in",
        "synthetic-refresh-signed-in",
        "id_token",
    ] {
        assert!(!view.contains(secret));
    }
    assert_eq!(before, std::fs::read(&path).unwrap());
    assert_eq!(
        manager
            .token(&launch.view.account_id, Some(&discovery))
            .await
            .unwrap(),
        "synthetic-access-signed-in"
    );
    assert_eq!(fixture.fields.lock().unwrap().len(), 1);

    let signed_out = manager
        .sign_out_with(&launch.view.account_id, Some(&discovery))
        .await
        .unwrap();
    assert!(signed_out.active_id.is_none());
    assert!(signed_out.accounts[0].identity.is_none());
    assert!(signed_out.accounts[0].last_error.is_none());
    let disk: Store = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(disk.host_id, host);
    assert!(disk.accounts[&launch.view.account_id]
        .registration
        .is_some());
    assert!(!String::from_utf8(std::fs::read(&path).unwrap())
        .unwrap()
        .contains("synthetic-refresh-signed-in"));
    assert!(manager
        .token(&launch.view.account_id, Some(&discovery))
        .await
        .is_err());
}

#[tokio::test]
async fn failed_exchange_survives_restart_and_reuses_registration_only_with_new_consent() {
    let fixture = Fixture::new().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plan.json");
    let manager = PlanAccounts::open(path.clone()).unwrap();
    let discovery = fixture.discovery().await;
    let first = manager
        .begin(discovery.clone(), None, "Failed first consent")
        .await
        .unwrap();
    *fixture.response.lock().unwrap() = (StatusCode::BAD_REQUEST, json!({"error":"invalid_grant"}));
    assert_eq!(
        callback(&first, "synthetic-old-code").await,
        StatusCode::BAD_REQUEST
    );
    assert!(manager.status().await.pending.is_none());
    drop(manager);
    let manager = PlanAccounts::open(path).unwrap();
    let fresh = manager
        .begin(
            discovery.clone(),
            Some(&first.view.account_id),
            "Failed first consent",
        )
        .await
        .unwrap();
    assert_eq!(fresh.view.account_id, first.view.account_id);
    assert_eq!(parameters(&fresh)["client_id"], "issued-test-client");
    assert_eq!(
        parameters(&fresh)["ext_agent_host_id"],
        parameters(&first)["ext_agent_host_id"]
    );
    assert_ne!(parameters(&fresh)["state"], parameters(&first)["state"]);
    assert_ne!(
        parameters(&fresh)["code_challenge"],
        parameters(&first)["code_challenge"]
    );
    fixture.reply(Some(&parameters(&fresh)["nonce"]), Some(SCOPES), "restored");
    assert_eq!(callback(&fresh, "synthetic-new-code").await, StatusCode::OK);
    let fields = fixture.fields.lock().unwrap();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0]["code"], "synthetic-old-code");
    assert_eq!(fields[1]["code"], "synthetic-new-code");
}

#[tokio::test]
async fn cancelling_after_exchange_dispatch_cannot_activate_its_result_or_close_a_new_login() {
    let fixture = Fixture::new().await;
    let directory = tempfile::tempdir().unwrap();
    let manager = PlanAccounts::open(directory.path().join("plan.json")).unwrap();
    let discovery = fixture.discovery().await;
    let first = manager
        .begin(discovery.clone(), None, "Cancelled")
        .await
        .unwrap();
    fixture.reply(Some(&parameters(&first)["nonce"]), Some(SCOPES), "ignored");
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    *fixture.gate.lock().unwrap() = Some((entered.clone(), release.clone()));
    let url = {
        let mut url = Url::parse(&first.callback_url).unwrap();
        url.query_pairs_mut()
            .append_pair("state", &parameters(&first)["state"])
            .append_pair("code", "synthetic-once-only")
            .append_pair("client_id", "issued-test-client");
        url
    };
    let request = tokio::spawn(async move { client().get(url).send().await.unwrap().status() });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    manager.cancel(&first.view.attempt_id).await.unwrap();
    let fresh = manager
        .begin(discovery, Some(&first.view.account_id), "Cancelled")
        .await
        .unwrap();
    release.notify_one();
    assert_eq!(request.await.unwrap(), StatusCode::BAD_REQUEST);
    let status = manager.status().await;
    assert!(status.active_id.is_none());
    assert!(status.accounts[0].identity.is_none());
    assert_eq!(status.pending.unwrap().attempt_id, fresh.view.attempt_id);
    assert_eq!(fixture.fields.lock().unwrap().len(), 1);
    manager.cancel(&fresh.view.attempt_id).await.unwrap();
}

#[tokio::test]
async fn simultaneous_consumers_refresh_once_and_sign_out_revokes_the_latest_rotated_token() {
    let fixture = Fixture::new().await;
    let directory = tempfile::tempdir().unwrap();
    let manager = PlanAccounts::open(directory.path().join("plan.json")).unwrap();
    let discovery = fixture.discovery().await;
    let grant = expired_grant(&manager, &fixture).await;
    let id = seed(&manager, grant).await;
    fixture.reply(None, None, "latest");
    let (first, second) = tokio::join!(
        manager.token(&id, Some(&discovery)),
        manager.token(&id, Some(&discovery))
    );
    assert_eq!(first.unwrap(), "synthetic-access-latest");
    assert_eq!(second.unwrap(), "synthetic-access-latest");
    assert_eq!(fixture.fields.lock().unwrap().len(), 2);
    manager.sign_out_with(&id, Some(&discovery)).await.unwrap();
    let fields = fixture.fields.lock().unwrap();
    assert_eq!(fields.len(), 3);
    assert_eq!(fields[2]["token"], "synthetic-refresh-latest");
}

#[tokio::test]
async fn unknown_refresh_is_durable_and_is_never_replayed_by_another_consumer_or_restart() {
    let fixture = Fixture::new().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plan.json");
    let manager = PlanAccounts::open(path.clone()).unwrap();
    let discovery = fixture.discovery().await;
    let grant = expired_grant(&manager, &fixture).await;
    let id = seed(&manager, grant).await;
    *fixture.response.lock().unwrap() = (
        StatusCode::SERVICE_UNAVAILABLE,
        json!({"error":"synthetic-private-error"}),
    );
    assert!(manager.token(&id, Some(&discovery)).await.is_err());
    assert!(manager.token(&id, Some(&discovery)).await.is_err());
    assert_eq!(fixture.fields.lock().unwrap().len(), 2);
    assert!(matches!(
        manager.status().await.accounts[0].phase,
        Phase::RefreshUnknown
    ));
    drop(manager);
    let manager = PlanAccounts::open(path).unwrap();
    assert!(manager.token(&id, Some(&discovery)).await.is_err());
    assert_eq!(fixture.fields.lock().unwrap().len(), 2);
    assert!(!serde_json::to_string(&manager.status().await)
        .unwrap()
        .contains("synthetic-private-error"));
    let signed_out = manager.sign_out_with(&id, Some(&discovery)).await.unwrap();
    assert_eq!(
        signed_out.accounts[0].last_error.as_deref(),
        Some("remote_revocation_unconfirmed")
    );
}

#[tokio::test]
async fn interrupted_refresh_and_atomic_save_failure_cannot_restore_or_dispatch_uncommitted_credentials(
) {
    let fixture = Fixture::new().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plan.json");
    let manager = PlanAccounts::open(path.clone()).unwrap();
    let discovery = fixture.discovery().await;
    let grant = expired_grant(&manager, &fixture).await;
    let id = seed(&manager, grant).await;
    let before = std::fs::read(&path).unwrap();
    let backup = path.with_extension("original");
    std::fs::rename(&path, &backup).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(manager.token(&id, Some(&discovery)).await.is_err());
    assert_eq!(std::fs::read(&backup).unwrap(), before);
    assert_eq!(fixture.fields.lock().unwrap().len(), 1);
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&backup, &path).unwrap();
    {
        let mut inner = manager.inner.lock().await;
        let mut interrupted = inner.store.clone();
        interrupted.accounts.get_mut(&id).unwrap().phase = Phase::Refreshing;
        manager.commit(&mut inner, interrupted).unwrap();
    }
    drop(manager);
    let manager = PlanAccounts::open(path).unwrap();
    assert!(matches!(
        manager.status().await.accounts[0].phase,
        Phase::RefreshUnknown
    ));
    assert!(manager.token(&id, Some(&discovery)).await.is_err());
    assert_eq!(fixture.fields.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn reauthorization_cancel_after_dispatch_preserves_late_refresh_as_unknown() {
    let fixture = Fixture::new().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plan.json");
    let manager = PlanAccounts::open(path.clone()).unwrap();
    let discovery = fixture.discovery().await;
    let id = seed(&manager, expired_grant(&manager, &fixture).await).await;
    fixture.reply(None, None, "late-rotation");
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    *fixture.gate.lock().unwrap() = Some((entered.clone(), release.clone()));
    let refresh = {
        let manager = manager.clone();
        let id = id.clone();
        let discovery = discovery.clone();
        tokio::spawn(async move { manager.token(&id, Some(&discovery)).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    let new_login = manager
        .begin(discovery.clone(), Some(&id), "Personal")
        .await
        .unwrap();
    manager.cancel(&new_login.view.attempt_id).await.unwrap();
    release.notify_one();
    assert_eq!(
        refresh.await.unwrap().unwrap_err(),
        "CHATGPT_PLAN_REFRESH_STALE"
    );
    assert!(
        matches!(
            manager.status().await.accounts[0].phase,
            Phase::RefreshUnknown
        ),
        "no worker remains to finish a superseded Refreshing phase"
    );
    assert!(manager.token(&id, Some(&discovery)).await.is_err());
    assert_eq!(fixture.fields.lock().unwrap().len(), 2);
    drop(manager);
    let manager = PlanAccounts::open(path).unwrap();
    assert!(matches!(
        manager.status().await.accounts[0].phase,
        Phase::RefreshUnknown
    ));
    assert!(manager.token(&id, Some(&discovery)).await.is_err());
    assert_eq!(fixture.fields.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn post_rotation_save_failure_keeps_late_refresh_unknown_without_replay() {
    let fixture = Fixture::new().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plan.json");
    let manager = PlanAccounts::open(path.clone()).unwrap();
    let discovery = fixture.discovery().await;
    let id = seed(&manager, expired_grant(&manager, &fixture).await).await;
    fixture.reply(None, None, "not-saved");
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    *fixture.gate.lock().unwrap() = Some((entered.clone(), release.clone()));
    let refresh = {
        let manager = manager.clone();
        let id = id.clone();
        let discovery = discovery.clone();
        tokio::spawn(async move { manager.token(&id, Some(&discovery)).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    let dispatched = std::fs::read(&path).unwrap();
    let backup = path.with_extension("original");
    std::fs::rename(&path, &backup).unwrap();
    std::fs::create_dir(&path).unwrap();
    release.notify_one();
    let result = refresh.await.unwrap();
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&backup, &path).unwrap();
    assert_eq!(result.unwrap_err(), "CHATGPT_PLAN_STORE_IO");
    assert_eq!(std::fs::read(&path).unwrap(), dispatched);
    assert!(
        matches!(
            manager.status().await.accounts[0].phase,
            Phase::RefreshUnknown
        ),
        "a failed post-response save is unknown now, not only after restart"
    );
    assert!(manager.token(&id, Some(&discovery)).await.is_err());
    assert_eq!(fixture.fields.lock().unwrap().len(), 2);
    drop(manager);
    let manager = PlanAccounts::open(path).unwrap();
    assert!(matches!(
        manager.status().await.accounts[0].phase,
        Phase::RefreshUnknown
    ));
    assert!(manager.token(&id, Some(&discovery)).await.is_err());
    assert_eq!(fixture.fields.lock().unwrap().len(), 2);
}
