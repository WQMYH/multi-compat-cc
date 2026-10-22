//! Qoder's explicit BYOK gateway configuration. Never writes a native product config.
use crate::{
    database::Database, error::AppError, provider::Provider,
    proxy::providers::chatgpt_plan_auth::inference::API_BASE,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashSet, sync::Mutex};

pub const APP: &str = "qoder";
const TOKEN_KEY: &str = "qoder_gateway_api_key_v1";
// shortcut: a single CC process owns these edits; use DAO transactions for multi-writer support.
static EDIT_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiFormat {
    OpenaiChat,
    OpenaiResponses,
    AnthropicMessages,
}

impl ApiFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenaiChat => "openai_chat",
            Self::OpenaiResponses => "openai_responses",
            Self::AnthropicMessages => "anthropic_messages",
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderInput {
    pub id: Option<String>,
    pub name: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub api_format: ApiFormat,
    pub models: Vec<String>,
    #[serde(default)]
    pub plan_account_id: Option<String>,
    #[serde(default)]
    pub codex_account_id: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StoredConfig {
    pub base_url: String,
    pub api_key: String,
    pub api_format: ApiFormat,
    pub models: Vec<String>,
    #[serde(skip)]
    pub model_details: Vec<Value>,
    #[serde(skip)]
    pub default_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plan_account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_account_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSummary {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_format: ApiFormat,
    pub models: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_account_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayStatus {
    pub selected_id: Option<String>,
    pub key_configured: bool,
    pub providers: Vec<ProviderSummary>,
}

fn invalid(code: &str) -> AppError {
    AppError::Config(code.into())
}

fn validate(config: &StoredConfig) -> Result<(), AppError> {
    let url =
        url::Url::parse(&config.base_url).map_err(|_| invalid("QODER_GATEWAY_BASE_URL_INVALID"))?;
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
    {
        return Err(invalid("QODER_GATEWAY_BASE_URL_INVALID"));
    }
    if let Some(id) = &config.codex_account_id {
        if id.trim().is_empty()
            || id.len() > 256
            || id.chars().any(char::is_control)
            || config.plan_account_id.is_some()
            || config.base_url != crate::proxy::providers::CHATGPT_CODEX_BASE_URL
            || config.api_format != ApiFormat::OpenaiResponses
            || !config.api_key.is_empty()
        {
            return Err(invalid("QODER_GATEWAY_CODEX_BINDING_INVALID"));
        }
    } else if let Some(id) = &config.plan_account_id {
        if uuid::Uuid::parse_str(id).is_err()
            || config.base_url != API_BASE
            || config.api_format != ApiFormat::OpenaiResponses
            || !config.api_key.is_empty()
        {
            return Err(invalid("QODER_GATEWAY_PLAN_BINDING_INVALID"));
        }
    } else if config.api_key.trim().is_empty() || config.api_key.chars().any(char::is_control) {
        return Err(invalid("QODER_GATEWAY_API_KEY_REQUIRED"));
    }
    if config.models.is_empty() || config.models.len() > 200 {
        return Err(invalid("QODER_GATEWAY_MODELS_REQUIRED"));
    }
    let mut seen = HashSet::new();
    for model in &config.models {
        if model.is_empty()
            || model.len() > 256
            || model.trim() != model
            || model.chars().any(char::is_control)
            || !seen.insert(model)
        {
            return Err(invalid("QODER_GATEWAY_MODEL_INVALID"));
        }
    }
    Ok(())
}

fn config(provider: &Provider) -> Result<StoredConfig, AppError> {
    if provider.settings_config.get("config").is_some() {
        return native_config(provider);
    }
    // Only explicit credentials or plan account references, never an ambient login.
    if provider.category.as_deref() != Some("custom")
        || provider.uses_managed_account_auth()
        || provider
            .meta
            .as_ref()
            .is_some_and(|m| m.auth_binding.is_some())
    {
        return Err(invalid("QODER_GATEWAY_AUTH_UNSUPPORTED"));
    }
    let config: StoredConfig = serde_json::from_value(provider.settings_config.clone())
        .map_err(|_| invalid("QODER_GATEWAY_PROVIDER_INVALID"))?;
    validate(&config)?;
    Ok(config)
}

/// The existing CC editor representation, not a second provider/account store.
pub(crate) fn native_config(provider: &Provider) -> Result<StoredConfig, AppError> {
    let text = provider.settings_config["config"].as_str()
        .ok_or_else(|| invalid("NATIVE_PROVIDER_CONFIG_REQUIRED"))?;
    let doc = text.parse::<toml::Value>().map_err(|_| invalid("NATIVE_PROVIDER_CONFIG_INVALID"))?;
    let model = doc.get("model").and_then(toml::Value::as_str)
        .filter(|model| !model.trim().is_empty()).ok_or_else(|| invalid("NATIVE_MODEL_REQUIRED"))?;
    let account = provider.meta.as_ref().and_then(|m| m.managed_account_id_for("codex_oauth"));
    if provider.uses_managed_account_auth() && account.is_none() {
        return Err(invalid("NATIVE_EXPLICIT_MANAGED_ACCOUNT_REQUIRED"));
    }
    if provider.meta.as_ref().and_then(|m| m.auth_binding.as_ref())
        .is_some_and(|b| b.source == crate::provider::AuthBindingSource::ManagedAccount) && account.is_none() {
        return Err(invalid("NATIVE_MANAGED_AUTH_UNSUPPORTED"));
    }
    let active = doc.get("model_provider").and_then(toml::Value::as_str)
        .and_then(|id| doc.get("model_providers")?.get(id));
    let wire = active.and_then(|p| p.get("wire_api")).and_then(toml::Value::as_str).unwrap_or("responses");
    let declared = provider.meta.as_ref().and_then(|meta| meta.api_format.as_deref());
    let format = if declared == Some("anthropic") { ApiFormat::AnthropicMessages } else { match wire {
        "responses" => ApiFormat::OpenaiResponses,
        "chat" => ApiFormat::OpenaiChat,
        _ => return Err(invalid("NATIVE_PROTOCOL_UNSUPPORTED")),
    } };
    if declared.is_some_and(|format| !matches!(format, "anthropic" | "openai_chat" | "openai_responses")) {
        return Err(invalid("NATIVE_PROTOCOL_UNSUPPORTED"));
    }
    let mut details = native_model_details(provider, model)?;
    let mut models = vec![model.into()];
    for detail in &details {
        let id = detail["id"].as_str().expect("validated model id");
        if id != model { models.push(id.into()); }
    }
    let selected = provider.settings_config.pointer("/modelCatalog/models").and_then(Value::as_array)
        .and_then(|models| models.iter().find(|row| row["model"].as_str() == Some(model)));
    let effort = doc.get("model_reasoning_effort").and_then(toml::Value::as_str)
        .or_else(|| selected.and_then(|row| row["defaultReasoningLevel"].as_str()));
    if let Some(effort) = effort {
        let level = if effort == "none" { "off" } else { effort };
        if !matches!(level, "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max") { return Err(invalid("NATIVE_MODEL_EFFORT_UNSUPPORTED")); }
        if let Some(detail) = details.iter_mut().find(|row| row["id"] == model) {
            if detail.get("reasoningEfforts").is_none() {
                detail["reasoningEfforts"] = json!({level: if level == "off" { Value::Null } else { json!(effort) }});
            } else if detail["reasoningEfforts"].get(level).is_none() {
                return Err(invalid("NATIVE_DEFAULT_EFFORT_NOT_IN_MODEL_CATALOG"));
            }
        }
    }
    let value = StoredConfig {
        base_url: if account.is_some() { crate::proxy::providers::CHATGPT_CODEX_BASE_URL.into() }
            else { crate::codex_config::extract_codex_base_url(text).ok_or_else(|| invalid("NATIVE_BASE_URL_REQUIRED"))?.trim_end_matches('/').into() },
        api_key: if account.is_some() { String::new() }
            else { crate::codex_config::extract_codex_api_key(provider.settings_config.get("auth"), Some(text))
                .ok_or_else(|| invalid("NATIVE_API_KEY_REQUIRED"))? },
        api_format: format,
        models,
        model_details: details,
        default_effort: effort.map(|level| if level == "none" { "off" } else { level }.into()),
        plan_account_id: None,
        codex_account_id: account,
    };
    validate(&value)?;
    Ok(value)
}

fn native_model_details(provider: &Provider, default: &str) -> Result<Vec<Value>, AppError> {
    let mut rows = provider.settings_config.pointer("/modelCatalog/models")
        .map(|rows| rows.as_array().cloned().ok_or_else(|| invalid("NATIVE_MODEL_CATALOG_INVALID")))
        .transpose()?.unwrap_or_default();
    if !rows.iter().any(|row| row["model"].as_str() == Some(default)) { rows.insert(0, json!({"model":default})); }
    if rows.len() > 200 { return Err(invalid("NATIVE_MODEL_CATALOG_TOO_LARGE")); }
    let mut seen = HashSet::new();
    rows.into_iter().map(|row| {
        let id = row["model"].as_str().filter(|value| !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control))
            .ok_or_else(|| invalid("NATIVE_MODEL_INVALID"))?;
        if !seen.insert(id.to_string()) { return Err(invalid("NATIVE_MODEL_DUPLICATE")); }
        let mut model = json!({"id":id});
        if let Some(name) = row.get("displayName") { model["name"] = name.clone(); }
        if let Some(window) = row.get("contextWindow") {
            let value = window.as_u64().or_else(|| window.as_str()?.parse::<u64>().ok()).filter(|v| *v > 0)
                .ok_or_else(|| invalid("NATIVE_MODEL_CONTEXT_INVALID"))?;
            model["contextWindow"] = json!(value);
        }
        if let Some(input) = row.get("inputModalities") {
            let values = input.as_array().ok_or_else(|| invalid("NATIVE_MODEL_INPUT_INVALID"))?;
            if values.is_empty() || values.iter().any(|value| !matches!(value.as_str(), Some("text" | "image"))) { return Err(invalid("NATIVE_MODEL_INPUT_UNSUPPORTED")); }
            model["input"] = input.clone();
        }
        if let Some(levels) = row.get("reasoningLevels") {
            let levels = levels.as_array().ok_or_else(|| invalid("NATIVE_MODEL_EFFORT_INVALID"))?;
            let mut efforts = serde_json::Map::new();
            for value in levels {
                let value = value.as_str().ok_or_else(|| invalid("NATIVE_MODEL_EFFORT_INVALID"))?;
                let level = if value == "none" { "off" } else { value };
                if !matches!(level, "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max") { return Err(invalid("NATIVE_MODEL_EFFORT_UNSUPPORTED")); }
                efforts.insert(level.into(), if level == "off" { Value::Null } else { json!(value) });
            }
            model["reasoningEfforts"] = if efforts.is_empty() { json!(false) } else { Value::Object(efforts) };
        }
        Ok(model)
    }).collect()
}

fn summary(provider: Provider) -> Result<ProviderSummary, AppError> {
    let config = config(&provider)?;
    Ok(ProviderSummary {
        id: provider.id,
        name: provider.name,
        base_url: config.base_url,
        api_format: config.api_format,
        models: config.models,
        plan_account_id: config.plan_account_id,
        codex_account_id: config.codex_account_id,
    })
}

pub fn status(db: &Database) -> Result<GatewayStatus, AppError> {
    Ok(GatewayStatus {
        selected_id: db.get_current_provider(APP)?,
        key_configured: db
            .get_setting(TOKEN_KEY)?
            .is_some_and(|value| !value.trim().is_empty()),
        providers: db
            .get_all_providers(APP)?
            .into_values()
            .map(summary)
            .collect::<Result<_, _>>()?,
    })
}

pub fn save(db: &Database, input: ProviderInput) -> Result<GatewayStatus, AppError> {
    if input.api_format == ApiFormat::AnthropicMessages { return Err(invalid("QODER_GATEWAY_API_FORMAT_UNSUPPORTED")); }
    let _edit = EDIT_LOCK
        .lock()
        .map_err(|_| invalid("QODER_GATEWAY_EDIT_FAILED"))?;
    if input.name.trim().is_empty() || input.name.len() > 256 {
        return Err(invalid("QODER_GATEWAY_NAME_REQUIRED"));
    }
    let previous = input
        .id
        .as_deref()
        .map(|id| {
            db.get_provider_by_id(id, APP)?
                .ok_or_else(|| invalid("QODER_GATEWAY_PROVIDER_NOT_FOUND"))
        })
        .transpose()?;
    let api_key = if input.plan_account_id.is_some() || input.codex_account_id.is_some() {
        if input.api_key.is_some() {
            return Err(invalid("QODER_GATEWAY_PLAN_BINDING_INVALID"));
        }
        String::new()
    } else {
        match input.api_key {
            Some(key) => key,
            None => {
                config(
                    previous
                        .as_ref()
                        .ok_or_else(|| invalid("QODER_GATEWAY_API_KEY_REQUIRED"))?,
                )?
                .api_key
            }
        }
    };
    let config = StoredConfig {
        base_url: input.base_url.trim_end_matches('/').into(),
        api_key,
        api_format: input.api_format,
        models: input.models,
        model_details: vec![],
        default_effort: None,
        plan_account_id: input.plan_account_id,
        codex_account_id: input.codex_account_id,
    };
    validate(&config)?;
    let mut provider = Provider::with_id(
        previous
            .as_ref()
            .map(|p| p.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        input.name.trim().into(),
        serde_json::to_value(config).map_err(|_| invalid("QODER_GATEWAY_PROVIDER_INVALID"))?,
        None,
    );
    provider.category = Some("custom".into());
    db.save_provider(APP, &provider)?;
    status(db)
}

pub fn select(db: &Database, id: &str) -> Result<GatewayStatus, AppError> {
    let _edit = EDIT_LOCK
        .lock()
        .map_err(|_| invalid("QODER_GATEWAY_EDIT_FAILED"))?;
    let provider = db
        .get_provider_by_id(id, APP)?
        .ok_or_else(|| invalid("QODER_GATEWAY_PROVIDER_NOT_FOUND"))?;
    config(&provider)?;
    db.set_current_provider(APP, id)?;
    status(db)
}

pub fn remove(db: &Database, id: &str) -> Result<GatewayStatus, AppError> {
    let _edit = EDIT_LOCK
        .lock()
        .map_err(|_| invalid("QODER_GATEWAY_EDIT_FAILED"))?;
    if db.get_provider_by_id(id, APP)?.is_none() {
        return Err(invalid("QODER_GATEWAY_PROVIDER_NOT_FOUND"));
    }
    db.delete_provider(APP, id)?;
    status(db)
}

/// Explicit local setup only. Receiving an unauthenticated request must never create a key.
pub fn prepare_key(db: &Database) -> Result<String, AppError> {
    let _edit = EDIT_LOCK
        .lock()
        .map_err(|_| invalid("QODER_GATEWAY_EDIT_FAILED"))?;
    if let Some(key) = db
        .get_setting(TOKEN_KEY)?
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(key);
    }
    let key = format!("ccq-{}", uuid::Uuid::new_v4().simple());
    db.set_setting(TOKEN_KEY, &key)?;
    Ok(key)
}

pub fn authorize(db: &Database, headers: &axum::http::HeaderMap) -> Result<(), AppError> {
    let expected = db
        .get_setting(TOKEN_KEY)?
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| invalid("QODER_GATEWAY_NOT_CONFIGURED"))?;
    let supplied = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, token)| token.trim());
    if supplied != Some(expected.as_str()) {
        return Err(invalid("QODER_GATEWAY_UNAUTHORIZED"));
    }
    Ok(())
}

