//! End-to-end discovery tests for the auth layer's well-known endpoints.

#![allow(clippy::expect_used)]

use codex_native_harness_mcp_auth::AuthConfig;
use codex_native_harness_mcp_auth::AuthState;
use codex_native_harness_mcp_auth::keyring::Keyring;
use codex_native_harness_mcp_auth::storage::Store;

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

#[tokio::test]
async fn discovery_authorization_server_includes_required_fields() {
    use axum::Router;
    use axum::routing::get;
    let state = test_state();
    let app = Router::new()
        .route(
            "/.well-known/oauth-authorization-server",
            get(codex_native_harness_mcp_auth::well_known::oauth_authorization_server),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let resp = reqwest::get(format!(
        "http://{addr}/.well-known/oauth-authorization-server"
    ))
    .await
    .unwrap();
    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["issuer"], "https://codex.example.test");
    assert_eq!(
        body["authorization_endpoint"],
        "https://codex.example.test/oauth/authorize"
    );
    assert_eq!(
        body["token_endpoint"],
        "https://codex.example.test/oauth/token"
    );
    assert_eq!(
        body["registration_endpoint"],
        "https://codex.example.test/oauth/register"
    );
    assert_eq!(
        body["jwks_uri"],
        "https://codex.example.test/.well-known/jwks.json"
    );
    let response_types = body["response_types_supported"].as_array().unwrap();
    assert!(response_types.contains(&serde_json::Value::String("code".to_string())));
    let grant_types = body["grant_types_supported"].as_array().unwrap();
    assert!(grant_types.contains(&serde_json::Value::String("authorization_code".to_string())));
    assert!(grant_types.contains(&serde_json::Value::String("refresh_token".to_string())));
    assert_eq!(body["code_challenge_methods_supported"][0], "S256");
    let scopes = body["scopes_supported"].as_array().unwrap();
    assert!(scopes.contains(&serde_json::Value::String("mcp:tools".to_string())));
    assert_eq!(body["authorization_response_iss_parameter_supported"], true);
    server.abort();
}

#[tokio::test]
async fn discovery_protected_resource_identifies_the_resource() {
    use axum::Router;
    use axum::routing::get;
    let state = test_state();
    let app = Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(codex_native_harness_mcp_auth::well_known::oauth_protected_resource),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let resp = reqwest::get(format!(
        "http://{addr}/.well-known/oauth-protected-resource"
    ))
    .await
    .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["resource"], "https://codex.example.test/mcp");
    let auth_servers = body["authorization_servers"].as_array().unwrap();
    assert!(auth_servers.contains(&serde_json::Value::String(
        "https://codex.example.test".to_string()
    )));
    server.abort();
}

#[tokio::test]
async fn jwks_endpoint_exposes_signing_key() {
    use axum::Router;
    use axum::routing::get;
    let state = test_state();
    let app = Router::new()
        .route(
            "/.well-known/jwks.json",
            get(codex_native_harness_mcp_auth::well_known::jwks),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let resp = reqwest::get(format!("http://{addr}/.well-known/jwks.json"))
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let keys = body["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 1);
    let jwk = &keys[0];
    assert_eq!(jwk["kty"], "RSA");
    assert_eq!(jwk["alg"], "RS256");
    assert!(jwk["n"].as_str().unwrap().len() > 100);
    assert_eq!(jwk["e"], "AQAB");
    server.abort();
}
