//! ZCode joins the shared CC Responses data plane and explicit routing mode.
use super::{handlers, server::ProxyState, ProxyError};
use crate::app_config::AppType;
use axum::{
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde_json::json;

pub(super) fn routes() -> Router<ProxyState> {
    Router::new().route("/zcode/v1/responses", post(responses))
}

async fn responses(
    State(state): State<ProxyState>,
    request: Request,
) -> Result<Response, ProxyError> {
    let mode = crate::mode::current::mode_state(&AppType::ZCode);
    if !mode.is_proxy() || !mode.attached {
        return Ok((
            StatusCode::CONFLICT,
            Json(json!({"error":{
                "code":"ZCODE_ROUTING_DISABLED","dispatchStarted":false,"retrySafe":true
            }})),
        )
            .into_response());
    }
    handlers::handle_responses_for_app(state, request, AppType::ZCode, "ZCode", "zcode").await
}
