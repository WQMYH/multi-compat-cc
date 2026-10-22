use super::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[cfg(test)]
thread_local! { static TEST_RUNTIME: std::cell::RefCell<Option<Result<PathBuf, String>>> = const { std::cell::RefCell::new(None) }; }

fn record_key(id: &str) -> String { format!("native_qoder_byok_v1:{}", dsh_id(id)) }

fn raw_record(state: &AppState, id: &str) -> Result<(String, Value), String> {
    let raw = state.db.get_setting(&record_key(id)).map_err(|e| e.to_string())?.unwrap_or_else(|| "null".into());
    let parsed = serde_json::from_str(&raw).map_err(|_| "QODER_BYOK_RECORD_INVALID")?;
    Ok((raw, parsed))
}

fn readback_key(id: &str, raw: &str) -> String { format!("{}:readback:{}", record_key(id), dsh_id(raw)) }

fn source_hash(provider: &Provider, base: &str) -> Result<String, String> {
    Ok(dsh_id(&serde_json::to_string(&(provider, base)).map_err(|_| "QODER_PROVIDER_SNAPSHOT_INVALID")?))
}

fn provider_hash(state: &AppState, id: &str) -> Result<String, String> {
    let provider = state.db.get_provider_by_id(id, "qoder").map_err(|e| e.to_string())?.ok_or("NATIVE_PROVIDER_NOT_FOUND")?;
    let config = crate::qoder_gateway::native_config(&provider).map_err(|e| e.to_string())?;
    let base = if config.codex_account_id.is_some() {
        let (address, port) = state.db.get_proxy_client_endpoint_sync();
        native_proxy_base("qoder", &address, port, id)?
    } else { config.base_url };
    source_hash(&provider, &base)
}

pub(super) fn pending(state: &AppState, id: &str) -> Result<bool, String> {
    Ok(raw_record(state, id)?.1["state"] == "pending")
}

pub(super) fn invalidate_readback(state: &AppState, id: &str) -> Result<(), String> {
    let (raw, original) = raw_record(state, id)?;
    if original["state"] != "pending" || original["action"] != "sync" { return Err("QODER_BYOK_RECONCILIATION_NOT_PENDING_SYNC".into()); }
    let key = readback_key(id, &raw);
    if let Some(text) = state.db.get_setting(&key).map_err(|e| e.to_string())? {
        let mut observed: Value = serde_json::from_str(&text).map_err(|_| "QODER_BYOK_OBSERVATION_INVALID")?;
        if observed["binding"]["state"] == "ready" { observed["lastMatchingBinding"] = observed["binding"].take(); }
        observed["binding"] = Value::Null;
        observed["attemptedAt"] = json!(chrono::Utc::now().to_rfc3339());
        state.db.set_setting(&key, &observed.to_string()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(super) fn record(state: &AppState, id: &str) -> Result<Value, String> {
    let (raw, original) = raw_record(state, id)?;
    if original["state"] == "pending" {
        if let Some(text) = state.db.get_setting(&readback_key(id, &raw)).map_err(|e| e.to_string())? {
            let observation: Value = serde_json::from_str(&text).map_err(|_| "QODER_BYOK_OBSERVATION_INVALID")?;
            if observation["originalRecord"] == raw && observation["providerHash"] == provider_hash(state, id)?
                && observation["configRoot"] == json!(config_root("qoder")?) && observation["binding"]["state"] == "ready" {
                return Ok(observation["binding"].clone());
            }
        }
    }
    Ok(original)
}

fn owned_values(record: &Value) -> impl Iterator<Item = &str> {
    record["modelValues"].as_array().into_iter().flatten().filter_map(|m| m["value"].as_str())
}

pub(super) fn current(state: &AppState, id: &str, current: &[Provider]) -> Result<bool, String> {
    let binding = record(state, id)?;
    Ok(binding["state"] == "ready" && !current.is_empty() && current.iter().all(|p| {
        owned_values(&binding).any(|value| p.settings_config["model"] == value)
    }))
}

pub(super) fn applied(state: &AppState, id: &str) -> Result<bool, String> {
    let binding = record(state, id)?;
    if binding["state"] == "pending" { return Err("QODER_BYOK_UNKNOWN_RECONCILIATION_REQUIRED: 上一次原生变更结果未知，未重试".into()); }
    Ok(binding["state"] == "ready")
}

fn worker_at(root: &Path) -> Option<PathBuf> {
    ["@qoder-ai/qoder-cn-agent-sdk", "@qodercn-ai/qodercn-agent-sdk"].iter().find_map(|package| {
        let p = root.join("resources/app.asar.unpacked/node_modules").join(package).join("dist/_worker/qoder-worker-runtime.obf.mjs");
        p.is_file().then_some(p)
    })
}

fn cli_at(directory: &Path) -> Option<PathBuf> {
    // npm's public bin declaration, not the similarly named IDE dispatcher.
    let package = directory.join("node_modules/@qodercn-ai/qoderclicn");
    let text = read_config(&package.join("package.json")).ok()?;
    let manifest: Value = serde_json::from_str(&text).ok()?;
    if manifest["name"] != "@qodercn-ai/qoderclicn" { return None; }
    let bin = manifest["bin"].as_str().or_else(|| manifest["bin"]["qodercn"].as_str())?;
    let p = package.join(bin).canonicalize().ok()?;
    (p.is_file() && p.starts_with(package.canonicalize().ok()?)).then_some(p)
}

fn choose_runtime(paths: &[PathBuf], roots: &[PathBuf]) -> Result<PathBuf, String> {
    let mut runtimes = vec![];
    for directory in paths {
        let native = directory.join(format!("qoderclicn{}", std::env::consts::EXE_SUFFIX))
            .canonicalize().ok().filter(|path| path.is_file());
        for path in [native, cli_at(directory)].into_iter().flatten() {
            if !runtimes.contains(&path) { runtimes.push(path); }
        }
    }
    if runtimes.is_empty() {
        for root in roots {
            if let Some(path) = worker_at(root).and_then(|p| p.canonicalize().ok()) {
                if !runtimes.contains(&path) { runtimes.push(path); }
            }
        }
    }
    match runtimes.as_slice() {
        [runtime] => Ok(runtime.clone()),
        [] => Err("QODER_SDK_RUNTIME_NOT_FOUND: 请通过 CC 应用管理安装官方 Qoder CN CLI".into()),
        _ => Err("QODER_SDK_RUNTIME_AMBIGUOUS: 请通过 CC 应用管理清理重复安装".into()),
    }
}

fn desktop_roots() -> Vec<PathBuf> {
    let mut roots = vec![];
    #[cfg(windows)] {
        use winreg::{RegKey, enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE}};
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            for key in ["Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall", "Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall"] {
                if let Ok(uninstall) = RegKey::predef(hive).open_subkey(key) {
                    for name in uninstall.enum_keys().flatten() {
                        let Ok(app) = uninstall.open_subkey(name) else { continue };
                        if !app.get_value::<String, _>("DisplayName").unwrap_or_default().eq_ignore_ascii_case("Qoder CN") { continue; }
                        if let Ok(path) = app.get_value::<String, _>("InstallLocation") { if !path.is_empty() { roots.push(PathBuf::from(path)); } }
                        if let Ok(icon) = app.get_value::<String, _>("DisplayIcon") {
                            let exe = icon.trim().trim_matches('"').split(",0").next().unwrap_or("").trim_matches('"');
                            if let Some(parent) = Path::new(exe).parent() { roots.push(parent.to_path_buf()); }
                        }
                    }
                }
            }
        }
        for key in ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(path) = std::env::var_os(key) {
                let directory = PathBuf::from(path);
                roots.push(if key == "LOCALAPPDATA" { directory.join("Programs/Qoder CN") } else { directory.join("Qoder CN") });
            }
        }
    }
    #[cfg(target_os = "macos")] roots.push(PathBuf::from("/Applications/Qoder CN.app/Contents"));
    roots
}

