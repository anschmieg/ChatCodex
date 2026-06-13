//! OAuth 2.1 / MCP 2025-11-25 discovery and JWKS endpoints.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::state::AuthState;

pub async fn oauth_authorization_server(State(state): State<AuthState>) -> impl IntoResponse {
    let cfg = state.config();
    let body = serde_json::json!({
        "issuer": cfg.issuer(),
        "authorization_endpoint": cfg.authorization_endpoint(),
        "token_endpoint": cfg.token_endpoint(),
        "registration_endpoint": cfg.registration_endpoint(),
        "jwks_uri": cfg.jwks_uri(),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "token_endpoint_auth_methods_supported": ["none", "client_secret_post"],
        "code_challenge_methods_supported": ["S256"],
        "scopes_supported": ["mcp:tools"],
        "authorization_response_iss_parameter_supported": true,
        "authorization_details_types_supported": [],
        "dpop_signing_alg_values_supported": [],
    });
    (StatusCode::OK, Json(body))
}

pub async fn oauth_protected_resource(State(state): State<AuthState>) -> impl IntoResponse {
    let cfg = state.config();
    let body = serde_json::json!({
        "resource": cfg.resource_indicator(),
        "authorization_servers": [cfg.issuer()],
        "scopes_supported": ["mcp:tools"],
        "bearer_methods_supported": ["header"],
    });
    (StatusCode::OK, Json(body))
}

pub async fn jwks(State(state): State<AuthState>) -> impl IntoResponse {
    let jwks = state.keyring().jwks().await;
    (StatusCode::OK, Json(jwks))
}
