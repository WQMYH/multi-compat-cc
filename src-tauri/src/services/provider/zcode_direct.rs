//! ZCode's native document adapter; accounts, routing and transactions stay in CC.
use crate::database::Database;
use crate::error::AppError;
use crate::live::engine::{digest, read_current, LiveFile};
use crate::live::patch::{Guarded, WholeFile};
use crate::mode::operation::{AppWrite, FileChange, OperationReport};
use crate::mode::state::{self, PendingTarget};
use crate::provider::{Provider, ZCodeProviderConfig};
use crate::zcode_config as native;
use serde_json::{json, Value};

pub(crate) const ROUTE_ID: &str = "cc-switch-zcode-route";
const SAVED_SELECTION: &str = "zcodeNativeDefault";

pub(crate) fn direct_provider_id() -> Result<Option<String>, AppError> {
    let doc = native::read_zcode_provider_config()?;
    let mut selection = doc.pointer("/config/defaultModelSelection").cloned();
    if selection
        .as_ref()
        .and_then(|value| value["providerId"].as_str())
        == Some(ROUTE_ID)
    {
        selection = state::written(&crate::live::engine::DeviceStore::for_device(), "zcode")?
            .and_then(|written| written.extra.get(SAVED_SELECTION).cloned());
    }
    if let Some(id) = selection
        .as_ref()
        .and_then(|value| value["providerId"].as_str())
    {
        return Ok(Some(id.into()));
    }
    Ok(None)
}

