use crate::{
    qoder_gateway::{self, GatewayStatus, ProviderInput},
    AppState,
};
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub async fn qoder_gateway_plan_models(
    account_id: String,
    state: State<'_, super::ChatgptPlanAuthState>,
) -> Result<Vec<crate::proxy::providers::chatgpt_plan_auth::inference::Model>, String> {
    let accounts = state.manager()?;
    crate::proxy::providers::chatgpt_plan_auth::inference::catalog(&accounts, &account_id)
        .await
        .map(|catalog| catalog.models)
}

#[tauri::command]
pub fn qoder_gateway_status(state: State<'_, AppState>) -> Result<GatewayStatus, String> {
    qoder_gateway::status(&state.db).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn qoder_gateway_save(
    input: ProviderInput,
    state: State<'_, AppState>,
) -> Result<GatewayStatus, String> {
    qoder_gateway::save(&state.db, input).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn qoder_gateway_select(
    id: String,
    state: State<'_, AppState>,
) -> Result<GatewayStatus, String> {
    qoder_gateway::select(&state.db, &id).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn qoder_gateway_remove(
    id: String,
    state: State<'_, AppState>,
) -> Result<GatewayStatus, String> {
    qoder_gateway::remove(&state.db, &id).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn qoder_gateway_prepare_key(state: State<'_, AppState>) -> Result<String, String> {
    qoder_gateway::prepare_key(&state.db).map_err(|e| e.to_string())
}
