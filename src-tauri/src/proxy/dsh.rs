//! DSH uses CC's Responses handler, provider catalog and explicit mode control.
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
    Router::new()
        .route("/dsh/v1/responses", post(responses))
        .route("/dsh/:provider_id/v1/messages", post(reapply_required))
        .route("/dsh/:provider_id/v1/responses", post(reapply_required))
        .route(
            "/dsh/:provider_id/v1/chat/completions",
            post(reapply_required),
        )
}
async fn responses(
    State(state): State<ProxyState>,
    request: Request,
) -> Result<Response, ProxyError> {
    let mode = crate::mode::current::mode_state(&AppType::Dsh);
    if !mode.is_proxy() || !mode.attached {
        return Ok((StatusCode::CONFLICT, Json(json!({"error":{"code":"DSH_ROUTING_DISABLED","dispatchStarted":false,"retrySafe":true}}))).into_response());
    }
    handlers::handle_responses_for_app(state, request, AppType::Dsh, "DSH", "dsh").await
}
async fn reapply_required() -> impl IntoResponse {
    (
        StatusCode::CONFLICT,
        Json(json!({"error":{
            "code":"DSH_REAPPLY_NATIVE_PROTOCOL_REQUIRED",
            "message":"Select Routing explicitly in CC Switch to apply the native Responses configuration. No request was forwarded.",
            "dispatchStarted":false,"retrySafe":true
        }})),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::Request,
    };
    use tower::Service;
    #[tokio::test]
    async fn source11_dsh_old_routes_fail_before_dispatch_and_do_not_enable_modes() {
        let db = std::sync::Arc::new(crate::Database::memory().unwrap());
        let mut router = routes().with_state(ProxyState::for_test(db.clone()));
        let response = router
            .clone()
            .call(
                Request::post("/dsh/old-row/v1/messages")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error"]["dispatchStarted"], false);
        assert!(db.get_all_providers("dsh").unwrap().is_empty());
        let response = router
            .call(
                Request::post("/dsh/v1/responses")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }
}
