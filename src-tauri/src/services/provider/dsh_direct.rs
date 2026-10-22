//! DSH's native projection inside ProviderService and the shared mode/write engine.
use crate::live::{
    engine::{digest, DeviceStore, LiveFile},
    patch::{Guarded, WholeFile},
};
use crate::mode::{
    operation::{AppWrite, FileChange, OperationReport},
    stack::{self, Member},
    state::{self, op, PendingTarget},
};
use crate::{error::AppError, provider::Provider, Database};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const PROXY_ID: &str = "cc-switch-dsh-route";
const SAVED_DEFAULTS: &str = "dshDefaults";
const OWNED_DEFAULT: &str = "dshOwnedDefault";

#[cfg(test)]
#[path = "dsh_direct_tests.rs"]
mod tests;

pub(crate) fn root() -> Result<PathBuf, AppError> {
    std::env::var_os("DSH_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".dsh")))
        .ok_or_else(|| AppError::Message("DSH_HOME_UNAVAILABLE".into()))
}

pub(crate) fn native_default_baseline() -> Result<Option<Value>, AppError> {
    let path = DeviceStore::for_device().state_path();
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return proxy_default_baseline(),
        Err(_) => return Err(error("DSH_NATIVE_DEFAULT_STATE_UNREADABLE")),
    };
    // state::load repairs corrupt state; an observation must never rewrite it.
    let saved: state::LiveState = serde_json::from_slice(&bytes)
        .map_err(|_| error("DSH_NATIVE_DEFAULT_STATE_INVALID"))?;
    let Some(app) = saved.apps.get("dsh") else {
        return proxy_default_baseline();
    };
    if app.pending.is_some() {
        return Err(error("DSH_NATIVE_DEFAULT_OPERATION_PENDING"));
    }
    if let Some(defaults) = app.written.as_ref()
        .and_then(|written| written.extra.get(SAVED_DEFAULTS))
        .filter(|defaults| defaults.is_object()) {
        return Ok(Some(defaults.clone()));
    }
    if app.attached && app.mode_state().is_proxy() {
        return Err(error("DSH_NATIVE_DEFAULT_BASELINE_UNAVAILABLE"));
    }
    proxy_default_baseline()
}