fn cli_paths() -> Vec<PathBuf> {
    let mut paths = super::super::misc::build_tool_search_paths("qodercn");
    if let Some(home) = dirs::home_dir() { paths.push(home.join(".qoder-cn/bin/qoderclicn")); }
    paths
}

pub(in crate::commands) fn cli_runtime() -> Result<PathBuf, String> {
    #[cfg(test)]
    if let Some(path) = TEST_RUNTIME.with(|p| p.borrow().clone()) { return path; }
    // The SDK worker can serve BYOK RPCs but is not an interactive login CLI.
    choose_runtime(&cli_paths(), &[])
}

fn request(state: &AppState, id: &str, action: &str, config: Value) -> Result<Value, String> {
    #[cfg(test)] let override_path = TEST_RUNTIME.with(|p| p.borrow().clone());
    #[cfg(not(test))] let override_path: Option<Result<PathBuf, String>> = None;
    let runtime = match override_path {
        Some(path) => path?,
        None => choose_runtime(&cli_paths(), &desktop_roots())?,
    };
    let prior = record(state, id)?;
    Ok(json!({"operation":"byok", "action":action, "cliPath":runtime,
        "source":{"type":"cli", "configDir":config_root("qoder")?},
        "expectedUserId":prior.get("userId").or_else(|| prior.pointer("/prior/userId")), "providerId":dsh_id(id), "config":config}))
}

fn sdk_config(provider: &Provider, base: &str, key: &str) -> Result<Value, String> {
    let config = crate::qoder_gateway::native_config(provider).map_err(|e| e.to_string())?;
    let protocol = match config.api_format {
        crate::qoder_gateway::ApiFormat::OpenaiChat => "openai",
        crate::qoder_gateway::ApiFormat::OpenaiResponses => "openai-responses",
        crate::qoder_gateway::ApiFormat::AnthropicMessages => "anthropic",
    };
    let models: Result<Vec<Value>, String> = config.model_details.iter().map(|m| {
        let mut row = json!({"model":m["id"],"displayName":m.get("name").unwrap_or(&m["id"]),
            "capabilities":{"vision":m["input"].as_array().is_some_and(|a| a.iter().any(|v| v == "image"))}});
        if let Some(context) = m.get("contextWindow") { row["contextWindow"] = context.clone(); }
        if let Some(efforts) = m["reasoningEfforts"].as_object() {
            let levels: Vec<&str> = efforts.keys().filter(|s| s.as_str() != "off").map(String::as_str).collect();
            if levels.iter().any(|s| !matches!(*s, "low" | "medium" | "high" | "xhigh" | "max")) { return Err("QODER_BYOK_REASONING_LEVEL_UNSUPPORTED".into()); }
            if !levels.is_empty() { row["capabilities"]["thinking"] = json!({"modes":["enabled"],"supportsEffort":true,"supportedEffortLevels":levels}); }
        }
        Ok(row)
    }).collect();
    Ok(json!({"providerId":dsh_id(&provider.id),"displayName":provider.name,"type":"openai-compatible",
        "protocol":protocol,"authType":if protocol == "anthropic" {"api-key"} else {"bearer"},
        "baseUrl":base,"apiKey":key,"model":config.models[0],"models":models?}))
}