pub fn selected(db: &Database) -> Result<Provider, AppError> {
    selected_for(db, APP)
}

pub fn selected_for(db: &Database, app: &str) -> Result<Provider, AppError> {
    if !matches!(app, "qoder" | "dsh") { return Err(invalid("NATIVE_APP_UNSUPPORTED")); }
    let id = db
        .get_current_provider(app)?
        .ok_or_else(|| invalid("QODER_GATEWAY_NO_PROVIDER"))?;
    provider_for(db, app, &id)
}

pub fn provider_for(db: &Database, app: &str, id: &str) -> Result<Provider, AppError> {
    if !matches!(app, "qoder" | "dsh") { return Err(invalid("NATIVE_APP_UNSUPPORTED")); }
    let provider = db
        .get_provider_by_id(id, app)?
        .ok_or_else(|| invalid("QODER_GATEWAY_NO_PROVIDER"))?;
    config(&provider)?;
    Ok(provider)
}

pub fn model_list(provider: &Provider) -> Result<Value, AppError> {
    let config = config(provider)?;
    Ok(
        json!({"object":"list", "data": config.models.into_iter().map(|id| {
        json!({"id":id, "object":"model", "owned_by":format!("qoder-gateway:{}", provider.id)})
    }).collect::<Vec<_>>() }),
    )
}

