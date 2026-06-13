//! MCP transport adapter for the deterministic ChatCodex harness.
//!
//! This crate deliberately depends only on public upstream Codex APIs. It does
//! not create a Codex session or turn and cannot invoke an upstream agent loop.

#![deny(clippy::print_stdout, clippy::print_stderr)]

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::response::Json;
use axum::routing::get;
use axum_prometheus::PrometheusMetricLayer;
use codex_arg0::Arg0DispatchPaths;
use codex_exec_server::Environment;
use codex_exec_server::ExecParams;
use codex_exec_server::ExecProcess;
use codex_exec_server::ExecServerRuntimePaths;
use codex_exec_server::FileSystemSandboxContext;
use codex_exec_server::ProcessId;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::SandboxPolicy;
use codex_sandboxing::SandboxCommand;
use codex_sandboxing::SandboxManager;
use codex_sandboxing::SandboxTransformRequest;
use codex_sandboxing::SandboxType;
use codex_shell_command::is_dangerous_command::command_might_be_dangerous;
use codex_utils_absolute_path::AbsolutePathBuf;
use rmcp::ErrorData as McpError;
use rmcp::handler::server::ServerHandler;
use rmcp::model::CallToolRequestParams;
use rmcp::model::CallToolResult;
use rmcp::model::Content;
use rmcp::model::JsonObject;
use rmcp::model::ListToolsResult;
use rmcp::model::PaginatedRequestParams;
use rmcp::model::ServerCapabilities;
use rmcp::model::ServerInfo;
use rmcp::model::Tool;
use rmcp::model::ToolAnnotations;
use rmcp::transport::StreamableHttpServerConfig;
use rmcp::transport::StreamableHttpService;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info_span;
use uuid::Uuid;

const SERVER_INSTRUCTIONS: &str = "\
ChatCodex is a deterministic coding harness. It does not run an agent or call a model. \
Commands execute in a read-only filesystem sandbox; use apply_patch for every workspace write. \
Inspect project manifests before running checks. Prefer mise exec -- <command> and use uv for \
Python environments. System package installation and privilege escalation are forbidden.";

const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const DEFAULT_YIELD_MS: u64 = 10_000;
const MAX_YIELD_MS: u64 = 30_000;

#[derive(Clone)]
pub struct NativeHarnessMcp {
    tools: Arc<Vec<Tool>>,
    harness: Arc<NativeHarness>,
}

struct NativeHarness {
    workspace: AbsolutePathBuf,
    environment: Environment,
    processes: Mutex<HashMap<ProcessId, ProcessSession>>,
    plan: Mutex<Value>,
    linux_sandbox_exe: Option<PathBuf>,
}

struct ProcessSession {
    process: Arc<dyn ExecProcess>,
    next_seq: u64,
}