fn perform(state: &AppState, id: &str, action: &str, source_provider: Option<&Provider>, input: Value,
    run: impl FnOnce(Value) -> Result<Value, String>, finish: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    let read_only = action == "reconcile";
    let (original_raw, original) = raw_record(state, id)?;
    let prior = if read_only { original } else { record(state, id)? };
    if read_only {
        if prior["state"] != "pending" || prior["action"] != "sync" { return Err("QODER_BYOK_RECONCILIATION_NOT_PENDING_SYNC".into()); }
    } else if prior["state"] == "pending" { return Err("QODER_BYOK_UNKNOWN_RECONCILIATION_REQUIRED".into()); }
    let pending = json!({"state":"pending","action":action,"prior":prior});
    let expected_model = input.pointer("/config/model").cloned();
    let expected_models: Vec<Value> = input.pointer("/config/models").and_then(Value::as_array).into_iter().flatten().map(|m| m["model"].clone()).collect();
    let save = |v: &Value| state.db.set_setting(&record_key(id), &v.to_string()).map_err(|e| e.to_string());
    let observation_key = readback_key(id, &original_raw);
    let mut observation = if read_only {
        let previous: Value = state.db.get_setting(&observation_key).map_err(|e| e.to_string())?
            .map(|text| serde_json::from_str(&text).map_err(|_| "QODER_BYOK_OBSERVATION_INVALID".to_string())).transpose()?.unwrap_or(Value::Null);
        let prepared_hash = source_hash(source_provider.ok_or("QODER_BYOK_RECONCILIATION_SOURCE_MISSING")?,
            input.pointer("/config/baseUrl").and_then(Value::as_str).ok_or("QODER_BYOK_RECONCILIATION_SOURCE_MISSING")?)?;
        let prepared_root = input.pointer("/source/configDir").filter(|value| value.is_string()).ok_or("QODER_BYOK_RECONCILIATION_SOURCE_MISSING")?;
        Some(json!({"originalRecord":original_raw, "providerHash":prepared_hash, "configRoot":prepared_root,
            "observedAt":chrono::Utc::now().to_rfc3339(), "binding":null,
            "lastMatchingBinding":if previous["binding"]["state"] == "ready" { &previous["binding"] } else { &previous["lastMatchingBinding"] }}))
    } else { None };
    if let Some(observed) = observation.as_ref() {
        state.db.set_setting(&observation_key, &observed.to_string()).map_err(|e| e.to_string())?;
    } else { save(&pending)?; }
    let check_source = || -> Result<(), String> {
        if let Some(observed) = observation.as_ref() {
            if raw_record(state, id)?.0 != original_raw || observed["providerHash"] != provider_hash(state, id)? || observed["configRoot"] != json!(config_root("qoder")?) {
                return Err("QODER_BYOK_RECONCILIATION_SOURCE_CHANGED".into());
            }
        }
        Ok(())
    };
    check_source()?;
    let reply = match run(input) { Ok(reply) => reply, Err(code) => return Err(format!("QODER_BYOK_UNKNOWN_RECONCILIATION_REQUIRED:{code}")) };
    if reply["ok"] != true {
        if !read_only && reply["nativeEffect"] == "none" { save(&prior)?; }
        let code = reply["code"].as_str().filter(|s| !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')).unwrap_or("QODER_BYOK_FAILED");
        return Err(format!("{code}: {}", if reply["nativeEffect"] == "none" {"原生配置未改动"} else {"结果未知，保留记录，不自动重试"}));
    }
    let result = &reply["byok"];
    if result["providerId"] != dsh_id(id) || !result["userId"].as_str().is_some_and(|s| !s.is_empty() && s.len() <= 256) ||
        (prior["userId"].is_string() && prior["userId"] != result["userId"]) ||
        (action == "delete" && result["deleted"] != true) ||
        (action != "delete" && (expected_model.as_ref() != result.get("defaultModelId") || result["scope"] != "byok_provider_default" || result["desktopDefaultChanged"] != false ||
            result["catalogSource"] != "native_byok_configs" ||
            !result["nativeModelIds"].as_array().is_some_and(|ids| ids.len() == expected_models.len() && ids.iter().all(|m| expected_models.contains(m)) && expected_models.iter().all(|m| ids.contains(m))) ||
            !result["modelValues"].as_array().is_some_and(|a| (a.is_empty() || a.len() == expected_models.len()) && a.len() <= 200 && a.iter().all(|m| expected_models.contains(&m["model"]) && m["value"].as_str().is_some_and(|s| !s.is_empty() && s.len() <= 2048))))) ||
        (read_only && (result["readOnlyReconciled"] != true || result["historicalRequestConfirmed"] != false)) {
        return Err("QODER_BYOK_REPLY_INVALID_RECONCILIATION_REQUIRED".into());
    }
    check_source()?;
    finish().map_err(|_| "QODER_BYOK_NATIVE_APPLIED_CC_SAVE_FAILED_RECONCILIATION_REQUIRED")?;
    let mut binding = result.clone();
    binding["state"] = json!(if action == "delete" {"deleted"} else {"ready"});
    if let Some(observed) = observation.as_mut() {
        observed["binding"] = binding;
        state.db.set_setting(&observation_key, &observed.to_string()).map_err(|e| e.to_string())
    } else { save(&binding) }
}

fn run(input: Value) -> Result<Value, String> {
    let output = super::super::qoder_auth::run_sdk_host(input, None, None, Duration::from_secs(125))?;
    serde_json::from_slice(&output.stdout).map_err(|_| "QODER_HOST_REPLY_INVALID".into())
}

pub(super) fn sync(state: &AppState, provider: &Provider, base: &str, key: &str,
    finish: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    let config = sdk_config(provider, base, key)?;
    if applied(state, &provider.id)? {
        let prior = record(state, &provider.id)?;
        if prior["modelValues"].as_array().is_none_or(Vec::is_empty) {
            let known_ids = prior["nativeModelIds"].as_array().filter(|ids| !ids.is_empty()).ok_or("QODER_BYOK_SELECTION_UNKNOWN_CANNOT_REMOVE_MODELS")?;
            if known_ids.iter().any(|id| !config["models"].as_array().unwrap().iter().any(|m| m["model"] == *id)) {
                return Err("QODER_BYOK_SELECTION_UNKNOWN_CANNOT_REMOVE_MODELS".into());
            }
        }
        for selected in current_at("qoder", &config_root("qoder")?)? {
            if let Some(active) = prior["modelValues"].as_array().into_iter().flatten().find(|m| m["value"] == selected.settings_config["model"]) {
                if !config["models"].as_array().unwrap().iter().any(|m| m["model"] == active["model"]) {
                    return Err("QODER_CURRENT_NATIVE_MODEL_CANNOT_REMOVE".into());
                }
            }
        }
    }
    let input = request(state, &provider.id, "sync", config)?;
    perform(state, &provider.id, "sync", None, input, run, finish)
}

pub(super) fn remove(state: &AppState, id: &str) -> Result<bool, String> {
    let prior = record(state, id)?;
    if applied(state, id)? {
        if prior["modelValues"].as_array().is_none_or(Vec::is_empty) { return Err("QODER_BYOK_SELECTION_UNKNOWN_CANNOT_DELETE".into()); }
        let observed = current_at("qoder", &config_root("qoder")?)?;
        if observed.iter().any(|p| owned_values(&prior).any(|v| p.settings_config["model"] == v)) {
            return Err("QODER_CURRENT_NATIVE_MODEL_CANNOT_DELETE".into());
        }
        let input = request(state, id, "delete", Value::Null)?;
        perform(state, id, "delete", None, input, run, || state.db.delete_provider("qoder", id).map(|_| ()).map_err(|e| e.to_string()))?;
    } else { state.db.delete_provider("qoder", id).map_err(|e| e.to_string())?; }
    Ok(true)
}

pub(super) fn reconcile(state: &AppState, provider: &Provider, base: &str) -> Result<(), String> {
    let mut expected = sdk_config(provider, base, "")?;
    expected.as_object_mut().unwrap().remove("apiKey");
    let input = request(state, &provider.id, "reconcile", expected)?;
    perform(state, &provider.id, "reconcile", Some(provider), input, run, || Ok(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Environment(Vec<(&'static str, Option<std::ffi::OsString>)>);
    impl Environment {
        fn isolated(root: &Path, runtime: &Path) -> Self {
            let before = Self(["CC_SWITCH_TEST_HOME", "QODERCN_CONFIG_DIR", "QODER_BYOK_FIXTURE_MODE"].into_iter().map(|key| (key, std::env::var_os(key))).collect());
            std::env::set_var("CC_SWITCH_TEST_HOME", root.join("cc"));
            std::env::set_var("QODERCN_CONFIG_DIR", root);
            std::env::remove_var("QODER_BYOK_FIXTURE_MODE");
            TEST_RUNTIME.with(|p| *p.borrow_mut() = Some(Ok(runtime.to_path_buf())));
            before
        }
    }
    impl Drop for Environment {
        fn drop(&mut self) {
            for (key, value) in &self.0 { if let Some(value) = value { std::env::set_var(key, value); } else { std::env::remove_var(key); } }
            TEST_RUNTIME.with(|p| *p.borrow_mut() = None);
        }
    }
    fn provider(id: &str, model: &str) -> Provider {
        Provider::with_id(id.into(), "Fixture CC provider".into(), json!({"auth":{"OPENAI_API_KEY":"synthetic-upstream-key"},
            "config":format!("model = \"{model}\"\nmodel_provider = \"custom\"\n[model_providers.custom]\nbase_url = \"https://fixture.invalid/v1\"\nwire_api = \"responses\"\n"),
            "modelCatalog":{"models":[{"model":"model-A","contextWindow":120000},{"model":"model-B","inputModalities":["text","image"]}]}}), None)
    }
    #[test]
    fn source23_native_login_reuses_byok_runtime_and_config_without_saving_a_profile() {
        let home = tempfile::tempdir().unwrap();
        let cli = home.path().join(format!("qoderclicn{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&cli, "synthetic CLI; never executed").unwrap();
        let _environment = Environment::isolated(home.path(), &cli);
        let state = AppState::new(std::sync::Arc::new(crate::database::Database::memory().unwrap()));
        state.db.set_setting("qoder_cn_auth_profiles_v1", "malformed sentinel; must not be read or rewritten").unwrap();
        let login = super::super::super::qoder_auth::account_cli_command(&state.db, None).unwrap();
        assert!(login.contains(if cfg!(windows) { "-EncodedCommand" } else { "QODERCN_CONFIG_DIR=" }));
        let byok = request(&state, "fixture", "sync", json!({})).unwrap();
        assert_eq!(byok["cliPath"], json!(cli));
        assert_eq!(byok["source"]["configDir"], json!(home.path()));
        assert_eq!(state.db.get_setting("qoder_cn_auth_profiles_v1").unwrap().as_deref(), Some("malformed sentinel; must not be read or rewritten"));
        assert_eq!(super::super::super::qoder_auth::account_cli_command(&state.db, Some("missing")).unwrap_err(), "QODER_PROFILE_STORE_INVALID");
        assert!(!home.path().join("settings.json").exists());
    }
    #[test]
    #[cfg(target_os = "windows")]
    fn source22_qoder_runtime_discovers_official_native_cli_without_npm() {
        let home = tempfile::tempdir().unwrap();
        let directory = home.path().join(".qoder-cn/bin/qoderclicn");
        std::fs::create_dir_all(&directory).unwrap();
        let executable = directory.join("qoderclicn.exe");
        std::fs::write(&executable, b"MZ-fixture-not-executed").unwrap();
        assert_eq!(choose_runtime(&[directory.clone(), directory], &[]).unwrap(), executable.canonicalize().unwrap());
    }
    #[test]
    fn source8_runtime_discovery_ignores_ide_dispatchers_and_has_no_version_pin() {
        let home = tempfile::tempdir().unwrap();
        let ide = home.path().join("ide/bin"); std::fs::create_dir_all(&ide).unwrap();
        std::fs::write(ide.join("qodercn.cmd"), "not a native CLI").unwrap();
        assert!(choose_runtime(&[ide.clone()], &[]).unwrap_err().contains("NOT_FOUND"));
        let package = home.path().join("npm/node_modules/@qodercn-ai/qoderclicn"); std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("package.json"), r#"{"name":"@qodercn-ai/qoderclicn","bin":{"qodercn":"cli.mjs"}}"#).unwrap();
        std::fs::write(package.join("cli.mjs"), "// fixture; never executed").unwrap();
        assert_eq!(choose_runtime(&[ide, home.path().join("npm")], &[]).unwrap(), package.join("cli.mjs").canonicalize().unwrap());
        let root = home.path().join("native-app");
        let worker = root.join("resources/app.asar.unpacked/node_modules/@qoder-ai/qoder-cn-agent-sdk/dist/_worker/qoder-worker-runtime.obf.mjs");
        std::fs::create_dir_all(worker.parent().unwrap()).unwrap(); std::fs::write(&worker, "// fixture").unwrap();
        assert_eq!(choose_runtime(&[], &[root.clone(), root]).unwrap(), worker.canonicalize().unwrap());
        assert_eq!(native_proxy_base("qoder", "127.0.0.1", 12345, "id/one").unwrap(), "http://127.0.0.1:12345/qoder/id%2Fone/v1");

    }
    #[test]
    fn source20_qoder_runtime_discovery_rejects_distinct_clis_but_not_repeated_paths() {
        let home = tempfile::tempdir().unwrap();
        let roots: Vec<_> = ["first", "second"].iter().map(|name| home.path().join(name)).collect();
        for root in &roots {
            let package = root.join("node_modules/@qodercn-ai/qoderclicn");
            std::fs::create_dir_all(&package).unwrap();
            std::fs::write(package.join("package.json"), r#"{"name":"@qodercn-ai/qoderclicn","bin":{"qodercn":"cli.mjs"}}"#).unwrap();
            std::fs::write(package.join("cli.mjs"), "// fixture; never executed").unwrap();
        }
        let first = roots[0].join("node_modules/@qodercn-ai/qoderclicn/cli.mjs").canonicalize().unwrap();
        assert_eq!(choose_runtime(&[roots[0].clone(), roots[0].clone()], &[]).unwrap(), first);
        assert!(choose_runtime(&roots, &[]).unwrap_err().starts_with("QODER_SDK_RUNTIME_AMBIGUOUS"));
    }
    #[test]
    fn source20_qoder_missing_default_keeps_saved_catalog_without_inventing_current() {
        let home = tempfile::tempdir().unwrap();
        let _environment = Environment::isolated(home.path(), &home.path().join("unused-fixture.mjs"));
        let state = AppState::new(std::sync::Arc::new(crate::database::Database::memory().unwrap()));
        let saved = provider("saved", "model-A");
        save_managed(&state, "qoder", &saved, false).unwrap();
        let rows = catalog(&state, "qoder").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows["saved"].settings_config, saved.settings_config);
        assert_eq!(current_id(&state, "qoder").unwrap(), "");
        assert!(state.db.get_current_provider("qoder").unwrap().is_none());
        let settings = home.path().join("settings.json");
        assert!(!settings.exists(), "listing must not initialize the native profile");
        std::fs::write(&settings, "{malformed").unwrap();
        assert_eq!(catalog(&state, "qoder").unwrap_err(), "QODER_CONFIG_INVALID");
        assert_eq!(current_id(&state, "qoder").unwrap_err(), "QODER_CONFIG_INVALID");
        assert_eq!(read_config(&settings).unwrap(), "{malformed");
        assert!(state.db.get_all_providers("qoder").unwrap().contains_key("saved"));
    }
    #[test]
    fn source25_observation_rejects_provider_and_proxy_endpoint_drift() {
        for drift in ["unchanged", "provider", "endpoint"] {
            let home = tempfile::tempdir().unwrap();
            let runtime = home.path().join("unused-fixture.mjs");
            let _environment = Environment::isolated(home.path(), &runtime);
            let state = AppState::new(std::sync::Arc::new(crate::database::Database::memory().unwrap()));
            let mut saved = provider("snapshot", "model-A");
            saved.meta = Some(crate::provider::ProviderMeta {
                auth_binding: Some(crate::provider::AuthBinding {
                    source: crate::provider::AuthBindingSource::ManagedAccount,
                    auth_provider: Some("codex_oauth".into()), account_id: Some("synthetic-account".into()),
                }), ..Default::default()
            });
            state.db.save_provider("qoder", &saved).unwrap();
            let original = r#"{"state":"pending","action":"sync","prior":null}"#;
            state.db.set_setting(&record_key(&saved.id), original).unwrap();
            let (address, port) = state.db.get_proxy_client_endpoint_sync();
            let base = native_proxy_base("qoder", &address, port, &saved.id).unwrap();
            let config = sdk_config(&saved, &base, "").unwrap();
            let input = json!({"source":{"configDir":home.path()},"config":config});
            let result = perform(&state, &saved.id, "reconcile", Some(&saved), input, |_| {
                if drift == "provider" {
                    let mut changed = saved.clone(); changed.name = "Changed during read".into();
                    state.db.save_provider("qoder", &changed).unwrap();
                } else if drift == "endpoint" {
                    let mut proxy = futures::executor::block_on(state.proxy_service.get_config()).unwrap();
                    proxy.listen_port += 1;
                    futures::executor::block_on(state.db.update_proxy_config(proxy)).unwrap();
                }
                Ok(json!({"ok":true,"byok":{"providerId":dsh_id(&saved.id),"userId":"fixture-user","defaultModelId":"model-A",
                    "nativeModelIds":["model-A","model-B"],"modelValues":[],"catalogSource":"native_byok_configs",
                    "scope":"byok_provider_default","desktopDefaultChanged":false,"readOnlyReconciled":true,"historicalRequestConfirmed":false}}))
            }, || Ok(()));
            assert_eq!(state.db.get_setting(&record_key(&saved.id)).unwrap().as_deref(), Some(original));
            if drift == "unchanged" { result.unwrap(); assert!(applied(&state, &saved.id).unwrap()); }
            else { assert_eq!(result.unwrap_err(), "QODER_BYOK_RECONCILIATION_SOURCE_CHANGED"); assert!(applied(&state, &saved.id).is_err()); }
        }
    }
    #[test]
    fn source25_preparation_failure_invalidates_the_previous_matching_observation() {
        let home = tempfile::tempdir().unwrap();
        let _environment = Environment::isolated(home.path(), &home.path().join("unused-fixture.mjs"));
        let state = AppState::new(std::sync::Arc::new(crate::database::Database::memory().unwrap()));
        let saved = provider("preparation-failure", "model-A");
        state.db.save_provider("qoder", &saved).unwrap();
        let saved = state.db.get_provider_by_id(&saved.id, "qoder").unwrap().unwrap();
        let original = r#"{"state":"pending","action":"sync","prior":null,"keep":"original"}"#;
        state.db.set_setting(&record_key(&saved.id), original).unwrap();
        let input = json!({"source":{"configDir":home.path()},"config":sdk_config(&saved,"https://fixture.invalid/v1", "").unwrap()});
        perform(&state, &saved.id, "reconcile", Some(&saved), input, |_| Ok(json!({"ok":true,"byok":{
            "providerId":dsh_id(&saved.id),"userId":"fixture-user","defaultModelId":"model-A",
            "nativeModelIds":["model-A","model-B"],"modelValues":[],"catalogSource":"native_byok_configs",
            "scope":"byok_provider_default","desktopDefaultChanged":false,"readOnlyReconciled":true,"historicalRequestConfirmed":false
        }})), || Ok(())).unwrap();
        assert!(applied(&state,&saved.id).unwrap());
        TEST_RUNTIME.with(|p| *p.borrow_mut() = Some(Err("QODER_SDK_RUNTIME_NOT_FOUND".into())));
        assert!(futures::executor::block_on(apply(&state,"qoder",&saved.id)).unwrap_err().starts_with("QODER_SDK_RUNTIME_NOT_FOUND"));
        assert!(applied(&state,&saved.id).is_err(), "a failed fresh preparation must not keep the old ready binding effective");
        let observation: Value = serde_json::from_str(&state.db.get_setting(&readback_key(&saved.id,original)).unwrap().unwrap()).unwrap();
        assert!(observation["binding"].is_null());
        assert_eq!(observation["lastMatchingBinding"]["state"], "ready");
        assert_eq!(state.db.get_setting(&record_key(&saved.id)).unwrap().as_deref(),Some(original));
        assert!(!home.path().join("calls.jsonl").exists());
        assert!(!futures::executor::block_on(state.proxy_service.get_status()).unwrap().running);
        assert!(state.db.get_setting("qoder_gateway_api_key_v1").unwrap().is_none());
    }
    #[test]
    fn source25_prepared_source_cannot_be_certified_with_a_later_database_fingerprint() {
        for drift in ["endpoint", "provider", "config-root"] {
            let home = tempfile::tempdir().unwrap();
            let _environment = Environment::isolated(home.path(),&home.path().join("unused-fixture.mjs"));
            let state = AppState::new(std::sync::Arc::new(crate::database::Database::memory().unwrap()));
            let mut saved = provider("prepared-source","model-A");
            saved.meta = Some(crate::provider::ProviderMeta { auth_binding:Some(crate::provider::AuthBinding {
                source:crate::provider::AuthBindingSource::ManagedAccount,auth_provider:Some("codex_oauth".into()),account_id:Some("synthetic-account".into())
            }),..Default::default() });
            state.db.save_provider("qoder",&saved).unwrap();
            let original = r#"{"state":"pending","action":"sync","prior":null}"#;
            state.db.set_setting(&record_key(&saved.id),original).unwrap();
            let proxy = futures::executor::block_on(state.proxy_service.get_config()).unwrap();
            let base = native_proxy_base("qoder",&proxy.listen_address,proxy.listen_port,&saved.id).unwrap();
            let input = json!({"source":{"configDir":home.path()},"config":sdk_config(&saved,&base, "").unwrap()});
            if drift == "endpoint" {
                let mut changed = proxy; changed.listen_port += 1;
                futures::executor::block_on(state.db.update_proxy_config(changed)).unwrap();
            } else if drift == "provider" { let mut changed=saved.clone(); changed.name="Changed after preparation".into(); state.db.save_provider("qoder",&changed).unwrap(); }
            else { std::env::set_var("QODERCN_CONFIG_DIR",home.path().join("changed-after-preparation")); }
            let native_called=std::cell::Cell::new(false);
            let result=perform(&state,&saved.id,"reconcile",Some(&saved),input, |_| { native_called.set(true); Ok(json!({"ok":true,"byok":{
                "providerId":dsh_id(&saved.id),"userId":"fixture-user","defaultModelId":"model-A",
                "nativeModelIds":["model-A","model-B"],"modelValues":[],"catalogSource":"native_byok_configs",
                "scope":"byok_provider_default","desktopDefaultChanged":false,"readOnlyReconciled":true,"historicalRequestConfirmed":false
            }})) },|| Ok(()));
            assert_eq!(result.unwrap_err(),"QODER_BYOK_RECONCILIATION_SOURCE_CHANGED");
            assert!(!native_called.get(),"preparation drift must stop before native RPC");
            assert!(applied(&state,&saved.id).is_err());
            assert_eq!(state.db.get_setting(&record_key(&saved.id)).unwrap().as_deref(),Some(original));
        }
    }
    #[test]
    fn source25_pending_reconciliation_preserves_original_and_unknown_selection_guards() {
        for managed in [false, true] {
            let home = tempfile::tempdir().unwrap();
            let fixture = home.path().join("byok-fixture.mjs");
            std::fs::write(&fixture, include_str!("../../../../compat/qoder-cn/sdk-contract/byok-fixture.mjs")).unwrap();
            let _environment = Environment::isolated(home.path(), &fixture);
            std::env::set_var("QODER_BYOK_FIXTURE_MODE", "system-only");
            let state = AppState::new(std::sync::Arc::new(crate::database::Database::memory().unwrap()));
            let mut saved = provider("pending", "model-A");
            if managed {
                saved.meta = Some(crate::provider::ProviderMeta {
                    auth_binding: Some(crate::provider::AuthBinding {
                        source: crate::provider::AuthBindingSource::ManagedAccount,
                        auth_provider: Some("codex_oauth".into()),
                        account_id: Some("nonexistent-synthetic-account".into()),
                    }), ..Default::default()
                });
            }
            save_managed(&state, "qoder", &saved, false).unwrap();
            let base = if managed {
                let proxy = futures::executor::block_on(state.proxy_service.get_config()).unwrap();
                native_proxy_base("qoder", &proxy.listen_address, proxy.listen_port, &saved.id).unwrap()
            } else { "https://fixture.invalid/v1".into() };
            let config = sdk_config(&saved, &base, "synthetic-upstream-key").unwrap();
            let native = json!({"globalModel":"original-desktop-model", "providers":{dsh_id(&saved.id):config}}).to_string();
            std::fs::write(home.path().join("native.json"), &native).unwrap();
            std::fs::write(home.path().join("settings.json"), r#"{"model":{"name":"original-desktop-model"},"keep":true}"#).unwrap();
            let pending = r#"{"state":"pending","action":"sync","prior":null,"retained":"original"}"#;
            state.db.set_setting(&record_key(&saved.id), pending).unwrap();
            let result = futures::executor::block_on(apply(&state, "qoder", &saved.id)).unwrap();
            assert_eq!(result.warnings, vec!["QODER_BYOK_RECONCILED_SELECTION_UNKNOWN"]);
            assert_eq!(state.db.get_setting(&record_key(&saved.id)).unwrap().as_deref(), Some(pending));
            let observed = record(&state, &saved.id).unwrap();
            assert_eq!(observed["readOnlyReconciled"], true);
            assert_eq!(observed["nativeModelIds"], json!(["model-A", "model-B"]));
            assert_eq!(observed["modelValues"], json!([]));
            assert!(applied(&state, &saved.id).unwrap(), "configured does not mean unselected");
            assert!(!current(&state, &saved.id, &current_at("qoder", home.path()).unwrap()).unwrap());
            let calls = read_config(&home.path().join("calls.jsonl")).unwrap();
            for line in calls.lines() {
                let frame: Value = serde_json::from_str(line).unwrap();
                assert!(matches!(frame["method"].as_str().unwrap(), "daemon.initialize" | "daemon.account_info" | "daemon.list_byok_configs" | "daemon.get_models" | "daemon.shutdown"));
            }
            assert!(!futures::executor::block_on(state.proxy_service.get_status()).unwrap().running);
            assert!(state.db.get_setting("qoder_gateway_api_key_v1").unwrap().is_none());
            assert!(remove(&state, &saved.id).unwrap_err().contains("SELECTION_UNKNOWN"));
            let mut shrunk = saved.clone();
            shrunk.settings_config["modelCatalog"]["models"] = json!([{"model":"model-A"}]);
            assert!(sync(&state, &shrunk, &base, "synthetic-upstream-key", || Ok(())).unwrap_err().contains("SELECTION_UNKNOWN"));
            assert_eq!(read_config(&home.path().join("calls.jsonl")).unwrap(), calls);
            assert_eq!(read_config(&home.path().join("native.json")).unwrap(), native);
        }
    }
    #[test]
    fn source8_shared_cc_lifecycle_reaches_real_sdk_with_native_fixture_and_never_changes_global_default() {
        let home = tempfile::tempdir().unwrap();
        let fixture = home.path().join("byok-fixture.mjs");
        std::fs::write(&fixture, include_str!("../../../../compat/qoder-cn/sdk-contract/byok-fixture.mjs")).unwrap();
        let _environment = Environment::isolated(home.path(), &fixture);
        let settings = home.path().join("settings.json");
        std::fs::write(&settings, r#"{"model":{"name":"original-desktop-model"},"unrelated":{"keep":true}}"#).unwrap();
        let before = read_config(&settings).unwrap();
        let state = AppState::new(std::sync::Arc::new(crate::database::Database::memory().unwrap()));
        let mut p = provider("first", "model-A");
        save_managed(&state, "qoder", &p, false).unwrap();
        let result = futures::executor::block_on(apply(&state, "qoder", &p.id)).unwrap();
        assert!(result.warnings[0].starts_with("QODER_BYOK_PROVIDER_DEFAULT_SYNCED:"));
        assert_eq!(record(&state, &p.id).unwrap()["defaultModelId"], "model-A");
        assert_eq!(read_config(&settings).unwrap(), before);
        assert_eq!(current_id(&state, "qoder").unwrap(), "native-current:current");
        p = provider("first", "model-B");
        save_managed(&state, "qoder", &p, true).unwrap();
        let binding = record(&state, &p.id).unwrap(); assert_eq!(binding["defaultModelId"], "model-B");
        assert_eq!(read_config(&settings).unwrap(), before);
        let active = binding["modelValues"].as_array().unwrap().iter().find(|m| m["model"] == "model-B").unwrap()["value"].clone();
        std::fs::write(&settings, json!({"model":{"name":active}}).to_string()).unwrap();
        assert_eq!(current_id(&state, "qoder").unwrap(), "first");
        assert_eq!(delete_managed(&state, "qoder", "first").unwrap_err(), "QODER_CURRENT_NATIVE_MODEL_CANNOT_DELETE");
        std::fs::write(&settings, &before).unwrap();
        delete_managed(&state, "qoder", "first").unwrap();
        let native: Value = serde_json::from_str(&read_config(&home.path().join("native.json")).unwrap()).unwrap();
        assert_eq!(native["globalModel"], "original-desktop-model");
        assert_eq!(native["providers"].as_object().unwrap().len(), 1);
        assert!(native["providers"].get("foreign").is_some());
        let unknown = provider("unknown", "model-A"); save_managed(&state, "qoder", &unknown, false).unwrap();
        std::env::set_var("QODER_BYOK_FIXTURE_MODE", "ack-lost");
        assert!(futures::executor::block_on(apply(&state, "qoder", &unknown.id)).is_err());
        assert_eq!(record(&state, &unknown.id).unwrap()["state"], "pending");
        let calls = read_config(&home.path().join("calls.jsonl")).unwrap();
        assert!(save_managed(&state, "qoder", &unknown, true).is_err());
        assert!(delete_managed(&state, "qoder", &unknown.id).is_err());
        assert_eq!(read_config(&home.path().join("calls.jsonl")).unwrap(), calls, "unknown mutation must not be replayed by edit or delete");
        let original_pending=state.db.get_setting(&record_key(&unknown.id)).unwrap();
        let readback=futures::executor::block_on(apply(&state,"qoder",&unknown.id)).unwrap();
        assert_eq!(readback.warnings,vec!["QODER_BYOK_RECONCILED"]);
        assert_eq!(state.db.get_setting(&record_key(&unknown.id)).unwrap(),original_pending);
        let after=read_config(&home.path().join("calls.jsonl")).unwrap();
        assert!(after.starts_with(&calls));
        for line in after[calls.len()..].lines() {
            let frame:Value=serde_json::from_str(line).unwrap();
            assert!(matches!(frame["method"].as_str().unwrap(),"daemon.initialize"|"daemon.account_info"|"daemon.list_byok_configs"|"daemon.get_models"|"daemon.shutdown"));
        }
        assert_eq!(read_config(&settings).unwrap(), before);
    }
}
