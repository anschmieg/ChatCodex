//! Hindsight memory tools.
//!
//! Exposes `memory_search`, `memory_retain`, and `memory_reflect` as
//! deterministic MCP tools backed by the Hindsight memory server
//! (REST API). The server URL and optional bearer token come from the
//! environment (`CHATCODEX_HINDSIGHT_URL`, `CHATCODEX_HINDSIGHT_TOKEN`,
//! `CHATCODEX_HINDSIGHT_BANK`); the token is never exposed to the model.
//!
//! Architecture note: `memory_search` and `memory_retain` are pure
//! retrieval/storage and add no LLM to the stack. `memory_reflect` calls
//! Hindsight's synthesis endpoint, which runs on Hindsight's own LLM
//! (server-side, configured by the Hindsight deployment). This is a
//! sanctioned exception to the "no model calls in the backend" rule:
//! the reflection service is a memory store, not a harness control loop,
//! and it is explicitly approved for use here.

use anyhow::Context;
use codex_http_client::HttpClientBuilder;
use rmcp::model::CallToolResult;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

/// Path prefix for all Hindsight bank operations.
const API_PREFIX: &str = "/v1/default/banks";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemorySearchArgs {
    query: String,
    /// Recall budget: "low" | "mid" | "high".
    #[serde(default)]
    budget: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    max_tokens: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemoryRetainArgs {
    content: String,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemoryReflectArgs {
    query: String,
    /// Synthesis budget: "low" | "mid" | "high".
    #[serde(default)]
    budget: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    max_tokens: Option<u32>,
}

struct HindsightConfig {
    api_url: String,
    bank_id: String,
    token: Option<String>,
}

impl HindsightConfig {
    fn from_env() -> anyhow::Result<Self> {
        let api_url = std::env::var("CHATCODEX_HINDSIGHT_URL").with_context(|| {
            "Hindsight memory tools are not configured: set CHATCODEX_HINDSIGHT_URL \
             (and optionally CHATCODEX_HINDSIGHT_TOKEN and CHATCODEX_HINDSIGHT_BANK) \
             on the server"
        })?;
        let bank_id = std::env::var("CHATCODEX_HINDSIGHT_BANK")
            .unwrap_or_else(|_| "chatcodex".to_string());
        let token = std::env::var("CHATCODEX_HINDSIGHT_TOKEN").ok();
        Ok(Self {
            api_url: api_url.trim_end_matches('/').to_string(),
            bank_id,
            token,
        })
    }
}

fn encode_bank_id(bank_id: &str) -> String {
    url::form_urlencoded::byte_serialize(bank_id.as_bytes()).collect()
}

async fn hindsight_post(
    config: &HindsightConfig,
    path: &str,
    body: &Value,
    timeout_secs: u64,
) -> anyhow::Result<Value> {
    let url = format!("{}{}", config.api_url, path);
    let client = HttpClientBuilder::new()
        .build_direct()
        .with_context(|| "failed to build Hindsight HTTP client")?;
    let mut builder = client
        .post(&url)
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .json(body);
    if let Some(token) = &config.token {
        builder = builder.bearer_auth(token);
    }
    let response = builder
        .send()
        .await
        .with_context(|| format!("Hindsight request failed: POST {path}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .with_context(|| "failed to read Hindsight response body")?;
    if !status.is_success() {
        let snippet: String = text.chars().take(500).collect();
        anyhow::bail!("Hindsight API error (HTTP {status}) on {path}: {snippet}");
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

/// `memory_search` — recall memories matching a query.
pub(crate) async fn memory_search(args: MemorySearchArgs) -> anyhow::Result<CallToolResult> {
    let config = HindsightConfig::from_env()?;
    let mut body = json!({
        "query": args.query,
        "max_tokens": args.max_tokens.unwrap_or(4096),
    });
    if let Some(budget) = args.budget {
        body["budget"] = json!(budget);
    }
    if let Some(tags) = args.tags {
        body["tags"] = json!(tags);
    }
    let path = format!(
        "{}/{}/memories/recall",
        API_PREFIX,
        encode_bank_id(&config.bank_id)
    );
    let parsed = hindsight_post(&config, &path, &body, 15).await?;

    let results = parsed
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let memories: Vec<Value> = results
        .into_iter()
        .map(|r| {
            let mut item = json!({});
            for key in ["id", "text", "context", "occurred_start"] {
                if let Some(v) = r.get(key).filter(|v| !v.is_null()) {
                    item[key] = v.clone();
                }
            }
            item
        })
        .collect();
    Ok(crate::text_result(json!({
        "query": args.query,
        "results": memories,
    })))
}

/// `memory_retain` — store a fact in Hindsight.
pub(crate) async fn memory_retain(args: MemoryRetainArgs) -> anyhow::Result<CallToolResult> {
    let config = HindsightConfig::from_env()?;
    let mut item = json!({ "content": args.content });
    if let Some(context) = &args.context {
        item["context"] = json!(context);
    }
    if let Some(tags) = &args.tags {
        item["tags"] = json!(tags);
    }
    let body = json!({ "items": [item], "async": false });
    let path = format!(
        "{}/{}/memories",
        API_PREFIX,
        encode_bank_id(&config.bank_id)
    );
    hindsight_post(&config, &path, &body, 15).await?;
    Ok(crate::text_result(json!({
        "stored": true,
        "content": args.content,
    })))
}

/// `memory_reflect` — ask Hindsight to synthesize an answer from stored memories.
///
/// Note: this endpoint runs Hindsight's own LLM server-side; it is not a
/// harness control loop and is the only sanctioned model call in the backend.
pub(crate) async fn memory_reflect(args: MemoryReflectArgs) -> anyhow::Result<CallToolResult> {
    let config = HindsightConfig::from_env()?;
    let mut body = json!({
        "query": args.query,
        "max_tokens": args.max_tokens.unwrap_or(4096),
    });
    if let Some(budget) = args.budget {
        body["budget"] = json!(budget);
    }
    if let Some(tags) = args.tags {
        body["tags"] = json!(tags);
    }
    let path = format!(
        "{}/{}/reflect",
        API_PREFIX,
        encode_bank_id(&config.bank_id)
    );
    let parsed = hindsight_post(&config, &path, &body, 60).await?;
    let reflection = parsed
        .get("text")
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| {
            serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| "".to_string())
        });
    Ok(crate::text_result(json!({ "reflection": reflection })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::sync::Mutex;

    /// Serializes tests that mutate process-global env vars, holding the lock
    /// across the whole call so no other env-mutating test can interleave.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct MockHindsight {
        base_url: String,
    }

    async fn spawn_mock(routes: axum::Router) -> MockHindsight {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, routes).await.expect("mock serve");
        });
        MockHindsight {
            base_url: format!("http://{addr}"),
        }
    }

    async fn with_env<T>(
        base_url: &str,
        f: impl FnOnce() -> T,
    ) -> T::Output
    where
        T: Future,
    {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // SAFETY: tests only; access serialized by ENV_LOCK.
        unsafe {
            std::env::set_var("CHATCODEX_HINDSIGHT_URL", base_url);
            std::env::set_var("CHATCODEX_HINDSIGHT_BANK", "chatcodex");
            std::env::remove_var("CHATCODEX_HINDSIGHT_TOKEN");
        }
        f().await
    }

    #[tokio::test]
    async fn memory_search_calls_recall_endpoint() {
        let mock = spawn_mock(axum::Router::new().route(
            "/v1/default/banks/chatcodex/memories/recall",
            axum::routing::post(|axum::Json(body): axum::Json<Value>| async move {
                assert_eq!(body["query"], "how does auth work");
                axum::Json(json!({
                    "results": [
                        {
                            "id": "mem-1",
                            "text": "Auth uses CHATCODEX_GITHUB_TOKEN via ephemeral credential helper.",
                            "context": "setup_workspace",
                            "occurred_start": "2026-07-29T10:00:00Z",
                        }
                    ]
                }))
            }),
        ))
        .await;

        let result = with_env(&mock.base_url, || {
            memory_search(MemorySearchArgs {
                query: "how does auth work".to_string(),
                budget: Some("mid".to_string()),
                tags: None,
                max_tokens: None,
            })
        })
        .await
        .expect("memory_search");
        let structured = result.structured_content.as_ref().expect("structured");
        assert_eq!(structured["query"], "how does auth work");
        assert_eq!(structured["results"][0]["id"], "mem-1");
        assert_eq!(
            structured["results"][0]["text"],
            "Auth uses CHATCODEX_GITHUB_TOKEN via ephemeral credential helper."
        );
    }

    #[tokio::test]
    async fn memory_retain_posts_memories() {
        let mock = spawn_mock(axum::Router::new().route(
            "/v1/default/banks/chatcodex/memories",
            axum::routing::post(|axum::Json(body): axum::Json<Value>| async move {
                assert_eq!(body["items"][0]["content"], "remember this fact");
                assert_eq!(body["items"][0]["context"], "project decision");
                axum::Json(json!({ "memory_id": "mem-9" }))
            }),
        ))
        .await;

        let result = with_env(&mock.base_url, || {
            memory_retain(MemoryRetainArgs {
                content: "remember this fact".to_string(),
                context: Some("project decision".to_string()),
                tags: Some(vec!["chatcodex".to_string()]),
            })
        })
        .await
        .expect("memory_retain");
        let structured = result.structured_content.as_ref().expect("structured");
        assert_eq!(structured["stored"], true);
        assert_eq!(structured["content"], "remember this fact");
    }

    #[tokio::test]
    async fn memory_reflect_returns_text() {
        let mock = spawn_mock(axum::Router::new().route(
            "/v1/default/banks/chatcodex/reflect",
            axum::routing::post(|axum::Json(body): axum::Json<Value>| async move {
                assert_eq!(body["query"], "summarize decisions");
                axum::Json(json!({ "text": "# Decisions\n\n- Use Hindsight for memory." }))
            }),
        ))
        .await;

        let result = with_env(&mock.base_url, || {
            memory_reflect(MemoryReflectArgs {
                query: "summarize decisions".to_string(),
                budget: None,
                tags: None,
                max_tokens: None,
            })
        })
        .await
        .expect("memory_reflect");
        let structured = result.structured_content.as_ref().expect("structured");
        assert_eq!(
            structured["reflection"],
            "# Decisions\n\n- Use Hindsight for memory."
        );
    }

    #[tokio::test]
    async fn memory_tools_error_when_unconfigured() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // SAFETY: tests only; serialized by ENV_LOCK.
        unsafe { std::env::remove_var("CHATCODEX_HINDSIGHT_URL") };
        let err = memory_search(MemorySearchArgs {
            query: "anything".to_string(),
            budget: None,
            tags: None,
            max_tokens: None,
        })
        .await
        .expect_err("should fail without CHATCODEX_HINDSIGHT_URL");
        assert!(
            err.to_string().contains("CHATCODEX_HINDSIGHT_URL"),
            "error should mention the env var: {err}"
        );
    }
}
