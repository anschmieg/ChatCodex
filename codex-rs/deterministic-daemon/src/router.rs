//! Axum router: `/healthz` and `/rpc`.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use deterministic_protocol::methods::Method;
use deterministic_protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse, ResponseEnvelope};
use std::sync::Arc;

use crate::handlers;
use crate::persistence::Store;

/// Shared application state.
pub struct AppState {
    pub store: Store,
}

/// Build the Axum router.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/rpc", post(rpc_handler))
        .with_state(state)
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn rpc_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    if req.jsonrpc != "2.0" {
        return Json(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: req.id,
            result: None,
            error: Some(JsonRpcError {
                code: -32600,
                message: "invalid jsonrpc version".into(),
                data: None,
            }),
        });
    }

    let method = match Method::parse_method(&req.method) {
        Some(m) => m,
        None => {
            return Json(JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: req.id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32601,
                    message: format!("method not found: {}", req.method),
                    data: None,
                }),
            });
        }
    };

    match handlers::dispatch(method, req.params, &state.store) {
        Ok((result, run_state)) => {
            let audit_id = format!("aud_{}", uuid::Uuid::new_v4());
            let envelope = ResponseEnvelope {
                ok: true,
                result,
                run_state,
                warnings: vec![],
                audit_id,
            };
            Json(JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: req.id,
                result: Some(
                    serde_json::to_value(envelope).unwrap_or(serde_json::Value::Null),
                ),
                error: None,
            })
        }
        Err(e) => Json(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: req.id,
            result: None,
            error: Some(JsonRpcError {
                code: -32000,
                message: format!("{e:#}"),
                data: None,
            }),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http::header;
    use tower::ServiceExt;

    fn test_app() -> (Router, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let state = Arc::new(AppState { store });
        (build_router(state), dir)
    }

    #[tokio::test]
    async fn healthz_ok() {
        let (app, _dir) = test_app();
        let resp = app
            .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn rpc_unknown_method() {
        let (app, _dir) = test_app();
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "1",
            "method": "turn.start",
            "params": {}
        });
        let resp = app
            .oneshot(
                Request::post("/rpc")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let rpc_resp: JsonRpcResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(rpc_resp.error.is_some());
        assert!(rpc_resp.error.unwrap().message.contains("method not found"));
    }

    #[tokio::test]
    async fn rpc_response_has_envelope() {
        let (app, _dir) = test_app();
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "1",
            "method": "run.prepare",
            "params": {
                "workspaceId": "/tmp/test",
                "userGoal": "fix bug",
                "focusPaths": []
            }
        });
        let resp = app
            .oneshot(
                Request::post("/rpc")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let rpc_resp: JsonRpcResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(rpc_resp.error.is_none());

        // Verify the envelope shape
        let envelope: ResponseEnvelope =
            serde_json::from_value(rpc_resp.result.unwrap()).unwrap();
        assert!(envelope.ok);
        assert!(envelope.run_state.is_some());
        assert!(envelope.audit_id.starts_with("aud_"));
    }
}
