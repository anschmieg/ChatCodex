//! OpenAI-compatible HTTP client for hybrid worker LLM calls.
//!
//! Makes blocking HTTP calls to the configured provider endpoint using the
//! reqwest blocking client.  Each call runs on a dedicated thread pool thread
//! via `tokio::task::spawn_blocking` so it never blocks the async Tokio runtime.

use deterministic_core::HybridProviderProfile;
use deterministic_protocol::PatchEdit;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Request / response types for /v1/chat/completions
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: Vec<Message>,
    temperature: f32,
    max_tokens: u32,
    #[serde(default)]
    tools: Vec<()>,
}

#[derive(Debug, Serialize)]
struct Message {
    role: &'static str,
    content: String,
}

// ---------------------------------------------------------------------------
// Response parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    content: Option<String>,
}

/// JSON wrapper returned by the LLM when it outputs a vector of patch edits.
/// The "summary" field is optional — if absent, a default one is synthesised.
#[derive(Debug, Deserialize)]
struct WorkerEditResponse {
    edits: Vec<PatchEdit>,
    #[serde(default)]
    summary: Option<String>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ProviderError {
    Http(reqwest::Error),
    MissingApiKey(String),
    EmptyResponse,
    ParseJson(serde_json::Error),
    InvalidEdits(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(e) => write!(f, "HTTP request failed: {e}"),
            Self::MissingApiKey(v) => write!(f, "API key env var '{v}' is set but not present in the environment"),
            Self::EmptyResponse => write!(f, "provider returned empty response content"),
            Self::ParseJson(e) => write!(f, "failed to parse provider response as JSON: {e}"),
            Self::InvalidEdits(msg) => write!(f, "provider response does not contain valid patch edits: {msg}"),
        }
    }
}

impl std::error::Error for ProviderError {}

impl From<reqwest::Error> for ProviderError {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e)
    }
}

impl From<serde_json::Error> for ProviderError {
    fn from(e: serde_json::Error) -> Self {
        Self::ParseJson(e)
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Call the LLM provider with the given prompt and return the proposed edits.
///
/// This function is synchronous and is intended to be called from within
/// `tokio::task::spawn_blocking` so it never blocks the async runtime.
pub fn call_worker_provider_sync(
    profile: &HybridProviderProfile,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<(Option<String>, Vec<PatchEdit>), ProviderError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(profile.timeout_seconds))
        .build()
        .map_err(ProviderError::Http)?;

    // Build the messages array.
    let messages = vec![
        Message {
            role: "system",
            content: system_prompt.to_string(),
        },
        Message {
            role: "user",
            content: user_prompt.to_string(),
        },
    ];

    let request_body = ChatCompletionRequest {
        model: &profile.model,
        messages,
        temperature: profile.temperature,
        max_tokens: profile.max_output_tokens,
        tools: vec![], // Explicitly disable tool calling.
    };

    // Build request headers.
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        "application/json".parse().unwrap(),
    );

    // Add Authorization header if api_key_env is configured.
    if let Some(ref env_var) = profile.api_key_env {
        if !env_var.is_empty() {
            let api_key = std::env::var(env_var).map_err(|_| {
                ProviderError::MissingApiKey(env_var.clone())
            })?;
            let auth_value = format!("Bearer {api_key}").parse().unwrap();
            headers.insert(reqwest::header::AUTHORIZATION, auth_value);
        }
    }

    let request = client
        .post(format!("{}/chat/completions", profile.base_url.trim_end_matches('/')))
        .headers(headers)
        .json(&request_body)
        .build()
        .map_err(ProviderError::Http)?;

    let response = client.execute(request)?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(ProviderError::InvalidEdits(format!(
            "provider returned HTTP {status}: {body}"
        )));
    }

    let completion: ChatCompletionResponse = response.json()?;

    let content = completion
        .choices
        .first()
        .and_then(|c| c.message.content.as_deref())
        .filter(|s| !s.trim().is_empty())
        .ok_or(ProviderError::EmptyResponse)?;

    // Try to parse the content as a WorkerEditResponse JSON object first.
    // If that fails, treat the content as a direct array.
    let edit_response: WorkerEditResponse = match serde_json::from_str(content) {
        Ok(resp) => resp,
        Err(_) => {
            // Try parsing as a direct array of PatchEdit objects.
            let edits: Vec<PatchEdit> = serde_json::from_str(content)
                .map_err(|e| ProviderError::InvalidEdits(e.to_string()))?;
            WorkerEditResponse { edits, summary: None }
        }
    };

    // Validate each edit before returning.
    for (i, edit) in edit_response.edits.iter().enumerate() {
        validate_edit(edit).map_err(|msg| {
            ProviderError::InvalidEdits(format!(
                "edit[{i}] rejected by server policy: {msg}"
            ))
        })?;
    }

    let summary = edit_response.summary.or_else(|| {
        if edit_response.edits.is_empty() {
            None
        } else {
            Some(format!(
                "Worker proposed {} edit(s)",
                edit_response.edits.len()
            ))
        }
    });

    Ok((summary, edit_response.edits))
}