pub(crate) fn proxy_default_baseline() -> Result<Option<Value>, AppError> {
    let mut defaults = serde_json::Map::new();
    for path in dsh_profile_paths(&root()?) {
        let (_, doc) = yaml_document(&path, serde_yaml::Value::Sequence(vec![])).map_err(error)?;
        let owned = doc.as_sequence().ok_or_else(|| error("DSH_PROFILE_INVALID"))?
            .iter().any(|entry| {
                (entry["id"].as_str() == Some("agent-default-model")
                    || entry["name"].as_str() == Some("@deepseek-ai/dsh-agent-default-model"))
                    && entry["config"]["provider"].as_str()
                        .is_some_and(|id| is_cc_proxy_default(&doc, id))
            });
        if owned {
            let profile = path.parent().and_then(Path::file_name).and_then(|name| name.to_str())
                .ok_or_else(|| error("DSH_PROFILE_INVALID"))?;
            defaults.insert(profile.into(), json!({}));
        }
    }
    Ok((!defaults.is_empty()).then_some(Value::Object(defaults)))
}
fn error(message: impl ToString) -> AppError {
    AppError::Message(message.to_string())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct Projection {
    id: String,
    name: String,
    api: String,
    base: String,
    key: String,
    models: Vec<Value>,
    default_model: String,
    effort: Option<String>,
}
pub(crate) fn needs_routing(provider: &Provider) -> bool {
    provider
        .meta
        .as_ref()
        .and_then(|m| m.managed_account_id_for("codex_oauth"))
        .is_some()
}

pub(crate) fn is_native_ref(provider: &Provider) -> bool {
    provider.settings_config["nativeRef"] == true
        && ["desktop", "web"].into_iter().any(|profile| {
            provider.id == format!("native-current:{profile}")
                && provider.settings_config["profile"] == profile
        })
}

pub(crate) fn register_native_refs(db: &Database, rows: Vec<Provider>) -> Result<Vec<Provider>, AppError> {
    let mut refs = Vec::new();
    for mut row in rows {
        let profile = row.settings_config["profile"].as_str()
            .filter(|profile| ["desktop", "web"].contains(profile))
            .ok_or_else(|| error("DSH_NATIVE_DEFAULTS_INVALID_RESPONSE"))?;
        let stable_id = format!("native-current:{profile}");
        if row.id != stable_id && !row.id.starts_with(&format!("{stable_id}:")) {
            return Err(error("DSH_NATIVE_DEFAULTS_INVALID_RESPONSE"));
        }
        if db.get_provider_by_id(&stable_id, "dsh")?.is_some_and(|existing| !is_native_ref(&existing)) {
            return Err(error("DSH_NATIVE_PROVIDER_ID_CONFLICT"));
        }
        if row.settings_config["nativeBalanceAuth"].is_string() {
            row.settings_config["nativeBalanceId"] = json!(row.id);
        }
        row.id = stable_id;
        row.settings_config["nativeRef"] = json!(true);
        refs.push(row);
    }
    for row in &refs {
        db.save_provider("dsh", row)?;
    }
    Ok(refs)
}
pub(crate) fn read_live() -> Result<Value, AppError> {
    Err(error(
        "DSH_NATIVE_IMPORT_UNSUPPORTED: use the existing CC provider and managed-account catalog",
    ))
}

pub(crate) fn validate(provider: &Provider) -> Result<(), AppError> {
    // The existing CC provider editor has one stored representation; this parser is pure.
    crate::qoder_gateway::native_config(provider).map(|_| ())
}
pub(crate) fn projection(
    provider: &Provider,
    proxy: Option<(&str, &str)>,
    members: &[Member],
) -> Result<Projection, AppError> {
    let config = crate::qoder_gateway::native_config(provider)?;
    if proxy.is_none() && config.codex_account_id.is_some() {
        return Err(error(
            "DSH_MANAGED_ACCOUNT_REQUIRES_ROUTING: select Routing in the existing mode control",
        ));
    }
    let mut models = config.model_details;
    for member in members.iter().filter(|m| !m.route) {
        let native = crate::qoder_gateway::native_config(&member.provider)?;
        for mut model in native.model_details {
            let original = model["id"].as_str().expect("validated model").to_string();
            let published = stack::encode(
                &crate::app_config::AppType::Dsh,
                &member.key,
                &original,
                false,
            );
            if member.model_ids.contains(&published) {
                model["id"] = json!(published);
                model["name"] = json!(stack::display_name(&original, &member.provider.name));
                models.push(model);
            }
        }
    }
    let (id, api, base, key) = match proxy {
        Some((base, key)) => (PROXY_ID.into(), "openai-responses", base.into(), key.into()),
        None => (
            id_for(&provider.id),
            match config.api_format {
                crate::qoder_gateway::ApiFormat::OpenaiResponses => "openai-responses",
                crate::qoder_gateway::ApiFormat::OpenaiChat => "openai-completions",
                crate::qoder_gateway::ApiFormat::AnthropicMessages => "anthropic-messages",
            },
            config.base_url,
            config.api_key,
        ),
    };
    Ok(Projection {
        id,
        name: provider.name.clone(),
        api: api.into(),
        base,
        key,
        models,
        default_model: config.models[0].clone(),
        effort: config.default_effort,
    })
}

// A selected model is the default, not the managed account's complete catalog.
pub(crate) async fn route_projection(
    db: &Database,
    manager: &crate::proxy::providers::codex_oauth_auth::CodexOAuthManager,
    provider: &Provider,
    base: &str,
    key: &str,
    members: &[Member],
) -> Result<Projection, AppError> {
    let mut provider = provider.clone();
    if provider.settings_config.get("modelCatalog").is_none() {
        if let Some(account) = provider.meta.as_ref().and_then(|m| m.managed_account_id_for("codex_oauth")) {
            let result = async {
                let bundle = manager.get_valid_token_bundle_for_account(&account).await.map_err(|_| error("DSH_MODEL_CATALOG_ACCOUNT_UNAVAILABLE"))?;
                crate::services::codex_oauth_models::fetch_models_with_token(&bundle.access_token, &bundle.chatgpt_account_id)
                    .await.map_err(|_| error("DSH_MODEL_CATALOG_UNAVAILABLE"))
            }.await;
            match result {
                Ok(models) if !models.is_empty() => {
                    provider = publish_managed_catalog(db, &provider, models.into_iter().map(|m| m.id))?;
                }
                Ok(_) => log::warn!("DSH managed model catalog is empty; keeping the configured model catalog"),
                Err(_) => log::warn!("DSH managed model catalog unavailable; keeping the configured model catalog"),
            }
        }
    }
    projection(&provider, Some((base, key)), members)
}

fn add_managed_catalog(provider: &mut Provider, models: impl Iterator<Item = String>) {
    provider.settings_config["modelCatalog"] = json!({"models": models.map(|model| json!({"model":model})).collect::<Vec<_>>()});
}

fn publish_managed_catalog(db: &Database, provider: &Provider, models: impl Iterator<Item = String>) -> Result<Provider, AppError> {
    let mut published = provider.clone();
    add_managed_catalog(&mut published, models);
    validate(&published)?;
    if !db.cache_dsh_model_catalog(provider, &published.settings_config["modelCatalog"])? {
        return Err(error("DSH_MODEL_CATALOG_PROVIDER_CHANGED"));
    }
    Ok(published)
}

pub(crate) fn switch_to(db: &Database, provider: &Provider) -> Result<OperationReport, AppError> {
    let projection = (!is_native_ref(provider)).then(|| projection(provider, None, &[])).transpose()?;
    run(
        db,
        op::SWITCH,
        Some(projection.as_ref().map(Target::Provider).unwrap_or(Target::NativeDefault)),
        PendingTarget::pointer(Some(provider.id.clone())),
    )
}
pub(crate) fn reapply(db: &Database, provider: &Provider) -> Result<OperationReport, AppError> {
    let projection = (!is_native_ref(provider)).then(|| projection(provider, None, &[])).transpose()?;
    run(
        db,
        op::APPLY,
        Some(projection.as_ref().map(Target::Provider).unwrap_or(Target::NativeDefault)),
        PendingTarget::default(),
    )
}
pub(crate) enum Target<'a> {
    Provider(&'a Projection),
    NativeDefault,
}

pub(crate) fn run(
    db: &Database,
    operation: &str,
    projection: Option<Target<'_>>,
    mut target: PendingTarget,
) -> Result<OperationReport, AppError> {
    let write = AppWrite::begin(db, "dsh")?;
    let Some(projection) = projection else {
        return write.run(operation, &[], target);
    };
    let root = root()?;
    let _locks = NativeLocks::acquire(&root).map_err(error)?;
    let mut written = state::written(&DeviceStore::for_device(), "dsh")?.unwrap_or_default();
    let edits = match projection {
        Target::Provider(projection) => {
            if !written.extra.contains_key(SAVED_DEFAULTS) {
                written
                    .extra
                    .insert(SAVED_DEFAULTS.into(), saved_defaults(&root).map_err(error)?);
            }
            written.extra.insert(OWNED_DEFAULT.into(), json!(projection.id));
            edits_for(&root, projection).map_err(error)?
        }
        Target::NativeDefault => {
            let owned = written.extra.remove(OWNED_DEFAULT);
            let edits = restore_defaults(&root, written.extra.remove(SAVED_DEFAULTS), owned.as_ref().and_then(Value::as_str)).map_err(error)?;
            edits
        }
    };
    target.written = Some(written);
    let patches: Vec<Guarded> = edits
        .iter()
        .map(|e| Guarded {
            expected_pre: digest(e.before.as_deref().map(str::as_bytes)),
            then: WholeFile::Write(e.after.as_bytes().to_vec()),
        })
        .collect();
    let changes: Vec<FileChange<'_>> = edits
        .iter()
        .zip(&patches)
        .map(|(e, patch)| FileChange {
            file: LiveFile::private(e.path.clone()),
            patch,
        })
        .collect();
    write.run(operation, &changes, target)
}
fn saved_defaults(root: &Path) -> Result<Value, String> {
    let mut saved = serde_json::Map::new();
    for path in dsh_patch_paths(root)? {
        let (_, doc) = yaml_document(&path, serde_yaml::Value::Sequence(vec![]))?;
        let selection = doc
            .as_sequence()
            .ok_or("DSH_PROFILE_INVALID")?
            .iter()
            .find(|entry| {
                entry["id"].as_str() == Some("agent-default-model")
                    || entry["name"].as_str() == Some("@deepseek-ai/dsh-agent-default-model")
            })
            .map(|e| &e["config"]);
        let mut fields = serde_json::Map::new();
        if let Some(selection) = selection {
            for key in ["provider", "model", "reasoningEffort"] {
                if let Some(value) = selection.get(key) {
                    fields.insert(
                        key.into(),
                        serde_json::to_value(value).map_err(|_| "DSH_CONFIG_INVALID")?,
                    );
                }
            }
            // The cc- prefix is also used by valid direct providers; exclude only our proxy.
            if selection["provider"]
                .as_str()
                .is_some_and(|id| is_cc_proxy_default(&doc, id))
            {
                fields.clear();
            }
        }
        let profile = path
            .parent()
            .and_then(Path::file_name)
            .and_then(|n| n.to_str())
            .ok_or("DSH_PROFILE_INVALID")?;
        saved.insert(profile.into(), Value::Object(fields));
    }
    Ok(Value::Object(saved))
}

fn is_cc_proxy_default(doc: &serde_yaml::Value, id: &str) -> bool {
    if id == PROXY_ID {
        return true;
    }
    let base = doc.as_sequence().into_iter().flatten().find_map(|entry| {
        (entry["id"].as_str() == Some("llm-pi-ai")
            || entry["name"].as_str() == Some("@deepseek-ai/dsh-llm-pi-ai"))
        .then(|| entry["config"]["providers"][id]["baseURL"].as_str())
        .flatten()
    });
    let Some(url) = base.and_then(|base| reqwest::Url::parse(base).ok()) else {
        return false;
    };
    let local = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if url.scheme() != "http"
        || !local
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    url.path()
        .trim_end_matches('/')
        .strip_prefix("/dsh/")
        .and_then(|path| path.strip_suffix("/v1"))
        .filter(|provider| !provider.is_empty() && !provider.contains('/'))
        .is_some_and(|provider| {
            id == id_for(&crate::services::subscription_grok::percent_decode(
                provider,
            ))
        })
}

fn restore_defaults(root: &Path, saved: Option<Value>, owned: Option<&str>) -> Result<Vec<NativeEdit>, String> {
    let mut edits = vec![];
    let mut inherited: Option<Vec<Provider>> = None;
    for path in dsh_patch_paths(root)? {
        let (before, mut doc) = yaml_document(&path, serde_yaml::Value::Sequence(vec![]))?;
        let profile = path
            .parent()
            .and_then(Path::file_name)
            .and_then(|n| n.to_str())
            .ok_or("DSH_PROFILE_INVALID")?;
        let mut original = saved
            .as_ref()
            .and_then(|s| s.get(profile))
            .and_then(Value::as_object)
            .filter(|fields| fields.get("provider").and_then(Value::as_str).is_some()
                && fields.get("model").and_then(Value::as_str).is_some());
        let owned_default = doc.as_sequence().ok_or("DSH_PROFILE_INVALID")?.iter()
            .find(|entry| entry["id"].as_str() == Some("agent-default-model")
                || entry["name"].as_str() == Some("@deepseek-ai/dsh-agent-default-model"))
            .and_then(|entry| entry["config"]["provider"].as_str())
            .filter(|id| Some(*id) == owned || is_cc_proxy_default(&doc, id)).map(str::to_owned);
        let proxy_default = owned_default.as_deref().is_some_and(|id| is_cc_proxy_default(&doc, id));
        let enabled = doc.as_sequence().ok_or("DSH_PROFILE_INVALID")?.iter()
            .find(|entry| entry["id"].as_str() == Some("agent-default-model")
                || entry["name"].as_str() == Some("@deepseek-ai/dsh-agent-default-model"))
            .is_some_and(|entry| entry["disabled"].as_bool() != Some(true));
        if owned_default.is_some() && enabled && original.is_none() {
            if inherited.is_none() {
                let baseline = match &saved {
                    Some(defaults) => Some(defaults.clone()),
                    None => proxy_default_baseline().map_err(|e| e.to_string())?,
                };
                inherited = Some(crate::commands::read_dsh_native_defaults(baseline)?);
            }
            original = inherited.as_ref().and_then(|rows| rows.iter()
                .find(|row| row.settings_config["profile"].as_str() == Some(profile)))
                .and_then(|row| row.settings_config.as_object());
            if original.is_none() { return Err("DSH_NATIVE_DEFAULT_INHERITANCE_UNAVAILABLE".into()); }
        }
        let entries = doc.as_sequence_mut().ok_or("DSH_PROFILE_INVALID")?;
        for entry in entries.iter_mut() {
            if (entry["id"].as_str() == Some("agent-default-model")
                || entry["name"].as_str() == Some("@deepseek-ai/dsh-agent-default-model"))
                && owned_default.as_deref().is_some_and(|id| entry["config"]["provider"].as_str() == Some(id))
            {
                for key in ["provider", "model", "reasoningEffort"] {
                    entry["config"].as_mapping_mut().ok_or("DSH_PROFILE_INVALID")?
                        .remove(serde_yaml::Value::String(key.into()));
                    if let Some(value) = original.and_then(|fields| fields.get(key)) {
                        entry["config"][key] = serde_yaml::to_value(value).map_err(|_| "DSH_CONFIG_INVALID")?;
                    }
                }
                if entry["config"].as_mapping().is_some_and(|config| config.is_empty()) {
                    entry.as_mapping_mut().ok_or("DSH_PROFILE_INVALID")?
                        .remove(serde_yaml::Value::String("config".into()));
                }
            }
            if entry["id"].as_str() == Some("llm-pi-ai")
                || entry["name"].as_str() == Some("@deepseek-ai/dsh-llm-pi-ai")
            {
                if let Some(providers) = entry
                    .get_mut("config")
                    .and_then(|config| config.get_mut("providers"))
                    .and_then(serde_yaml::Value::as_mapping_mut)
                {
                    providers.remove(serde_yaml::Value::String(PROXY_ID.into()));
                    if let Some(id) = owned_default.as_ref().filter(|_| proxy_default) {
                        providers.remove(serde_yaml::Value::String(id.clone()));
                    }
                }
            }
        }
        edits.push(NativeEdit {
            path,
            before,
            after: serde_yaml::to_string(&doc).map_err(|_| "DSH_CONFIG_INVALID")?,
        });
    }
    Ok(edits)
}

fn read_config(path: &std::path::Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|_| "NATIVE_CONFIG_UNREADABLE")?;
    if file
        .metadata()
        .map_err(|_| "NATIVE_CONFIG_UNREADABLE")?
        .len()
        > 2_000_000
    {
        return Err("NATIVE_CONFIG_TOO_LARGE".into());
    }
    use std::io::Read;
    let mut text = String::new();
    file.take(2_000_001)
        .read_to_string(&mut text)
        .map_err(|_| "NATIVE_CONFIG_UNREADABLE")?;
    if text.len() > 2_000_000 {
        return Err("NATIVE_CONFIG_TOO_LARGE".into());
    }
    Ok(text)
}

