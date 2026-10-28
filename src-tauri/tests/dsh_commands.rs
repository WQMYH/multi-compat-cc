use cc_switch_lib::{AppState, AppType, McpApps, McpServer, McpService, Provider, ProviderService};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{ffi::OsString, fs, path::PathBuf};

#[path = "support.rs"]
mod support;
use support::{create_test_state, reset_test_fs, test_mutex};

struct DshHome {
    directory: tempfile::TempDir,
    previous: Option<OsString>,
}
impl DshHome {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("DSH_HOME");
        std::env::set_var("DSH_HOME", directory.path());
        for profile in ["desktop", "web"] {
            let dir = directory.path().join("profiles").join(profile);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("cordis.patch.yml"), format!(
                "- id: agent-default-model\n  name: '@deepseek-ai/dsh-agent-default-model'\n  config: {{provider: native-{profile}, model: native-{profile}-model, other: keep}}\n- id: native-plugin\n  config: {{provider: native-{profile}, untouched: true}}\n- id: other\n  config: {{value: !!js process.env.USER_VALUE}}\n"
            )).unwrap();
        }
        fs::write(
            directory.path().join(".credentials.yaml"),
            "version: 1\nrefs: {UNCHANGED: fixture-old}\n",
        )
        .unwrap();
        Self {
            directory,
            previous,
        }
    }
    fn profile(&self, profile: &str) -> PathBuf {
        self.directory
            .path()
            .join("profiles")
            .join(profile)
            .join("cordis.patch.yml")
    }
    fn doc(&self, profile: &str) -> serde_yaml::Value {
        serde_yaml::from_slice(&fs::read(self.profile(profile)).unwrap()).unwrap()
    }
    fn snapshot(&self) -> [Vec<u8>; 3] {
        [
            fs::read(self.profile("desktop")).unwrap(),
            fs::read(self.profile("web")).unwrap(),
            fs::read(self.directory.path().join(".credentials.yaml")).unwrap(),
        ]
    }
}
impl Drop for DshHome {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var("DSH_HOME", value),
            None => std::env::remove_var("DSH_HOME"),
        }
    }
}
fn provider(id: &str, model: &str, base: &str) -> Provider {
    // The existing native editor representation, also used by dsh_direct tests.
    Provider::with_id(
        id.into(),
        id.into(),
        json!({
            "auth": {"OPENAI_API_KEY":"fixture-only"},
            "config": format!("model = \"{model}\"\nmodel_provider = \"custom\"\n[model_providers.custom]\nname = \"custom\"\nbase_url = \"{base}\"\nwire_api = \"responses\"\n")
        }),
        None,
    )
}
fn row<'a>(doc: &'a serde_yaml::Value, id: &str) -> &'a serde_yaml::Value {
    doc.as_sequence()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id)
        .unwrap()
}
fn managed_state(state: &AppState) -> Value {
    serde_json::to_value(state.db.get_all_mcp_servers().unwrap()).unwrap()
}
fn server(id: &str, apps: McpApps) -> McpServer {
    McpServer {
        id: id.into(),
        name: id.into(),
        server: json!({"type":"stdio","command":"node","args":["server.js"]}),
        apps,
        description: None,
        homepage: None,
        docs: None,
        tags: vec![],
    }
}

