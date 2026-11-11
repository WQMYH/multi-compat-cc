//! Isolated end-to-end checks for the DSH account-card lifecycle.
use super::ProviderService;
use crate::app_config::AppType;
use crate::database::Database;
use crate::provider::{AuthBinding, AuthBindingSource, Provider, ProviderMeta};
use crate::store::AppState;
use serde_json::json;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::Arc;

struct Sandbox {
    dir: tempfile::TempDir,
    previous_dsh: Option<OsString>,
    previous_test_root: Option<OsString>,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let previous_dsh = std::env::var_os("DSH_HOME");
        let previous_test_root = std::env::var_os("CC_SWITCH_TEST_HOME");
        std::env::set_var("DSH_HOME", dir.path().join("dsh"));
        std::env::set_var("CC_SWITCH_TEST_HOME", dir.path().join("cc"));
        crate::settings::update_settings(Default::default()).unwrap();
        Self {
            dir,
            previous_dsh,
            previous_test_root,
        }
    }

    fn dsh(&self) -> std::path::PathBuf {
        self.dir.path().join("dsh")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        for (name, prior) in [
            ("DSH_HOME", self.previous_dsh.take()),
            ("CC_SWITCH_TEST_HOME", self.previous_test_root.take()),
        ] {
            match prior {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

fn account_card() -> Provider {
    let mut provider = Provider::with_id(
        "chatgpt-card".into(),
        "ChatGPT test".into(),
        json!({"api":"openai-responses", "models":{"model-one":{},"model-two":{}}}),
        None,
    );
    provider.meta = Some(ProviderMeta {
        provider_type: Some("codex_oauth".into()),
        auth_binding: Some(AuthBinding {
            source: AuthBindingSource::ManagedAccount,
            auth_provider: Some("codex_oauth".into()),
            account_id: Some("synthetic-account".into()),
        }),
        ..Default::default()
    });
    provider
}

async fn test_state() -> AppState {
    let state = AppState::new(Arc::new(Database::memory().unwrap()));
    let mut proxy = state.db.get_global_proxy_config().await.unwrap();
    proxy.listen_address = "127.0.0.1".into();
    proxy.listen_port = 0;
    proxy.proxy_enabled = false;
    state.db.update_global_proxy_config(proxy).await.unwrap();
    state
        .codex_oauth_manager
        .add_test_account_with_access_token("synthetic-account", "synthetic-oauth-token", None)
        .await
        .unwrap();
    state
}

fn web_profile(root: &Path) -> std::path::PathBuf {
    root.join("profiles/web/cordis.patch.yml")
}

#[tokio::test]
#[serial_test::serial]
async fn account_card_preserves_native_data_and_stores_no_route_key_in_cc() {
    let sandbox = Sandbox::new();
    let web = web_profile(&sandbox.dsh());
    fs::create_dir_all(web.parent().unwrap()).unwrap();
    fs::write(&web, "- id: user-plugin\n  config: {keep: true}\n").unwrap();
    let state = test_state().await;
    let before = serde_json::to_value(state.db.get_global_proxy_config().await.unwrap()).unwrap();
    let card = account_card();

    // Generic add cannot accidentally publish an account card.
    ProviderService::add(&state, AppType::Dsh, card, true).unwrap();
    assert!(!state.proxy_service.is_running().await);
    assert_eq!(
        fs::read_to_string(&web).unwrap(),
        "- id: user-plugin\n  config: {keep: true}\n"
    );

    state
        .proxy_service
        .activate_dsh_oauth_provider(&state, "chatgpt-card")
        .await
        .unwrap();
    assert!(
        state
            .proxy_service
            .dsh_oauth_route_active("chatgpt-card")
            .await
    );
    state.proxy_service.stop_if_unused().await.unwrap();
    assert!(state.proxy_service.is_running().await);
    let native = fs::read_to_string(&web).unwrap();
    assert!(native.contains("user-plugin"));
    assert!(native.contains("chatgpt-card"));
    assert!(!native.contains("synthetic-oauth-token"));
    let credentials = fs::read_to_string(sandbox.dsh().join(".credentials.yaml")).unwrap();
    assert!(!credentials.contains("synthetic-oauth-token"));

    let cards = ProviderService::list(&state, AppType::Dsh).unwrap();
    let returned = &cards["chatgpt-card"];
    assert!(returned
        .settings_config
        .pointer("/options/apiKey")
        .is_none());
    assert!(returned
        .settings_config
        .pointer("/options/baseURL")
        .is_none());
    let saved = state
        .db
        .get_provider_by_id("chatgpt-card", "dsh")
        .unwrap()
        .unwrap();
    assert_eq!(saved.settings_config, returned.settings_config);
    assert_eq!(saved.meta.as_ref().unwrap().live_config_managed, Some(true));
    assert_eq!(
        before,
        serde_json::to_value(state.db.get_global_proxy_config().await.unwrap()).unwrap()
    );

    let native_before_edit = fs::read(&web).unwrap();
    let mut edited = saved;
    edited.name = "Updated ChatGPT test".into();
    ProviderService::update(&state, AppType::Dsh, None, edited).unwrap();
    assert_eq!(fs::read(&web).unwrap(), native_before_edit);
    assert!(
        !state
            .proxy_service
            .dsh_oauth_route_active("chatgpt-card")
            .await
    );
    state
        .proxy_service
        .activate_dsh_oauth_provider(&state, "chatgpt-card")
        .await
        .unwrap();
    assert!(
        state
            .proxy_service
            .dsh_oauth_route_active("chatgpt-card")
            .await
    );

    ProviderService::remove_from_live_config(&state, AppType::Dsh, "chatgpt-card").unwrap();
    state
        .proxy_service
        .deactivate_dsh_oauth_route("chatgpt-card")
        .await;
    assert!(!state.proxy_service.is_running().await);
    assert!(fs::read_to_string(&web).unwrap().contains("user-plugin"));
}

#[tokio::test]
#[serial_test::serial]
async fn conflicting_native_provider_is_not_replaced_or_left_with_listener() {
    let sandbox = Sandbox::new();
    let state = test_state().await;
    ProviderService::add(&state, AppType::Dsh, account_card(), false).unwrap();
    crate::dsh_config::set_provider(
        "chatgpt-card",
        json!({"api":"openai-responses", "options":{"baseURL":"https://native.example/v1","apiKey":"fixture"},
               "models":{"model-one":{}}}),
    ).unwrap();
    let web = web_profile(&sandbox.dsh());
    let before = fs::read(&web).unwrap();
    let error = state
        .proxy_service
        .activate_dsh_oauth_provider(&state, "chatgpt-card")
        .await
        .unwrap_err();
    assert!(error.contains("DSH_OAUTH_CONFIG_CONFLICT"));
    assert_eq!(fs::read(&web).unwrap(), before);
    assert!(!state.proxy_service.is_running().await);
    assert!(
        !state
            .db
            .get_global_proxy_config()
            .await
            .unwrap()
            .proxy_enabled
    );
    ProviderService::remove_from_live_config(&state, AppType::Dsh, "chatgpt-card").unwrap();
    assert_eq!(fs::read(&web).unwrap(), before);
    ProviderService::delete(&state, AppType::Dsh, "chatgpt-card").unwrap();
    assert_eq!(fs::read(&web).unwrap(), before);
}

#[tokio::test]
#[serial_test::serial]
async fn removing_the_selected_account_revokes_the_route_but_keeps_dsh_data() {
    let sandbox = Sandbox::new();
    let state = test_state().await;
    ProviderService::add(&state, AppType::Dsh, account_card(), false).unwrap();
    state
        .proxy_service
        .activate_dsh_oauth_provider(&state, "chatgpt-card")
        .await
        .unwrap();
    let web = web_profile(&sandbox.dsh());
    let before = fs::read(&web).unwrap();

    crate::commands::remove_codex_oauth_account_with_switch_lock(&state, "synthetic-account")
        .await
        .unwrap();

    assert!(
        !state
            .proxy_service
            .dsh_oauth_route_active("chatgpt-card")
            .await
    );
    assert!(!state.proxy_service.is_running().await);
    assert_eq!(fs::read(&web).unwrap(), before);
    let card = state
        .db
        .get_provider_by_id("chatgpt-card", "dsh")
        .unwrap()
        .unwrap();
    assert_eq!(
        super::dsh_oauth::account_id(&card).unwrap(),
        "synthetic-account"
    );
}
