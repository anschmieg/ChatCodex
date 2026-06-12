//! End-to-end test: OAuth flow → MCP initialize → list_tools.
//!
//! Sets up the full auth stack with a stub Cloudflare Access verifier,
//! registers a client, completes the authorization-code flow, then uses
//! the issued access token to call the MCP streamable HTTP endpoint.

#![allow(clippy::expect_used)]

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use codex_native_harness_mcp::http_router_with_state;
use codex_native_harness_mcp_auth::AuthConfig;
use codex_native_harness_mcp_auth::AuthState;
use codex_native_harness_mcp_auth::keyring::Keyring;
use codex_native_harness_mcp_auth::storage::Store;
use codex_arg0::Arg0DispatchPaths;
use serde_json::json;
use sha2::Digest;
use sha2::Sha256;

const REDIRECT_URI: &str = "https://localhost/cb";

fn make_auth_state(temp: &tempfile::TempDir) -> AuthState {
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
    AuthState::new_for_test(config, store, keyring, cf)
}

#[tokio::test]
async fn oauth_flow_then_mcp_initialize() {
    let workspace = tempfile::tempdir().expect("workspace");
    let data = tempfile::tempdir().expect("data");

    // SAFETY: test-only, single-threaded, no concurrent readers
    unsafe {
        std::env::set_var("CHATCODEX_WORKSPACE_ROOT", workspace.path());
        std::env::set_var("CHATCODEX_DATA_DIR", data.path());
    }

    let auth_state = make_auth_state(&data);

    // Build the full application router with a test workspace and stub auth.
    let app = http_router_with_state(Arg0DispatchPaths::default(), auth_state)
        .await
        .expect("router");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    // ── 1. Register a public PKCE client ──────────────────────────────
    let resp = client
        .post(format!("{base}/oauth/register"))
        .json(&json!({
            "client_name": "e2e-test",
            "redirect_uris": [REDIRECT_URI],
            "token_endpoint_auth_method": "none",
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body_text = resp.text().await.unwrap();
    assert_eq!(
        status, 201,
        "register endpoint: expected 201, got {status}: {body_text}"
    );
    let body: serde_json::Value = serde_json::from_str(&body_text).unwrap();
    let client_id = body["client_id"].as_str().unwrap().to_string();

    // ── 2. PKCE pair ─────────────────────────────────────────────────
    let verifier = "test-verifier-abcdefghijklmnopqrstuvwxyz-ABCDEFGH";
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

    // ── 3. Authorize (with stub CF cookie) → consent page ────────────
    let resp = client
        .get(format!(
            "{base}/oauth/authorize?response_type=code&client_id={client_id}&redirect_uri={REDIRECT_URI}&code_challenge={challenge}&code_challenge_method=S256&state=abc"
        ))
        .header("cookie", "CF_Authorization=stub-jwt")
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body = resp.text().await.unwrap();
    assert_eq!(
        status, 200,
        "authorize endpoint: expected 200, got {status}: {body:.200}"
    );
    assert!(body.contains("Authorize ChatCodex"));

    // ── 4. Decide → authorization code ─────────────────────────────────
    let resp = client
        .post(format!("{base}/oauth/authorize/decide"))
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
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap()
        .to_string();
    assert!(location.starts_with(REDIRECT_URI));
    let parsed = url::Url::parse(&location).unwrap();
    let code = parsed
        .query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.into_owned())
        .expect("code");

    // ── 5. Exchange code for tokens ───────────────────────────────────
    let resp = client
        .post(format!("{base}/oauth/token"))
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
    let access_token = body["access_token"].as_str().unwrap().to_string();

    // ── 6. MCP initialize ────────────────────────────────────────────
    let resp = client
        .post(format!("{base}/mcp"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("Authorization", format!("Bearer {access_token}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-05",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1.0" },
            },
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // The response is an SSE stream. Check it contains the initialize result.
    assert!(
        body.contains(r#""protocolVersion""#),
        "SSE response should contain protocolVersion: {body}"
    );
    assert!(
        body.contains(r#""tools""#),
        "SSE response should contain tools capability: {body}"
    );

    server.abort();
}

#[tokio::test]
async fn mcp_rejects_requests_without_bearer() {
    let workspace = tempfile::tempdir().expect("workspace");
    let data = tempfile::tempdir().expect("data");

    // SAFETY: test-only, single-threaded, no concurrent readers
    unsafe {
        std::env::set_var("CHATCODEX_WORKSPACE_ROOT", workspace.path());
        std::env::set_var("CHATCODEX_DATA_DIR", data.path());
    }

    let auth_state = make_auth_state(&data);

    let app = http_router_with_state(Arg0DispatchPaths::default(), auth_state)
        .await
        .expect("router");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/mcp"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {},
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    server.abort();
}
