//! ZCode's native document adapter; accounts, routing and transactions stay in CC.
use crate::database::Database;
use crate::error::AppError;
use crate::live::engine::{digest, read_current, LiveFile};
use crate::live::patch::{Guarded, WholeFile};
use crate::mode::operation::{AppWrite, FileChange, OperationReport};
use crate::mode::state::{self, PendingTarget};
use crate::native_provider_config::{native_config, ApiFormat};
use crate::provider::{Provider, ZCodeProviderConfig};
use crate::zcode_config as native;
use serde_json::{json, Value};

pub(crate) const ROUTE_ID: &str = "cc-switch-zcode-route";
const SAVED_SELECTION: &str = "zcodeNativeDefault";

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
            native_config(&common).unwrap().models,
            ["model-a", "model-b"]
        );
        assert_eq!(
            direct_config(&common).unwrap().options.api_key.as_deref(),
            Some("fixture-only")
        );
    }
}

/// Imported native fragments use the same runtime representation as CC's editor.
pub(crate) fn common_provider(provider: &Provider) -> Result<Provider, AppError> {
    if provider
        .settings_config
        .get("config")
        .is_some_and(Value::is_string)
    {
        native_config(provider)?;
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
        "config":text,"modelCatalog":{"models":models.iter().map(|id| json!({"model":id})).collect::<Vec<_>>()}});
    result.meta.get_or_insert_with(Default::default).api_format = Some(
        match config.kind.as_str() {
            "anthropic" => "anthropic",
            "openai-compatible" => "openai_chat",
            "openai" => "openai_responses",
            _ => return Err(AppError::InvalidInput("Unsupported ZCode API kind".into())),
        }
        .into(),
    );
    native_config(&result)?;
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
    let config = native_config(provider)?;
    projected(
        provider,
        config.base_url,
        config.api_key,
        config.api_format,
        config.models,
    )
}

fn projected(
    provider: &Provider,
    base: String,
    key: String,
    format: ApiFormat,
    models: Vec<String>,
) -> Result<ZCodeProviderConfig, AppError> {
    serde_json::from_value(json!({"name":provider.name,"enabled":true,
        "kind": match format { ApiFormat::OpenaiResponses=>"openai", ApiFormat::OpenaiChat=>"openai-compatible", ApiFormat::AnthropicMessages=>"anthropic" },
        "options":{"baseURL":base,"apiKey":key},
        "models":models.into_iter().map(|id| (id,json!({}))).collect::<serde_json::Map<_,_>>()
    })).map_err(|e| AppError::Config(e.to_string()))
}

pub(crate) fn route_config(
    provider: &Provider,
    base: &str,
    token: &str,
) -> Result<ZCodeProviderConfig, AppError> {
    let config = native_config(&common_provider(provider)?)?;
    projected(
        provider,
        base.into(),
        token.into(),
        ApiFormat::OpenaiResponses,
        config.models,
    )
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
                if !needs_routing(provider) {
                    let direct = direct_config(provider)?;
                    native::upsert_provider_rule(
                        &mut doc,
                        &provider.id,
                        native::provider_config_to_rule(&provider.id, &direct),
                    );
                }
                native::upsert_provider_rule(
                    &mut doc,
                    ROUTE_ID,
                    native::provider_config_to_rule(ROUTE_ID, config),
                );
                native::apply_default_model_selection(&mut doc, ROUTE_ID)?;
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
                    let config = direct_config(provider)?;
                    native::upsert_provider_rule(
                        &mut doc,
                        &provider.id,
                        native::provider_config_to_rule(&provider.id, &config),
                    );
                    native::apply_default_model_selection(&mut doc, &provider.id)?;
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
                native::upsert_provider_rule(
                    &mut doc,
                    &provider.id,
                    native::provider_config_to_rule(&provider.id, &config),
                );
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