fn id_for(id: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("cc-{:x}", Sha256::digest(id.as_bytes()))
}

struct NativeEdit {
    path: PathBuf,
    before: Option<String>,
    after: String,
}

// DSH's official writer protocol is exclusive <file>.lock with a live PID record.
// Do not take over or retry another writer's lock here.
struct NativeLocks(Vec<PathBuf>);
impl NativeLocks {
    fn acquire(root: &std::path::Path) -> Result<Self, String> {
        use std::io::Write;
        let mut files = vec![root.join(".credentials.yaml")];
        for profile in ["desktop", "web"] {
            let directory = root.join("profiles").join(profile);
            if directory.join("cordis.patch.yml").is_file() {
                files.push(directory.join("package.json"));
            }
        }
        files.sort();
        let mut guard = Self(vec![]);
        for file in files {
            let path = file.with_file_name(format!(
                "{}.lock",
                file.file_name().unwrap().to_string_lossy()
            ));
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut lock = options
                .open(&path)
                .map_err(|_| "DSH_NATIVE_WRITER_BUSY_OR_LOCK_UNAVAILABLE_NO_WRITE")?;
            guard.0.push(path);
            if writeln!(lock, "{}", std::process::id()).is_err() {
                drop(lock);
                if let Some(path) = guard.0.pop() {
                    let _ = std::fs::remove_file(path);
                }
                return Err("DSH_NATIVE_LOCK_FAILED".into());
            }
        }
        Ok(guard)
    }
}
impl Drop for NativeLocks {
    fn drop(&mut self) {
        let record = format!("{}\n", std::process::id());
        for path in self.0.iter().rev() {
            if std::fs::read_to_string(path).ok().as_deref() == Some(&record) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

fn dsh_patch_paths(root: &std::path::Path) -> Result<Vec<PathBuf>, String> {
    let global = root.join("cordis.patch.yml");
    if global.exists() {
        let (_, doc) = yaml_document(&global, serde_yaml::Value::Sequence(vec![]))?;
        if !doc.is_null() && !doc.as_sequence().is_some_and(Vec::is_empty) {
            return Err("DSH_GLOBAL_OVERLAY_REQUIRES_NATIVE_SETTINGS_API_NO_WRITE".into());
        }
    }
    Ok(dsh_profile_paths(root))
}

fn dsh_profile_paths(root: &Path) -> Vec<PathBuf> {
    ["desktop", "web"]
        .into_iter()
        .map(|profile| root.join("profiles").join(profile).join("cordis.patch.yml"))
        .filter(|path| path.is_file() || path.with_file_name("package.json").is_file())
        .collect()
}

fn yaml_document(
    path: &std::path::Path,
    empty: serde_yaml::Value,
) -> Result<(Option<String>, serde_yaml::Value), String> {
    let before = if path.exists() {
        Some(read_config(path)?)
    } else {
        None
    };
    let value = before
        .as_deref()
        .map(|text| serde_yaml::from_str(text).map_err(|_| "DSH_CONFIG_INVALID".to_string()))
        .transpose()?
        .unwrap_or(empty);
    Ok((before, value))
}

fn yaml_entry<'a>(
    entries: &'a mut Vec<serde_yaml::Value>,
    id: &str,
    name: &str,
) -> Result<&'a mut serde_yaml::Value, String> {
    let matches: Vec<_> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e["id"].as_str() == Some(id) || e["name"].as_str() == Some(name))
        .map(|(i, _)| i)
        .collect();
    if matches.len() > 1 {
        return Err("DSH_AMBIGUOUS_PLUGIN_CONFIGURATION".into());
    }
    let index = if let Some(index) = matches.first() {
        *index
    } else {
        entries.push(
            serde_yaml::to_value(serde_json::json!({"id":id,"name":name,"config":{}}))
                .map_err(|_| "DSH_CONFIG_INVALID")?,
        );
        entries.len() - 1
    };
    let entry = &mut entries[index];
    if entry.get("config").is_none() {
        entry["config"] = serde_yaml::Value::Mapping(Default::default());
    }
    if !entry["config"].is_mapping() {
        return Err("DSH_CONFIG_INVALID".into());
    }
    Ok(&mut entry["config"])
}