struct ProcessRead {
    output: String,
    exited: bool,
    exit_code: Option<i32>,
    next_seq: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecCommandArgs {
    cmd: String,
    #[serde(default)]
    yield_time_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteStdinArgs {
    session_id: String,
    #[serde(default)]
    chars: String,
    #[serde(default)]
    yield_time_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyPatchArgs {
    input: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewImageArgs {
    path: PathBuf,
}

impl NativeHarnessMcp {
    pub async fn new() -> anyhow::Result<Self> {
        Self::new_with_arg0_paths(Arg0DispatchPaths::default()).await
    }

    pub async fn new_with_arg0_paths(arg0_paths: Arg0DispatchPaths) -> anyhow::Result<Self> {
        let workspace = std::env::var_os("CHATCODEX_WORKSPACE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/workspaces"));
        Self::new_for_paths(workspace, arg0_paths).await
    }

    async fn new_for_paths(
        workspace: impl AsRef<Path>,
        arg0_paths: Arg0DispatchPaths,
    ) -> anyhow::Result<Self> {
        let workspace = AbsolutePathBuf::from_absolute_path(workspace.as_ref())?;
        let environment = match arg0_paths.codex_self_exe {
            Some(self_exe) => {
                let runtime_paths = ExecServerRuntimePaths::new(
                    self_exe,
                    arg0_paths.codex_linux_sandbox_exe.clone(),
                )?;
                Environment::create(std::env::var("CODEX_EXEC_SERVER_URL").ok(), runtime_paths)?
            }
            None => Environment::create_for_tests(std::env::var("CODEX_EXEC_SERVER_URL").ok())?,
        };
        Ok(Self {
            tools: Arc::new(tool_catalog()?),
            harness: Arc::new(NativeHarness {
                workspace,
                environment,
                processes: Mutex::new(HashMap::new()),
                plan: Mutex::new(json!({"plan": []})),
                linux_sandbox_exe: arg0_paths.codex_linux_sandbox_exe,
            }),
        })
    }

    pub fn tools(&self) -> &[Tool] {
        self.tools.as_slice()
    }
}

impl NativeHarness {
    async fn call(&self, name: &str, arguments: Value) -> anyhow::Result<CallToolResult> {
        match name {
            "exec_command" => self.exec_command(serde_json::from_value(arguments)?).await,
            "write_stdin" => self.write_stdin(serde_json::from_value(arguments)?).await,
            "update_plan" => self.update_plan(arguments).await,
            "apply_patch" => self.apply_patch(serde_json::from_value(arguments)?).await,
            "view_image" => self.view_image(serde_json::from_value(arguments)?).await,
            _ => anyhow::bail!("unknown deterministic tool: {name}"),
        }
    }

    async fn exec_command(&self, args: ExecCommandArgs) -> anyhow::Result<CallToolResult> {
        let shell_argv = vec!["bash".to_string(), "-lc".to_string(), args.cmd];
        if command_might_be_dangerous(&shell_argv) {
            anyhow::bail!("command rejected by the deterministic command policy");
        }

        let policy = SandboxPolicy::ReadOnly {
            network_access: true,
        };
        let permissions = PermissionProfile::from_legacy_sandbox_policy_for_cwd(
            &policy,
            self.workspace.as_path(),
        );
        let (file_system_sandbox_policy, _) = permissions.to_runtime_permissions();
        let transformed = SandboxManager::new().transform(SandboxTransformRequest {
            command: SandboxCommand {
                program: "bash".into(),
                args: shell_argv[1..].to_vec(),
                cwd: self.workspace.clone(),
                env: HashMap::new(),
                additional_permissions: None,
            },
            permissions: &permissions,
            sandbox: platform_sandbox()?,
            enforce_managed_network: false,
            network: None,
            sandbox_policy_cwd: self.workspace.as_path(),
            codex_linux_sandbox_exe: self.linux_sandbox_exe.as_deref(),
            use_legacy_landlock: false,
            windows_sandbox_level: WindowsSandboxLevel::Disabled,
            windows_sandbox_private_desktop: false,
        })?;
        debug_assert_eq!(
            transformed.file_system_sandbox_policy,
            file_system_sandbox_policy
        );

        let process_id = ProcessId::new(Uuid::new_v4().to_string());
        let started = self
            .environment
            .get_exec_backend()
            .start(ExecParams {
                process_id: process_id.clone(),
                argv: transformed.command,
                cwd: transformed.cwd.into_path_buf(),
                env_policy: None,
                env: transformed.env,
                tty: false,
                pipe_stdin: true,
                arg0: transformed.arg0,
            })
            .await?;
        let process = started.process;
        let result = read_process(
            Arc::clone(&process),
            None,
            args.yield_time_ms.unwrap_or(DEFAULT_YIELD_MS),
        )
        .await?;
        if !result.exited {
            self.processes.lock().await.insert(
                process_id.clone(),
                ProcessSession {
                    process,
                    next_seq: result.next_seq,
                },
            );
        }
        Ok(text_result(json!({
            "output": result.output,
            "exit_code": result.exit_code,
            "session_id": if !result.exited {
                Some(process_id.into_inner())
            } else {
                None
            }
        })))
    }

    async fn write_stdin(&self, args: WriteStdinArgs) -> anyhow::Result<CallToolResult> {
        let process_id = ProcessId::new(args.session_id);
        let (process, after_seq) = {
            let processes = self.processes.lock().await;
            let session = processes
                .get(&process_id)
                .ok_or_else(|| anyhow::anyhow!("unknown command session"))?;
            (Arc::clone(&session.process), Some(session.next_seq))
        };
        if !args.chars.is_empty() {
            process.write(args.chars.into_bytes()).await?;
        }
        let result = read_process(
            Arc::clone(&process),
            after_seq,
            args.yield_time_ms.unwrap_or(DEFAULT_YIELD_MS),
        )
        .await?;
        if result.exited {
            self.processes.lock().await.remove(&process_id);
        } else if let Some(session) = self.processes.lock().await.get_mut(&process_id) {
            session.next_seq = result.next_seq;
        }
        Ok(text_result(json!({
            "output": result.output,
            "exited": result.exited,
            "exit_code": result.exit_code
        })))
    }

    async fn update_plan(&self, arguments: Value) -> anyhow::Result<CallToolResult> {
        let plan = arguments
            .get("plan")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("plan must be an array"))?;
        if plan
            .iter()
            .filter(|item| item["status"] == "in_progress")
            .count()
            > 1
        {
            anyhow::bail!("at most one plan item may be in progress");
        }
        *self.plan.lock().await = arguments.clone();
        Ok(text_result(arguments))
    }

    async fn apply_patch(&self, args: ApplyPatchArgs) -> anyhow::Result<CallToolResult> {
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: Vec::new(),
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        };
        let sandbox =
            FileSystemSandboxContext::from_legacy_sandbox_policy(policy, self.workspace.clone());
        let sandbox =
            if self.environment.is_remote() || self.environment.local_runtime_paths().is_some() {
                Some(&sandbox)
            } else {
                None
            };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = codex_apply_patch::apply_patch(
            &args.input,
            &self.workspace,
            &mut stdout,
            &mut stderr,
            self.environment.get_filesystem().as_ref(),
            sandbox,
        )
        .await;
        let output = format!(
            "{}{}",
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr)
        );
        match result {
            Ok(_) => Ok(text_result(json!({"result": output}))),
            Err(error) => Ok(error_result(format!("{output}{error}"))),
        }
    }

