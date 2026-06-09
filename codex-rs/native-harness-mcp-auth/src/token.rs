//! `/oauth/token` (authorization-code grant and refresh-token grant),
//! `/oauth/introspect`, and `/oauth/revoke`.

use axum::Form;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;

use crate::state::AuthState;
use crate::storage::AccessTokenJtiRecord;
use crate::storage::RefreshTokenRecord;
use crate::storage::hash_token;
use crate::storage::new_opaque_token;
use crate::storage::now;

const REFRESH_TTL_SECS: i64 = 60 * 60 * 24 * 30;

#[derive(Debug, Deserialize)]
pub struct TokenForm {
    pub grant_type: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub redirect_uri: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub code_verifier: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub resource: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
    pub refresh_token: String,
    pub scope: String,
}

#[derive(Debug, Serialize)]
pub struct TokenError {
    pub error: String,
    pub error_description: String,
}

pub async fn token(
    State(state): State<AuthState>,
    Form(form): Form<TokenForm>,
) -> impl IntoResponse {
    match form.grant_type.as_str() {
        "authorization_code" => handle_authorization_code(&state, form).await,
        "refresh_token" => handle_refresh_token(&state, form).await,
        other => token_error_response(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            format!("grant_type '{other}' is not supported"),
        ),
    }
}

async fn handle_authorization_code(state: &AuthState, form: TokenForm) -> axum::response::Response {
    let code = match form.code.as_ref() {
        Some(value) => value,
        None => return token_error(StatusCode::BAD_REQUEST, "invalid_request", "code is required"),
    };
    let client_id = match form.client_id.as_ref() {
        Some(value) => value,
        None => {
            return token_error(
                StatusCode::BAD_REQUEST,
                "invalid_client",
                "client_id is required",
            )
        }
    };
    let redirect_uri = match form.redirect_uri.as_ref() {
        Some(value) => value,
        None => {
            return token_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "redirect_uri is required",
            )
        }
    };
    let verifier = match form.code_verifier.as_ref() {
        Some(value) => value,
        None => {
            return token_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "code_verifier is required",
            )
        }
    };
    let client = match state.store().get_client(client_id) {
        Ok(Some(client)) => client,
        _ => return token_error(StatusCode::UNAUTHORIZED, "invalid_client", "unknown client"),
    };
    let code_hash = hash_token(code);
    let record = match state.store().consume_auth_code(&code_hash, now()) {
        Ok(Some(record)) => record,
        _ => {
            return token_error(
                StatusCode::BAD_REQUEST,
                "invalid_grant",
                "authorization code is invalid or expired",
            )
        }
    };
    if record.client_id != client.client_id || record.redirect_uri != *redirect_uri {
        return token_error(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "client_id or redirect_uri does not match the code",
        );
    }
    let challenge = match record.code_challenge.as_deref() {
        Some(value) => value,
        None => {
            return token_error(
                StatusCode::BAD_REQUEST,
                "invalid_grant",
                "code has no associated PKCE challenge",
            )
        }
    };
    if record.code_challenge_method.as_deref() != Some("S256") {
        return token_error(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "only S256 PKCE is supported",
        );
    }
    let computed = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    if !constant_time_eq(computed.as_bytes(), challenge.as_bytes()) {
        return token_error(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "code_verifier does not match the challenge",
        );
    }
    let (access_token, refresh) = match issue_tokens(state, &client.client_id, &record.subject, &record.scope).await {
        Ok(value) => value,
        Err(error) => {
            return token_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                error.to_string(),
            )
        }
    };
    let response = TokenResponse {
        access_token,
        token_type: "Bearer",
        expires_in: state.config().access_ttl.as_secs() as i64,
        refresh_token: refresh,
        scope: record.scope.clone(),
    };
    (StatusCode::OK, Json(response)).into_response()
}

async fn handle_refresh_token(state: &AuthState, form: TokenForm) -> axum::response::Response {
    let refresh = match form.refresh_token.as_ref() {
        Some(value) => value,
        None => {
            return token_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "refresh_token is required",
            )
        }
    };
    let client_id = match form.client_id.as_ref() {
        Some(value) => value,
        None => {
            return token_error(
                StatusCode::BAD_REQUEST,
                "invalid_client",
                "client_id is required",
            )
        }
    };
    let client = match state.store().get_client(client_id) {
        Ok(Some(client)) => client,
        _ => return token_error(StatusCode::UNAUTHORIZED, "invalid_client", "unknown client"),
    };
    let token_hash = hash_token(refresh);
    let record = match state.store().get_refresh_token(&token_hash) {
        Ok(Some(record)) => record,
        _ => {
            return token_error(
                StatusCode::BAD_REQUEST,
                "invalid_grant",
                "refresh_token is invalid",
            )
        }
    };
    if record.client_id != client.client_id {
        return token_error(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "refresh_token does not belong to this client",
        );
    }
    if record.revoked_at.is_some() {
        return token_error(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "refresh_token has been revoked",
        );
    }
    if record.expires_at <= now() {
        return token_error(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "refresh_token has expired",
        );
    }
    // Rotate: revoke the presented token and issue a new pair.
    state
        .store()
        .revoke_refresh_token(&token_hash, now())
        .map_err(|error| {
            token_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                error.to_string(),
            )
        })
        .ok();
    let (access_token, new_refresh) = match issue_tokens(state, &client.client_id, &record.subject, &record.scope).await {
        Ok(value) => value,
        Err(error) => {
            return token_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                error.to_string(),
            )
        }
    };
    let _ = token_hash;
    let response = TokenResponse {
        access_token,
        token_type: "Bearer",
        expires_in: state.config().access_ttl.as_secs() as i64,
        refresh_token: new_refresh,
        scope: record.scope,
    };
    (StatusCode::OK, Json(response)).into_response()
}

