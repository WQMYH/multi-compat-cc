//! Validate account-bound DSH drafts and build the official writer's native input.
//! The caller owns account lookup, local credential generation, and publication.

use crate::error::AppError;
use crate::provider::{AuthBindingSource, Provider};
use serde_json::{json, Map, Value};
use url::{Host, Url};

const AUTH_PROVIDER: &str = "codex_oauth";
const PROTOCOL: &str = "openai-responses";
const ROUTE_KEY_PREFIX: &str = "ccsw-dsh-v1-";

fn invalid(code: &str) -> AppError {
    AppError::InvalidInput(code.into())
}

/// Recognize account-bound rows even when an incomplete binding needs repair.
pub(crate) fn is_managed(provider: &Provider) -> bool {
    provider.is_codex_oauth()
        || provider
            .meta
            .as_ref()
            .and_then(|meta| meta.auth_binding.as_ref())
            .is_some_and(|binding| {
                binding.source == AuthBindingSource::ManagedAccount
                    || binding.auth_provider.as_deref() == Some(AUTH_PROVIDER)
            })
}

/// Resolve the explicit account reference without selecting a default account.
pub(crate) fn account_id(provider: &Provider) -> Result<&str, AppError> {
    provider
        .meta
        .as_ref()
        .filter(|meta| meta.provider_type.as_deref() == Some(AUTH_PROVIDER))
        .and_then(|meta| meta.auth_binding.as_ref())
        .filter(|binding| {
            binding.source == AuthBindingSource::ManagedAccount
                && binding.auth_provider.as_deref() == Some(AUTH_PROVIDER)
        })
        .and_then(|binding| binding.account_id.as_deref())
        .filter(|id| valid_id(id))
        .ok_or_else(|| invalid("DSH_OAUTH_EXPLICIT_ACCOUNT_REQUIRED"))
}

/// List the model IDs allowed to use this card's account.
pub(crate) fn model_ids(provider: &Provider) -> Result<Vec<&str>, AppError> {
    Ok(draft_models(provider)?.keys().map(String::as_str).collect())
}

/// Validate a persisted draft before it can be activated. The synthetic values
/// are used only to reuse DSH's validation for its native writer input shape.
pub(crate) fn validate_draft(provider: &Provider) -> Result<(), AppError> {
    account_id(provider)?;
    let models = draft_models(provider)?;
    let config = native_input(provider, models, "http://127.0.0.1", "validation-only");
    crate::dsh_config::validate_provider(&provider.id, &config)
}

/// Build the config passed to the existing DSH writer. Keep the original draft
/// in CC's database. The writer stores the caller's random local Bearer through
/// its native credential reference and applies its normal profile rules.
pub(crate) fn project(
    provider: &Provider,
    local_base_url: &str,
    local_bearer: &str,
) -> Result<Value, AppError> {
    account_id(provider)?;
    let models = draft_models(provider)?;
    let endpoint = loopback_endpoint(&provider.id, local_base_url)
        .ok_or_else(|| invalid("DSH_OAUTH_PROXY_ENDPOINT_INVALID"))?;
    if !(32..=128).contains(&local_bearer.len())
        || !local_bearer
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(invalid("DSH_OAUTH_LOCAL_BEARER_INVALID"));
    }
    let config = native_input(provider, models, endpoint.as_str(), local_bearer);
    crate::dsh_config::validate_provider(&provider.id, &config)?;
    Ok(config)
}

/// Recognize the card's projection in the official DSH reader's JSON shape.
pub(crate) fn is_own_projection(provider_id: &str, config: &Value) -> bool {
    config["api"].as_str() == Some(PROTOCOL)
        && config["options"]["baseURL"]
            .as_str()
            .is_some_and(|base| loopback_endpoint(provider_id, base).is_some())
        && has_local_bearer(config)
}

