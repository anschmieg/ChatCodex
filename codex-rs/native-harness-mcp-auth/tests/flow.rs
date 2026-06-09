//! End-to-end authorization-code + refresh-token flow with the stub
//! Cloudflare Access verifier (no network). Drives register, authorize,
//! decide, token, and refresh-token rotation.

#![allow(clippy::expect_used)]

use axum::Router;
use axum::routing::get;
use axum::routing::post;
use codex_native_harness_mcp_auth::AuthConfig;
use codex_native_harness_mcp_auth::AuthState;
use codex_native_harness_mcp_auth::keyring::Keyring;
use codex_native_harness_mcp_auth::storage::Store;

const REDIRECT_URI: &str = "https://chat.openai.com/callback";

fn make_state() -> (tempfile::TempDir, AuthState) {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = Store::open(temp.path()).expect("store");
    let config = AuthConfig {
        public_base_url: "https://codex.example.test".to_string(),
        data_dir: temp.path().to_path_buf(),
        cf_access_team: "https://team.example.test".to_string(),
        cf_access_aud: "aud-test".to_string(),
        access_ttl: std::time::Duration::from_secs(600),
        refresh_ttl: std::time::Duration::from_secs(3600),
        allow_client_credentials: false,
    };
    let keyring = Keyring::load_or_create(
        store.clone(),
        config.issuer(),
        config.resource_indicator(),
    )
    .expect("keyring");
    let cf = codex_native_harness_mcp_auth::cf_access::CfAccessVerifier::new_stub(
        config.cf_access_aud.clone(),
        "user-42".to_string(),
        Some("user@example.test".to_string()),
    );
    (temp, AuthState::new_for_test(config, store, keyring, cf))
}

#[tokio::test]
async fn register_authorize_token_refresh_flow() {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use serde_json::json;
    use sha2::Digest;
    use sha2::Sha256;

    let (_temp, state) = make_state();
    let app = Router::new()
        .route(
            "/.well-known/jwks.json",
            get(codex_native_harness_mcp_auth::well_known::jwks),
        )
        .route(
            "/oauth/register",
            post(codex_native_harness_mcp_auth::clients::register),
        )
        .route(
            "/oauth/authorize",
            get(codex_native_harness_mcp_auth::authorize::authorize),
        )
        .route(
            "/oauth/authorize/decide",
            post(codex_native_harness_mcp_auth::authorize::decide),
        )
        .route(
            "/oauth/token",
            post(codex_native_harness_mcp_auth::token::token),
        )
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let client_id: String;

    // 1. Register a public PKCE client.
    {
        let resp = client
            .post(format!("http://{addr}/oauth/register"))
            .json(&json!({
                "client_name": "chatgpt-test",
                "redirect_uris": [REDIRECT_URI],
                "token_endpoint_auth_method": "none",
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 201);
        let body: serde_json::Value = resp.json().await.unwrap();
        client_id = body["client_id"].as_str().unwrap().to_string();
        assert!(client_id.starts_with("chatcodex-"));
    }

    // 2. PKCE pair: S256(verifier) = challenge.
    let verifier = "verifier-1234567890-abcdefghijklmnopqrstuvwxyz-ABCDEFGH";
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

    // 3. /oauth/authorize with CF cookie -> 200 consent HTML.
    {
        let resp = client
            .get(format!(
                "http://{addr}/oauth/authorize?response_type=code&client_id={client_id}&redirect_uri={REDIRECT_URI}&code_challenge={challenge}&code_challenge_method=S256&state=abc"
            ))
            .header("cookie", "CF_Authorization=stub-jwt")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body = resp.text().await.unwrap();
        assert!(body.contains("Authorize ChatCodex"));
        assert!(body.contains("user-42"));
    }

    // 4. /oauth/authorize/decide -> 302 with code.
    let code: String;
    {
        let resp = client
            .post(format!("http://{addr}/oauth/authorize/decide"))
            .header("cookie", "CF_Authorization=stub-jwt")
            .form(&[
                ("decision", "allow"),
                ("client_id", client_id.as_str()),
                ("redirect_uri", REDIRECT_URI),
                ("scope", "mcp:tools"),
                ("state", "abc"),
                ("code_challenge", challenge.as_str()),
                ("code_challenge_method", "S256"),
            ])
            .send()
            .await
            .unwrap();
        let status = resp.status();
        let location = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(
            status.is_redirection() || status.is_success(),
            "unexpected status {status} -> {location}"
        );
        assert!(location.starts_with(REDIRECT_URI), "redirect: {location}");
        let parsed = url::Url::parse(&location).unwrap();
        code = parsed
            .query_pairs()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.into_owned())
            .expect("code");
        assert_eq!(
            parsed
                .query_pairs()
                .find(|(k, _)| k == "state")
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default(),
            "abc"
        );
    }

    // 5. /oauth/token with the code -> access + refresh.
    let (access_token, refresh_token): (String, String);
    {
        let resp = client
            .post(format!("http://{addr}/oauth/token"))
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("redirect_uri", REDIRECT_URI),
                ("client_id", client_id.as_str()),
                ("code_verifier", verifier),
            ])
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        access_token = body["access_token"].as_str().unwrap().to_string();
        refresh_token = body["refresh_token"].as_str().unwrap().to_string();
        assert_eq!(body["token_type"], "Bearer");
        assert_eq!(body["scope"], "mcp:tools");
    }

    // 6. Validate JWT claims.
    {
        let mut parts = access_token.split('.');
        let _h = parts.next().unwrap();
        let p = parts.next().unwrap();
        let bytes = URL_SAFE_NO_PAD.decode(p).unwrap();
        let claims: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(claims["iss"], "https://codex.example.test");
        assert_eq!(claims["aud"], "https://codex.example.test/mcp");
        assert_eq!(claims["sub"], "user-42");
        assert_eq!(claims["client_id"], client_id);
        assert_eq!(claims["scope"], "mcp:tools");
    }

    // 7. /oauth/refresh_token rotates the refresh.
    {
        let resp = client
            .post(format!("http://{addr}/oauth/token"))
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", client_id.as_str()),
                ("refresh_token", refresh_token.as_str()),
            ])
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        let new_access = body["access_token"].as_str().unwrap();
        let new_refresh = body["refresh_token"].as_str().unwrap();
        assert_ne!(new_access, access_token);
        assert_ne!(new_refresh, refresh_token);
    }

    // 8. The old refresh token is now revoked.
    {
        let resp = client
            .post(format!("http://{addr}/oauth/token"))
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", client_id.as_str()),
                ("refresh_token", refresh_token.as_str()),
            ])
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["error"], "invalid_grant");
    }

    // 9. Replaying the authorization code is rejected.
    {
        let resp = client
            .post(format!("http://{addr}/oauth/token"))
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("redirect_uri", REDIRECT_URI),
                ("client_id", client_id.as_str()),
                ("code_verifier", verifier),
            ])
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["error"], "invalid_grant");
    }

    server.abort();
}
