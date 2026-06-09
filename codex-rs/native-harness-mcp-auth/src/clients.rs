//! Dynamic client registration per RFC 7591.
//!
//! Only public clients (PKCE, `token_endpoint_auth_method=none`) are
//! accepted by default. Confidential clients require the
//! `CHATCODEX_OAUTH_ALLOW_CONFIDENTIAL` env to be set to "1".

use anyhow::Context;
use anyhow::Result;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde::Serialize;

use crate::state::AuthState;
use crate::storage::ClientRecord;
use crate::storage::now;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ClientRegistrationRequest {
    #[serde(default)]
    pub client_name: Option<String>,
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub grant_types: Option<Vec<String>>,
    #[serde(default = "default_token_endpoint_auth_method")]
    pub token_endpoint_auth_method: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
}

fn default_token_endpoint_auth_method() -> String {
    "none".to_string()
}

#[derive(Debug, Serialize)]
pub struct ClientRegistrationResponse {
    pub client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    pub redirect_uris: Vec<String>,
    pub grant_types: Vec<String>,
    pub token_endpoint_auth_method: String,
    pub client_id_issued_at: i64,
}

#[derive(Debug, Serialize)]
pub struct ClientRegistrationError {
    pub error: String,
    pub error_description: String,
}

impl ClientRegistrationError {
    pub fn invalid(error_description: impl Into<String>) -> Self {
        Self {
            error: "invalid_client_metadata".to_string(),
            error_description: error_description.into(),
        }
    }
}

pub async fn register(
    State(state): State<AuthState>,
    Json(req): Json<ClientRegistrationRequest>,
) -> impl IntoResponse {
    match register_inner(&state, req).await {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ClientRegistrationError::invalid(error.to_string())),
        )
            .into_response(),
    }
}

async fn register_inner(
    state: &AuthState,
    req: ClientRegistrationRequest,
) -> Result<ClientRegistrationResponse> {
    if req.redirect_uris.is_empty() {
        anyhow::bail!("redirect_uris must not be empty");
    }
    for uri in &req.redirect_uris {
        let parsed = url::Url::parse(uri).context("invalid redirect_uri")?;
        if parsed.scheme() != "https" && parsed.scheme() != "http" {
            anyhow::bail!("redirect_uri must use http(s)");
        }
    }
    let auth_method = req.token_endpoint_auth_method.as_str();
    let (client_secret_hash, client_secret) = match auth_method {
        "none" => {
            if req.client_secret.is_some() {
                anyhow::bail!("public clients must not provide a client_secret");
            }
            (None, None)
        }
        "client_secret_post" => {
            if !state.config().allow_client_credentials {
                anyhow::bail!(
                    "confidential clients are not enabled; set CHATCODEX_OAUTH_ALLOW_CONFIDENTIAL=1"
                );
            }
            let secret = req
                .client_secret
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("client_secret is required for confidential clients"))?;
            if secret.len() < 16 {
                anyhow::bail!("client_secret must be at least 16 characters");
            }
            let hash = crate::storage::hash_token(secret);
            (Some(hash), Some(secret.clone()))
        }
        other => anyhow::bail!("unsupported token_endpoint_auth_method: {other}"),
    };
    let grant_types = req
        .grant_types
        .unwrap_or_else(|| vec!["authorization_code".to_string(), "refresh_token".to_string()]);
    for grant in &grant_types {
        if !matches!(grant.as_str(), "authorization_code" | "refresh_token") {
            anyhow::bail!("unsupported grant_type: {grant}");
        }
    }
    let client_id = req
        .client_id
        .unwrap_or_else(|| format!("chatcodex-{}", crate::storage::new_opaque_token()));
    let now_ts = now();
    let record = ClientRecord {
        client_id: client_id.clone(),
        client_secret_hash,
        client_name: req.client_name.clone(),
        redirect_uris: req.redirect_uris.clone(),
        grant_types: grant_types.clone(),
        token_endpoint_auth_method: auth_method.to_string(),
        created_at: now_ts,
        disabled_at: None,
    };
    state.store().insert_client(&record)?;
    Ok(ClientRegistrationResponse {
        client_id,
        client_secret,
        client_name: req.client_name,
        redirect_uris: req.redirect_uris,
        grant_types,
        token_endpoint_auth_method: auth_method.to_string(),
        client_id_issued_at: now_ts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthConfig;
    use crate::keyring::Keyring;
    use crate::state::AuthState;
    use std::path::PathBuf;
    use std::time::Duration;

    fn test_state() -> AuthState {
        let store = crate::storage::Store::open_in_memory().unwrap();
        let config = AuthConfig {
            public_base_url: "https://example".to_string(),
            data_dir: PathBuf::from("/data"),
            cf_access_team: "https://team.cloudflareaccess.com".to_string(),
            cf_access_aud: "aud".to_string(),
            access_ttl: Duration::from_secs(60),
            refresh_ttl: Duration::from_secs(600),
            allow_client_credentials: false,
        };
        let keyring = Keyring::load_or_create(
            store.clone(),
            config.issuer(),
            config.resource_indicator(),
        )
        .unwrap();
        let cf = crate::cf_access::CfAccessVerifier::new(
            config.cf_access_certs_uri(),
            config.cf_access_aud.clone(),
        );
        AuthState::new_for_test(config, store, keyring, cf)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn registers_public_pkce_client() {
        let state = test_state();
        let response = register_inner(
            &state,
            ClientRegistrationRequest {
                client_name: Some("ChatGPT".to_string()),
                redirect_uris: vec!["https://chatgpt.com/cb".to_string()],
                grant_types: None,
                token_endpoint_auth_method: "none".to_string(),
                client_secret: None,
                client_id: Some("client-1".to_string()),
            },
        )
        .await
        .expect("register");
        assert_eq!(response.client_id, "client-1");
        assert_eq!(response.token_endpoint_auth_method, "none");
        let stored = state.store().get_client("client-1").unwrap().unwrap();
        assert_eq!(stored.redirect_uris, vec!["https://chatgpt.com/cb".to_string()]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_confidential_without_flag() {
        let state = test_state();
        let result = register_inner(
            &state,
            ClientRegistrationRequest {
                client_name: None,
                redirect_uris: vec!["https://example.com/cb".to_string()],
                grant_types: None,
                token_endpoint_auth_method: "client_secret_post".to_string(),
                client_secret: Some("longerthan16characters".to_string()),
                client_id: None,
            },
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_invalid_redirect_uri() {
        let state = test_state();
        let result = register_inner(
            &state,
            ClientRegistrationRequest {
                client_name: None,
                redirect_uris: vec!["not-a-url".to_string()],
                grant_types: None,
                token_endpoint_auth_method: "none".to_string(),
                client_secret: None,
                client_id: None,
            },
        )
        .await;
        assert!(result.is_err());
    }
}