    async fn view_image(&self, args: ViewImageArgs) -> anyhow::Result<CallToolResult> {
        let policy = SandboxPolicy::ReadOnly {
            network_access: false,
        };
        let sandbox =
            FileSystemSandboxContext::from_legacy_sandbox_policy(policy, self.workspace.clone());
        let sandbox =
            if self.environment.is_remote() || self.environment.local_runtime_paths().is_some() {
                Some(&sandbox)
            } else {
                None
            };
        let fs = self.environment.get_filesystem();
        let path =
            resolve_workspace_path(fs.as_ref(), &self.workspace, &args.path, sandbox).await?;
        let data = fs.read_file(&path, sandbox).await?;
        let mime = mime_guess::from_path(path.as_path())
            .first_raw()
            .unwrap_or("application/octet-stream");
        let content: Content = serde_json::from_value(json!({
            "type": "image",
            "data": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                data
            ),
            "mimeType": mime
        }))?;
        let mut result = CallToolResult::success(vec![content]);
        result.structured_content = Some(json!({"path": path.as_path()}));
        Ok(result)
    }
}

async fn read_process(
    process: Arc<dyn ExecProcess>,
    after_seq: Option<u64>,
    yield_time_ms: u64,
) -> anyhow::Result<ProcessRead> {
    let response = process
        .read(
            after_seq,
            Some(MAX_OUTPUT_BYTES),
            Some(yield_time_ms.min(MAX_YIELD_MS)),
        )
        .await?;
    let output = response
        .chunks
        .into_iter()
        .flat_map(|chunk| chunk.chunk.into_inner())
        .collect::<Vec<_>>();
    Ok(ProcessRead {
        output: String::from_utf8_lossy(&output).into_owned(),
        exited: response.exited,
        exit_code: response.exit_code,
        next_seq: response.next_seq,
    })
}

fn platform_sandbox() -> anyhow::Result<SandboxType> {
    codex_sandboxing::get_platform_sandbox(false)
        .ok_or_else(|| anyhow::anyhow!("this platform has no supported read-only sandbox"))
}