pub struct RequestTarget {
    pub url: String,
    pub api_key: String,
    pub plan_account_id: Option<String>,
    pub codex_account_id: Option<String>,
}

pub fn plan_account_id(provider: &Provider) -> Result<Option<String>, AppError> {
    config(provider).map(|value| value.plan_account_id)
}

pub fn codex_account_id(provider: &Provider) -> Result<Option<String>, AppError> {
    config(provider).map(|value| value.codex_account_id)
}

/// Qoder selects the matching BYOK API; do not wrap prompts or infer another protocol.
pub fn request_target(
    provider: &Provider,
    requested_model: &str,
    requested_format: ApiFormat,
) -> Result<RequestTarget, AppError> {
    let config = config(provider)?;
    if config.api_format != requested_format {
        return Err(invalid("QODER_GATEWAY_API_FORMAT_MISMATCH"));
    }
    if !config.models.iter().any(|model| model == requested_model) {
        return Err(invalid("QODER_GATEWAY_MODEL_NOT_ALLOWED"));
    }
    let endpoint = match requested_format {
        ApiFormat::OpenaiChat => "chat/completions",
        ApiFormat::OpenaiResponses => "responses",
        ApiFormat::AnthropicMessages => return Err(invalid("QODER_GATEWAY_API_FORMAT_UNSUPPORTED")),
    };
    Ok(RequestTarget {
        url: format!("{}/{endpoint}", config.base_url),
        api_key: config.api_key,
        plan_account_id: config.plan_account_id,
        codex_account_id: config.codex_account_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderMeta;
    #[test]
    fn shared_native_editor_and_managed_account_bindings_use_existing_contracts() {
        let mut provider = Provider::with_id("standard".into(), "Standard editor".into(), json!({"auth":{"OPENAI_API_KEY":"fixture-key"},
            "config":"model = \"fixture-model\"\nmodel_provider = \"active\"\n[model_providers.active]\nbase_url = \"https://fixture.invalid/v1\"\nwire_api = \"responses\""}), None);
        assert_eq!(request_target(&provider, "fixture-model", ApiFormat::OpenaiResponses).unwrap().api_key, "fixture-key");
        let mut effort_provider = provider.clone();
        effort_provider.settings_config["config"] = json!(format!("model_reasoning_effort = \"high\"\n{}", effort_provider.settings_config["config"].as_str().unwrap()));
        let effort = native_config(&effort_provider).unwrap();
        assert_eq!(effort.default_effort.as_deref(), Some("high"));
        assert_eq!(effort.model_details[0]["reasoningEfforts"]["high"], "high");
        provider.meta = Some(ProviderMeta { provider_type: Some("codex_oauth".into()), auth_binding: Some(crate::provider::AuthBinding {
            source: crate::provider::AuthBindingSource::ManagedAccount, auth_provider: Some("codex_oauth".into()), account_id: Some("existing-cc-account".into()),
        }), ..Default::default() });
        provider.settings_config["auth"] = json!({});
        let target = request_target(&provider, "fixture-model", ApiFormat::OpenaiResponses).unwrap();
        assert_eq!(target.codex_account_id.as_deref(), Some("existing-cc-account"));
        assert!(target.api_key.is_empty());
        assert!(target.url.starts_with(crate::proxy::providers::CHATGPT_CODEX_BASE_URL));
        assert!(request_target(&provider, "different-model", ApiFormat::OpenaiResponses).is_err());
        provider.meta.as_mut().unwrap().auth_binding.as_mut().unwrap().account_id = None;
        assert!(native_config(&provider).is_err());
    }
    fn input() -> ProviderInput {
        ProviderInput {
            id: None,
            name: "Isolated fixture".into(),
            plan_account_id: None,
            codex_account_id: None,
            base_url: "http://127.0.0.1:19399/v1".into(),
            api_key: Some("synthetic-upstream-key".into()),
            api_format: ApiFormat::OpenaiChat,
            models: vec!["fixture-model".into()],
        }
    }

    #[test]
    fn explicit_cc_codex_binding_uses_the_managed_account_without_copying_a_key() {
        let db = Database::memory().unwrap();
        let mut value = input();
        value.codex_account_id = Some("cc-managed-account".into());
        value.base_url = crate::proxy::providers::CHATGPT_CODEX_BASE_URL.into();
        value.api_format = ApiFormat::OpenaiResponses;
        value.api_key = None;
        let status = save(&db, value).unwrap();
        let summary = &status.providers[0];
        assert_eq!(
            summary.codex_account_id.as_deref(),
            Some("cc-managed-account")
        );
        let serialized = serde_json::to_value(summary).unwrap();
        assert!(serialized.get("apiKey").is_none());
        let provider = db.get_provider_by_id(&summary.id, APP).unwrap().unwrap();
        let target =
            request_target(&provider, "fixture-model", ApiFormat::OpenaiResponses).unwrap();
        assert_eq!(
            target.url,
            format!(
                "{}/responses",
                crate::proxy::providers::CHATGPT_CODEX_BASE_URL
            )
        );
        assert!(target.api_key.is_empty());
        assert_eq!(
            target.codex_account_id.as_deref(),
            Some("cc-managed-account")
        );
    }

    #[test]
    fn managed_codex_binding_rejects_another_upstream_and_mixed_account_sources() {
        let db = Database::memory().unwrap();
        let mut value = input();
        value.codex_account_id = Some("cc-managed-account".into());
        value.api_key = None;
        assert!(save(&db, value).is_err());
        let mut value = input();
        value.codex_account_id = Some("cc-managed-account".into());
        value.plan_account_id = Some(uuid::Uuid::new_v4().to_string());
        value.api_key = None;
        value.base_url = crate::proxy::providers::CHATGPT_CODEX_BASE_URL.into();
        value.api_format = ApiFormat::OpenaiResponses;
        assert!(save(&db, value).is_err());
        assert!(status(&db).unwrap().providers.is_empty());
    }
    #[test]
    fn qoder_selection_and_models_never_fall_back_to_codex_or_expose_keys() {
        let db = Database::memory().unwrap();
        let codex = Provider::with_id(
            "codex-only".into(),
            "Existing".into(),
            json!({"unchanged":true}),
            None,
        );
        db.save_provider("codex", &codex).unwrap();
        db.set_current_provider("codex", &codex.id).unwrap();
        assert!(selected(&db).is_err());
        let created = save(&db, input()).unwrap();
        assert!(created.selected_id.is_none());
        let id = &created.providers[0].id;
        select(&db, id).unwrap();
        let provider = selected(&db).unwrap();
        assert_eq!(
            model_list(&provider).unwrap()["data"][0]["id"],
            "fixture-model"
        );
        assert!(!serde_json::to_string(&status(&db).unwrap())
            .unwrap()
            .contains("synthetic-upstream-key"));
        let target = request_target(&provider, "fixture-model", ApiFormat::OpenaiChat).unwrap();
        assert_eq!(target.api_key, "synthetic-upstream-key");
        assert_eq!(target.url, "http://127.0.0.1:19399/v1/chat/completions");
        assert!(request_target(&provider, "codex-model", ApiFormat::OpenaiChat).is_err());
        assert!(request_target(&provider, "fixture-model", ApiFormat::OpenaiResponses).is_err());
        remove(&db, id).unwrap();
        assert!(selected(&db).is_err());
        assert_eq!(db.get_current_provider("codex").unwrap(), Some(codex.id));
        assert_eq!(
            db.get_all_providers("codex").unwrap()["codex-only"].settings_config,
            json!({"unchanged":true})
        );
    }
    #[test]
    fn unauthenticated_requests_never_arm_the_gateway_or_reuse_another_key() {
        let db = Database::memory().unwrap();
        db.set_setting("claude_desktop_gateway_token", "unrelated-key")
            .unwrap();
        let mut headers = axum::http::HeaderMap::new();
        assert!(authorize(&db, &headers).is_err());
        assert!(db.get_setting(TOKEN_KEY).unwrap().is_none());
        let key = prepare_key(&db).unwrap();
        headers.insert("authorization", "Bearer unrelated-key".parse().unwrap());
        assert!(authorize(&db, &headers).is_err());
        headers.insert("authorization", format!("Bearer {key}").parse().unwrap());
        authorize(&db, &headers).unwrap();
        assert_eq!(prepare_key(&db).unwrap(), key);
    }
    #[test]
    fn invalid_providers_reject_before_persistence_and_managed_auth_is_not_inferred() {
        let db = Database::memory().unwrap();
        let mut invalid_input = input();
        invalid_input.base_url = "https://secret@invalid.example/v1".into();
        assert!(save(&db, invalid_input).is_err());
        let mut invalid_input = input();
        invalid_input.models.push("fixture-model".into());
        assert!(save(&db, invalid_input).is_err());
        assert!(status(&db).unwrap().providers.is_empty());
        let created = save(&db, input()).unwrap();
        let mut provider = db
            .get_provider_by_id(&created.providers[0].id, APP)
            .unwrap()
            .unwrap();
        provider.meta = Some(ProviderMeta {
            provider_type: Some("codex_oauth".into()),
            ..Default::default()
        });
        assert!(request_target(&provider, "fixture-model", ApiFormat::OpenaiChat).is_err());
    }
}
