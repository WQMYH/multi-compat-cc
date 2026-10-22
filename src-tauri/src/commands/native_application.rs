use crate::AppState;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};
use tauri::State;
use crate::provider::Provider;
use indexmap::IndexMap;
pub(super) mod qoder;

pub fn is_native_app(app: &str) -> bool {
    app == "qoder"
}

pub(super) fn config_root(app: &str) -> Result<PathBuf, String> {
    let (key, directory) = match app {
        "qoder" => ("QODERCN_CONFIG_DIR", ".qoder-cn"),
        _ => return Err("UNSUPPORTED_NATIVE_APPLICATION".into()),
    };
    std::env::var_os(key).filter(|v| !v.is_empty()).map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(directory)))
        .ok_or_else(|| "NATIVE_HOME_UNAVAILABLE".into())
}

fn read_config(path: &std::path::Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => "NATIVE_CONFIG_NOT_FOUND",
        _ => "NATIVE_CONFIG_UNREADABLE",
    })?;
    if file.metadata().map_err(|_| "NATIVE_CONFIG_UNREADABLE")?.len() > 2_000_000 {
        return Err("NATIVE_CONFIG_TOO_LARGE".into());
    }
    use std::io::Read;
    let mut text = String::new();
    file.take(2_000_001).read_to_string(&mut text).map_err(|_| "NATIVE_CONFIG_UNREADABLE")?;
    if text.len() > 2_000_000 { return Err("NATIVE_CONFIG_TOO_LARGE".into()); }
    Ok(text)
}

fn detected_provider(app: &str, profile: &str, provider: &str, model: &str) -> Provider {
    let mut value = Provider::with_id(
        format!("native-current:{profile}"), format!("{provider} · {model}"),
        serde_json::json!({"provider": provider, "model": model, "profile": profile}), None,
    );
    value.category = Some("official".into());
    value.sort_index = Some(0);
    value.notes = Some(format!("从 {app} 现有配置只读识别 · {profile} 默认模型（不是所有会话的运行模型）"));
    value
}

fn qoder_selection(value: &serde_json::Value) -> Option<Provider> {
    let selection = value.get("model")?;
    let model = selection.as_str().or_else(|| selection.get("name")?.as_str())?;
    if model.trim().is_empty() { return None; }
    let provider = selection.get("provider").and_then(|v| v.as_str()).unwrap_or("Qoder CN");
    Some(detected_provider("qoder", "current", provider, model))
}



// Detection is an observation, never an automatic import or a saved current-provider selection.
fn current_at(app: &str, root: &std::path::Path) -> Result<Vec<Provider>, String> {
    if !is_native_app(app) { return Err("UNSUPPORTED_NATIVE_APPLICATION".into()); }
    let text = match read_config(&root.join("settings.json")) {
        Ok(text) => text,
        Err(error) if error == "NATIVE_CONFIG_NOT_FOUND" => return Ok(vec![]),
        Err(error) => return Err(error),
    };
    let value = serde_json::from_str(&text).map_err(|_| "QODER_CONFIG_INVALID")?;
    Ok(qoder_selection(&value).into_iter().collect())
}

pub fn catalog(state: &AppState, app: &str) -> Result<IndexMap<String, Provider>, String> {
    let mut rows = state.db.get_all_providers(app).map_err(|e| e.to_string())?;
    if app == "qoder" {
        for provider in rows.values_mut() {
            let binding = qoder::record(state, &provider.id)?;
            if binding["state"] == "ready" { provider.notes = Some(format!("Qoder 原生 BYOK 已同步 · 供应商内默认模型 {}；桌面当前选择单独检测", binding["defaultModelId"].as_str().unwrap_or(""))); }
            if binding["readOnlyReconciled"] == true { provider.notes = Some("当前原生 BYOK 目录已只读回读；原请求记录保留，未重新提交。桌面当前选择单独检测。".into()); }
            if binding["state"] == "pending" { provider.notes = Some("上次 Qoder 原生变更结果未知；保留记录，停止自动重试。".into()); }
        }
    }
    for current in current_at(app, &config_root(app)?)? {
        rows.insert(current.id.clone(), current);
    }
    Ok(rows)
}