/// Reserve local route credentials at provider import and IPC boundaries even
/// if the surrounding native configuration has subsequently changed.
pub(crate) fn has_local_bearer(config: &Value) -> bool {
    config["options"]["apiKey"]
        .as_str()
        .and_then(|key| key.strip_prefix(ROUTE_KEY_PREFIX))
        .is_some_and(|key| {
            key.len() == 64
                && key
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

fn loopback_endpoint(provider_id: &str, base: &str) -> Option<Url> {
    if provider_id.is_empty() || provider_id.contains('/') {
        return None;
    }
    let endpoint = Url::parse(base).ok()?;
    let loopback = match endpoint.host() {
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    if endpoint.scheme() != "http"
        || !loopback
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || endpoint.path() != format!("/dsh/oauth/{provider_id}/v1")
    {
        return None;
    }
    Some(endpoint)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.trim() == id && !id.chars().any(char::is_control)
}

fn draft_models(provider: &Provider) -> Result<&Map<String, Value>, AppError> {
    let draft = provider
        .settings_config
        .as_object()
        .ok_or_else(|| invalid("DSH_OAUTH_DRAFT_INVALID"))?;
    if draft.get("api").and_then(Value::as_str) != Some(PROTOCOL)
        || draft.iter().any(|(field, value)| match field.as_str() {
            "api" | "models" => false,
            "name" => !value.is_string(),
            "options" => !value.as_object().is_some_and(Map::is_empty),
            _ => true,
        })
    {
        return Err(invalid("DSH_OAUTH_DRAFT_INVALID"));
    }
    let models = draft
        .get("models")
        .and_then(Value::as_object)
        .filter(|models| !models.is_empty())
        .ok_or_else(|| invalid("DSH_OAUTH_MODEL_REQUIRED"))?;
    if models
        .iter()
        .any(|(id, model)| !valid_id(id) || !valid_model(model))
    {
        return Err(invalid("DSH_OAUTH_MODEL_INVALID"));
    }
    Ok(models)
}

fn valid_model(model: &Value) -> bool {
    model.as_object().is_some_and(|fields| {
        fields.iter().all(|(field, value)| match field.as_str() {
            "name" | "defaultReasoningEffort" => value.is_string(),
            "reasoning" => value.is_boolean(),
            "input" => string_array(value),
            "modalities" => value.as_object().is_some_and(|modalities| {
                modalities.iter().all(|(direction, kinds)| {
                    matches!(direction.as_str(), "input" | "output") && string_array(kinds)
                })
            }),
            "limit" => value.as_object().is_some_and(|limits| {
                limits.iter().all(|(kind, capacity)| {
                    matches!(kind.as_str(), "context" | "output")
                        && (capacity.is_null() || capacity.as_u64().is_some_and(|n| n > 0))
                })
            }),
            "reasoningEfforts" => {
                value == &Value::Bool(false)
                    || value.as_object().is_some_and(|efforts| {
                        efforts
                            .values()
                            .all(|effort| effort.is_null() || effort.is_string())
                    })
            }
            _ => false,
        })
    })
}

fn string_array(value: &Value) -> bool {
    value
        .as_array()
        .is_some_and(|values| values.iter().all(Value::is_string))
}

fn native_input(
    provider: &Provider,
    models: &Map<String, Value>,
    base: &str,
    bearer: &str,
) -> Value {
    json!({
        "name": provider.name,
        "api": PROTOCOL,
        "models": models,
        "options": { "baseURL": base, "apiKey": bearer },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{AuthBinding, ProviderMeta};

    const BASE: &str = "http://127.0.0.1:15721/dsh/oauth/chatgpt-card/v1";
    const LOCAL_BEARER: &str = "synthetic-local-credential-for-one-card";

    fn draft() -> Provider {
        let mut provider = Provider::with_id(
            "chatgpt-card".into(),
            "ChatGPT card".into(),
            json!({
                "api": PROTOCOL,
                "models": {"model-one": {}, "model-two": {"reasoning": true}},
            }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            provider_type: Some(AUTH_PROVIDER.into()),
            auth_binding: Some(AuthBinding {
                source: AuthBindingSource::ManagedAccount,
                auth_provider: Some(AUTH_PROVIDER.into()),
                account_id: Some("synthetic-account".into()),
            }),
            ..Default::default()
        });
        provider
    }

    #[test]
    fn projects_explicit_models_with_only_the_local_bearer() {
        let provider = draft();
        let before = serde_json::to_value(&provider).unwrap();
        validate_draft(&provider).unwrap();
        assert_eq!(account_id(&provider).unwrap(), "synthetic-account");
        let mut ids = model_ids(&provider).unwrap();
        ids.sort_unstable();
        assert_eq!(ids, ["model-one", "model-two"]);
        let projected = project(&provider, BASE, LOCAL_BEARER).unwrap();
        assert_eq!(projected["models"], provider.settings_config["models"]);
        assert_eq!(
            projected["options"],
            json!({"baseURL": BASE, "apiKey": LOCAL_BEARER})
        );
        assert!(!projected.to_string().contains("synthetic-account"));
        assert_eq!(project(&provider, BASE, LOCAL_BEARER).unwrap(), projected);
        assert_eq!(serde_json::to_value(&provider).unwrap(), before);
    }

    #[test]
    fn rejects_implicit_accounts_and_non_draft_credentials() {
        let mut missing = draft();
        missing
            .meta
            .as_mut()
            .unwrap()
            .auth_binding
            .as_mut()
            .unwrap()
            .account_id = None;
        assert!(is_managed(&missing));
        assert!(validate_draft(&missing).is_err());

        let mut legacy = draft();
        legacy.settings_config = json!({"config": "model = 'old'", "auth": {}});
        assert!(project(&legacy, BASE, LOCAL_BEARER).is_err());

        let mut injected = draft();
        injected.settings_config["models"]["model-one"]["headers"] =
            json!({"Authorization": "synthetic-external-token"});
        assert!(project(&injected, BASE, LOCAL_BEARER).is_err());

        let mut empty = draft();
        empty.settings_config["models"] = json!({});
        assert!(validate_draft(&empty).is_err());
    }

    #[test]
    fn accepts_only_the_matching_loopback_endpoint_and_local_bearer_shape() {
        let provider = draft();
        for url in [
            "http://example.invalid:15721/dsh/oauth/chatgpt-card/v1",
            "http://127.0.0.1:15721/dsh/oauth/another-card/v1",
            "http://127.0.0.1:15721/dsh/oauth/chatgpt-card/v1?key=synthetic",
            "http://localhost:15721/dsh/oauth/chatgpt-card/v1",
        ] {
            assert!(project(&provider, url, LOCAL_BEARER).is_err());
        }
        assert!(project(&provider, BASE, "short").is_err());
    }

    #[test]
    fn recognizes_own_native_readback_and_rejects_replaced_configuration() {
        let provider = draft();
        let key = format!("{ROUTE_KEY_PREFIX}{}", "a".repeat(64));
        let config = project(&provider, BASE, &key).unwrap();
        assert!(is_own_projection(&provider.id, &config));
        assert!(!is_own_projection("another-card", &config));

        let mut replaced = config.clone();
        replaced["api"] = json!("openai-completions");
        assert!(!is_own_projection(&provider.id, &replaced));

        let mut replaced = config.clone();
        replaced["options"]["baseURL"] =
            json!("http://example.invalid:15721/dsh/oauth/chatgpt-card/v1");
        assert!(!is_own_projection(&provider.id, &replaced));

        for replacement in [
            LOCAL_BEARER.to_owned(),
            format!("{ROUTE_KEY_PREFIX}{}", "a".repeat(63)),
            format!("{ROUTE_KEY_PREFIX}{}", "A".repeat(64)),
        ] {
            let mut replaced = config.clone();
            replaced["options"]["apiKey"] = json!(replacement);
            assert!(!is_own_projection(&provider.id, &replaced));
        }
    }
}
