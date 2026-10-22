//! Shared native provider parsing used by DSH. No gateway or account store.
use crate::{error::AppError, provider::Provider};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiFormat {
    OpenaiChat,
    OpenaiResponses,
    AnthropicMessages,
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

fn invalid(code: &str) -> AppError {
    AppError::Config(code.into())
}

fn validate(config: &StoredConfig) -> Result<(), AppError> {
    // Preserve the legacy field without reviving the removed parallel login flow.
    if config.plan_account_id.is_some() {
        return Err(invalid("QODER_GATEWAY_PLAN_BINDING_INVALID"));
    }
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
            || config.base_url != crate::proxy::providers::CHATGPT_CODEX_BASE_URL
            || config.api_format != ApiFormat::OpenaiResponses
            || !config.api_key.is_empty()
        {
            return Err(invalid("QODER_GATEWAY_CODEX_BINDING_INVALID"));
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

/// The existing CC editor representation, not a second provider/account store.
pub(crate) fn native_config(provider: &Provider) -> Result<StoredConfig, AppError> {
    let text = provider.settings_config["config"]
        .as_str()
        .ok_or_else(|| invalid("NATIVE_PROVIDER_CONFIG_REQUIRED"))?;
    let doc = text
        .parse::<toml::Value>()
        .map_err(|_| invalid("NATIVE_PROVIDER_CONFIG_INVALID"))?;
    let model = doc
        .get("model")
        .and_then(toml::Value::as_str)
        .filter(|model| !model.trim().is_empty())
        .ok_or_else(|| invalid("NATIVE_MODEL_REQUIRED"))?;
    let account = provider
        .meta
        .as_ref()
        .and_then(|m| m.managed_account_id_for("codex_oauth"));
    if provider.uses_managed_account_auth() && account.is_none() {
        return Err(invalid("NATIVE_EXPLICIT_MANAGED_ACCOUNT_REQUIRED"));
    }
    if provider
        .meta
        .as_ref()
        .and_then(|m| m.auth_binding.as_ref())
        .is_some_and(|b| b.source == crate::provider::AuthBindingSource::ManagedAccount)
        && account.is_none()
    {
        return Err(invalid("NATIVE_MANAGED_AUTH_UNSUPPORTED"));
    }
    let active = doc
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .and_then(|id| doc.get("model_providers")?.get(id));
    let wire = active
        .and_then(|p| p.get("wire_api"))
        .and_then(toml::Value::as_str)
        .unwrap_or("responses");
    let declared = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.api_format.as_deref());
    let format = if declared == Some("anthropic") {
        ApiFormat::AnthropicMessages
    } else {
        match wire {
            "responses" => ApiFormat::OpenaiResponses,
            "chat" => ApiFormat::OpenaiChat,
            _ => return Err(invalid("NATIVE_PROTOCOL_UNSUPPORTED")),
        }
    };
    if declared
        .is_some_and(|format| !matches!(format, "anthropic" | "openai_chat" | "openai_responses"))
    {
        return Err(invalid("NATIVE_PROTOCOL_UNSUPPORTED"));
    }
    let mut details = native_model_details(provider, model)?;
    let mut models = vec![model.into()];
    for detail in &details {
        let id = detail["id"].as_str().expect("validated model id");
        if id != model {
            models.push(id.into());
        }
    }
    let selected = provider
        .settings_config
        .pointer("/modelCatalog/models")
        .and_then(Value::as_array)
        .and_then(|models| {
            models
                .iter()
                .find(|row| row["model"].as_str() == Some(model))
        });
    let effort = doc
        .get("model_reasoning_effort")
        .and_then(toml::Value::as_str)
        .or_else(|| selected.and_then(|row| row["defaultReasoningLevel"].as_str()));
    if let Some(effort) = effort {
        let level = if effort == "none" { "off" } else { effort };
        if !matches!(
            level,
            "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
        ) {
            return Err(invalid("NATIVE_MODEL_EFFORT_UNSUPPORTED"));
        }
        if let Some(detail) = details.iter_mut().find(|row| row["id"] == model) {
            if detail.get("reasoningEfforts").is_none() {
                detail["reasoningEfforts"] =
                    json!({level: if level == "off" { Value::Null } else { json!(effort) }});
            } else if detail["reasoningEfforts"].get(level).is_none() {
                return Err(invalid("NATIVE_DEFAULT_EFFORT_NOT_IN_MODEL_CATALOG"));
            }
        }
    }
    let value = StoredConfig {
        base_url: if account.is_some() {
            crate::proxy::providers::CHATGPT_CODEX_BASE_URL.into()
        } else {
            crate::codex_config::extract_codex_base_url(text)
                .ok_or_else(|| invalid("NATIVE_BASE_URL_REQUIRED"))?
                .trim_end_matches('/')
                .into()
        },
        api_key: if account.is_some() {
            String::new()
        } else {
            crate::codex_config::extract_codex_api_key(
                provider.settings_config.get("auth"),
                Some(text),
            )
            .ok_or_else(|| invalid("NATIVE_API_KEY_REQUIRED"))?
        },
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
    let mut rows = provider
        .settings_config
        .pointer("/modelCatalog/models")
        .map(|rows| {
            rows.as_array()
                .cloned()
                .ok_or_else(|| invalid("NATIVE_MODEL_CATALOG_INVALID"))
        })
        .transpose()?
        .unwrap_or_default();
    if !rows
        .iter()
        .any(|row| row["model"].as_str() == Some(default))
    {
        rows.insert(0, json!({"model":default}));
    }
    if rows.len() > 200 {
        return Err(invalid("NATIVE_MODEL_CATALOG_TOO_LARGE"));
    }
    let mut seen = HashSet::new();
    rows.into_iter()
        .map(|row| {
            let id = row["model"]
                .as_str()
                .filter(|value| {
                    !value.trim().is_empty()
                        && value.len() <= 256
                        && !value.chars().any(char::is_control)
                })
                .ok_or_else(|| invalid("NATIVE_MODEL_INVALID"))?;
            if !seen.insert(id.to_string()) {
                return Err(invalid("NATIVE_MODEL_DUPLICATE"));
            }
            let mut model = json!({"id":id});
            if let Some(name) = row.get("displayName") {
                model["name"] = name.clone();
            }
            if let Some(window) = row.get("contextWindow") {
                let value = window
                    .as_u64()
                    .or_else(|| window.as_str()?.parse::<u64>().ok())
                    .filter(|v| *v > 0)
                    .ok_or_else(|| invalid("NATIVE_MODEL_CONTEXT_INVALID"))?;
                model["contextWindow"] = json!(value);
            }
            if let Some(input) = row.get("inputModalities") {
                let values = input
                    .as_array()
                    .ok_or_else(|| invalid("NATIVE_MODEL_INPUT_INVALID"))?;
                if values.is_empty()
                    || values
                        .iter()
                        .any(|value| !matches!(value.as_str(), Some("text" | "image")))
                {
                    return Err(invalid("NATIVE_MODEL_INPUT_UNSUPPORTED"));
                }
                model["input"] = input.clone();
            }
            if let Some(levels) = row.get("reasoningLevels") {
                let levels = levels
                    .as_array()
                    .ok_or_else(|| invalid("NATIVE_MODEL_EFFORT_INVALID"))?;
                let mut efforts = serde_json::Map::new();
                for value in levels {
                    let value = value
                        .as_str()
                        .ok_or_else(|| invalid("NATIVE_MODEL_EFFORT_INVALID"))?;
                    // Codex's delegated ultra mode has no DSH thinking-level equivalent.
                    if value == "ultra" {
                        continue;
                    }
                    let level = if value == "none" { "off" } else { value };
                    if !matches!(
                        level,
                        "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
                    ) {
                        return Err(invalid("NATIVE_MODEL_EFFORT_UNSUPPORTED"));
                    }
                    efforts.insert(
                        level.into(),
                        if level == "off" {
                            Value::Null
                        } else {
                            json!(value)
                        },
                    );
                }
                model["reasoningEfforts"] = if efforts.is_empty() {
                    json!(false)
                } else {
                    Value::Object(efforts)
                };
            }
            Ok(model)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderMeta;
    #[test]
    fn legacy_plan_registration_is_preserved_but_not_reactivated() {
        let config: StoredConfig = serde_json::from_value(json!({
            "baseUrl":"https://api.openai.com/v1",
            "apiKey":"",
            "apiFormat":"openai_responses",
            "models":["fixture-model"],
            "planAccountId":"d572c0ba-b5ac-4a94-ad08-fab7a672f4b6"
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(&config).unwrap()["planAccountId"],
            "d572c0ba-b5ac-4a94-ad08-fab7a672f4b6"
        );
        assert!(validate(&config)
            .unwrap_err()
            .to_string()
            .contains("QODER_GATEWAY_PLAN_BINDING_INVALID"));
    }

    #[test]
    fn shared_native_editor_and_managed_account_bindings_use_existing_contracts() {
        let mut provider = Provider::with_id(
            "standard".into(),
            "Standard editor".into(),
            json!({"auth":{"OPENAI_API_KEY":"fixture-key"},
            "config":"model = \"fixture-model\"\nmodel_provider = \"active\"\n[model_providers.active]\nbase_url = \"https://fixture.invalid/v1\"\nwire_api = \"responses\""}),
            None,
        );
        assert_eq!(native_config(&provider).unwrap().api_key, "fixture-key");
        let mut effort_provider = provider.clone();
        effort_provider.settings_config["config"] = json!(format!(
            "model_reasoning_effort = \"high\"\n{}",
            effort_provider.settings_config["config"].as_str().unwrap()
        ));
        let effort = native_config(&effort_provider).unwrap();
        assert_eq!(effort.default_effort.as_deref(), Some("high"));
        assert_eq!(effort.model_details[0]["reasoningEfforts"]["high"], "high");
        provider.meta = Some(ProviderMeta {
            provider_type: Some("codex_oauth".into()),
            auth_binding: Some(crate::provider::AuthBinding {
                source: crate::provider::AuthBindingSource::ManagedAccount,
                auth_provider: Some("codex_oauth".into()),
                account_id: Some("existing-cc-account".into()),
            }),
            ..Default::default()
        });
        provider.settings_config["auth"] = json!({});
        let target = native_config(&provider).unwrap();
        assert_eq!(
            target.codex_account_id.as_deref(),
            Some("existing-cc-account")
        );
        assert!(target.api_key.is_empty());
        assert!(target
            .base_url
            .starts_with(crate::proxy::providers::CHATGPT_CODEX_BASE_URL));
        assert_eq!(target.models, ["fixture-model"]);
        provider
            .meta
            .as_mut()
            .unwrap()
            .auth_binding
            .as_mut()
            .unwrap()
            .account_id = None;
        assert!(native_config(&provider).is_err());
    }
}