pub fn current_id(state: &AppState, app: &str) -> Result<String, String> {
    if !is_native_app(app) { return Err("UNSUPPORTED_NATIVE_APPLICATION".into()); }
    let current = current_at(app, &config_root(app)?)?;
    for id in state.db.get_all_providers(app).map_err(|e| e.to_string())?.keys() {
        if qoder::current(state, id, &current)? { return Ok(id.clone()); }
    }
    Ok(current.first().map(|p| p.id.clone()).unwrap_or_default())
}

pub fn save_managed(state: &AppState, app: &str, provider: &Provider, update: bool) -> Result<bool, String> {
    if crate::readonly_provider_preview() { return Err("READONLY_PREVIEW_NO_CONFIGURATION_CHANGES".into()); }
    let _guard = futures::executor::block_on(state.proxy_service.lock_switch_for_app(app));
    if !is_native_app(app) || provider.id.starts_with("native-current:") || provider.id.trim().is_empty()
        || provider.name.trim().is_empty() || !provider.settings_config.is_object() {
        return Err("INVALID_NATIVE_MANAGED_PROVIDER".into());
    }
    // Shared form representation only; storage still belongs to the actual native app.
    crate::services::ProviderService::validate_provider_settings(&crate::app_config::AppType::Codex, provider)
        .map_err(|e| e.to_string())?;
    let existing = state.db.get_provider_by_id(&provider.id, app).map_err(|e| e.to_string())?;
    if update != existing.is_some() { return Err("NATIVE_PROVIDER_CONFLICT".into()); }
    if app == "qoder" && update && qoder::applied(state, &provider.id)? {
        let config = crate::qoder_gateway::native_config(provider).map_err(|e| e.to_string())?;
        let (base, key) = if config.codex_account_id.is_some() {
            let status = futures::executor::block_on(state.proxy_service.get_status())?;
            if !status.running { return Err("QODER_PROXY_NOT_RUNNING_NO_CHANGES: 请先启动 CC 现有路由服务".into()); }
            (native_proxy_base("qoder", &status.address, status.port, &provider.id)?,
                state.db.get_setting("qoder_gateway_api_key_v1").map_err(|e| e.to_string())?.ok_or("QODER_PROXY_NOT_CONFIGURED")?)
        } else { (config.base_url, config.api_key) };
        qoder::sync(state, provider, &base, &key, || state.db.save_provider(app, provider).map_err(|e| e.to_string()))?;
        return Ok(true);
    }

    state.db.save_provider(app, provider).map_err(|e| e.to_string())?;
    Ok(true)
}

pub fn delete_managed(state: &AppState, app: &str, id: &str) -> Result<bool, String> {
    if crate::readonly_provider_preview() { return Err("READONLY_PREVIEW_NO_CONFIGURATION_CHANGES".into()); }
    if id.starts_with("native-current:") { return Err("NATIVE_CURRENT_CONFIGURATION_READ_ONLY".into()); }
    let _guard = futures::executor::block_on(state.proxy_service.lock_switch_for_app(app));
    if state.db.get_provider_by_id(id, app).map_err(|e| e.to_string())?.is_none() { return Ok(true); }
    if app == "qoder" { return qoder::remove(state, id); }

    state.db.delete_provider(app, id).map_err(|e| e.to_string())?;
    Ok(true)
}



fn dsh_id(id: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("cc-{:x}", Sha256::digest(id.as_bytes()))
}

fn native_proxy_base(app: &str, address: &str, port: u16, id: &str) -> Result<String, String> {
    if !is_native_app(app) { return Err("UNSUPPORTED_NATIVE_APPLICATION".into()); }
    let host = match address { "0.0.0.0" | "::" => "127.0.0.1", "::1" => "[::1]", value => value };
    let mut url = url::Url::parse(&format!("http://{host}:{port}")).map_err(|_| "DSH_PROXY_ADDRESS_INVALID")?;
    url.path_segments_mut().map_err(|_| "DSH_PROXY_ADDRESS_INVALID")?.extend([app, id, "v1"]);
    Ok(url.to_string())
}



