//! The DeepSeek account login shared by DSH Web and Desktop.

use tauri::Manager;

#[derive(serde::Serialize)]
pub struct DshOAuthRouteStatus {
    active: bool,
}

/// Publish an explicitly selected ChatGPT account card into DSH.
#[tauri::command(rename_all = "camelCase")]
pub async fn activate_dsh_oauth_provider(
    app_handle: tauri::AppHandle,
    provider_id: String,
) -> Result<(), String> {
    let state = app_handle
        .try_state::<crate::store::AppState>()
        .ok_or_else(|| "Application state is unavailable".to_string())?
        .inner()
        .clone();
    state
        .proxy_service
        .activate_dsh_oauth_provider(&state, &provider_id)
        .await
}

/// Report only whether this card's current listener binding is active.
#[tauri::command(rename_all = "camelCase")]
pub async fn get_dsh_oauth_route_status(
    app_handle: tauri::AppHandle,
    provider_id: String,
) -> Result<DshOAuthRouteStatus, String> {
    let state = app_handle
        .try_state::<crate::store::AppState>()
        .ok_or_else(|| "Application state is unavailable".to_string())?
        .inner()
        .clone();
    Ok(DshOAuthRouteStatus {
        active: state
            .proxy_service
            .dsh_oauth_route_active(&provider_id)
            .await,
    })
}

/// The DeepSeek account (OAuth) login shared by DSH Web and Desktop, with profile and balance.
#[tauri::command]
pub async fn dsh_account_status() -> Result<crate::services::dsh_account::DshAccountStatus, String>
{
    crate::services::dsh_account::status().await
}

/// Sign out of the DeepSeek account for both DSH Web and Desktop.
#[tauri::command]
pub async fn dsh_account_sign_out() -> Result<(), String> {
    crate::services::dsh_account::sign_out().await
}

/// Start a DeepSeek account browser sign-in; the UI opens `authUrl` and polls.
#[tauri::command]
pub async fn dsh_account_login_start() -> Result<crate::services::dsh_account::DshLoginStart, String>
{
    crate::services::dsh_account::start_login().await
}

/// `false` while waiting for the browser, `true` once the shared login is stored.
#[tauri::command(rename_all = "camelCase")]
pub fn dsh_account_login_poll(flow_id: String) -> Result<bool, String> {
    crate::services::dsh_account::poll_login(&flow_id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn dsh_account_login_cancel(flow_id: String) {
    crate::services::dsh_account::cancel_login(&flow_id)
}
