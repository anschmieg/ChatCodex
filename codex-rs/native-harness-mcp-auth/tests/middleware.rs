//! Tests for the bearer-JWT middleware that protects `/mcp`.

#![allow(clippy::expect_used)]

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::routing::get;
use codex_native_harness_mcp_auth::AuthConfig;
use codex_native_harness_mcp_auth::AuthState;
use codex_native_harness_mcp_auth::keyring::Keyring;
use codex_native_harness_mcp_auth::middleware::require_bearer;
use codex_native_harness_mcp_auth::storage::Store;
use tower::ServiceExt;

fn test_state() -> AuthState {
    let store = Store::open_in_memory().expect("store");
    let config = AuthConfig {
        public_base_url: "https://codex.example.test".to_string(),
        data_dir: std::path::PathBuf::from("/tmp"),
        cf_access_team: "https://team.example.test".to_string(),
        cf_access_aud: "aud-test".to_string(),
        access_ttl: std::time::Duration::from_secs(60),
        refresh_ttl: std::time::Duration::from_secs(600),
        allow_client_credentials: false,
    };
    let keyring =
        Keyring::load_or_create(store.clone(), config.issuer(), config.resource_indicator())
            .expect("keyring");
    let cf = codex_native_harness_mcp_auth::cf_access::CfAccessVerifier::new(
        config.cf_access_certs_uri(),
        config.cf_access_aud.clone(),
    );
    AuthState::new_for_test(config, store, keyring, cf)
}

async fn echo() -> &'static str {
    "ok"
}

#[tokio::test]
async fn missing_authorization_header_is_rejected() {
    let state = test_state();
    let app = Router::new()
        .route("/protected", get(echo))
        .layer(axum::middleware::from_fn_with_state(state, require_bearer));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/protected")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn malformed_bearer_is_rejected() {
    let state = test_state();
    let app = Router::new()
        .route("/protected", get(echo))
        .layer(axum::middleware::from_fn_with_state(state, require_bearer));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/protected")
                .header("Authorization", "Bearer not-a-jwt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn healthz_is_allowed_without_bearer() {
    let state = test_state();
    let app = Router::new()
        .route("/healthz", get(echo))
        .route("/protected", get(echo))
        .layer(axum::middleware::from_fn_with_state(state, require_bearer));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 64)
            .await
            .unwrap(),
        axum::body::to_bytes(axum::body::Body::from("ok"), 64)
            .await
            .unwrap()
    );
}