pub(crate) fn needs_routing(provider: &Provider) -> bool {
    provider.uses_managed_account_auth()
        || provider
            .meta
            .as_ref()
            .and_then(|meta| meta.auth_binding.as_ref())
            .is_some_and(|binding| {
                binding.source == crate::provider::AuthBindingSource::ManagedAccount
            })
        || provider
            .meta
            .as_ref()
            .is_some_and(|meta| meta.is_full_url == Some(true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> Provider {
        let mut provider = Provider::with_id(
            "cc-account".into(),
            "CC managed account".into(),
            json!({"auth":{},"config":"model = \"model-a\"\n",
                "modelCatalog":{"models":[{"model":"model-a"},{"model":"model-b"}]}}),
            None,
        );
        provider.meta = Some(serde_json::from_value(json!({"providerType":"codex_oauth",
            "authBinding":{"source":"managed_account","authProvider":"codex_oauth","accountId":"fixture-existing-account"}})).unwrap());
        provider
    }

    #[test]
    fn managed_account_publishes_models_through_cc_without_exporting_identity_or_tokens() {
        let provider = account();
        let projected = route_config(
            &provider,
            "http://127.0.0.1:12345/zcode/v1",
            "CC_LOCAL_PROXY",
        )
        .unwrap();
        assert_eq!(projected.kind, "openai");
        assert_eq!(
            projected.models.keys().cloned().collect::<Vec<_>>(),
            ["model-a", "model-b"]
        );
        let rule = native::provider_config_to_rule(ROUTE_ID, &projected);
        assert_eq!(rule["config"]["api"]["type"], "openai-responses");
        assert_eq!(rule["config"]["access"]["apiKey"], "CC_LOCAL_PROXY");
        assert!(!serde_json::to_string(&rule)
            .unwrap()
            .contains("fixture-existing-account"));
        assert_eq!(
            common_provider(&provider)
                .unwrap()
                .meta
                .unwrap()
                .managed_account_id_for("codex_oauth")
                .as_deref(),
            Some("fixture-existing-account")
        );
        assert!(direct_config(&provider).is_err());
        assert_eq!(provider.settings_config["auth"], json!({}));
    }

    #[test]
    fn imported_api_provider_uses_common_responses_and_preserves_catalog_selection() {
        let provider = Provider::with_id(
            "native-api".into(),
            "Native API".into(),
            json!({
            "kind":"openai", "options":{"baseURL":"https://fixture.invalid/v1","apiKey":"fixture-only"},
            "models":{"model-a":{},"model-b":{}}}),
            None,
        );
        let common = common_provider(&provider).unwrap();
        let mut request = json!({"model":"model-b","input":"fixture"});
        crate::proxy::providers::apply_codex_upstream_model(&common, &mut request);
        assert_eq!(request["model"], "model-b");
        assert_eq!(
            direct_config(&common)
                .unwrap()
                .models
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["model-a", "model-b"]
        );
        assert_eq!(
            direct_config(&common).unwrap().options.api_key.as_deref(),
            Some("fixture-only")
        );
    }

    #[test]
    fn model_catalog_projection_preserves_reasoning_levels_and_selected_default() {
        let mut provider = account();
        provider.settings_config["config"] =
            json!("model = \"model-a\"\nmodel_reasoning_effort = \"max\"\n");
        let levels = json!(["none", "minimal", "low", "medium", "high", "xhigh", "max"]);
        provider.settings_config["modelCatalog"]["models"] = json!([
            {"model":"model-a","contextWindow":400000,"reasoningLevels":levels,
             "defaultReasoningLevel":"medium"},
            {"model":"model-b","reasoningLevels":["low","high"]},
            {"model":"model-c","reasoningLevels":[]}
        ]);
        let projected = route_config(
            &provider,
            "http://127.0.0.1:12345/zcode/v1",
            "CC_LOCAL_PROXY",
        )
        .unwrap();
        assert_eq!(
            projected
                .models
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["model-a", "model-b", "model-c"]
        );
        assert_eq!(
            projected.models["model-a"].limit.as_ref().unwrap().context,
            Some(400000)
        );
        assert_eq!(
            projected.models["model-a"].reasoning.as_ref().unwrap(),
            &json!({"enabled":true,"variants":levels,"defaultVariant":"max"})
        );
        assert_eq!(
            projected.models["model-b"].reasoning.as_ref().unwrap()["variants"],
            json!(["low", "high"])
        );
        assert_eq!(
            projected.models["model-c"].reasoning.as_ref().unwrap()["enabled"],
            false
        );
    }

    #[test]
    fn managed_catalog_fills_missing_models_but_preserves_explicit_selection_and_efforts() {
        let fetched = crate::services::codex_oauth_models::parse_model_catalog(json!({"models":[
            {"slug":"model-a","supported_reasoning_levels":[{"effort":"none"},{"effort":"max"},{"effort":"ultra"}],"default_reasoning_level":"max"},
            {"slug":"model-b","supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}],"default_reasoning_level":"high"}
        ]}));
        let mut provider = account();
        provider
            .settings_config
            .as_object_mut()
            .unwrap()
            .remove("modelCatalog");
        add_managed_catalog(&mut provider, fetched.clone());
        let projected = route_config(
            &provider,
            "http://127.0.0.1:12345/zcode/v1",
            "CC_LOCAL_PROXY",
        )
        .unwrap();
        assert_eq!(projected.models.len(), 2);
        assert_eq!(
            projected.models["model-a"].reasoning.as_ref().unwrap()["variants"],
            json!(["none", "max", "ultra"])
        );
        provider.settings_config["modelCatalog"]["models"] = json!([
            {"model":"model-a","reasoningLevels":["none"],"defaultReasoningLevel":"none"}
        ]);
        add_managed_catalog(&mut provider, fetched);
        assert_eq!(
            provider.settings_config["modelCatalog"]["models"],
            json!([
                {"model":"model-a","reasoningLevels":["none"],"defaultReasoningLevel":"none"}
            ])
        );
        assert_eq!(
            route_config(
                &provider,
                "http://127.0.0.1:12345/zcode/v1",
                "CC_LOCAL_PROXY"
            )
            .unwrap()
            .models
            .len(),
            1
        );
    }

    #[test]
    fn common_config_keeps_explicit_default_and_rejects_invalid_account_or_protocol() {
        let mut provider = account();
        provider.settings_config["config"] =
            json!("model = \"model-b\"\nmodel_reasoning_effort = \"max\"\n");
        provider.settings_config["modelCatalog"]["models"] = json!([
            {"model":"model-a","reasoningLevels":["low"]},
            {"model":"model-b","reasoningLevels":["none","max"],"defaultReasoningLevel":"none"}
        ]);
        let projected = route_config(
            &provider,
            "http://127.0.0.1:12345/zcode/v1",
            "CC_LOCAL_PROXY",
        )
        .unwrap();
        assert_eq!(projected.models.first().unwrap().0, "model-b");
        assert_eq!(
            projected.models["model-b"].reasoning.as_ref().unwrap()["defaultVariant"],
            "max"
        );
        let mut unbound = provider.clone();
        unbound.meta.as_mut().unwrap().auth_binding = None;
        assert!(validate(&unbound).is_err());
        provider.meta.as_mut().unwrap().api_format = Some("unknown".into());
        assert!(validate(&provider).is_err());
        provider.meta.as_mut().unwrap().api_format = Some("anthropic".into());
        assert!(validate(&provider).is_err());
    }

    #[test]
    fn common_toml_errors_do_not_echo_credential_source_lines() {
        let mut provider = account();
        provider.settings_config["config"] = json!("model = \"model-a\"\nexperimental_bearer_token = \"fixture-secret-without-closing-quote\n");
        let errors = [
            common_provider(&provider).unwrap_err().to_string(),
            validate(&provider).unwrap_err().to_string(),
        ];
        for error in errors {
            assert!(!error.contains("fixture-secret"));
            assert!(!error.contains("experimental_bearer_token"));
            assert!(error.contains("Invalid ZCode common config"));
        }
    }
}

/// Imported native fragments use the same runtime representation as CC's editor.
pub(crate) fn common_provider(provider: &Provider) -> Result<Provider, AppError> {
    if provider
        .settings_config
        .get("config")
        .is_some_and(Value::is_string)
    {
        provider.settings_config["config"]
            .as_str()
            .unwrap()
            .parse::<toml::Value>()
            .map_err(|_| AppError::Config("Invalid ZCode common config".into()))?;
        return Ok(provider.clone());
    }
    let config: ZCodeProviderConfig = serde_json::from_value(provider.settings_config.clone())
        .map_err(|e| AppError::Config(format!("Invalid ZCode provider: {e}")))?;
    let rule = native::provider_config_to_rule(&provider.id, &config);
    let models = rule["config"]["modelOrder"]
        .as_array()
        .filter(|models| !models.is_empty())
        .ok_or_else(|| AppError::InvalidInput("ZCode provider has no selectable model".into()))?;
    let mut result = provider.clone();
    let wire = if config.kind == "openai-compatible" {
        "chat"
    } else {
        "responses"
    };
    let document = json!({"model":models[0],"model_provider":"cc",
        "model_providers":{"cc":{"name":provider.name,"base_url":config.options.base_url,
            "wire_api":wire}}});
    let text = toml::to_string(&document).map_err(|e| AppError::Config(e.to_string()))?;
    result.settings_config = json!({"auth":{"OPENAI_API_KEY":config.options.api_key},
        "config":text,"modelCatalog":{"models":models.iter().map(|id| {
            let mut row = json!({"model":id});
            if let Some(model) = id.as_str().and_then(|id| config.models.get(id)) {
                if let Some(context) = model.limit.as_ref().and_then(|limit| limit.context) { row["contextWindow"] = json!(context); }
                if let Some(reasoning) = &model.reasoning {
                    if let Some(levels) = reasoning.get("variants") { row["reasoningLevels"] = levels.clone(); }
                    if reasoning["enabled"] == false { row["reasoningLevels"] = json!([]); }
                    if let Some(default) = reasoning.get("defaultVariant") { row["defaultReasoningLevel"] = default.clone(); }
                }
            }
            row
        }).collect::<Vec<_>>()}});
    result.meta.get_or_insert_with(Default::default).api_format = Some(
        match config.kind.as_str() {
            "anthropic" => "anthropic",
            "openai-compatible" => "openai_chat",
            "openai" => "openai_responses",
            _ => return Err(AppError::InvalidInput("Unsupported ZCode API kind".into())),
        }
        .into(),
    );
    Ok(result)
}

pub(crate) fn direct_config(provider: &Provider) -> Result<ZCodeProviderConfig, AppError> {
    if needs_routing(provider) {
        return Err(AppError::InvalidInput(
            "ZCode managed accounts require CC routing".into(),
        ));
    }
    if !provider
        .settings_config
        .get("config")
        .is_some_and(Value::is_string)
    {
        return serde_json::from_value(provider.settings_config.clone())
            .map_err(|e| AppError::Config(format!("Invalid ZCode provider: {e}")));
    }
    projected(provider, None)
}

fn projected(
    provider: &Provider,
    route: Option<(&str, &str)>,
) -> Result<ZCodeProviderConfig, AppError> {
    let text = provider.settings_config["config"]
        .as_str()
        .ok_or_else(|| AppError::Config("ZCode common config is required".into()))?;
    let doc = text
        .parse::<toml::Value>()
        .map_err(|_| AppError::Config("Invalid ZCode common config".into()))?;
    let default = crate::proxy::providers::codex_provider_upstream_model(provider)
        .ok_or_else(|| AppError::InvalidInput("ZCode default model is required".into()))?;
    let account = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.managed_account_id_for("codex_oauth"));
    if needs_routing(provider)
        && provider.meta.as_ref().is_some_and(|meta| {
            meta.auth_binding.as_ref().is_some_and(|binding| {
                binding.source == crate::provider::AuthBindingSource::ManagedAccount
            }) || provider.uses_managed_account_auth()
        })
        && account.is_none()
    {
        return Err(AppError::InvalidInput(
            "ZCode requires an explicit supported CC managed account".into(),
        ));
    }
    let declared = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.api_format.as_deref());
    let wire = doc
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .and_then(|id| doc.get("model_providers")?.get(id)?.get("wire_api"))
        .and_then(toml::Value::as_str)
        .unwrap_or("responses");
    if !matches!(wire, "responses" | "chat")
        || declared
            .is_some_and(|kind| !matches!(kind, "openai_responses" | "openai_chat" | "anthropic"))
    {
        return Err(AppError::InvalidInput(
            "Unsupported ZCode API format".into(),
        ));
    }
    let kind = if crate::proxy::providers::codex_provider_uses_anthropic(provider) {
        "anthropic"
    } else if crate::proxy::providers::codex_provider_uses_chat_completions(provider) {
        "openai-compatible"
    } else {
        "openai"
    };
    if account.as_deref().is_some_and(|id| {
        id.trim().is_empty() || id.len() > 256 || id.chars().any(char::is_control)
    }) || (account.is_some() && kind != "openai")
    {
        return Err(AppError::InvalidInput(
            "Invalid ZCode CC managed account binding".into(),
        ));
    }
    let (base, key) = if account.is_some() {
        (
            crate::proxy::providers::CHATGPT_CODEX_BASE_URL.to_string(),
            String::new(),
        )
    } else {
        (
            crate::codex_config::extract_codex_base_url(text)
                .ok_or_else(|| AppError::InvalidInput("ZCode API URL is required".into()))?,
            crate::codex_config::extract_codex_api_key(
                provider.settings_config.get("auth"),
                Some(text),
            )
            .ok_or_else(|| AppError::InvalidInput("ZCode API key is required".into()))?,
        )
    };
    let url = url::Url::parse(&base)
        .map_err(|_| AppError::InvalidInput("Invalid ZCode API URL".into()))?;
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || (account.is_none() && (key.trim().is_empty() || key.chars().any(char::is_control)))
    {
        return Err(AppError::InvalidInput(
            "Invalid ZCode API configuration".into(),
        ));
    }
    let rows = provider
        .settings_config
        .pointer("/modelCatalog/models")
        .map(|rows| {
            rows.as_array()
                .ok_or_else(|| AppError::InvalidInput("Invalid ZCode model catalog".into()))
        })
        .transpose()?;
    let mut seen = std::collections::HashSet::new();
    if let Some(rows) = rows {
        for row in rows {
            let id = row["model"]
                .as_str()
                .ok_or_else(|| AppError::InvalidInput("Invalid ZCode model ID".into()))?;
            if id.trim().is_empty()
                || id.trim() != id
                || id.len() > 256
                || id.chars().any(char::is_control)
            {
                return Err(AppError::InvalidInput("Invalid ZCode model ID".into()));
            }
            if !seen.insert(id) {
                return Err(AppError::InvalidInput("Duplicate ZCode model ID".into()));
            }
        }
    }
    let mut models = crate::codex_config::codex_published_models(&provider.settings_config, text);
    models.retain(|id| id != &default);
    models.insert(0, default.clone());
    if models.len() > 200
        || models.iter().any(|id| {
            id.trim().is_empty()
                || id.trim() != id
                || id.len() > 256
                || id.chars().any(char::is_control)
        })
    {
        return Err(AppError::InvalidInput("Invalid ZCode model catalog".into()));
    }
    let mut definitions = serde_json::Map::new();
    for id in models {
        let mut model = json!({});
        if let Some(row) = rows.and_then(|rows| rows.iter().find(|row| row["model"] == id)) {
            if let Some(context) = row.get("contextWindow") {
                let context = context
                    .as_u64()
                    .or_else(|| context.as_str()?.parse().ok())
                    .filter(|value| *value > 0)
                    .ok_or_else(|| {
                        AppError::InvalidInput("Invalid ZCode model context window".into())
                    })?;
                model["limit"] = json!({"context":context});
            }
            let default_effort = (id == default)
                .then(|| {
                    doc.get("model_reasoning_effort")
                        .and_then(toml::Value::as_str)
                })
                .flatten()
                .or_else(|| row["defaultReasoningLevel"].as_str());
            if let Some(levels) = row.get("reasoningLevels") {
                let values = levels.as_array().ok_or_else(|| {
                    AppError::InvalidInput("Invalid ZCode reasoning levels".into())
                })?;
                model["reasoning"] = json!({"enabled":!values.is_empty(),"variants":levels});
                if let Some(effort) = default_effort {
                    if values.contains(&json!(effort)) {
                        model["reasoning"]["defaultVariant"] = json!(effort);
                    } else {
                        return Err(AppError::InvalidInput(
                            "ZCode default reasoning level is not in the model catalog".into(),
                        ));
                    }
                }
            } else if let Some(effort) = default_effort {
                model["reasoning"] =
                    json!({"enabled":true,"variants":[effort],"defaultVariant":effort});
            }
        } else if id == default {
            if let Some(effort) = doc
                .get("model_reasoning_effort")
                .and_then(toml::Value::as_str)
            {
                model["reasoning"] =
                    json!({"enabled":true,"variants":[effort],"defaultVariant":effort});
            }
        }
        definitions.insert(id, model);
    }
    let (base, key, kind) = route
        .map(|(base, key)| (base, key, "openai"))
        .unwrap_or((&base, &key, kind));
    let result: ZCodeProviderConfig =
        serde_json::from_value(json!({"name":provider.name,"enabled":true,
            "kind":kind,
            "options":{"baseURL":base,"apiKey":key},
            "models":definitions
        }))
        .map_err(|e| AppError::Config(e.to_string()))?;
    for model in result.models.values() {
        native::model_config(model)?;
    }
    Ok(result)
}

pub(crate) fn validate(provider: &Provider) -> Result<(), AppError> {
    projected(&common_provider(provider)?, None).map(|_| ())
}

pub(crate) fn route_config(
    provider: &Provider,
    base: &str,
    token: &str,
) -> Result<ZCodeProviderConfig, AppError> {
    projected(&common_provider(provider)?, Some((base, token)))
}

fn add_managed_catalog(provider: &mut Provider, fetched: Vec<Value>) {
    if let Some(rows) = provider
        .settings_config
        .pointer_mut("/modelCatalog/models")
        .and_then(Value::as_array_mut)
        .filter(|rows| !rows.is_empty())
    {
        for row in rows {
            if let Some(remote) = fetched
                .iter()
                .find(|remote| remote["model"] == row["model"])
            {
                for field in ["reasoningLevels", "defaultReasoningLevel"] {
                    if row.get(field).is_none() {
                        if let Some(value) = remote.get(field) {
                            if field != "defaultReasoningLevel"
                                || row
                                    .get("reasoningLevels")
                                    .and_then(Value::as_array)
                                    .is_none_or(|levels| levels.contains(value))
                            {
                                row[field] = value.clone();
                            }
                        }
                    }
                }
            }
        }
    } else {
        provider.settings_config["modelCatalog"] = json!({"models":fetched});
    }
}

pub(crate) async fn route_projection(
    db: &Database,
    manager: &crate::proxy::providers::codex_oauth_auth::CodexOAuthManager,
    provider: &Provider,
    base: &str,
    token: &str,
) -> Result<ZCodeProviderConfig, AppError> {
    let mut enriched = common_provider(provider)?;
    let needs_catalog = enriched
        .settings_config
        .pointer("/modelCatalog/models")
        .and_then(Value::as_array)
        .is_none_or(|rows| {
            rows.is_empty() || rows.iter().any(|row| row.get("reasoningLevels").is_none())
        });
    if let Some(account) = enriched
        .meta
        .as_ref()
        .and_then(|meta| meta.managed_account_id_for("codex_oauth"))
        .filter(|_| needs_catalog)
    {
        let fetched = async {
            let bundle = manager
                .get_valid_token_bundle_for_account(&account)
                .await
                .map_err(|_| ())?;
            crate::services::codex_oauth_models::fetch_model_catalog_with_token(
                &bundle.access_token,
                &bundle.chatgpt_account_id,
            )
            .await
            .map_err(|_| ())
        }
        .await;
        let current = db.get_provider_by_id(&provider.id, "zcode")?;
        if current.as_ref().is_none_or(|current| {
            current.settings_config != provider.settings_config
                || serde_json::to_value(&current.meta).ok()
                    != serde_json::to_value(&provider.meta).ok()
        }) {
            return Err(AppError::Config(
                "ZCode provider changed while fetching its model catalog".into(),
            ));
        }
        match fetched {
            Ok(models) if !models.is_empty() => {
                add_managed_catalog(&mut enriched, models);
            }
            _ => log::warn!("ZCode managed model catalog unavailable; keeping configured models"),
        }
    }
    route_config(&enriched, base, token)
}

pub(crate) enum Target<'a> {
    Direct(Option<&'a Provider>),
    Append(&'a Provider),
    Route(&'a Provider, &'a ZCodeProviderConfig),
}

pub(crate) fn run(
    db: &Database,
    operation: &str,
    projection: Option<Target<'_>>,
    mut target: PendingTarget,
) -> Result<OperationReport, AppError> {
    let write = AppWrite::begin(db, "zcode")?;
    let Some(projection) = projection else {
        return write.run(operation, &[], target);
    };
    let path = native::get_zcode_provider_config_path();
    native::with_zcode_file_lock(&path, || {
        let before = read_current(&path)?;
        let mut doc: Value = match &before {
            Some(bytes) => serde_json::from_slice(bytes)
                .map_err(|e| AppError::Config(format!("Invalid ZCode document: {e}")))?,
            None => json!({}),
        };
        native::validate_provider_document(&doc)?;
        let mut written = state::written(&write.store, "zcode")?.unwrap_or_default();
        match projection {
            Target::Route(provider, config) => {
                if !written.extra.contains_key(SAVED_SELECTION)
                    && doc
                        .pointer("/config/providerConfigRules/providerRules")
                        .and_then(Value::as_array)
                        .is_some_and(|rules| {
                            rules.iter().any(|rule| rule["providerId"] == ROUTE_ID)
                        })
                {
                    return Err(AppError::Config(
                        "ZCode CC route ID is already occupied; nothing written".into(),
                    ));
                }
                written.extra.entry(SAVED_SELECTION).or_insert_with(|| {
                    doc.pointer("/config/defaultModelSelection")
                        .cloned()
                        .unwrap_or(Value::Null)
                });
                if state::mode_state(&write.store, "zcode")?
                    .proxy_route
                    .as_deref()
                    != Some(provider.id.as_str())
                {
                    native::clear_projected_model_capabilities(&mut doc, ROUTE_ID);
                }
                if !needs_routing(provider) {
                    let direct = direct_config(provider)?;
                    native::upsert_provider_config(&mut doc, &provider.id, &direct)?;
                }
                native::upsert_provider_config(&mut doc, ROUTE_ID, config)?;
                native::apply_default_model_selection(&mut doc, ROUTE_ID, Some(config))?;
            }
            Target::Direct(provider) => {
                let owned = doc
                    .pointer("/config/defaultModelSelection/providerId")
                    .and_then(Value::as_str)
                    == Some(ROUTE_ID);
                let saved = written.extra.remove(SAVED_SELECTION);
                if saved.is_some() {
                    native::remove_provider_rule(&mut doc, ROUTE_ID);
                }
                if let Some(provider) = provider {
                    if native::is_native_official(provider) {
                        let previous = saved
                            .as_ref()
                            .or_else(|| doc.pointer("/config/defaultModelSelection"))
                            .filter(|selection| {
                                selection["providerId"] == provider.id
                                    && selection["modelId"]
                                        .as_str()
                                        .is_some_and(|id| !id.trim().is_empty())
                            });
                        let selection = match previous {
                            Some(selection) => selection.clone(),
                            None => native::native_official_selection(&provider.id)?,
                        };
                        doc.as_object_mut()
                            .expect("validated ZCode document")
                            .entry("config")
                            .or_insert_with(|| json!({}))["defaultModelSelection"] = selection;
                    } else {
                        let config = direct_config(provider)?;
                        native::upsert_provider_config(&mut doc, &provider.id, &config)?;
                        native::apply_default_model_selection(
                            &mut doc,
                            &provider.id,
                            Some(&config),
                        )?;
                    }
                } else if owned {
                    if let Some(config) = doc.get_mut("config").and_then(Value::as_object_mut) {
                        match saved.filter(|value| !value.is_null()) {
                            Some(selection) => {
                                config.insert("defaultModelSelection".into(), selection);
                            }
                            None => {
                                config.remove("defaultModelSelection");
                            }
                        }
                    }
                }
            }
            Target::Append(provider) => {
                let config = direct_config(provider)?;
                native::upsert_provider_config(&mut doc, &provider.id, &config)?;
            }
        }
        target.written = Some(written);
        let patch = Guarded {
            expected_pre: digest(before.as_deref()),
            then: WholeFile::Write(
                serde_json::to_vec_pretty(&doc).map_err(|e| AppError::Config(e.to_string()))?,
            ),
        };
        write.run(
            operation,
            &[FileChange {
                file: LiveFile::private(path.clone()),
                patch: &patch,
            }],
            target,
        )
    })
}
