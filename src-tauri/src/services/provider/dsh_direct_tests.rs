#![cfg(test)]

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

fn selected(doc: &serde_yaml::Value) -> &serde_yaml::Value {
    &doc.as_sequence()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "agent-default-model")
        .unwrap()["config"]
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

#[test]
#[serial]
fn official_dsh_profiles_ignore_legacy_projections_without_deleting_saved_rows() {
    let fixture = Fixture::new();
    let state = AppState::new(Arc::new(Database::memory().unwrap()));
    let mut synthetic = inventory_provider("web");
    let mut legacy = api("legacy-toml", "legacy-model");
    for provider in [&mut synthetic, &mut legacy] {
        provider.meta = Some(ProviderMeta {
            live_config_managed: Some(true),
            ..Default::default()
        });
        state.db.save_provider("dsh", provider).unwrap();
    }

    let web = fixture.profile("web");
    let native_profile = concat!(
        "- id: llm-pi-ai\n",
        "  config:\n",
        "    providers:\n",
        "      native-fixture: {api: openai-responses, baseURL: 'https://fixture.invalid/v1', apiKeyEnv: NATIVE_KEY, models: [{id: native-model}]}\n",
    );
    std::fs::write(&web, native_profile).unwrap();
    let desktop_before = std::fs::read(fixture.profile("desktop")).unwrap();
    let credentials_path = fixture.root().join(".credentials.yaml");
    let credentials_before = std::fs::read(&credentials_path).unwrap();

    crate::services::provider::initialize_dsh_profiles(&state).unwrap();
    let listed = ProviderService::list(&state, AppType::Dsh).unwrap();
    assert!(listed.contains_key("native-fixture"));
    assert!(!listed.contains_key(&synthetic.id));
    for provider in [synthetic, legacy] {
        let saved = state
            .db
            .get_provider_by_id(&provider.id, "dsh")
            .unwrap()
            .unwrap();
        assert_eq!(saved.settings_config, provider.settings_config);
    }
    assert_eq!(std::fs::read_to_string(web).unwrap(), native_profile);
    assert_eq!(
        std::fs::read(fixture.profile("desktop")).unwrap(),
        desktop_before
    );
    assert_eq!(std::fs::read(credentials_path).unwrap(), credentials_before);
}