#[test]
fn dsh_provider_lifecycle_preserves_native_plugins_and_reusable_profiles() {
    let _guard = test_mutex()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    reset_test_fs();
    let home = DshHome::new();
    let state = create_test_state().unwrap();
    let original = home.snapshot();
    let native_docs = [home.doc("desktop"), home.doc("web")];
    // Native configurations are observed separately, never imported as BYOK cards.
    assert!(ProviderService::list(&state, AppType::Dsh)
        .unwrap()
        .is_empty());
    // This fixture represents a registered, profile-scoped native default card.
    let native = Provider::with_id(
        "native-current:web".into(),
        "Native Web".into(),
        json!({"nativeRef":true,"profile":"web"}),
        None,
    );
    state.db.save_provider("dsh", &native).unwrap();
    state.db.set_current_provider("dsh", &native.id).unwrap();
    ProviderService::add(
        &state,
        AppType::Dsh,
        provider("draft", "model-a", "https://fixture.invalid/v1"),
        false,
    )
    .unwrap();
    assert_eq!(
        home.snapshot(),
        original,
        "saving a non-current card must not publish it"
    );
    let entries = ProviderService::list(&state, AppType::Dsh).unwrap();
    assert!(entries.contains_key(&native.id) && entries.contains_key("draft"));
    assert_eq!(
        entries["draft"].settings_config["config"],
        provider("draft", "model-a", "https://fixture.invalid/v1").settings_config["config"]
    );

    ProviderService::switch(&state, AppType::Dsh, "draft").unwrap();
    let projected = format!("cc-{:x}", Sha256::digest(b"draft"));
    for profile in ["desktop", "web"] {
        let doc = home.doc(profile);
        assert_eq!(
            row(&doc, "agent-default-model")["config"]["provider"],
            projected
        );
        assert_eq!(
            row(&doc, "agent-default-model")["config"]["model"],
            "model-a"
        );
    }
    assert_eq!(
        ProviderService::current(&state, AppType::Dsh).unwrap(),
        "draft"
    );
    let selected = home.snapshot();
    assert!(ProviderService::delete(&state, AppType::Dsh, "draft").is_err());
    assert_eq!(
        home.snapshot(),
        selected,
        "delete protection must not write profiles"
    );
    assert!(state
        .db
        .get_provider_by_id("draft", "dsh")
        .unwrap()
        .is_some());

    ProviderService::update(
        &state,
        AppType::Dsh,
        None,
        provider("draft", "model-b", "https://updated.invalid/v1"),
    )
    .unwrap();
    for (index, profile) in ["desktop", "web"].iter().enumerate() {
        let doc = home.doc(profile);
        assert_eq!(
            row(&doc, "agent-default-model")["config"]["model"],
            "model-b"
        );
        assert_eq!(
            row(&doc, "llm-pi-ai")["config"]["providers"][&projected]["baseURL"],
            "https://updated.invalid/v1"
        );
        for id in ["native-plugin", "other"] {
            assert_eq!(row(&doc, id), row(&native_docs[index], id));
        }
    }
    // Returning to the registered native default restores each profile's own
    // provider/model; it must not copy Web's native choice into Desktop.
    ProviderService::switch(&state, AppType::Dsh, &native.id).unwrap();
    for profile in ["desktop", "web"] {
        let doc = home.doc(profile);
        assert_eq!(
            row(&doc, "agent-default-model")["config"]["provider"],
            format!("native-{profile}")
        );
        assert_eq!(
            row(&doc, "agent-default-model")["config"]["model"],
            format!("native-{profile}-model")
        );
    }
    let restored = home.snapshot();
    ProviderService::delete(&state, AppType::Dsh, "draft").unwrap();
    assert!(state
        .db
        .get_provider_by_id("draft", "dsh")
        .unwrap()
        .is_none());
    assert_eq!(home.snapshot(), restored);
    let credentials: serde_yaml::Value = serde_yaml::from_slice(&restored[2]).unwrap();
    assert_eq!(credentials["refs"]["UNCHANGED"], "fixture-old");
}

