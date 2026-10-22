//! Public ChatGPT plan inference only; never resolve an ambient Codex login.
use super::{
    accounts::{AccessLease, PlanAccounts},
    client, read_json,
};
use serde::Serialize;
use serde_json::{
    value::{to_raw_value, RawValue},
    Value,
};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

pub const API_BASE: &str = "https://api.openai.com/v1";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    pub display_name: String,
}

// Credentials accompany this exact catalog in memory, never in a serializable view.
pub(crate) struct Catalog {
    pub lease: AccessLease,
    pub models: Vec<Model>,
}

pub(crate) async fn catalog(accounts: &Arc<PlanAccounts>, id: &str) -> Result<Catalog, String> {
    catalog_at(accounts, id, API_BASE).await
}

pub(crate) async fn catalog_at(
    accounts: &Arc<PlanAccounts>,
    id: &str,
    base: &str,
) -> Result<Catalog, String> {
    #[cfg(not(test))]
    if base != API_BASE {
        return Err("CHATGPT_PLAN_ENDPOINT_INVALID".into());
    }
    let lease = accounts.lease_for(id).await?;
    let url = format!("{base}/models");
    let response = client(&url)
        .map_err(|error| error.to_string())?
        .get(&url)
        .timeout(std::time::Duration::from_secs(30))
        .bearer_auth(lease.token())
        .send()
        .await
        .map_err(|_| "CHATGPT_PLAN_CATALOG_UNAVAILABLE")?;
    let value = read_json(response)
        .await
        .map_err(|error| error.to_string())?;
    let mut seen = HashSet::new();
    let models = value
        .get("models")
        .and_then(Value::as_array)
        .ok_or("CHATGPT_PLAN_CATALOG_INVALID")?
        .iter()
        .filter(|model| model.get("visibility").and_then(Value::as_str) == Some("list"))
        .map(|model| {
            let id = model
                .get("slug")
                .and_then(Value::as_str)
                .ok_or("CHATGPT_PLAN_CATALOG_INVALID")?;
            let display_name = model
                .get("display_name")
                .and_then(Value::as_str)
                .ok_or("CHATGPT_PLAN_CATALOG_INVALID")?;
            if id.is_empty()
                || id.len() > 256
                || id.trim() != id
                || id.chars().any(char::is_control)
                || display_name.trim().is_empty()
                || !seen.insert(id.to_owned())
            {
                return Err("CHATGPT_PLAN_CATALOG_INVALID");
            }
            Ok(Model {
                id: id.into(),
                display_name: display_name.into(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    lease.ensure_current().await?;
    Ok(Catalog { lease, models })
}

/// Reject unsupported plan inputs and preserve client tools in the public input format.
pub(crate) fn prepare_request(body: &Value, raw: &[u8]) -> Result<Option<Vec<u8>>, &'static str> {
    if body.get("store") != Some(&Value::Bool(false))
        || body.get("stream") != Some(&Value::Bool(true))
        || !body.get("input").is_some_and(Value::is_array)
    {
        return Err("QODER_GATEWAY_PLAN_REQUIRES_STATELESS_STREAM");
    }
    for field in [
        "background",
        "conversation",
        "max_output_tokens",
        "max_tool_calls",
        "metadata",
        "moderation",
        "multi_agent",
        "prompt",
        "prompt_cache_retention",
        "safety_identifier",
        "temperature",
        "top_logprobs",
        "top_p",
        "truncation",
        "user",
        "previous_response_id",
    ] {
        if body.get(field).is_some() {
            return Err("QODER_GATEWAY_PLAN_FIELD_UNSUPPORTED");
        }
    }
    for item in body["input"].as_array().expect("validated array") {
        if item.get("role").and_then(Value::as_str) == Some("system") {
            return Err("QODER_GATEWAY_PLAN_SYSTEM_ITEM_UNSUPPORTED");
        }
        if item.get("type").and_then(Value::as_str) == Some("additional_tools") {
            if item.get("role").and_then(Value::as_str) != Some("developer") {
                return Err("QODER_GATEWAY_PLAN_TOOLS_UNSUPPORTED");
            }
            validate_tools(
                item.get("tools")
                    .ok_or("QODER_GATEWAY_PLAN_TOOLS_UNSUPPORTED")?,
            )?;
        }
    }
    let tools = match body.get("tools") {
        Some(tools) => {
            validate_tools(tools)?;
            tools.as_array().expect("validated tools")
        }
        None => return Ok(None),
    };
    if !tools.iter().any(client_tool) {
        return Ok(None);
    }
    // Preserve opaque schemas/history as raw JSON; Value numbers can lose precision.
    let invalid = |_: serde_json::Error| "QODER_GATEWAY_JSON_INVALID";
    let mut fields: BTreeMap<String, &RawValue> = serde_json::from_slice(raw).map_err(invalid)?;
    let raw_tools = *fields.get("tools").ok_or("QODER_GATEWAY_JSON_INVALID")?;
    let raw_input = *fields.get("input").ok_or("QODER_GATEWAY_JSON_INVALID")?;
    let raw_tools: Vec<&RawValue> = serde_json::from_str(raw_tools.get()).map_err(invalid)?;
    let mut input: Vec<&RawValue> = serde_json::from_str(raw_input.get()).map_err(invalid)?;
    if raw_tools.len() != tools.len() {
        return Err("QODER_GATEWAY_JSON_INVALID");
    }
    let mut client_tools = Vec::new();
    let mut retained = Vec::new();
    for (raw_tool, tool) in raw_tools.into_iter().zip(tools) {
        if client_tool(tool) {
            client_tools.push(raw_tool);
        } else {
            retained.push(raw_tool);
        }
    }
    #[derive(Serialize)]
    struct AdditionalTools<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        role: &'static str,
        tools: Vec<&'a RawValue>,
    }
    let added = to_raw_value(&AdditionalTools {
        kind: "additional_tools",
        role: "developer",
        tools: client_tools,
    })
    .map_err(invalid)?;
    // Eager client tools apply to the whole input, before the preserved history.
    input.insert(0, &added);
    let input = to_raw_value(&input).map_err(invalid)?;
    let tools = to_raw_value(&retained).map_err(invalid)?;
    fields.insert("input".into(), &input);
    fields.insert("tools".into(), &tools);
    serde_json::to_vec(&fields).map(Some).map_err(invalid)
}

fn client_tool(tool: &Value) -> bool {
    matches!(
        tool.get("type").and_then(Value::as_str),
        Some("function" | "custom")
    )
}

fn validate_tools(value: &Value) -> Result<(), &'static str> {
    for tool in value
        .as_array()
        .ok_or("QODER_GATEWAY_PLAN_TOOLS_UNSUPPORTED")?
    {
        let supported = match tool.get("type").and_then(Value::as_str) {
            Some("web_search") => true,
            Some("namespace") => tool
                .get("tools")
                .and_then(Value::as_array)
                .is_some_and(|tools| tools.iter().all(client_tool)),
            _ => client_tool(tool),
        };
        if !supported {
            return Err("QODER_GATEWAY_PLAN_TOOLS_UNSUPPORTED");
        }
    }
    Ok(())
}