/// Validate a single edit per server-side policy.
///
/// Rejects:
/// - Absolute paths (must be relative)
/// - Path components containing ".." (traversal attack)
fn validate_edit(edit: &PatchEdit) -> Result<(), String> {
    if edit.path.starts_with('/') {
        return Err(format!(
            "absolute paths are not allowed; got: {}",
            edit.path
        ));
    }
    for component in edit.path.split('/') {
        if component == ".." {
            return Err(format!(
                "path traversal '..' is not allowed; got: {}",
                edit.path
            ));
        }
    }
    Ok(())
}

/// System prompt template instructing the worker LLM how to produce edits.
pub fn worker_system_prompt() -> String {
    r#"You are a code-editing assistant. Produce exactly one JSON object with an "edits" field containing an array of patch edit objects. Each edit must have this structure:
{
  "path": "relative/file/path",
  "operation": "replace|insert|delete",
  "startLine": <number or null>,
  "endLine": <number or null>,
  "oldText": <string or null>,
  "newText": <string>
}

Rules:
- Only emit JSON. No markdown fences, no commentary, no explanation.
- Use repo-relative paths only. Do not use absolute paths.
- "operation": "replace" requires startLine, endLine, oldText, and newText.
- "operation": "insert" requires startLine (insert after) and newText.
- "operation": "delete" requires startLine and endLine.
- If no edits are needed, return { "edits": [] }.
- All file paths must stay inside the workspace and must not contain `..` traversal."#
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_is_non_empty() {
        let prompt = worker_system_prompt();
        assert!(!prompt.is_empty());
        assert!(prompt.contains("edits"));
        assert!(prompt.contains("repo-relative"));
        assert!(!prompt.contains("/absolute/file/path"));
    }

    #[test]
    fn system_prompt_matches_validation_rules() {
        let prompt = worker_system_prompt();
        assert!(prompt.contains("Do not use absolute paths"));
        assert!(prompt.contains("must not contain `..` traversal"));
    }

    #[test]
    fn parse_empty_edits_response() {
        let json = r#"{"edits": []}"#;
        let resp: WorkerEditResponse = serde_json::from_str(json).unwrap();
        assert!(resp.edits.is_empty());
        assert!(resp.summary.is_none());
    }

    #[test]
    fn parse_response_with_summary() {
        let json = r#"{"edits": [], "summary": "no changes needed"}"#;
        let resp: WorkerEditResponse = serde_json::from_str(json).unwrap();
        assert!(resp.edits.is_empty());
        assert_eq!(resp.summary.as_deref(), Some("no changes needed"));
    }

    #[test]
    fn parse_single_edit_response() {
        let json = r#"{
          "edits": [
            {
              "path": "src/main.rs",
              "operation": "replace",
              "startLine": 10,
              "endLine": 12,
              "oldText": "old",
              "newText": "new"
            }
          ]
        }"#;
        let resp: WorkerEditResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.edits.len(), 1);
        assert_eq!(resp.edits[0].path, "src/main.rs");
        assert_eq!(resp.edits[0].operation, "replace");
    }

    #[test]
    fn validate_edit_accepts_relative_path() {
        let edit = PatchEdit {
            path: "src/lib.rs".to_string(),
            operation: "replace".to_string(),
            start_line: Some(1),
            end_line: Some(5),
            old_text: Some("old".to_string()),
            new_text: "new".to_string(),
            anchor_text: None,
            reason: None,
        };
        assert!(validate_edit(&edit).is_ok());
    }

    #[test]
    fn validate_edit_rejects_absolute_path() {
        let edit = PatchEdit {
            path: "/etc/passwd".to_string(),
            operation: "replace".to_string(),
            start_line: Some(1),
            end_line: Some(1),
            old_text: None,
            new_text: "new".to_string(),
            anchor_text: None,
            reason: None,
        };
        let err = validate_edit(&edit).unwrap_err();
        assert!(err.contains("absolute paths are not allowed"));
    }

    #[test]
    fn validate_edit_rejects_path_traversal() {
        let edit = PatchEdit {
            path: "../secrets/key".to_string(),
            operation: "replace".to_string(),
            start_line: Some(1),
            end_line: Some(1),
            old_text: None,
            new_text: "new".to_string(),
            anchor_text: None,
            reason: None,
        };
        let err = validate_edit(&edit).unwrap_err();
        assert!(err.contains("path traversal"));
    }

    #[test]
    fn validate_edit_rejects_deep_traversal() {
        let edit = PatchEdit {
            path: "src/../../../etc/passwd".to_string(),
            operation: "replace".to_string(),
            start_line: Some(1),
            end_line: Some(1),
            old_text: None,
            new_text: "new".to_string(),
            anchor_text: None,
            reason: None,
        };
        let err = validate_edit(&edit).unwrap_err();
        assert!(err.contains("path traversal"));
    }
}