pub async fn apply(state: &AppState, app: &str, id: &str) -> Result<crate::services::SwitchResult, String> {
    if crate::readonly_provider_preview() { return Err("READONLY_PREVIEW_NO_CONFIGURATION_CHANGES".into()); }
    if !is_native_app(app) { return Err("UNSUPPORTED_NATIVE_APPLICATION".into()); }
    let _guard = state.proxy_service.lock_switch_for_app(app).await;
    let pending = qoder::pending(state, id)?;
    if pending { qoder::invalidate_readback(state, id)?; }
    let provider = state.db.get_provider_by_id(id, app).map_err(|e| e.to_string())?.ok_or("NATIVE_PROVIDER_NOT_FOUND")?;
    let config = crate::qoder_gateway::native_config(&provider).map_err(|e| e.to_string())?;
    if pending {
        let base = if config.codex_account_id.is_some() {
            let (address, port) = state.db.get_proxy_client_endpoint_sync();
            native_proxy_base(app, &address, port, id)?
        } else { config.base_url };
        qoder::reconcile(state, &provider, &base)?;
        let binding = qoder::record(state, id)?;
        let warning = if binding["modelValues"].as_array().is_some_and(Vec::is_empty) {
            "QODER_BYOK_RECONCILED_SELECTION_UNKNOWN"
        } else { "QODER_BYOK_RECONCILED" };
        return Ok(crate::services::SwitchResult { warnings: vec![warning.into()], ..Default::default() });
    }
    qoder::applied(state, id)?;
    let (base, key) = if config.codex_account_id.is_some() {
        let info = state.proxy_service.start().await?;
        (native_proxy_base(app, &info.address, info.port, id)?, crate::qoder_gateway::prepare_key(&state.db).map_err(|e| e.to_string())?)
    } else { (config.base_url, config.api_key) };
    qoder::sync(state, &provider, &base, &key, || Ok(()))?;
    Ok(crate::services::SwitchResult { warnings: vec!["QODER_BYOK_PROVIDER_DEFAULT_SYNCED: 原生 BYOK 与供应商内默认模型已回读；桌面全局默认及正在运行的会话未更改。请在 Qoder 原生模型选择器选用同步后的模型；SDK setModel 只对其自身会话临时生效。".into()], ..Default::default() })
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedProviderRef {
    source_app: String,
    provider_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeApplicationStatus {
    cli_path: Option<String>,
    data_dir: Option<String>,
    detection_error: Option<String>,
    provider: Option<ManagedProviderRef>,
    readonly_preview: bool,
}

fn validate_target(target: &str, source: Option<&str>) -> Result<&'static str, String> {
    match (target, source) {
        ("qoder", None | Some("codex")) => Ok("qodercn"),
        _ => Err("UNSUPPORTED_NATIVE_PROVIDER_SOURCE".into()),
    }
}

fn binding_key(target: &str) -> String {
    format!("native_application_provider_v1:{target}")
}

#[tauri::command]
pub fn native_application_preview_mode() -> bool {
    crate::readonly_provider_preview()
}

#[tauri::command]
pub async fn native_application_status(
    state: State<'_, AppState>,
    target: String,
) -> Result<NativeApplicationStatus, String> {
    let tool = validate_target(&target, None)?;
    let provider = state
        .db
        .get_setting(&binding_key(&target))
        .map_err(|e| e.to_string())?
        .map(|value| {
            serde_json::from_str(&value).map_err(|_| "INVALID_MANAGED_PROVIDER_REF".to_string())
        })
        .transpose()?;
    tauri::async_runtime::spawn_blocking(move || {
        let deadline = super::misc::CommandDeadline::from_timeout(Some(Duration::from_secs(5)));
        let located = super::misc::locate_default_tool(tool, deadline);
        let (cli_path, detection_error) = match located {
            Ok(path) => (Some(path.to_string_lossy().into_owned()), None),
            Err(error) => (None, Some(error)),
        };
        let data_dir = if target == "dsh" {
            std::env::var_os("DSH_HOME")
                .map(PathBuf::from)
                .or_else(|| dirs::home_dir().map(|home| home.join(".dsh")))
        } else {
            std::env::var_os("QODERCN_CONFIG_DIR")
                .map(PathBuf::from)
                .or_else(|| dirs::home_dir().map(|home| home.join(".qoder-cn")))
        };
        Ok(NativeApplicationStatus {
            cli_path,
            detection_error,
            provider,
            data_dir: data_dir.map(|path| path.to_string_lossy().into_owned()),
            readonly_preview: crate::readonly_provider_preview(),
        })
    })
    .await
    .map_err(|_| "NATIVE_APPLICATION_DISCOVERY_FAILED".to_string())?
}

#[tauri::command]
pub fn native_application_select_provider(
    state: State<'_, AppState>,
    target: String,
    provider: ManagedProviderRef,
) -> Result<ManagedProviderRef, String> {
    validate_target(&target, Some(&provider.source_app))?;
    if crate::readonly_provider_preview() {
        return Err("READONLY_PREVIEW_NO_CONFIGURATION_CHANGES".into());
    }
    let managed = state
        .db
        .get_all_providers(&provider.source_app)
        .map_err(|e| e.to_string())?;
    if !managed.contains_key(&provider.provider_id) {
        return Err("MANAGED_PROVIDER_NOT_FOUND".into());
    }
    state
        .db
        .set_setting(
            &binding_key(&target),
            &serde_json::to_string(&provider).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    Ok(provider)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_native_selections_without_credentials_or_phantom_providers() {
        let qoder = qoder_selection(&serde_json::json!({"model": {"name":"auto"}, "providers":{"unused":{"apiKey":"not-output"}}})).unwrap();
        assert_eq!(qoder.settings_config, serde_json::json!({"provider":"Qoder CN", "model":"auto", "profile":"current"}));
    }

    #[test]
    fn own_catalog_changes_do_not_mirror_or_mutate_codex() {
        let state = AppState::new(std::sync::Arc::new(crate::database::Database::memory().unwrap()));
        let codex = Provider::with_id("other".into(), "Codex only".into(), serde_json::json!({}), None);
        state.db.save_provider("codex", &codex).unwrap();
        assert!(state.db.get_all_providers("qoder").unwrap().is_empty());
        assert!(state.db.get_all_providers("dsh").unwrap().is_empty());
        let qoder = Provider::with_id("extra".into(), "Explicit Qoder addition".into(), serde_json::json!({"auth":{},"config":"model = \"m\""}), None);
        save_managed(&state, "qoder", &qoder, false).unwrap();
        assert!(state.db.get_all_providers("qoder").unwrap().contains_key("extra"));
        assert!(!state.db.get_all_providers("dsh").unwrap().contains_key("extra"));
        assert_eq!(state.db.get_all_providers("codex").unwrap().len(), 1);
        assert!(save_managed(&state, "qoder", &detected_provider("qoder", "current", "official", "auto"), false).is_err());
    }

    #[test]
    fn universal_native_membership_is_explicit_and_not_inherited_from_codex() {
        let mut provider = crate::provider::UniversalProvider::new("u".into(), "Shared".into(), "custom".into(), "https://example.test".into(), "fixture".into());
        provider.apps.codex = true;
        assert!(provider.to_native_provider("qoder").is_none());
        provider.apps.qoder = true;
        provider.models.qoder = Some(crate::provider::CodexModelConfig { model: Some("qoder-model".into()), ..Default::default() });
        assert_eq!(provider.to_native_provider("qoder").unwrap().id, "universal-qoder-u");
        assert!(provider.to_native_provider("dsh").is_none());
    }
    #[test]
    fn native_targets_only_reference_supported_managed_sources() {
        assert_eq!(validate_target("qoder", Some("codex")).unwrap(), "qodercn");
        assert!(validate_target("dsh", Some("claude")).is_err());
        assert!(validate_target("qoder", Some("claude")).is_err());
        assert!(validate_target("unknown", None).is_err());
        assert!(serde_json::from_str::<ManagedProviderRef>(
            r#"{"sourceApp":"codex","providerId":"a","apiKey":"must-not-copy"}"#
        )
        .is_err());
    }

    #[test]
    fn preview_allows_catalog_but_not_accounts_models_or_configuration_changes() {
        assert!(crate::readonly_preview_command("get_providers"));
        assert!(crate::readonly_preview_command("native_application_status"));
        assert!(crate::readonly_preview_command("queryProviderUsage"));
        for command in [
            "save_settings",
            "switch_provider",
            "auth_start_login",
            "native_application_select_provider",
            "get_codex_oauth_models",
            "stream_check_provider",
            "install_cli",
            "auth_remove_account",
        ] {
            assert!(!crate::readonly_preview_command(command), "{command}");
        }
    }
}