async fn resolve_workspace_path(
    fs: &dyn codex_exec_server::ExecutorFileSystem,
    workspace: &AbsolutePathBuf,
    requested: &Path,
    sandbox: Option<&FileSystemSandboxContext>,
) -> anyhow::Result<AbsolutePathBuf> {
    let path = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        workspace.as_path().join(requested)
    };
    let path = AbsolutePathBuf::from_absolute_path(path)?;
    let canonical_workspace = fs.canonicalize(workspace, sandbox).await?;
    let canonical_path = fs.canonicalize(&path, sandbox).await?;
    if !canonical_path
        .as_path()
        .starts_with(canonical_workspace.as_path())
    {
        anyhow::bail!("path is outside the configured workspace");
    }
    Ok(canonical_path)
}

fn text_result(value: Value) -> CallToolResult {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    let mut result = CallToolResult::success(vec![Content::text(text)]);
    result.structured_content = Some(value);
    result
}

fn error_result(message: String) -> CallToolResult {
    CallToolResult::error(vec![Content::text(message)])
}

fn tool_catalog() -> anyhow::Result<Vec<Tool>> {
    [
        (
            "exec_command",
            "Run a command in the workspace under a read-only filesystem sandbox.",
            json!({"type":"object","properties":{"cmd":{"type":"string"},"yield_time_ms":{"type":"integer","minimum":0,"maximum":30000}},"required":["cmd"],"additionalProperties":false}),
        ),
        (
            "write_stdin",
            "Write to or poll a running command session.",
            json!({"type":"object","properties":{"session_id":{"type":"string"},"chars":{"type":"string"},"yield_time_ms":{"type":"integer","minimum":0,"maximum":30000}},"required":["session_id"],"additionalProperties":false}),
        ),
        (
            "update_plan",
            "Replace the deterministic task plan.",
            json!({"type":"object","properties":{"explanation":{"type":"string"},"plan":{"type":"array","items":{"type":"object","properties":{"step":{"type":"string"},"status":{"enum":["pending","in_progress","completed"]}},"required":["step","status"],"additionalProperties":false}}},"required":["plan"],"additionalProperties":false}),
        ),
        (
            "apply_patch",
            "Apply a structured patch inside the workspace.",
            json!({"type":"object","properties":{"input":{"type":"string"}},"required":["input"],"additionalProperties":false}),
        ),
        (
            "view_image",
            "Read an image located inside the workspace.",
            json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}),
        ),
    ]
    .into_iter()
    .map(|(name, description, schema)| {
        Ok(Tool::new(
            Cow::Borrowed(name),
            Cow::Borrowed(description),
            Arc::new(serde_json::from_value::<JsonObject>(schema)?),
        )
        .with_annotations(tool_annotations(name)?))
    })
    .collect()
}

fn tool_annotations(name: &str) -> anyhow::Result<ToolAnnotations> {
    Ok(match name {
        "exec_command" => ToolAnnotations::with_title("Run command")
            .read_only(true)
            .destructive(false)
            .idempotent(false)
            .open_world(true),
        "write_stdin" => ToolAnnotations::with_title("Interact with command")
            .read_only(true)
            .destructive(false)
            .idempotent(false)
            .open_world(true),
        "update_plan" => ToolAnnotations::with_title("Update task plan")
            .read_only(false)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        "apply_patch" => ToolAnnotations::with_title("Apply workspace patch")
            .read_only(false)
            .destructive(true)
            .idempotent(false)
            .open_world(false),
        "view_image" => ToolAnnotations::with_title("View local image")
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        _ => anyhow::bail!("missing annotation policy for {name}"),
    })
}

impl ServerHandler for NativeHarnessMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(SERVER_INSTRUCTIONS)
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
        self.harness
            .call(
                request.name.as_ref(),
                Value::Object(request.arguments.unwrap_or_default()),
            )
            .await
            .map_err(|error| McpError::invalid_params(error.to_string(), None))
    }
}

