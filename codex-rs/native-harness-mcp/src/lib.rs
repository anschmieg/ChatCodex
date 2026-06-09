//! MCP transport adapter for the deterministic Codex harness.

#![deny(clippy::print_stdout, clippy::print_stderr)]

use std::borrow::Cow;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::middleware;
use axum::middleware::Next;
use axum::response::Json;
use axum::response::Response;
use axum::routing::get;
use codex_arg0::Arg0DispatchPaths;
use codex_core::harness_mcp::HarnessToolSpec;
use codex_core::harness_mcp::NativeHarness;
use codex_core::harness_mcp::native_tool_catalog;
use rmcp::ErrorData as McpError;
use rmcp::handler::server::ServerHandler;
use rmcp::model::CallToolRequestParams;
use rmcp::model::CallToolResult;
use rmcp::model::JsonObject;
use rmcp::model::ListToolsResult;
use rmcp::model::PaginatedRequestParams;
use rmcp::model::ServerCapabilities;
use rmcp::model::ServerInfo;
use rmcp::model::Tool;
use rmcp::transport::StreamableHttpServerConfig;
use rmcp::transport::StreamableHttpService;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use serde_json::json;

#[derive(Clone)]
pub struct NativeHarnessMcp {
    tools: Arc<Vec<Tool>>,
    harness: NativeHarness,
}

impl NativeHarnessMcp {
    pub async fn new() -> anyhow::Result<Self> {
        Self::new_with_arg0_paths(Arg0DispatchPaths::default()).await
    }

    pub async fn new_with_arg0_paths(arg0_paths: Arg0DispatchPaths) -> anyhow::Result<Self> {
        let workspace = std::env::var_os("CHATCODEX_WORKSPACE_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("/workspaces"));
        Self::new_for_paths(workspace, None, arg0_paths).await
    }

    async fn new_for_paths(
        workspace: impl AsRef<std::path::Path>,
        data_dir: Option<&std::path::Path>,
        arg0_paths: Arg0DispatchPaths,
    ) -> anyhow::Result<Self> {
        let tools = native_tool_catalog()?
            .into_iter()
            .map(tool_from_native_spec)
            .collect::<anyhow::Result<Vec<_>>>()?;
        let harness = match data_dir {
            Some(data_dir) => {
                NativeHarness::new_with_runtime_paths(
                    workspace,
                    data_dir,
                    arg0_paths.codex_linux_sandbox_exe,
                    arg0_paths.main_execve_wrapper_exe,
                )
                .await?
            }
            None => {
                let data_dir = std::env::var_os("CHATCODEX_DATA_DIR")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::path::PathBuf::from("/data"));
                NativeHarness::new_with_runtime_paths(
                    workspace,
                    data_dir,
                    arg0_paths.codex_linux_sandbox_exe,
                    arg0_paths.main_execve_wrapper_exe,
                )
                .await?
            }
        };
        Ok(Self {
            tools: Arc::new(tools),
            harness,
        })
    }

    pub fn tools(&self) -> &[Tool] {
        self.tools.as_slice()
    }
}

pub async fn http_router(
    bearer_token: String,
    arg0_paths: Arg0DispatchPaths,
) -> anyhow::Result<Router> {
    anyhow::ensure!(!bearer_token.is_empty(), "CHATCODEX_BEARER_TOKEN is empty");
    let service = NativeHarnessMcp::new_with_arg0_paths(arg0_paths).await?;
    let expected_bearer = Arc::new(format!("Bearer {bearer_token}"));
    let mcp_service = StreamableHttpService::new(
        move || Ok(service.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );

    Ok(Router::new()
        .route("/healthz", get(healthz))
        .nest_service("/mcp", mcp_service)
        .layer(middleware::from_fn_with_state(
            expected_bearer,
            require_bearer,
        )))
}

async fn healthz() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

async fn require_bearer(
    State(expected): State<Arc<String>>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    if request.uri().path() == "/healthz" {
        return Ok(next.run(request).await);
    }

    let supplied = request
        .headers()
        .get(AUTHORIZATION)
        .map(axum::http::HeaderValue::as_bytes)
        .unwrap_or_default();
    if constant_time_eq(supplied, expected.as_bytes()) {
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
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

fn tool_from_native_spec(native: HarnessToolSpec) -> anyhow::Result<Tool> {
    let definition = native
        .definition
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("native tool {} is not an object", native.name))?;
    let description = definition
        .get("description")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let input_schema = definition
        .get("parameters")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("native tool {} has no parameters", native.name))?;
    let input_schema: JsonObject = serde_json::from_value(input_schema)?;
    let output_schema = definition
        .get("output_schema")
        .cloned()
        .map(serde_json::from_value)
        .transpose()?
        .map(Arc::new);

    Ok(Tool {
        name: Cow::Owned(native.name),
        title: None,
        description: Some(Cow::Owned(description)),
        input_schema: Arc::new(input_schema),
        output_schema,
        annotations: None,
        execution: None,
        icons: None,
        meta: None,
    })
}

impl ServerHandler for NativeHarnessMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..ServerInfo::default()
        }
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        let tools = Arc::clone(&self.tools);
        async move {
            Ok(ListToolsResult {
                tools: (*tools).clone(),
                next_cursor: None,
                meta: None,
            })
        }
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let arguments = serde_json::Value::Object(request.arguments.unwrap_or_default());
        let result = self
            .harness
            .call(request.name.as_ref(), arguments)
            .await
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        let content = result
            .content
            .into_iter()
            .map(serde_json::from_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;

        Ok(CallToolResult {
            content,
            structured_content: result.structured_content,
            is_error: result.is_error,
            meta: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::NativeHarnessMcp;
    use super::constant_time_eq;

    #[tokio::test]
    async fn mcp_catalog_preserves_native_names_and_input_schemas() {
        let workspace = tempfile::tempdir().expect("workspace");
        let data = tempfile::tempdir().expect("data");
        let server = NativeHarnessMcp::new_for_paths(
            workspace.path(),
            Some(data.path()),
            codex_arg0::Arg0DispatchPaths::default(),
        )
        .await
        .expect("native MCP server");
        let names = server
            .tools()
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            [
                "exec_command",
                "write_stdin",
                "update_plan",
                "apply_patch",
                "view_image",
            ]
        );

        let apply_patch = server
            .tools()
            .iter()
            .find(|tool| tool.name == "apply_patch")
            .expect("apply_patch");
        assert_eq!(
            apply_patch.input_schema["required"],
            serde_json::json!(["input"])
        );
    }

    #[test]
    fn bearer_comparison_checks_content_and_length() {
        assert!(constant_time_eq(b"Bearer secret", b"Bearer secret"));
        assert!(!constant_time_eq(b"Bearer secret", b"Bearer other"));
        assert!(!constant_time_eq(b"Bearer secret", b"Bearer secret-longer"));
    }
}