#[test]
fn dsh_mcp_failure_leaves_native_file_and_managed_flags_unchanged() {
    let _guard = test_mutex()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    reset_test_fs();
    let home = DshHome::new();
    let state = create_test_state().unwrap();
    let existing = server(
        "test",
        McpApps {
            claude: true,
            ..Default::default()
        },
    );
    state.db.save_mcp_server(&existing).unwrap();
    let native = home.snapshot();
    let saved = managed_state(&state);
    let mut unsupported = existing.clone();
    unsupported.apps.dsh = true;
    unsupported.server["args"] = json!(["must-not-publish.js"]);
    assert!(McpService::upsert_server(&state, unsupported).is_err());
    let new = server(
        "new-dsh",
        McpApps {
            dsh: true,
            ..Default::default()
        },
    );
    assert!(McpService::upsert_server(&state, new).is_err());
    for enabled in [true, false] {
        assert!(McpService::toggle_app(&state, "test", AppType::Dsh, enabled).is_err());
    }
    assert!(McpService::resync_targets(Some(&["dsh".into()])).is_err());
    assert!(!McpService::resync_app(&state, &AppType::Dsh).ok);
    assert!(McpService::sync_enabled_for_app(&state, &AppType::Dsh).is_err());
    assert_eq!(home.snapshot(), native);
    assert_eq!(managed_state(&state), saved);
    assert!(state.db.get_all_mcp_servers().unwrap()["test"].apps.claude);
}

#[test]
fn dsh_requests_do_not_write_omp_and_legacy_flags_do_not_authorize_native_writes() {
    let _guard = test_mutex()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    reset_test_fs();
    let home = DshHome::new();
    let omp_dir = home.directory.path().join("omp");
    fs::create_dir_all(&omp_dir).unwrap();
    let omp_path = omp_dir.join("mcp.json");
    cc_switch_lib::update_settings(cc_switch_lib::AppSettings {
        omp_config_dir: Some(omp_dir.to_string_lossy().into_owned()),
        ..Default::default()
    })
    .unwrap();
    let state = create_test_state().unwrap();
    let supported = server(
        "shared",
        McpApps {
            omp: true,
            ..Default::default()
        },
    );
    McpService::upsert_server(&state, supported.clone()).unwrap();
    let native = home.snapshot();
    let omp_before = fs::read(&omp_path).unwrap();
    let saved = managed_state(&state);
    let mut rejected = supported.clone();
    rejected.apps.dsh = true;
    rejected.server["args"] = json!(["must-not-publish.js"]);
    assert!(McpService::upsert_server(&state, rejected).is_err());
    assert!(McpService::toggle_app(&state, "shared", AppType::Dsh, true).is_err());
    assert!(McpService::resync_targets(Some(&["omp".into(), "dsh".into()])).is_err());
    assert!(!McpService::resync_app(&state, &AppType::Dsh).ok);
    assert_eq!(home.snapshot(), native);
    assert_eq!(fs::read(&omp_path).unwrap(), omp_before);
    assert_eq!(managed_state(&state), saved);

    // An old hidden true flag remains serializable metadata. Editing a supported
    // target may preserve it, but cannot use it to read or write DSH files.
    let mut legacy = supported;
    legacy.apps.dsh = true;
    state.db.save_mcp_server(&legacy).unwrap();
    legacy.server["args"] = json!(["updated.js"]);
    McpService::upsert_server(&state, legacy.clone()).unwrap();
    assert_eq!(home.snapshot(), native);
    let edited = state.db.get_all_mcp_servers().unwrap()["shared"].clone();
    assert!(edited.apps.dsh && edited.apps.omp);
    let omp_after = fs::read(&omp_path).unwrap();
    let omp: Value = serde_json::from_slice(&omp_after).unwrap();
    assert_eq!(omp["mcpServers"]["shared"]["args"], json!(["updated.js"]));
    let saved_legacy = managed_state(&state);
    let mut cleared = legacy;
    cleared.apps.dsh = false;
    assert!(McpService::upsert_server(&state, cleared).is_err());
    assert!(McpService::toggle_app(&state, "shared", AppType::Dsh, false).is_err());
    assert_eq!(fs::read(&omp_path).unwrap(), omp_after);
    assert_eq!(managed_state(&state), saved_legacy);
    assert_eq!(home.snapshot(), native);
    assert!(McpService::delete_server(&state, "shared").unwrap());
    assert!(state.db.get_all_mcp_servers().unwrap().is_empty());
    assert_eq!(home.snapshot(), native);
    let omp: Value = serde_json::from_slice(&fs::read(&omp_path).unwrap()).unwrap();
    assert!(omp["mcpServers"].get("shared").is_none());
}