async fn issue_tokens(
    state: &AuthState,
    client_id: &str,
    subject: &str,
    scope: &str,
) -> anyhow::Result<(String, String)> {
    let now_ts = now();
    let access_ttl = state.config().access_ttl.as_secs() as i64;
    let refresh_ttl = REFRESH_TTL_SECS;
    let jti = new_opaque_token();
    let claims = serde_json::json!({
        "iss": state.config().issuer(),
        "aud": state.config().resource_indicator(),
        "sub": subject,
        "client_id": client_id,
        "scope": scope,
        "iat": now_ts,
        "exp": now_ts + access_ttl,
        "jti": jti,
    });
    let payload_b64 = URL_SAFE_NO_PAD.encode(claims.to_string().as_bytes());
    let access_token = state.keyring().sign(&payload_b64).await?;
    let jti_record = AccessTokenJtiRecord {
        jti: jti.clone(),
        client_id: client_id.to_string(),
        subject: subject.to_string(),
        expires_at: now_ts + access_ttl,
        revoked_at: None,
    };
    state.store().insert_access_jti(&jti_record)?;
    let refresh_token = new_opaque_token();
    let refresh_hash = hash_token(&refresh_token);
    let refresh_record = RefreshTokenRecord {
        token_hash: refresh_hash,
        client_id: client_id.to_string(),
        subject: subject.to_string(),
        scope: scope.to_string(),
        expires_at: now_ts + refresh_ttl,
        revoked_at: None,
        replaced_by: None,
    };
    state.store().insert_refresh_token(&refresh_record)?;
    Ok((access_token, refresh_token))
}

#[derive(Debug, Deserialize)]
pub struct IntrospectForm {
    pub token: String,
    #[serde(default)]
    pub token_type_hint: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
}

pub async fn introspect(
    State(state): State<AuthState>,
    Form(form): Form<IntrospectForm>,
) -> impl IntoResponse {
    // For introspection we only support the JTI-backed access tokens.
    let claims = match state.keyring().verify(&form.token, now()) {
        Ok(Some(claims)) => claims,
        _ => {
            return (
                StatusCode::OK,
                Json(serde_json::json!({ "active": false })),
            )
                .into_response();
        }
    };
    let jti = match claims.get("jti").and_then(Value::as_str) {
        Some(value) => value.to_string(),
        None => {
            return (
                StatusCode::OK,
                Json(serde_json::json!({ "active": false })),
            )
                .into_response();
        }
    };
    let active = state
        .store()
        .jti_is_active(&jti, now())
        .unwrap_or(false);
    let body = if active {
        serde_json::json!({
            "active": true,
            "sub": claims.get("sub"),
            "client_id": claims.get("client_id"),
            "scope": claims.get("scope"),
            "exp": claims.get("exp"),
            "iat": claims.get("iat"),
            "iss": claims.get("iss"),
            "aud": claims.get("aud"),
            "jti": jti,
        })
    } else {
        serde_json::json!({ "active": false })
    };
    (StatusCode::OK, Json(body)).into_response()
}

#[derive(Debug, Deserialize)]
pub struct RevokeForm {
    pub token: String,
    #[serde(default)]
    pub token_type_hint: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
}

pub async fn revoke(
    State(state): State<AuthState>,
    Form(form): Form<RevokeForm>,
) -> impl IntoResponse {
    let _ = form.client_id.as_ref();
    // Accept both refresh-token and access-token revocations. RFC 7009
    // prescribes a 200 response for successful or no-op revocations.
    let refresh_hash = hash_token(&form.token);
    let _ = state.store().revoke_refresh_token(&refresh_hash, now());
    if let Ok(Some(claims)) = state.keyring().verify(&form.token, now())
        && let Some(jti) = claims.get("jti").and_then(Value::as_str)
    {
        let _ = state.store().revoke_jti(jti, now());
    }
    (StatusCode::OK, Json(serde_json::json!({}))).into_response()
}

fn token_error(
    status: StatusCode,
    error: &'static str,
    description: impl Into<String>,
) -> axum::response::Response {
    token_error_response(status, error, description.into())
}

fn token_error_response(
    status: StatusCode,
    error: impl Into<String>,
    description: impl Into<String>,
) -> axum::response::Response {
    (
        status,
        Json(TokenError {
            error: error.into(),
            error_description: description.into(),
        }),
    )
        .into_response()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        let left_byte = left.get(index).copied().unwrap_or_default();
        let right_byte = right.get(index).copied().unwrap_or_default();
        difference |= usize::from(left_byte ^ right_byte);
    }
    difference == 0
}
