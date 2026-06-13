//! Bearer-JWT validation middleware for `/mcp`.

use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::Response;

use crate::state::AuthState;
use crate::storage::now;

pub async fn require_bearer(
    State(state): State<AuthState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    if request.uri().path() == "/healthz" {
        return Ok(next.run(request).await);
    }
    let token = match request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.strip_prefix("Bearer "))
    {
        Some(value) => value,
        None => return Err(StatusCode::UNAUTHORIZED),
    };
    let claims = state
        .keyring()
        .verify(token, now())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let claims = match claims {
        Some(claims) => claims,
        None => return Err(StatusCode::UNAUTHORIZED),
    };
    let jti = match claims.get("jti").and_then(serde_json::Value::as_str) {
        Some(value) => value,
        None => return Err(StatusCode::UNAUTHORIZED),
    };
    if !state
        .store()
        .jti_is_active(jti, now())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(request).await)
}
