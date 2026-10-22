use crate::proxy::providers::chatgpt_plan_auth::{
    accounts::{AccountsView, LoginView, PlanAccounts},
    Discovery,
};
use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

pub struct ChatgptPlanAuthState(pub Result<Arc<PlanAccounts>, String>);

impl ChatgptPlanAuthState {
    pub fn manager(&self) -> Result<Arc<PlanAccounts>, String> {
        self.0.as_ref().cloned().map_err(Clone::clone)
    }
}

#[tauri::command]
pub async fn chatgpt_plan_status(
    state: State<'_, ChatgptPlanAuthState>,
) -> Result<AccountsView, String> {
    Ok(state.manager()?.status().await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn chatgpt_plan_start_login(
    app: AppHandle,
    state: State<'_, ChatgptPlanAuthState>,
    account_id: Option<String>,
    label: String,
) -> Result<LoginView, String> {
    let manager = state.manager()?;
    let discovery = Discovery::official()
        .await
        .map_err(|error| error.to_string())?;
    let launch = manager
        .begin(discovery, account_id.as_deref(), &label)
        .await?;
    if app
        .opener()
        .open_url(&launch.authorize_url, None::<String>)
        .is_err()
    {
        let _ = manager.cancel(&launch.view.attempt_id).await;
        // Opener diagnostics can contain the sensitive authorization URL.
        return Err("CHATGPT_PLAN_BROWSER_OPEN_FAILED".into());
    }
    Ok(launch.view)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn chatgpt_plan_cancel_login(
    state: State<'_, ChatgptPlanAuthState>,
    attempt_id: String,
) -> Result<AccountsView, String> {
    state.manager()?.cancel(&attempt_id).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn chatgpt_plan_select(
    state: State<'_, ChatgptPlanAuthState>,
    account_id: Option<String>,
) -> Result<AccountsView, String> {
    state.manager()?.select(account_id).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn chatgpt_plan_sign_out(
    state: State<'_, ChatgptPlanAuthState>,
    account_id: String,
) -> Result<AccountsView, String> {
    state.manager()?.sign_out(&account_id).await
}