fn edits_for(root: &Path, projection: &Projection) -> Result<Vec<NativeEdit>, String> {
    let id = projection.id.clone();
    let credential = format!(
        "CC_DSH_{}",
        if id == PROXY_ID {
            id_for(&id)[3..].to_uppercase()
        } else {
            id[3..].to_uppercase()
        }
    );
    let api = projection.api.as_str();
    let base = &projection.base;
    let key = &projection.key;
    let mut edits = Vec::new();
    for path in dsh_patch_paths(root)? {
        let (before, mut doc) = yaml_document(&path, serde_yaml::Value::Sequence(vec![]))?;
        let entries = doc.as_sequence_mut().ok_or("DSH_PROFILE_INVALID")?;
        // A custom credential-store path cannot be replaced with the default store by assumption.
        if entries.iter().any(|e| {
            (e["id"].as_str() == Some("credentials-local")
                || e["name"].as_str() == Some("@deepseek-ai/dsh-credentials-local"))
                && (e["config"].get("path").is_some() || e["config"].get("dshHome").is_some())
        }) {
            return Err("DSH_CUSTOM_CREDENTIAL_STORE_UNSUPPORTED".into());
        }
        let llm = yaml_entry(entries, "llm-pi-ai", "@deepseek-ai/dsh-llm-pi-ai")?;
        if llm.get("providers").is_none() {
            llm["providers"] = serde_yaml::Value::Mapping(Default::default());
        }
        if !llm["providers"].is_mapping() {
            return Err("DSH_PROVIDERS_INVALID".into());
        }
        let mut route = llm["providers"]
            .get(&id)
            .cloned()
            .unwrap_or(serde_yaml::Value::Mapping(Default::default()));
        if !route.is_mapping() {
            return Err("DSH_PROVIDER_INVALID".into());
        }
        let previous_models = route
            .get("models")
            .cloned()
            .unwrap_or(serde_yaml::Value::Sequence(vec![]));
        let existing_models = previous_models.as_sequence().ok_or("DSH_MODELS_INVALID")?;
        let mut models = Vec::new();
        for detail in &projection.models {
            let id = detail["id"].as_str().unwrap();
            let mut model = existing_models
                .iter()
                .find(|model| model["id"].as_str() == Some(id))
                .cloned()
                .unwrap_or(serde_yaml::Value::Mapping(Default::default()));
            for (key, value) in detail.as_object().unwrap() {
                model[key.as_str()] =
                    serde_yaml::to_value(value).map_err(|_| "DSH_MODELS_INVALID")?;
            }
            models.push(model);
        }
        for (field, value) in serde_json::json!({"displayName":projection.name,"api":api,"baseURL":base,"apiKeyEnv":credential}).as_object().unwrap() {
            route[field.as_str()] = serde_yaml::to_value(value).map_err(|_| "DSH_CONFIG_INVALID")?;
        }
        route["models"] = serde_yaml::Value::Sequence(models);
        llm["providers"][id.as_str()] = route;
        let selected = yaml_entry(
            entries,
            "agent-default-model",
            "@deepseek-ai/dsh-agent-default-model",
        )?;
        {
            selected["provider"] = serde_yaml::Value::String(id.clone());
            selected["model"] = serde_yaml::Value::String(projection.default_model.clone());
            // A previous provider's effort is not portable to a newly selected model.
            selected
                .as_mapping_mut()
                .unwrap()
                .remove(serde_yaml::Value::String("reasoningEffort".into()));
            if let Some(effort) = &projection.effort {
                selected["reasoningEffort"] = serde_yaml::Value::String(effort.clone());
            }
        }
        edits.push(NativeEdit {
            path,
            before,
            after: serde_yaml::to_string(&doc).map_err(|_| "DSH_CONFIG_INVALID")?,
        });
    }
    if edits.is_empty() {
        return Err("DSH_NATIVE_PROFILE_NOT_FOUND".into());
    }
    let path = root.join(".credentials.yaml");
    let (before, mut doc) = yaml_document(
        &path,
        serde_yaml::to_value(serde_json::json!({"version":1,"refs":{}})).unwrap(),
    )?;
    if doc["version"].as_i64() != Some(1) {
        return Err("DSH_CREDENTIAL_STORE_VERSION_UNSUPPORTED".into());
    }
    if doc.get("refs").is_none() {
        doc["refs"] = serde_yaml::Value::Mapping(Default::default());
    }
    if !doc["refs"].is_mapping() {
        return Err("DSH_CREDENTIAL_STORE_INVALID".into());
    }
    doc["refs"][credential.as_str()] = serde_yaml::Value::String(key.clone());
    edits.insert(
        0,
        NativeEdit {
            path,
            before,
            after: serde_yaml::to_string(&doc).map_err(|_| "DSH_CREDENTIAL_STORE_INVALID")?,
        },
    );
    Ok(edits)
}
