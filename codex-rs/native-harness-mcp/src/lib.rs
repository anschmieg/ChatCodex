//! MCP transport adapter for the deterministic Codex harness.
//!
//! The HTTP transport is mounted behind an OAuth 2.1 / MCP 2025-11-25
//! authorization layer provided by `codex-native-harness-mcp-auth`. The
//! `/mcp` route is protected by a bearer-JWT middleware; discovery,
//! client registration, authorize/token, and JWKS routes are exposed
//! publicly so the ChatGPT client can complete the flow.

#![deny(clippy::print_stdout, clippy::print_stderr)]

use std::borrow::Cow;
use std::sync::Arc;

use axum::Router;
use axum::response::Json;
use axum::routing::get;
use axum_prometheus::PrometheusMetricLayer;
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
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info_span;

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
    arg0_paths: Arg0DispatchPaths,
) -> anyhow::Result<Router> {
    let auth_state = codex_native_harness_mcp_auth::AuthState::from_env()?;
    http_router_with_state(arg0_paths, auth_state).await
}

/// Like [] but accepts a pre-built [], useful for
/// tests that want to inject a stub Cloudflare Access verifier.
pub async fn http_router_with_state(
    arg0_paths: Arg0DispatchPaths,
    auth_state: codex_native_harness_mcp_auth::AuthState,
) -> anyhow::Result<Router> {
    let service = NativeHarnessMcp::new_with_arg0_paths(arg0_paths).await?;
    let mcp_service = StreamableHttpService::new(
        move || Ok(service.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );

    // CORS: permissive for well-known and OAuth endpoints; the MCP endpoint
    // is protected by bearer auth so CORS is less critical there, but we
    // allow all origins for browser-based MCP clients.
    let cors = CorsLayer::permissive();

    // Prometheus metrics: track request count, duration, and expose /metrics.
    // Use OnceLock to ensure the global metrics recorder is only initialized once
    // (PrometheusMetricLayer::pair() calls metrics::set_boxed_recorder() internally).
    static PROMETHEUS_HANDLE: std::sync::OnceLock<metrics_exporter_prometheus::PrometheusHandle> = std::sync::OnceLock::new();
    let prometheus_layer = PrometheusMetricLayer::new();
    let _ = PROMETHEUS_HANDLE.get_or_init(|| {
        let (_layer, handle) = PrometheusMetricLayer::pair();
        handle
    });

    let router = Router::new()
        .route("/metrics", get(|| async {
            PROMETHEUS_HANDLE.get().expect("Prometheus handle initialized").clone().render()
        }))
        .route("/healthz", get(healthz))
        .merge(
            Router::new()
                .route(
                    "/.well-known/oauth-authorization-server",
                    get(codex_native_harness_mcp_auth::well_known::oauth_authorization_server),
                )
                .route(
                    "/.well-known/oauth-protected-resource",
                    get(codex_native_harness_mcp_auth::well_known::oauth_protected_resource),
                )
                .route(
                    "/.well-known/jwks.json",
                    get(codex_native_harness_mcp_auth::well_known::jwks),
                )
                .route(
                    "/oauth/authorize",
                    get(codex_native_harness_mcp_auth::authorize::authorize),
                )
                .route(
                    "/oauth/authorize/decide",
                    axum::routing::post(codex_native_harness_mcp_auth::authorize::decide),
                )
                .route(
                    "/oauth/register",
                    axum::routing::post(codex_native_harness_mcp_auth::clients::register),
                )
                .route(
                    "/oauth/token",
                    axum::routing::post(codex_native_harness_mcp_auth::token::token)
                        .layer(axum::middleware::from_fn_with_state(
                            codex_native_harness_mcp_auth::ratelimit::RateLimiter::new(10, 60),
                            codex_native_harness_mcp_auth::ratelimit::rate_limit_token,
                        )),
                )
                .route(
                    "/oauth/introspect",
                    axum::routing::post(codex_native_harness_mcp_auth::token::introspect),
                )
                .route(
                    "/oauth/revoke",
                    axum::routing::post(codex_native_harness_mcp_auth::token::revoke),
                )
                .with_state(auth_state.clone()),
        )
        .nest(
                    "/mcp",
                    Router::new()
                        .fallback_service(mcp_service)
                        .layer(axum::middleware::from_fn_with_state(
                            auth_state,
                            codex_native_harness_mcp_auth::middleware::require_bearer,
                        )),
                )
        // Tracing: log method, uri, status, and duration for every request.
        .layer(TraceLayer::new_for_http().make_span_with(|request: &axum::extract::Request| {
            let uri = request.uri().to_string();
            let method = request.method().to_string();
            info_span!("http_request", method, uri)
        }))
        .layer(prometheus_layer.clone())
        .layer(cors);

    Ok(router)
}

async fn healthz() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
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

}