pub async fn http_router(arg0_paths: Arg0DispatchPaths) -> anyhow::Result<Router> {
    let auth_state = codex_native_harness_mcp_auth::AuthState::from_env()?;
    http_router_with_state(arg0_paths, auth_state).await
}

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
    static PROMETHEUS_HANDLE: std::sync::OnceLock<metrics_exporter_prometheus::PrometheusHandle> =
        std::sync::OnceLock::new();
    let prometheus_layer = PrometheusMetricLayer::new();
    let _ = PROMETHEUS_HANDLE.get_or_init(|| {
        let (_layer, handle) = PrometheusMetricLayer::pair();
        handle
    });

    Ok(Router::new()
        .route(
            "/metrics",
            get(|| async {
                PROMETHEUS_HANDLE
                    .get()
                    .expect("Prometheus handle initialized")
                    .clone()
                    .render()
            }),
        )
        .route("/healthz", get(|| async { Json(json!({"status":"ok"})) }))
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
                    axum::routing::post(codex_native_harness_mcp_auth::token::token).layer(
                        axum::middleware::from_fn_with_state(
                            codex_native_harness_mcp_auth::ratelimit::RateLimiter::new(10, 60),
                            codex_native_harness_mcp_auth::ratelimit::rate_limit_token,
                        ),
                    ),
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
            Router::new().fallback_service(mcp_service).layer(
                axum::middleware::from_fn_with_state(
                    auth_state,
                    codex_native_harness_mcp_auth::middleware::require_bearer,
                ),
            ),
        )
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &axum::extract::Request| {
                info_span!(
                    "http_request",
                    method = %request.method(),
                    uri = %request.uri()
                )
            }),
        )
        .layer(prometheus_layer)
        .layer(CorsLayer::permissive()))
}

#[cfg(test)]
mod tests {
    use super::NativeHarnessMcp;

    #[tokio::test]
    async fn catalog_is_strictly_allowlisted() {
        let workspace = tempfile::tempdir().expect("workspace");
        let server = NativeHarnessMcp::new_for_paths(
            workspace.path(),
            codex_arg0::Arg0DispatchPaths::default(),
        )
        .await
        .expect("server");
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
                "view_image"
            ]
        );
        assert!(server.tools().iter().all(|tool| tool.annotations.is_some()));
    }

    #[tokio::test]
    async fn path_resolution_rejects_escape() {
        let workspace_dir = tempfile::tempdir().expect("workspace");
        let workspace =
            codex_utils_absolute_path::AbsolutePathBuf::from_absolute_path(workspace_dir.path())
                .expect("absolute");
        let environment =
            codex_exec_server::Environment::create_for_tests(None).expect("test environment");
        assert!(
            super::resolve_workspace_path(
                environment.get_filesystem().as_ref(),
                &workspace,
                std::path::Path::new("/etc/passwd"),
                None,
            )
            .await
            .is_err()
        );

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/passwd", workspace_dir.path().join("escape"))
                .expect("symlink");
            assert!(
                super::resolve_workspace_path(
                    environment.get_filesystem().as_ref(),
                    &workspace,
                    std::path::Path::new("escape"),
                    None,
                )
                .await
                .is_err()
            );
        }
    }

    #[test]
    fn dangerous_shell_commands_are_rejected_by_upstream_policy() {
        let command = vec![
            "bash".to_string(),
            "-lc".to_string(),
            "rm -rf /".to_string(),
        ];
        assert!(codex_shell_command::is_dangerous_command::command_might_be_dangerous(&command));
    }

    #[tokio::test]
    async fn apply_patch_is_the_workspace_write_path() {
        let workspace = tempfile::tempdir().expect("workspace");
        let server = NativeHarnessMcp::new_for_paths(
            workspace.path(),
            codex_arg0::Arg0DispatchPaths::default(),
        )
        .await
        .expect("server");
        let result = server
            .harness
            .call(
                "apply_patch",
                serde_json::json!({
                    "input": "*** Begin Patch\n*** Add File: proof.txt\n+patched\n*** End Patch\n"
                }),
            )
            .await
            .expect("apply patch");
        assert_eq!(result.is_error, Some(false), "{result:?}");
        assert_eq!(
            std::fs::read_to_string(workspace.path().join("proof.txt")).expect("patched file"),
            "patched\n"
        );
    }
}
