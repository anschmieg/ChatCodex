#![allow(clippy::unwrap_used, clippy::expect_used)]
//! End-to-end test: OAuth flow → MCP initialize → list_tools.
//!
//! Sets up the full auth stack with a stub Cloudflare Access verifier,
//! registers a client, completes the authorization-code flow, then uses
//! the issued access token to call the MCP streamable HTTP endpoint.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chatcodex_mcp_server::http_router_with_state;
use chatcodex_oauth::AuthConfig;
use chatcodex_oauth::AuthState;
use chatcodex_oauth::keyring::Keyring;
use chatcodex_oauth::storage::Store;
use codex_arg0::Arg0DispatchPaths;
use serde_json::json;
use sha2::Digest;
use sha2::Sha256;

const REDIRECT_URI: &str = "https://localhost/cb";

/// Serialize test setup that mutates process-wide environment variables.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    let keyring =
        Keyring::load_or_create(store.clone(), config.issuer(), config.resource_indicator())
            .expect("keyring");
    let cf = chatcodex_oauth::cf_access::CfAccessVerifier::new_stub(
        config.cf_access_aud.clone(),
        "user-42".to_string(),
        Some("user@example.test".to_string()),
    );
    AuthState::new_for_test(config, store, keyring, cf)
}

/// Run the OAuth registration/authorization/token exchange and then the MCP
/// `initialize` handshake. Returns the HTTP client, base URL, MCP session id,
/// the registered OAuth client id, access token, and the server join handle.
async fn oauth_and_mcp_session(
    workspace: &tempfile::TempDir,
    data: &tempfile::TempDir,
) -> (
    reqwest::Client,
    String,
    String,
    String,
    String,
    tokio::task::JoinHandle<()>,
) {
    // SAFETY: test-only; the global lock prevents concurrent env mutations.
    let _env_guard = ENV_LOCK.lock().unwrap();
    unsafe {
        std::env::set_var("CHATCODEX_WORKSPACE_ROOT", workspace.path());
        std::env::set_var("CHATCODEX_DATA_DIR", data.path());
    }

    let auth_state = make_auth_state(data);

    let mut arg0_paths = Arg0DispatchPaths::default();
    let self_exe = std::path::PathBuf::from(
        std::env::var("CARGO_BIN_EXE_chatcodex-mcp-server")
            .expect("CARGO_BIN_EXE_chatcodex-mcp-server"),
    );
    arg0_paths.codex_self_exe = Some(self_exe.clone());
    arg0_paths.codex_linux_sandbox_exe = Some(self_exe);

    let app = http_router_with_state(arg0_paths, auth_state)
        .await
        .expect("router");

    // Release the env lock now that the server has captured its config.
    drop(_env_guard);

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

    // 1. Register a public PKCE client.
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

    // 2. PKCE pair.
    let verifier = "test-verifier-abcdefghijklmnopqrstuvwxyz-ABCDEFGH";
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

    // 3. Authorize (with stub CF cookie) → consent page.
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

    // 4. Decide → authorization code.
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

    // 5. Exchange code for tokens.
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

    // 6. MCP initialize.
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
    let session_id = resp
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .expect("mcp-session-id header")
        .to_string();
    let body = resp.text().await.unwrap();
    assert!(
        body.contains(r#""protocolVersion""#),
        "SSE response should contain protocolVersion: {body}"
    );
    assert!(
        body.contains(r#""tools""#),
        "SSE response should contain tools capability: {body}"
    );

    (client, base, session_id, client_id, access_token, server)
}

/// Extract the JSON payloads from an SSE response body.
fn parse_sse_data(body: &str) -> Vec<serde_json::Value> {
    body.split("\n\n")
        .filter(|event| !event.is_empty())
        .filter_map(|event| {
            event
                .lines()
                .find_map(|line| line.strip_prefix("data: "))
                .and_then(|data| serde_json::from_str(data).ok())
        })
        .collect()
}

#[tokio::test]
async fn oauth_flow_then_mcp_initialize() {
    let workspace = tempfile::tempdir().expect("workspace");
    let data = tempfile::tempdir().expect("data");
    let (_client, _base, _session_id, _client_id, _access_token, _server) =
        oauth_and_mcp_session(&workspace, &data).await;
}

#[tokio::test]
async fn mcp_rejects_requests_without_bearer() {
    let workspace = tempfile::tempdir().expect("workspace");
    let data = tempfile::tempdir().expect("data");

    // SAFETY: test-only; the global lock prevents concurrent env mutations.
    let _env_guard = ENV_LOCK.lock().unwrap();
    unsafe {
        std::env::set_var("CHATCODEX_WORKSPACE_ROOT", workspace.path());
        std::env::set_var("CHATCODEX_DATA_DIR", data.path());
    }

    let auth_state = make_auth_state(&data);

    let app = http_router_with_state(Arg0DispatchPaths::default(), auth_state)
        .await
        .expect("router");

    drop(_env_guard);

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
            "params": {
                "protocolVersion": "2025-11-05",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1.0" },
            },
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    server.abort();
}

#[tokio::test]
async fn mcp_setup_workspace_and_git_tools() {
    let workspace = tempfile::tempdir().expect("workspace");
    let data = tempfile::tempdir().expect("data");
    let (client, base, session_id, _client_id, access_token, _server) =
        oauth_and_mcp_session(&workspace, &data).await;

    async fn call_tool(
        client: &reqwest::Client,
        base: &str,
        session_id: &str,
        access_token: &str,
        id: i64,
        name: &str,
        arguments: serde_json::Value,
    ) -> serde_json::Value {
        let resp = client
            .post(format!("{base}/mcp"))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("Authorization", format!("Bearer {access_token}"))
            .header("mcp-session-id", session_id)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": { "name": name, "arguments": arguments },
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "tools/call {name} failed");
        let body = resp.text().await.unwrap();
        let events = parse_sse_data(&body);
        events
            .into_iter()
            .find(|event| event.get("id") == Some(&json!(id)))
            .unwrap_or_else(|| panic!("response for {name} not found in SSE: {body}"))
    }

    // Helper to parse the JSON embedded in the first text content of a tool result.
    fn parse_tool_result(response: &serde_json::Value) -> serde_json::Value {
        let content = response["result"]["content"]
            .as_array()
            .expect("tool content array");
        let text = content[0]["text"].as_str().expect("tool text content");
        serde_json::from_str(text).expect("tool result is valid JSON")
    }

    // setup_workspace(source: "sandbox") creates a persistent sandbox with git init.
    let response = call_tool(
        &client,
        &base,
        &session_id,
        &access_token,
        2,
        "setup_workspace",
        json!({"source": "sandbox"}),
    )
    .await;
    let result = parse_tool_result(&response);
    assert_eq!(result["action"], "created");
    assert!(
        result["workspace_root"]
            .as_str()
            .unwrap()
            .contains("sandboxes")
    );
    assert!(
        std::path::Path::new(result["workspace_root"].as_str().unwrap())
            .join(".git")
            .exists(),
        "sandbox should be a git repo"
    );

    // git_status returns structured entries.
    let response = call_tool(
        &client,
        &base,
        &session_id,
        &access_token,
        3,
        "git_status",
        json!({}),
    )
    .await;
    let result = parse_tool_result(&response);
    assert!(result.get("entries").is_some());

    // git push is blocked by policy and returns an MCP error.
    let response = call_tool(
        &client,
        &base,
        &session_id,
        &access_token,
        4,
        "git",
        json!({"command": "push origin main"}),
    )
    .await;
    let error = response
        .get("error")
        .expect(&format!("git push should return an MCP error: {response}"));
    let error_text = error["message"].as_str().unwrap_or("");
    assert!(
        error_text.contains("blocked by policy"),
        "git push error should mention policy: {error_text}"
    );
}

#[tokio::test]
async fn mcp_prompts_list_and_get() {
    let workspace = tempfile::tempdir().expect("workspace");
    let data = tempfile::tempdir().expect("data");
    let (client, base, session_id, _client_id, access_token, _server) =
        oauth_and_mcp_session(&workspace, &data).await;

    async fn call_method(
        client: &reqwest::Client,
        base: &str,
        session_id: &str,
        access_token: &str,
        id: i64,
        method: &str,
        params: serde_json::Value,
    ) -> serde_json::Value {
        let resp = client
            .post(format!("{base}/mcp"))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("Authorization", format!("Bearer {access_token}"))
            .header("mcp-session-id", session_id)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "{method} failed");
        let body = resp.text().await.unwrap();
        let events = parse_sse_data(&body);
        events
            .into_iter()
            .find(|event| event.get("id") == Some(&json!(id)))
            .unwrap_or_else(|| panic!("response for {method} not found in SSE: {body}"))
    }

    // prompts/list
    let response = call_method(
        &client,
        &base,
        &session_id,
        &access_token,
        5,
        "prompts/list",
        json!({}),
    )
    .await;
    let result = response["result"].as_object().expect("prompts/list result");
    let prompts = result["prompts"].as_array().expect("prompts array");
    assert!(
        prompts.iter().any(|p| p["name"] == "apply-patch-guide"),
        "expected apply-patch-guide prompt"
    );
    assert!(
        prompts.iter().any(|p| p["name"] == "git-operations-guide"),
        "expected git-operations-guide prompt"
    );

    // prompts/get
    let response = call_method(
        &client,
        &base,
        &session_id,
        &access_token,
        6,
        "prompts/get",
        json!({"name": "apply-patch-guide"}),
    )
    .await;
    let result = response["result"].as_object().expect("prompts/get result");
    assert!(result.contains_key("messages"));
    assert!(!result["messages"].as_array().unwrap().is_empty());

    // prompts/get for an unknown prompt returns an error.
    let response = call_method(
        &client,
        &base,
        &session_id,
        &access_token,
        7,
        "prompts/get",
        json!({"name": "unknown-prompt"}),
    )
    .await;
    assert!(
        response.get("error").is_some(),
        "unknown prompt should error"
    );
}
