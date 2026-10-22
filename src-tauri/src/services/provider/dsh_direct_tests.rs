use super::*;
use crate::{
    app_config::AppType,
    provider::{AuthBinding, AuthBindingSource, ProviderMeta},
    services::ProviderService,
    store::AppState,
};
use serial_test::serial;
use std::{ffi::OsString, sync::Arc};

struct Fixture {
    home: tempfile::TempDir,
    old: Vec<(&'static str, Option<OsString>)>,
}
impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let old = ["CC_SWITCH_TEST_HOME", "DSH_HOME"]
            .into_iter()
            .map(|key| (key, std::env::var_os(key)))
            .collect();
        std::env::set_var("CC_SWITCH_TEST_HOME", home.path());
        std::env::set_var("DSH_HOME", home.path().join(".dsh"));
        let value = Self { home, old };
        for name in ["desktop", "web"] {
            let file = value.profile(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file,"- id: agent-default-model\n  name: '@deepseek-ai/dsh-agent-default-model'\n  config: {provider: deepseek-official, model: deepseek-flash, reasoningEffort: high, other: keep}\n- id: unrelated\n  config: {untouched: true}\n").unwrap();
        }
        std::fs::write(
            value.root().join(".credentials.yaml"),
            "version: 1\nrefs: {UNCHANGED: fixture-old}\n",
        )
        .unwrap();
        crate::settings::reload_settings().unwrap();
        value
    }
    fn root(&self) -> PathBuf {
        self.home.path().join(".dsh")
    }
    fn profile(&self, name: &str) -> PathBuf {
        self.root()
            .join("profiles")
            .join(name)
            .join("cordis.patch.yml")
    }
    fn doc(&self, name: &str) -> serde_yaml::Value {
        serde_yaml::from_str(&std::fs::read_to_string(self.profile(name)).unwrap()).unwrap()
    }
    fn catalog(&self) {
        for profile in ["desktop", "web"] {
            let dir = self.profile(profile).parent().unwrap().to_path_buf();
            let bundle = dir.join("node_modules/@fixture/catalog");
            std::fs::create_dir_all(&bundle).unwrap();
            std::fs::write(
                dir.join("package.json"),
                r#"{"dsh":{"profile":{"bundles":["@fixture/catalog"]}}}"#,
            )
            .unwrap();
            std::fs::write(bundle.join("package.json"), r#"{"name":"@fixture/catalog","version":"1.0.0","dsh":{"bundle":{"patch":"defaults.yml"}}}"#).unwrap();
            std::fs::write(bundle.join("defaults.yml"), r#"[{"insert":[{"id":"agent-default-model","name":"@deepseek-ai/dsh-agent-default-model","config":{"provider":"deepseek-official","model":"deepseek-flash"}},{"id":"llm-deepseek","name":"@deepseek-ai/dsh-llm-deepseek-api-key","config":{}}]}]"#).unwrap();
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for (key, value) in &self.old {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}
fn inventory_provider(profile: &str) -> Provider {
    Provider::with_id(
        native_provider_id(profile, "deepseek-official"),
        "Native".into(),
        json!({"nativeRef":true,"nativeProvider":true,"profile":profile,"provider":"deepseek-official",
            "model":"deepseek-flash","modelCatalog":{"models":[{"model":"deepseek-flash"},{"model":"deepseek-v4-pro"}]}}),
        None,
    )
}

#[test]
#[serial]
fn source2_native_model_edit_is_profile_scoped_and_catalog_checked() {
    let fixture = Fixture::new();
    let provider = inventory_provider("desktop");
    let desktop = std::fs::read(fixture.profile("desktop")).unwrap();
    let web = std::fs::read(fixture.profile("web")).unwrap();
    let credentials = std::fs::read(fixture.root().join(".credentials.yaml")).unwrap();
    let edits = native_selection_edits(&fixture.root(), &provider, "deepseek-v4-pro").unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].path, fixture.profile("desktop"));
    let doc: serde_yaml::Value = serde_yaml::from_str(&edits[0].after).unwrap();
    assert_eq!(selected(&doc)["provider"], "deepseek-official");
    assert_eq!(selected(&doc)["model"], "deepseek-v4-pro");
    assert_eq!(selected(&doc)["other"], "keep");
    assert!(selected(&doc).get("reasoningEffort").is_none());
    assert!(
        native_selection_edits(&fixture.root(), &provider, "not-listed")
            .err()
            .unwrap()
            .contains("NOT_IN_CATALOG")
    );
    let mut wrong = provider.clone();
    wrong.id.push('x');
    assert!(native_selection_edits(&fixture.root(), &wrong, "deepseek-flash").is_err());
    assert_eq!(std::fs::read(fixture.profile("desktop")).unwrap(), desktop);
    assert_eq!(std::fs::read(fixture.profile("web")).unwrap(), web);
    assert_eq!(
        std::fs::read(fixture.root().join(".credentials.yaml")).unwrap(),
        credentials
    );
}

#[test]
#[serial]
fn source2_native_inventory_registration_rejects_identity_collisions_before_any_save() {
    let _fixture = Fixture::new();
    let db = Database::memory().unwrap();
    let desktop = inventory_provider("desktop");
    let web = inventory_provider("web");
    let mut collision = web.clone();
    collision.settings_config = json!({"unrelated":true});
    db.save_provider("dsh", &collision).unwrap();
    assert!(register_native_refs(&db, vec![desktop.clone(), web])
        .unwrap_err()
        .to_string()
        .contains("ID_CONFLICT"));
    assert!(db.get_provider_by_id(&desktop.id, "dsh").unwrap().is_none());
    let mut invalid = desktop.clone();
    invalid.id = native_provider_id("desktop", "different");
    assert!(register_native_refs(&db, vec![invalid]).is_err());
    assert_eq!(
        register_native_refs(&db, vec![desktop.clone()]).unwrap()[0].id,
        desktop.id
    );
}

#[test]
#[serial]
fn source2_native_model_switch_uses_common_publication_and_preserves_other_profile() {
    let fixture = Fixture::new();
    fixture.catalog();
    let state = AppState::new(Arc::new(Database::memory().unwrap()));
    let db = state.db.as_ref();
    let rows = crate::commands::read_dsh_native_providers(db, None).unwrap();
    register_native_refs(&db, rows).unwrap();
    let provider = inventory_provider("desktop");
    let web = std::fs::read(fixture.profile("web")).unwrap();
    let credentials = std::fs::read(fixture.root().join(".credentials.yaml")).unwrap();
    select_native_model(&state, &provider.id, "deepseek-v4-pro").unwrap();
    assert_eq!(
        selected(&fixture.doc("desktop"))["model"],
        "deepseek-v4-pro"
    );
    assert_eq!(std::fs::read(fixture.profile("web")).unwrap(), web);
    assert_eq!(
        std::fs::read(fixture.root().join(".credentials.yaml")).unwrap(),
        credentials
    );
    assert_eq!(
        db.get_current_provider("dsh").unwrap().as_deref(),
        Some(provider.id.as_str())
    );
    let after = crate::commands::read_dsh_native_providers(&db, native_default_baseline().unwrap())
        .unwrap();
    let current = after.iter().find(|p| p.id == provider.id).unwrap();
    assert_eq!(current.settings_config["nativeIsCurrent"], true);
    assert_eq!(current.settings_config["model"], "deepseek-v4-pro");
}

fn api(id: &str, model: &str) -> Provider {
    Provider::with_id(
        id.into(),
        id.into(),
        json!({"auth":{"OPENAI_API_KEY":"fixture-only"},
        "config":format!("model = \"{model}\"\nmodel_provider = \"custom\"\n[model_providers.custom]\nname = \"custom\"\nbase_url = \"https://fixture.invalid/v1\"\nwire_api = \"responses\"\n")}),
        None,
    )
}

#[test]
#[serial]
fn source2_review_disabled_native_default_never_publishes_a_selection() {
    let fixture = Fixture::new();
    let provider = inventory_provider("desktop");
    let mut disabled = provider.clone();
    disabled.settings_config["nativeSelectionDisabled"] = json!(true);
    assert!(
        native_selection_edits(&fixture.root(), &disabled, "deepseek-flash")
            .err()
            .unwrap()
            .contains("DEFAULT_MODEL_DISABLED")
    );
    let before = std::fs::read_to_string(fixture.profile("desktop"))
        .unwrap()
        .replace(
            "- id: agent-default-model",
            "- disabled: true\n  id: agent-default-model",
        );
    std::fs::write(fixture.profile("desktop"), &before).unwrap();
    assert!(
        native_selection_edits(&fixture.root(), &provider, "deepseek-flash")
            .err()
            .unwrap()
            .contains("DEFAULT_MODEL_DISABLED")
    );
    assert_eq!(
        std::fs::read_to_string(fixture.profile("desktop")).unwrap(),
        before
    );
}

#[test]
#[serial]
fn source2_review_native_selection_waits_for_mode_lock_and_rechecks_the_settled_mode() {
    use std::sync::mpsc;
    use std::time::Duration;
    let fixture = Fixture::new();
    let state = Arc::new(AppState::new(Arc::new(Database::memory().unwrap())));
    let guard = crate::mode::controller::lock_settled_blocking(&state, &AppType::Dsh).unwrap();
    let before = std::fs::read(fixture.profile("desktop")).unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = state.clone();
    let handle = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        done_tx
            .send(select_native_model(&worker, "missing-provider", "model"))
            .unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let early = done_rx.recv_timeout(Duration::from_millis(100));
    state::update(&DeviceStore::for_device(), |live| {
        let app = live.apps.entry("dsh".into()).or_default();
        app.mode = Some(state::Mode::Proxy);
        app.attached = true;
    })
    .unwrap();
    drop(guard);
    let blocked = early.is_err();
    let result = early.unwrap_or_else(|_| done_rx.recv_timeout(Duration::from_secs(5)).unwrap());
    handle.join().unwrap();
    assert!(blocked, "selection must wait for the common mode lock");
    assert!(result.unwrap_err().to_string().contains("REQUIRES_DIRECT"));
    assert_eq!(std::fs::read(fixture.profile("desktop")).unwrap(), before);
    assert!(state.db.get_current_provider("dsh").unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn source2_review_mixed_native_cc_profiles_remain_observed_and_restore_through_route() {
    let fixture = Fixture::new();
    fixture.catalog();
    let state = AppState::new(Arc::new(Database::memory().unwrap()));
    for id in ["api-a", "route-b"] {
        state.db.save_provider("dsh", &api(id, "cc-model")).unwrap();
    }
    ProviderService::switch(&state, AppType::Dsh, "api-a").unwrap();
    let rows =
        crate::commands::read_dsh_native_providers(&state.db, native_default_baseline().unwrap())
            .unwrap();
    register_native_refs(&state.db, rows.into_iter().filter(is_native_ref).collect()).unwrap();
    let provider = inventory_provider("desktop");
    let original_credentials = std::fs::read(fixture.root().join(".credentials.yaml")).unwrap();
    select_native_model(&state, &provider.id, "deepseek-v4-pro").unwrap();
    assert_eq!(
        std::fs::read(fixture.root().join(".credentials.yaml")).unwrap(),
        original_credentials
    );
    let web = selected(&fixture.doc("web")).clone();
    assert_eq!(web["provider"], id_for("api-a"));
    let rows =
        crate::commands::read_dsh_native_providers(&state.db, native_default_baseline().unwrap())
            .unwrap();
    assert!(rows.iter().any(|row| row.id == "api-a"
        && row.settings_config["nativeObservedProfile"] == "web"
        && row.settings_config["nativeIsCurrent"] == true));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut config = state.db.get_global_proxy_config().await.unwrap();
    config.listen_address = "127.0.0.1".into();
    config.listen_port = port;
    state.db.update_global_proxy_config(config).await.unwrap();
    let entered =
        crate::mode::controller::enter_with_route(&state, &AppType::Dsh, false, Some("route-b"))
            .await;
    let exited = crate::mode::controller::exit(&state, &AppType::Dsh).await;
    entered.unwrap();
    exited.unwrap();
    assert_eq!(
        selected(&fixture.doc("desktop"))["model"],
        "deepseek-v4-pro"
    );
    assert_eq!(selected(&fixture.doc("web")), &web);
    let credentials: serde_yaml::Value = serde_yaml::from_slice(&original_credentials).unwrap();
    let after: serde_yaml::Value =
        serde_yaml::from_slice(&std::fs::read(fixture.root().join(".credentials.yaml")).unwrap())
            .unwrap();
    for (key, value) in credentials["refs"].as_mapping().unwrap() {
        assert_eq!(&after["refs"][key], value);
    }
    assert!(!state.proxy_service.is_running().await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn source2_review_inherited_profile_replaces_a_stale_return_baseline() {
    let fixture = Fixture::new();
    fixture.catalog();
    let state = AppState::new(Arc::new(Database::memory().unwrap()));
    for id in ["api-a", "route-b"] {
        state.db.save_provider("dsh", &api(id, "cc-model")).unwrap();
    }
    ProviderService::switch(&state, AppType::Dsh, "api-a").unwrap();
    let bundle = fixture
        .profile("web")
        .parent()
        .unwrap()
        .join("node_modules/@fixture/catalog/defaults.yml");
    let updated = std::fs::read_to_string(&bundle)
        .unwrap()
        .replace("deepseek-flash", "deepseek-v4-pro");
    std::fs::write(bundle, updated).unwrap();
    let web_patch = "- id: agent-default-model\n  name: '@deepseek-ai/dsh-agent-default-model'\n- id: unrelated\n  config: {other: keep}\n";
    std::fs::write(fixture.profile("web"), web_patch).unwrap();
    assert_eq!(
        native_default_baseline().unwrap().unwrap()["web"]["model"],
        "deepseek-flash"
    );
    let rows =
        crate::commands::read_dsh_native_providers(&state.db, native_default_baseline().unwrap())
            .unwrap();
    register_native_refs(&state.db, rows.into_iter().filter(is_native_ref).collect()).unwrap();
    select_native_model(&state, &inventory_provider("desktop").id, "deepseek-flash").unwrap();
    assert_eq!(
        std::fs::read_to_string(fixture.profile("web")).unwrap(),
        web_patch
    );
    assert_eq!(
        native_default_baseline().unwrap().unwrap()["web"]["model"],
        "deepseek-v4-pro"
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut config = state.db.get_global_proxy_config().await.unwrap();
    config.listen_address = "127.0.0.1".into();
    config.listen_port = port;
    state.db.update_global_proxy_config(config).await.unwrap();
    let entered =
        crate::mode::controller::enter_with_route(&state, &AppType::Dsh, false, Some("route-b"))
            .await;
    let exited = crate::mode::controller::exit(&state, &AppType::Dsh).await;
    entered.unwrap();
    exited.unwrap();
    assert_eq!(selected(&fixture.doc("desktop"))["model"], "deepseek-flash");
    assert_eq!(
        selected(&fixture.doc("web"))["provider"],
        "deepseek-official"
    );
    assert_eq!(selected(&fixture.doc("web"))["model"], "deepseek-v4-pro");
    assert_eq!(
        fixture
            .doc("web")
            .as_sequence()
            .unwrap()
            .iter()
            .find(|row| row["id"] == "unrelated")
            .unwrap()["config"]["other"],
        "keep"
    );
    assert!(!state.proxy_service.is_running().await);
}

#[test]
#[serial]
fn source19_cc_direct_keeps_and_restores_each_native_profile_without_credentials() {
    let fixture = Fixture::new();
    let web = std::fs::read_to_string(fixture.profile("web"))
        .unwrap()
        .replace("deepseek-flash", "deepseek-pro")
        .replace("reasoningEffort: high", "reasoningEffort: low");
    std::fs::write(fixture.profile("web"), web).unwrap();
    let before = [fixture.doc("desktop"), fixture.doc("web")];
    let state = AppState::new(Arc::new(Database::memory().unwrap()));
    let managed = api("api-a", "managed-model");
    state.db.save_provider("dsh", &managed).unwrap();
    ProviderService::switch(&state, AppType::Dsh, &managed.id).unwrap();
    let credentials = std::fs::read(fixture.root().join(".credentials.yaml")).unwrap();
    let baseline = native_default_baseline()
        .unwrap()
        .expect("CC direct must retain the native baseline too");
    assert_eq!(baseline["desktop"]["model"], "deepseek-flash");
    assert_eq!(baseline["web"]["model"], "deepseek-pro");
    let mut native_ref = Provider::with_id(
        "native-current:desktop".into(),
        "Native Desktop".into(),
        json!({"nativeRef":true,"profile":"desktop","provider":"deepseek-official","model":"deepseek-flash"}),
        None,
    );
    native_ref.category = Some("official".into());
    state.db.save_provider("dsh", &native_ref).unwrap();
    ProviderService::switch(&state, AppType::Dsh, &native_ref.id)
        .expect("native ref must use the common switch service, not require an API key");
    for (index, profile) in ["desktop", "web"].into_iter().enumerate() {
        assert_eq!(selected(&fixture.doc(profile)), selected(&before[index]));
    }
    assert_eq!(
        ProviderService::current(&state, AppType::Dsh).unwrap(),
        native_ref.id
    );
    assert_eq!(
        std::fs::read(fixture.root().join(".credentials.yaml")).unwrap(),
        credentials
    );
    assert!(state
        .db
        .get_provider_by_id("api-a", "dsh")
        .unwrap()
        .is_some());
    assert!(
        native(&fixture.doc("web"), &id_for("api-a")).is_mapping(),
        "keep the added provider, only change its selection"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn source19_profiles_with_inherited_defaults_are_written_even_without_local_patches() {
    for missing in [vec!["desktop", "web"], vec!["web"]] {
        for routing in [false, true] {
            let fixture = Fixture::new();
            for (profile, model) in [("desktop", "deepseek-flash"), ("web", "deepseek-pro")] {
                let dir = fixture.profile(profile).parent().unwrap().to_path_buf();
                let bundle = dir.join("node_modules/@fixture/defaults");
                std::fs::create_dir_all(&bundle).unwrap();
                std::fs::write(
                    dir.join("package.json"),
                    r#"{"dsh":{"profile":{"bundles":["@fixture/defaults"]}}}"#,
                )
                .unwrap();
                std::fs::write(bundle.join("package.json"), r#"{"name":"@fixture/defaults","version":"1.0.0","dsh":{"bundle":{"patch":"defaults.yml"}}}"#).unwrap();
                std::fs::write(bundle.join("defaults.yml"), format!("- insert:\n  - id: agent-default-model\n    name: '@deepseek-ai/dsh-agent-default-model'\n    config: {{provider: deepseek-official, model: {model}}}\n")).unwrap();
            }
            for profile in &missing {
                let file = fixture.profile(profile);
                std::fs::remove_file(file).unwrap();
            }
            let state = AppState::new(Arc::new(Database::memory().unwrap()));
            state
                .db
                .save_provider("dsh", &api("a", "chosen-model"))
                .unwrap();
            if routing {
                let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                let port = listener.local_addr().unwrap().port();
                drop(listener);
                let mut config = state.db.get_global_proxy_config().await.unwrap();
                config.listen_address = "127.0.0.1".into();
                config.listen_port = port;
                state.db.update_global_proxy_config(config).await.unwrap();
                let entered = crate::mode::controller::enter_with_route(
                    &state,
                    &AppType::Dsh,
                    false,
                    Some("a"),
                )
                .await;
                let observed: Vec<_> = ["desktop", "web"].map(|p| fixture.doc(p)).into();
                let exited = crate::mode::controller::exit(&state, &AppType::Dsh).await;
                entered.unwrap();
                exited.unwrap();
                for doc in observed {
                    assert_eq!(selected(&doc)["model"], "chosen-model");
                }
                assert!(!state.proxy_service.is_running().await);
            } else {
                ProviderService::switch(&state, AppType::Dsh, "a").unwrap();
            }
            for profile in ["desktop", "web"] {
                assert!(
                    fixture.profile(profile).exists(),
                    "the shared app switch must reach {profile}"
                );
                assert_eq!(
                    selected(&fixture.doc(profile))["model"],
                    if routing {
                        if profile == "web" {
                            "deepseek-pro"
                        } else {
                            "deepseek-flash"
                        }
                    } else {
                        "chosen-model"
                    }
                );
            }
            let before = std::fs::read(fixture.profile("desktop")).unwrap();
            let global = fixture.root().join("cordis.patch.yml");
            std::fs::write(global, "- id: unrelated\n  config: {home: keep}\n").unwrap();
            assert!(ProviderService::switch(&state, AppType::Dsh, "a").is_err());
            assert_eq!(std::fs::read(fixture.profile("desktop")).unwrap(), before);
        }
    }
    let fixture = Fixture::new();
    for profile in ["desktop", "web"] {
        std::fs::remove_file(fixture.profile(profile)).unwrap();
    }
    assert!(dsh_patch_paths(&fixture.root()).unwrap().is_empty());
    let state = AppState::new(Arc::new(Database::memory().unwrap()));
    state
        .db
        .save_provider("dsh", &api("a", "chosen-model"))
        .unwrap();
    assert!(ProviderService::switch(&state, AppType::Dsh, "a")
        .unwrap_err()
        .to_string()
        .contains("PROFILE_NOT_FOUND"));
}

#[test]
#[serial]
fn source19_cc_direct_inherited_defaults_do_not_restore_the_cc_projection_as_native() {
    let fixture = Fixture::new();
    for (profile, model) in [("desktop", "deepseek-flash"), ("web", "deepseek-pro")] {
        let dir = fixture.profile(profile).parent().unwrap().to_path_buf();
        let bundle = dir.join("node_modules/@fixture/defaults");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(
            dir.join("package.json"),
            r#"{"dsh":{"profile":{"bundles":["@fixture/defaults"]}}}"#,
        )
        .unwrap();
        std::fs::write(bundle.join("package.json"), r#"{"name":"@fixture/defaults","version":"1.0.0","dsh":{"bundle":{"patch":"defaults.yml"}}}"#).unwrap();
        std::fs::write(bundle.join("defaults.yml"), format!("- insert:\n  - id: agent-default-model\n    name: '@deepseek-ai/dsh-agent-default-model'\n    config: {{provider: deepseek-official, model: {model}, reasoningEffort: low}}\n")).unwrap();
        std::fs::write(fixture.profile(profile), "- id: agent-default-model\n  name: '@deepseek-ai/dsh-agent-default-model'\n  config: {other: keep}\n").unwrap();
    }
    let state = AppState::new(Arc::new(Database::memory().unwrap()));
    state
        .db
        .save_provider("dsh", &api("api-a", "cc-model"))
        .unwrap();
    ProviderService::switch(&state, AppType::Dsh, "api-a").unwrap();
    let native = register_native_refs(
        &state.db,
        vec![Provider::with_id(
            "native-current:desktop".into(),
            "Native".into(),
            json!({"profile":"desktop"}),
            None,
        )],
    )
    .unwrap()
    .remove(0);
    ProviderService::switch(&state, AppType::Dsh, &native.id).unwrap();
    for (profile, model) in [("desktop", "deepseek-flash"), ("web", "deepseek-pro")] {
        let doc = fixture.doc(profile);
        assert_eq!(selected(&doc)["provider"], "deepseek-official");
        assert_eq!(selected(&doc)["model"], model);
        assert_eq!(selected(&doc)["reasoningEffort"], "low");
        assert_eq!(selected(&doc)["other"], "keep");
    }
}

#[test]
fn source19_native_refs_keep_stable_selection_and_separate_balance_identity() {
    let db = Database::memory().unwrap();
    let row = |profile: &str, revision: &str| {
        Provider::with_id(
            format!("native-current:{profile}:{revision}"),
            profile.into(),
            json!({"profile":profile,"provider":"deepseek-official","model":"deepseek-flash","nativeBalanceAuth":"api-key"}),
            None,
        )
    };
    let registered =
        register_native_refs(&db, vec![row("desktop", "first"), row("web", "web-first")]).unwrap();
    assert_eq!(registered[0].id, "native-current:desktop");
    assert!(is_native_ref(&registered[0]));
    assert_eq!(
        registered[0].settings_config["nativeBalanceId"],
        "native-current:desktop:first"
    );
    let changed = register_native_refs(&db, vec![row("desktop", "second")]).unwrap();
    assert_eq!(changed[0].id, registered[0].id);
    assert_eq!(
        changed[0].settings_config["nativeBalanceId"],
        "native-current:desktop:second"
    );
    assert_eq!(db.get_all_providers("dsh").unwrap().len(), 2);
    db.save_provider("dsh", &api("native-current:web", "unrelated"))
        .unwrap();
    assert!(register_native_refs(&db, vec![row("desktop", "third"), row("web", "third")]).is_err());
    assert_eq!(
        db.get_provider_by_id("native-current:desktop", "dsh")
            .unwrap()
            .unwrap()
            .settings_config["nativeBalanceId"],
        "native-current:desktop:second",
        "preflight every collision before updating any native reference"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn source19_public_route_exit_restores_distinct_native_profiles_without_account_or_model_calls(
) {
    let fixture = Fixture::new();
    let web = std::fs::read_to_string(fixture.profile("web"))
        .unwrap()
        .replace("deepseek-flash", "deepseek-pro");
    std::fs::write(fixture.profile("web"), web).unwrap();
    let original = [fixture.doc("desktop"), fixture.doc("web")];
    let state = AppState::new(Arc::new(Database::memory().unwrap()));
    let native = register_native_refs(
        &state.db,
        vec![Provider::with_id(
            "native-current:desktop".into(),
            "Native".into(),
            json!({"profile":"desktop","provider":"deepseek-official","model":"deepseek-flash"}),
            None,
        )],
    )
    .unwrap()
    .remove(0);
    ProviderService::switch(&state, AppType::Dsh, &native.id).unwrap();
    let mut managed = api("managed", "fixture-model");
    managed.settings_config["auth"] = json!({});
    managed.settings_config["modelCatalog"] = json!({"models":[{"model":"fixture-model"}]});
    managed.meta = Some(ProviderMeta {
        auth_binding: Some(AuthBinding {
            source: AuthBindingSource::ManagedAccount,
            auth_provider: Some("codex_oauth".into()),
            account_id: Some("fixture-no-token".into()),
        }),
        ..Default::default()
    });
    state.db.save_provider("dsh", &managed).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut config = state.db.get_global_proxy_config().await.unwrap();
    config.listen_address = "127.0.0.1".into();
    config.listen_port = port;
    state.db.update_global_proxy_config(config).await.unwrap();
    let entered =
        crate::mode::controller::enter_with_route(&state, &AppType::Dsh, false, Some("managed"))
            .await;
    let routed = [fixture.doc("desktop"), fixture.doc("web")];
    let credentials = std::fs::read(fixture.root().join(".credentials.yaml")).unwrap();
    let exited = crate::mode::controller::exit(&state, &AppType::Dsh).await;
    entered.unwrap();
    exited.unwrap();
    for (index, profile) in ["desktop", "web"].into_iter().enumerate() {
        assert_eq!(selected(&routed[index])["provider"], PROXY_ID);
        assert_eq!(selected(&fixture.doc(profile)), selected(&original[index]));
    }
    assert_eq!(
        ProviderService::current(&state, AppType::Dsh).unwrap(),
        native.id
    );
    assert_eq!(
        std::fs::read(fixture.root().join(".credentials.yaml")).unwrap(),
        credentials
    );
    assert!(!state.proxy_service.is_running().await);
}

#[test]
fn source15_dsh_account_catalog_expands_choices_without_changing_default_or_native_rows() {
    let mut provider = api("managed", "chosen-model");
    let original = provider.clone();
    add_managed_catalog(
        &mut provider,
        ["other-model", "chosen-model", "third-model"]
            .map(|model| json!({"model":model}))
            .into_iter(),
    );
    let projected = projection(
        &provider,
        Some(("http://127.0.0.1:15721/dsh/v1", "PROXY_MANAGED")),
        &[],
    )
    .unwrap();
    assert_eq!(
        projected
            .models
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["other-model", "chosen-model", "third-model"]
    );
    assert_eq!(projected.default_model, "chosen-model");
    assert_eq!(projected.id, PROXY_ID);
    assert!(original.settings_config.get("modelCatalog").is_none());
    assert_eq!(projected.api, "openai-responses");
}

#[test]
#[serial]
fn source4_dsh_explicit_efforts_do_not_inherit_an_incompatible_remote_default() {
    let db = Database::memory().unwrap();
    let mut provider = api("managed", "chosen-model");
    provider.settings_config["modelCatalog"] = json!({"models":[
        {"model":"chosen-model", "reasoningLevels":["high"]},
        {"model":"other-model"}
    ]});
    db.save_provider("dsh", &provider).unwrap();
    let original = db.get_provider_by_id("managed", "dsh").unwrap().unwrap();
    let published = publish_managed_catalog(&db, &original, [
        json!({"model":"chosen-model", "reasoningLevels":["low","medium","high"], "defaultReasoningLevel":"low"}),
        json!({"model":"other-model", "reasoningLevels":["medium","high"], "defaultReasoningLevel":"medium"})
    ].into_iter()).unwrap();
    assert_eq!(
        published.settings_config["modelCatalog"]["models"][0],
        original.settings_config["modelCatalog"]["models"][0]
    );
    let projected = projection(
        &published,
        Some(("http://127.0.0.1:15721/dsh/v1", "PROXY_MANAGED")),
        &[],
    )
    .unwrap();
    assert!(projected.effort.is_none());
    assert_eq!(
        projected.models[0]["reasoningEfforts"],
        json!({"high":"high"})
    );
    let mut bad_default = published;
    bad_default.settings_config["modelCatalog"]["models"][0]["defaultReasoningLevel"] =
        json!("low");
    assert!(validate(&bad_default)
        .unwrap_err()
        .to_string()
        .contains("NATIVE_DEFAULT_EFFORT_NOT_IN_MODEL_CATALOG"));
}

#[test]
#[serial]
fn source4_dsh_efforts_survive_catalog_enrichment_and_both_profile_projections() {
    let fixture = Fixture::new();
    let db = Database::memory().unwrap();
    let mut provider = api("managed", "gpt-6.1-sol");
    provider.settings_config["config"] = json!(format!(
        "model_reasoning_effort = \"max\"\n{}",
        provider.settings_config["config"].as_str().unwrap()
    ));
    provider.settings_config["modelCatalog"] = json!({"autoReviewModel":"keep", "models":[
        {"model":"gpt-6.1-sol"},
        {"model":"gpt-6-luna", "displayName":"My model", "reasoningLevels":["high"], "defaultReasoningLevel":"high"}
    ]});
    db.save_provider("dsh", &provider).unwrap();
    let original = db.get_provider_by_id("managed", "dsh").unwrap().unwrap();
    let remote = crate::services::codex_oauth_models::parse_model_catalog(json!({"models":[
        {"slug":"gpt-6.1-sol", "default_reasoning_level":"low", "supported_reasoning_levels":[
            {"effort":"low"},{"effort":"medium"},{"effort":"high"},
            {"effort":"xhigh"},{"effort":"max"},{"effort":"ultra"}
        ]},
        {"slug":"gpt-6-luna", "default_reasoning_level":"medium", "supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}]},
        {"slug":"not-selected", "supported_reasoning_levels":[{"effort":"high"}]}
    ]}));
    let published = publish_managed_catalog(&db, &original, remote.into_iter()).unwrap();
    assert_eq!(
        published.settings_config["config"],
        original.settings_config["config"]
    );
    assert_eq!(
        published.settings_config["auth"],
        original.settings_config["auth"]
    );
    assert_eq!(
        published.settings_config["modelCatalog"]["autoReviewModel"],
        "keep"
    );
    assert_eq!(
        published.settings_config["modelCatalog"]["models"][1],
        original.settings_config["modelCatalog"]["models"][1]
    );
    let projected = projection(
        &published,
        Some(("http://127.0.0.1:15721/dsh/v1", "PROXY_MANAGED")),
        &[],
    )
    .unwrap();
    assert_eq!(projected.effort.as_deref(), Some("max"));
    assert_eq!(projected.models.len(), 2);
    assert_eq!(
        projected.models[0]["reasoningEfforts"],
        json!({"low":"low","medium":"medium","high":"high","xhigh":"xhigh","max":"max"})
    );
    assert_eq!(
        projected.models[1]["reasoningEfforts"],
        json!({"high":"high"})
    );
    assert!(projected.models[0]["reasoningEfforts"]
        .get("ultra")
        .is_none());
    let edits = edits_for(&fixture.root(), &projected).unwrap();
    for profile in ["desktop", "web"] {
        let edit = edits
            .iter()
            .find(|edit| edit.path == fixture.profile(profile))
            .unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&edit.after).unwrap();
        assert_eq!(selected(&doc)["reasoningEffort"], "max");
        let model = &doc
            .as_sequence()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == "llm-pi-ai")
            .unwrap()["config"]["providers"][PROXY_ID]["models"][0];
        assert_eq!(model["reasoningEfforts"].as_mapping().unwrap().len(), 5);
        assert_eq!(model["reasoningEfforts"]["low"], "low");
        assert_eq!(model["reasoningEfforts"]["max"], "max");
        assert_eq!(
            fixture.doc(profile)[0]["config"]["provider"],
            "deepseek-official"
        );
    }
    assert!(
        publish_managed_catalog(&db, &original, [json!({"model":"unwanted"})].into_iter()).is_err()
    );
}

#[test]
fn source15_dsh_catalog_publication_preserves_account_and_rejects_drift_or_explicit_catalog() {
    let db = Database::memory().unwrap();
    let mut provider = api("managed", "chosen-model");
    provider.meta = Some(ProviderMeta {
        auth_binding: Some(AuthBinding {
            source: AuthBindingSource::ManagedAccount,
            auth_provider: Some("codex_oauth".into()),
            account_id: Some("fixture-account".into()),
        }),
        ..Default::default()
    });
    provider.settings_config["auth"] = json!({});
    db.save_provider("dsh", &provider).unwrap();
    let original = db.get_provider_by_id("managed", "dsh").unwrap().unwrap();
    let published = publish_managed_catalog(
        &db,
        &original,
        ["chosen-model", "other-model"]
            .map(|model| json!({"model":model}))
            .into_iter(),
    )
    .unwrap();
    let saved = db.get_provider_by_id("managed", "dsh").unwrap().unwrap();
    assert_eq!(saved.settings_config, published.settings_config);
    assert_eq!(
        saved.settings_config["config"],
        original.settings_config["config"]
    );
    assert_eq!(
        saved.settings_config["auth"],
        original.settings_config["auth"]
    );
    assert_eq!(
        serde_json::to_value(&saved.meta).unwrap(),
        serde_json::to_value(&original.meta).unwrap()
    );
    assert!(
        publish_managed_catalog(&db, &original, [json!({"model":"unwanted"})].into_iter()).is_err()
    );
    assert_eq!(
        db.get_provider_by_id("managed", "dsh")
            .unwrap()
            .unwrap()
            .settings_config,
        saved.settings_config
    );
    db.save_provider("dsh", &original).unwrap();
    let mut changed = original.clone();
    changed
        .meta
        .as_mut()
        .unwrap()
        .auth_binding
        .as_mut()
        .unwrap()
        .account_id = Some("another-account".into());
    db.save_provider("dsh", &changed).unwrap();
    assert!(
        publish_managed_catalog(&db, &original, [json!({"model":"unwanted"})].into_iter()).is_err()
    );
    assert_eq!(
        serde_json::to_value(db.get_provider_by_id("managed", "dsh").unwrap().unwrap()).unwrap(),
        serde_json::to_value(changed).unwrap()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn source15_dsh_published_nondefault_model_reaches_loopback_without_stack_or_default_change()
{
    use axum::{routing::post, Json, Router};
    let fixture = Fixture::new();
    let received = Arc::new(tokio::sync::Mutex::new(Vec::<Value>::new()));
    let capture = received.clone();
    let router = Router::new().route("/v1/responses", post(move |Json(body): Json<Value>| {
        let capture = capture.clone();
        async move {
            capture.lock().await.push(body.clone());
            Json(json!({"id":"synthetic-response", "object":"response", "status":"completed", "model":body["model"], "output":[]}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_base = format!("http://{}/v1", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let state = AppState::new(Arc::new(Database::memory().unwrap()));
    let mut provider = api("route", "chosen-model");
    provider.settings_config["config"] = json!(format!("model = \"chosen-model\"\nmodel_provider = \"custom\"\n[model_providers.custom]\nbase_url = \"{upstream_base}\"\nwire_api = \"responses\"\n"));
    ProviderService::add_without_switching(&state, AppType::Dsh, provider.clone()).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut config = state.db.get_global_proxy_config().await.unwrap();
    config.listen_address = "127.0.0.1".into();
    config.listen_port = port;
    state.db.update_global_proxy_config(config).await.unwrap();
    let observed = async {
        let published = publish_managed_catalog(
            &state.db,
            &provider,
            ["chosen-model", "other-model"]
                .map(|model| json!({"model":model}))
                .into_iter(),
        )
        .unwrap();
        crate::mode::controller::enter_with_route(&state, &AppType::Dsh, false, Some("route"))
            .await
            .unwrap();
        assert!(
            !state::stack(&DeviceStore::for_device(), "dsh")
                .unwrap()
                .enabled
        );
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap();
        let response = client
            .post(format!("http://127.0.0.1:{port}/dsh/v1/responses"))
            .bearer_auth("PROXY_MANAGED")
            .json(&json!({"model":"other-model", "stream":false, "input":"synthetic-only"}))
            .send()
            .await
            .unwrap();
        let status = response.status();
        let result: Value = response.json().await.unwrap();
        (status, result, published)
    }
    .await;
    let exit = crate::mode::controller::exit(&state, &AppType::Dsh).await;
    upstream.abort();
    exit.unwrap();
    assert_eq!(observed.0, reqwest::StatusCode::OK, "{}", observed.1);
    let requests = received.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["model"], "other-model");
    assert_eq!(
        observed.2.settings_config["config"],
        provider.settings_config["config"]
    );
    assert_eq!(selected(&fixture.doc("web"))["model"], "deepseek-flash");
    assert!(!state.proxy_service.is_running().await);
}
fn selected(doc: &serde_yaml::Value) -> &serde_yaml::Value {
    &doc.as_sequence()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "agent-default-model")
        .unwrap()["config"]
}
fn native<'a>(doc: &'a serde_yaml::Value, id: &str) -> &'a serde_yaml::Value {
    &doc.as_sequence()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "llm-pi-ai")
        .unwrap()["config"]["providers"][id]
}

#[test]
#[serial]
fn source11_dsh_common_crud_applies_only_selected_provider_without_mirroring_codex() {
    let fixture = Fixture::new();
    let state = AppState::new(Arc::new(Database::memory().unwrap()));
    let a = api("a", "model-a");
    let b = api("b", "model-b");
    state
        .db
        .save_provider("codex", &api("separate", "codex-model"))
        .unwrap();
    ProviderService::add(&state, AppType::Dsh, a.clone(), true).unwrap();
    let before = std::fs::read(fixture.profile("web")).unwrap();
    ProviderService::add(&state, AppType::Dsh, b.clone(), false).unwrap();
    ProviderService::update(&state, AppType::Dsh, None, api("b", "model-b2")).unwrap();
    assert_eq!(std::fs::read(fixture.profile("web")).unwrap(), before);
    ProviderService::switch(&state, AppType::Dsh, "b").unwrap();
    assert_eq!(ProviderService::current(&state, AppType::Dsh).unwrap(), "b");
    for profile in ["desktop", "web"] {
        let doc = fixture.doc(profile);
        assert_eq!(selected(&doc)["provider"], id_for("b"));
        assert_eq!(selected(&doc)["model"], "model-b2");
        assert_eq!(selected(&doc)["other"], "keep");
        assert_eq!(native(&doc, &id_for("b"))["api"], "openai-responses");
        assert!(doc
            .as_sequence()
            .unwrap()
            .iter()
            .any(|entry| entry["id"] == "unrelated"));
    }
    assert_eq!(state.db.get_all_providers("codex").unwrap().len(), 1);
    assert!(
        !state::stack(&DeviceStore::for_device(), "dsh")
            .unwrap()
            .enabled
    );
    let credentials: serde_yaml::Value = serde_yaml::from_str(
        &std::fs::read_to_string(fixture.root().join(".credentials.yaml")).unwrap(),
    )
    .unwrap();
    assert_eq!(credentials["refs"]["UNCHANGED"], "fixture-old");
}
#[test]
#[serial]
fn source11_dsh_corrupt_profile_and_native_lock_fail_before_pointer_or_file_publication() {
    let fixture = Fixture::new();
    let state = AppState::new(Arc::new(Database::memory().unwrap()));
    ProviderService::add(&state, AppType::Dsh, api("a", "a"), true).unwrap();
    ProviderService::add(&state, AppType::Dsh, api("b", "b"), false).unwrap();
    let before = std::fs::read(fixture.profile("desktop")).unwrap();
    let credential_before = std::fs::read(fixture.root().join(".credentials.yaml")).unwrap();
    std::fs::write(fixture.root().join(".credentials.yaml.lock"), "999999\n").unwrap();
    assert!(ProviderService::switch(&state, AppType::Dsh, "b")
        .unwrap_err()
        .to_string()
        .contains("DSH_NATIVE_WRITER"));
    std::fs::remove_file(fixture.root().join(".credentials.yaml.lock")).unwrap();
    std::fs::write(fixture.profile("web"), "invalid: [").unwrap();
    assert!(ProviderService::switch(&state, AppType::Dsh, "b").is_err());
    assert_eq!(ProviderService::current(&state, AppType::Dsh).unwrap(), "a");
    assert_eq!(std::fs::read(fixture.profile("desktop")).unwrap(), before);
    assert_eq!(
        std::fs::read(fixture.root().join(".credentials.yaml")).unwrap(),
        credential_before
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn source11_dsh_common_modes_keep_stack_opt_in_and_restore_direct_selection() {
    let fixture = Fixture::new();
    let state = AppState::new(Arc::new(Database::memory().unwrap()));
    ProviderService::add(&state, AppType::Dsh, api("a", "a"), true).unwrap();
    ProviderService::add(&state, AppType::Dsh, api("b", "b"), false).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut config = state.db.get_global_proxy_config().await.unwrap();
    config.listen_address = "127.0.0.1".into();
    config.listen_port = port;
    state.db.update_global_proxy_config(config).await.unwrap();
    crate::mode::controller::enter_with_route(&state, &AppType::Dsh, false, Some("b"))
        .await
        .unwrap();
    let mode = crate::mode::current::mode_state(&AppType::Dsh);
    assert!(mode.is_proxy() && mode.attached);
    assert!(
        !state::stack(&DeviceStore::for_device(), "dsh")
            .unwrap()
            .enabled
    );
    for profile in ["desktop", "web"] {
        let doc = fixture.doc(profile);
        assert_eq!(selected(&doc)["provider"], PROXY_ID);
        assert_eq!(native(&doc, PROXY_ID)["api"], "openai-responses");
        assert_eq!(
            native(&doc, PROXY_ID)["baseURL"],
            format!("http://127.0.0.1:{port}/dsh/v1")
        );
    }
    crate::mode::controller::enter_with_route(&state, &AppType::Dsh, true, Some("b"))
        .await
        .unwrap();
    crate::mode::controller::set_stack_member(&state, &AppType::Dsh, "a", true)
        .await
        .unwrap();
    let stack = state::stack(&DeviceStore::for_device(), "dsh").unwrap();
    assert!(stack.enabled);
    let published = stack::encode(&AppType::Dsh, stack.key_of("a").unwrap(), "a", false);
    assert!(
        matches!(stack::resolve(&state.db,&DeviceStore::for_device(),&AppType::Dsh,&published).unwrap(),stack::Resolved::Hit(target) if target.provider.id=="a" && target.upstream_model=="a")
    );
    assert!(native(&fixture.doc("web"), PROXY_ID)["models"]
        .as_sequence()
        .unwrap()
        .iter()
        .any(|m| m["id"].as_str() == Some(published.as_str())));
    crate::mode::controller::exit(&state, &AppType::Dsh)
        .await
        .unwrap();
    assert_eq!(selected(&fixture.doc("web"))["provider"], id_for("a"));
    assert_eq!(selected(&fixture.doc("web"))["model"], "a");
    assert!(!state.proxy_service.is_running().await);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn source11_dsh_preserves_native_cc_prefixed_default_without_a_cc_direct_row() {
    let fixture = Fixture::new();
    let native_id = id_for("native-existing");
    for profile in ["desktop", "web"] {
        let mut doc = fixture.doc(profile);
        doc[0]["config"]["provider"] = serde_yaml::Value::String(native_id.clone());
        doc[1]["config"]["providers"] = serde_yaml::to_value(json!({
            (PROXY_ID): {"unrelated": "keep"}
        }))
        .unwrap();
        doc.as_sequence_mut().unwrap().push(
            serde_yaml::to_value(json!({"id":"llm-pi-ai","name":"@deepseek-ai/dsh-llm-pi-ai","config":{"providers":{
                (native_id.clone()): {"api":"openai-responses","baseURL":"https://fixture.invalid/v1","apiKeyEnv":"UNCHANGED","models":[{"id":"deepseek-flash","name":"Original"}]}
            }}})).unwrap()
        );
        std::fs::write(
            fixture.profile(profile),
            serde_yaml::to_string(&doc).unwrap(),
        )
        .unwrap();
    }
    let original = fixture.doc("web");
    let state = AppState::new(Arc::new(Database::memory().unwrap()));
    let mut managed = api("managed", "model-managed");
    managed.settings_config["auth"] = json!({});
    managed.category = Some("official".into());
    managed.meta = Some(ProviderMeta {
        auth_binding: Some(AuthBinding {
            source: AuthBindingSource::ManagedAccount,
            auth_provider: Some("codex_oauth".into()),
            account_id: Some("fixture-existing-account".into()),
        }),
        provider_type: Some("codex_oauth".into()),
        ..Default::default()
    });
    ProviderService::add(&state, AppType::Dsh, managed, false).unwrap();
    assert_eq!(ProviderService::current(&state, AppType::Dsh).unwrap(), "");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut config = state.db.get_global_proxy_config().await.unwrap();
    config.listen_address = "127.0.0.1".into();
    config.listen_port = port;
    state.db.update_global_proxy_config(config).await.unwrap();
    crate::mode::controller::enter_with_route(&state, &AppType::Dsh, false, Some("managed"))
        .await
        .unwrap();
    crate::mode::controller::exit(&state, &AppType::Dsh)
        .await
        .unwrap();
    for profile in ["desktop", "web"] {
        let restored = fixture.doc(profile);
        assert_eq!(selected(&restored), selected(&original));
        assert_eq!(native(&restored, &native_id), native(&original, &native_id));
        assert_eq!(restored[1], original[1]);
    }
    assert!(!state.proxy_service.is_running().await);
}

#[test]
fn source11_dsh_legacy_proxy_recognition_preserves_other_cc_prefixed_providers() {
    for (original_id, base, owned) in [
        ("legacy", "http://127.0.0.1:15721/dsh/legacy/v1", true),
        ("id/one", "http://127.0.0.1:15721/dsh/id%2Fone/v1", true),
        ("space id", "http://127.0.0.1:15721/dsh/space%20id/v1", true),
        (
            "literal%2Fid",
            "http://127.0.0.1:15721/dsh/literal%252Fid/v1",
            true,
        ),
        (
            "literal+id",
            "http://127.0.0.1:15721/dsh/literal+id/v1",
            true,
        ),
        ("legacy", "https://api.deepseek.com", false),
        ("legacy", "http://127.0.0.1:15721/v1", false),
        ("legacy", "https://fixture.invalid/dsh/legacy/v1", false),
        ("legacy", "http://127.0.0.1:15721/dsh/another/v1", false),
    ] {
        let id = id_for(original_id);
        let doc = serde_yaml::to_value(json!([{"id":"llm-pi-ai","config":{"providers":{
            (id.clone()): {"baseURL":base}
        }}}]))
        .unwrap();
        assert_eq!(is_cc_proxy_default(&doc, &id), owned, "{base}");
    }
    assert!(is_cc_proxy_default(&serde_yaml::Value::Null, PROXY_ID));
}

#[test]
fn source11_dsh_managed_account_reuses_cc_binding_and_never_becomes_a_direct_credential() {
    let mut provider = api("managed", "fixture");
    provider.settings_config["auth"] = json!({});
    provider.category = Some("official".into());
    provider.meta = Some(ProviderMeta {
        auth_binding: Some(AuthBinding {
            source: AuthBindingSource::ManagedAccount,
            auth_provider: Some("codex_oauth".into()),
            account_id: Some("account-a".into()),
        }),
        ..Default::default()
    });
    let before = serde_json::to_value(&provider).unwrap();
    assert!(projection(&provider, None, &[])
        .unwrap_err()
        .to_string()
        .contains("REQUIRES_ROUTING"));
    let routed = projection(
        &provider,
        Some(("http://127.0.0.1:12345/dsh/v1", "PROXY_MANAGED")),
        &[],
    )
    .unwrap();
    assert_eq!(routed.api, "openai-responses");
    assert_eq!(routed.base, "http://127.0.0.1:12345/dsh/v1");
    assert_eq!(routed.key, "PROXY_MANAGED");
    assert_eq!(serde_json::to_value(&provider).unwrap(), before);
    provider
        .meta
        .as_mut()
        .unwrap()
        .auth_binding
        .as_mut()
        .unwrap()
        .account_id = None;
    assert!(validate(&provider).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn source13_dsh_native_default_does_not_follow_route_changes() {
    let fixture = Fixture::new();
    let state = AppState::new(Arc::new(Database::memory().unwrap()));
    ProviderService::add_without_switching(&state, AppType::Dsh, api("a", "model-a")).unwrap();
    ProviderService::add_without_switching(&state, AppType::Dsh, api("b", "model-b")).unwrap();
    for profile in ["desktop", "web"] {
        assert_eq!(
            selected(&fixture.doc(profile))["provider"],
            "deepseek-official"
        );
        assert_eq!(selected(&fixture.doc(profile))["model"], "deepseek-flash");
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut config = state.db.get_global_proxy_config().await.unwrap();
    config.listen_address = "127.0.0.1".into();
    config.listen_port = port;
    state.db.update_global_proxy_config(config).await.unwrap();
    assert_eq!(native_default_baseline().unwrap(), None);

    let observed = async {
        crate::mode::controller::enter_with_route(&state, &AppType::Dsh, false, Some("a")).await?;
        let a = native_default_baseline().map_err(|e| e.to_string())?;
        let a_model = selected(&fixture.doc("web"))["model"].clone();
        crate::mode::controller::set_route(&state, &AppType::Dsh, "b").await?;
        let b = native_default_baseline().map_err(|e| e.to_string())?;
        let b_model = selected(&fixture.doc("web"))["model"].clone();
        Ok::<_, String>((a, b, a_model, b_model))
    }
    .await;
    // Stop the fixture proxy even when an observation returns an error.
    let exited = crate::mode::controller::exit(&state, &AppType::Dsh).await;
    let (a, b, a_model, b_model) = observed.unwrap();
    exited.unwrap();
    assert_eq!(a, b);
    assert_eq!(a_model, "model-a");
    assert_eq!(b_model, "model-b");
    for profile in ["desktop", "web"] {
        assert_eq!(
            a.as_ref().unwrap()[profile]["provider"],
            "deepseek-official"
        );
        assert_eq!(a.as_ref().unwrap()[profile]["model"], "deepseek-flash");
        assert_eq!(
            selected(&fixture.doc(profile))["provider"],
            "deepseek-official"
        );
        assert_eq!(selected(&fixture.doc(profile))["model"], "deepseek-flash");
    }
    assert_eq!(native_default_baseline().unwrap(), None);
    assert!(!state.proxy_service.is_running().await);
}

#[test]
#[serial]
fn source13_dsh_default_observation_does_not_repair_corrupt_state() {
    let _fixture = Fixture::new();
    let path = DeviceStore::for_device().state_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{invalid").unwrap();
    let before: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(native_default_baseline()
        .unwrap_err()
        .to_string()
        .contains("DSH_NATIVE_DEFAULT_STATE_INVALID"));
    assert_eq!(std::fs::read(&path).unwrap(), b"{invalid");
    let after: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(before, after);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
#[ignore = "native-library integration: requires installed DSH and Node; run with --include-ignored"]
async fn source14_dsh_startup_restores_native_default_from_owned_legacy_route() {
    let fixture = Fixture::new();
    let legacy_id = id_for("legacy-managed");
    for profile in ["desktop", "web"] {
        let dir = fixture.profile(profile).parent().unwrap().to_path_buf();
        let bundle = dir.join("node_modules/@fixture/defaults");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(
            dir.join("package.json"),
            r#"{"dsh":{"profile":{"bundles":["@fixture/defaults"]}}}"#,
        )
        .unwrap();
        std::fs::write(bundle.join("package.json"), r#"{"name":"@fixture/defaults","version":"1.0.0","dsh":{"bundle":{"patch":"defaults.yml"}}}"#).unwrap();
        std::fs::write(bundle.join("defaults.yml"), "- insert:\n  - id: agent-default-model\n    name: '@deepseek-ai/dsh-agent-default-model'\n    config: {provider: deepseek-official, model: deepseek-flash, reasoningEffort: low}\n").unwrap();
        let mut doc = fixture.doc(profile);
        doc[0]["config"]["provider"] = serde_yaml::to_value(&legacy_id).unwrap();
        doc[0]["config"]["model"] = serde_yaml::to_value("old-route-model").unwrap();
        doc.as_sequence_mut().unwrap().push(serde_yaml::to_value(json!({
            "id":"llm-pi-ai", "config":{"providers":{
                (legacy_id.clone()): {"api":"anthropic-messages","baseURL":"http://127.0.0.1:15721/dsh/legacy-managed/v1","apiKey":"PROXY_MANAGED"},
                "unrelated": {"baseURL":"https://fixture.invalid/v1","keep":true}
            }}
        })).unwrap());
        std::fs::write(
            fixture.profile(profile),
            serde_yaml::to_string(&doc).unwrap(),
        )
        .unwrap();
    }
    let state = AppState::new(Arc::new(Database::memory().unwrap()));
    let mut managed = api("legacy-managed", "old-route-model");
    managed.settings_config["auth"] = json!({});
    managed.meta = Some(ProviderMeta {
        auth_binding: Some(AuthBinding {
            source: AuthBindingSource::ManagedAccount,
            auth_provider: Some("codex_oauth".into()),
            account_id: Some("existing-fixture-account".into()),
        }),
        ..Default::default()
    });
    state.db.save_provider("dsh", &managed).unwrap();
    state.db.set_current_provider("dsh", &managed.id).unwrap();
    state::update(&DeviceStore::for_device(), |live| {
        live.apps.entry("dsh".into()).or_default().mode = Some(crate::mode::state::Mode::Direct);
    })
    .unwrap();
    let before: Vec<_> = ["desktop", "web"]
        .map(|p| std::fs::read(fixture.profile(p)).unwrap())
        .into();
    assert_eq!(
        native_default_baseline().unwrap(),
        Some(json!({"desktop":{},"web":{}}))
    );
    assert_eq!(
        before,
        ["desktop", "web"]
            .map(|p| std::fs::read(fixture.profile(p)).unwrap())
            .to_vec()
    );
    assert!(state
        .proxy_service
        .live_has_proxy_placeholder(&AppType::Dsh));
    assert_eq!(ProviderService::current(&state, AppType::Dsh).unwrap(), "");

    crate::mode::controller::startup(&state).await;

    assert_eq!(native_default_baseline().unwrap(), None);
    for profile in ["desktop", "web"] {
        let doc = fixture.doc(profile);
        assert_eq!(selected(&doc)["provider"], "deepseek-official");
        assert_eq!(selected(&doc)["model"], "deepseek-flash");
        assert_eq!(selected(&doc)["reasoningEffort"], "low");
        assert_eq!(selected(&doc)["other"], "keep");
        assert_eq!(doc[1]["config"]["untouched"], true);
        assert!(native(&doc, &legacy_id).is_null());
        assert_eq!(native(&doc, "unrelated")["keep"], true);
    }
    let actual = crate::commands::read_dsh_native_defaults(None).unwrap();
    assert_eq!(actual.len(), 2);
    for row in actual {
        assert_eq!(row.settings_config["provider"], "deepseek-official");
        assert_eq!(row.settings_config["model"], "deepseek-flash");
        assert_eq!(row.settings_config["reasoningEffort"], "low");
    }
    assert!(state
        .db
        .get_provider_by_id(&managed.id, "dsh")
        .unwrap()
        .is_some());
    assert_eq!(ProviderService::current(&state, AppType::Dsh).unwrap(), "");
    assert_eq!(
        std::fs::read_to_string(fixture.root().join(".credentials.yaml")).unwrap(),
        "version: 1\nrefs: {UNCHANGED: fixture-old}\n"
    );
    assert!(!state.proxy_service.is_running().await);
}

#[test]
#[serial]
fn source14_dsh_readonly_legacy_observation_keeps_valid_global_overlay() {
    let fixture = Fixture::new();
    let id = id_for("legacy");
    let profile = fixture.profile("desktop");
    let overlay = format!("- id: agent-default-model\n  config: {{provider: {id}, model: old}}\n- id: llm-pi-ai\n  config:\n    providers:\n      {id}: {{baseURL: 'http://127.0.0.1:15721/dsh/legacy/v1'}}\n");
    std::fs::write(&profile, &overlay).unwrap();
    let global = fixture.root().join("cordis.patch.yml");
    let global_overlay = "- id: unrelated\n  config: {home: keep}\n";
    std::fs::write(&global, global_overlay).unwrap();
    assert_eq!(
        native_default_baseline().unwrap(),
        Some(json!({"desktop":{}}))
    );
    assert_eq!(std::fs::read_to_string(&profile).unwrap(), overlay);
    assert_eq!(std::fs::read_to_string(&global).unwrap(), global_overlay);
    assert!(dsh_patch_paths(&fixture.root())
        .unwrap_err()
        .contains("GLOBAL_OVERLAY"));
}
