//! MCP transport adapter for the deterministic ChatCodex harness.
//!
//! This crate deliberately depends only on public upstream Codex APIs. It does
//! not create a Codex session or turn and cannot invoke an upstream agent loop.

#![deny(clippy::print_stdout, clippy::print_stderr)]

mod app_resources;
mod lifecycle;

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

mod hindsight;

use anyhow::Context;
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
use codex_http_client::HttpClientFactory;
use codex_http_client::OutboundProxyPolicy;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::SandboxPolicy;
use codex_sandboxing::SandboxCommand;
use codex_sandboxing::SandboxManager;
use codex_sandboxing::SandboxTransformRequest;
use codex_sandboxing::SandboxType;
use codex_shell_command::is_dangerous_command::dangerous_command_match;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_path_uri::PathUri;
use rmcp::ErrorData as McpError;
use rmcp::handler::server::ServerHandler;
use rmcp::model::AnnotateAble;
use rmcp::model::CallToolRequestParams;
use rmcp::model::CallToolResult;
use rmcp::model::Content;
use rmcp::model::GetPromptRequestParams;
use rmcp::model::GetPromptResult;
use rmcp::model::JsonObject;
use rmcp::model::ListPromptsResult;
use rmcp::model::ListResourceTemplatesResult;
use rmcp::model::ListResourcesResult;
use rmcp::model::ListToolsResult;
use rmcp::model::Meta;
use rmcp::model::PaginatedRequestParams;
use rmcp::model::Prompt;
use rmcp::model::PromptMessage;
use rmcp::model::PromptMessageRole;
use rmcp::model::RawResource;
use rmcp::model::ReadResourceRequestParams;
use rmcp::model::ReadResourceResult;
use rmcp::model::ResourceContents;
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
use std::time::Duration;
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info_span;
use uuid::Uuid;

const SERVER_INSTRUCTIONS: &str = "\
# ChatCodex — Deterministic Coding Harness

You are connected to a deterministic coding harness. Your goal is to complete the \
user's coding task end-to-end. ChatGPT is the only reasoning engine: the server only \
records deterministic project/run state and executes explicit tools.

## Golden rules
1. Commands execute in a read-only filesystem sandbox — use `apply_patch` for ALL workspace source writes.
2. Create or select a persistent project, then start or resume a persistent run for the first coding request.
3. Keep the run phase accurate: inspect, plan, execute, then verify.
4. Track the plan with `update_plan` and the checklist with `todo`; these persist on the active run.
5. Chain fine-grained deterministic tools while work remains and the run is active.
6. Verify every acceptance criterion before marking the run completed.
7. Stop only when the run is completed, cancelled, blocked, paused, or awaiting approval.
8. External effects require awaiting_approval unless an existing authorized tool policy explicitly permits them.
9. System package installation and privilege escalation are FORBIDDEN.

## Tool usage
- `project_create` / `project_select` / `project_list` / `project_get` — manage persistent projects
- `run_start` / `run_list` / `run_get` / `run_update` / `run_resume` / `run_cancel` — manage persistent runs
- `exec_command` / `write_stdin` — run commands, start interactive sessions
- `apply_patch` — the ONLY workspace source write path
- `read_file` / `search_code` / `list_directory` — inspect the workspace
- `git_status` / `git_diff` — inspect git state
- `git` / `git_commit` / `git_branch` / `git_checkout` — local git operations with outbound network commands blocked
- `setup_workspace` — legacy compatibility path that creates/selects a persistent project
- `update_plan` / `todo` — track run progress
- `view_image` — view images in the workspace

## Completion protocol
1. **Select context**: Create/select a project and start/resume a run.
2. **Plan**: Record phase `plan`, then create concrete plan and TODO items.
3. **Execute**: Record phase `execute`, work one item at a time, and update the checklist after each item.
4. **Verify**: Record phase `verify`, run the real checks, and compare results to acceptance criteria.
5. **Finish**: Use `run_update(status: \"completed\", work_remaining: false)` only after verification passes. Use `blocked`, `paused`, or `awaiting_approval` when work cannot safely continue.";

const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const DEFAULT_YIELD_MS: u64 = 10_000;
const MAX_YIELD_MS: u64 = 30_000;

#[derive(Clone)]
pub struct NativeHarnessMcp {
    tools: Arc<Vec<Tool>>,
    prompts: Arc<Vec<Prompt>>,
    harness: Arc<NativeHarness>,
}

struct NativeHarness {
    workspace_base: AbsolutePathBuf,
    lifecycle: lifecycle::LifecycleStore,
    environment: Environment,
    processes: Mutex<HashMap<ProcessId, ProcessSession>>,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadFileArgs {
    path: PathBuf,
    #[serde(default)]
    start_line: Option<usize>,
    #[serde(default)]
    end_line: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchCodeArgs {
    query: String,
    #[serde(default)]
    path_glob: Option<String>,
    #[serde(default)]
    max_results: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GitStatusArgs {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetTimeArgs {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GitDiffArgs {
    #[serde(default)]
    paths: Option<Vec<String>>,
    #[serde(default)]
    staged: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListDirectoryArgs {
    #[serde(default)]
    path: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetupWorkspaceArgs {
    source: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    /// Branch, tag, or commit-ish to check out after clone (JSON key: `ref`).
    #[serde(default, rename = "ref")]
    ref_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GitToolArgs {
    command: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GitCommitArgs {
    message: String,
    #[serde(default)]
    allow_empty: bool,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GitBranchArgs {
    name: String,
    #[serde(default)]
    start_point: Option<String>,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GitCheckoutArgs {
    target: String,
    #[serde(default)]
    create_branch: bool,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GitPushArgs {
    /// Branch to push. Defaults to the current branch.
    #[serde(default)]
    branch: Option<String>,
    /// When true, create a pull request after the push using the gh CLI
    /// (requires CHATCODEX_GITHUB_TOKEN and a github.com remote).
    #[serde(default)]
    create_pr: bool,
    /// Base branch for the pull request (default: the remote default branch).
    #[serde(default)]
    base: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TodoArgs {
    items: Vec<TodoItemInput>,
    #[serde(default)]
    action: TodoAction,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TodoItemInput {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum TodoAction {
    #[default]
    Replace,
    Update,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectCreateArgs {
    kind: lifecycle::ProjectKind,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    path: Option<PathBuf>,
    #[serde(default = "default_true")]
    select: bool,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectSelectArgs {
    project_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectListArgs {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectGetArgs {
    #[serde(default)]
    project_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunStartArgs {
    #[serde(default)]
    project_id: Option<String>,
    objective: String,
    #[serde(default)]
    acceptance_criteria: Vec<String>,
    #[serde(default)]
    autonomy: lifecycle::AutonomyEnvelope,
    #[serde(default = "default_true")]
    select: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunListArgs {
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    status: Option<lifecycle::RunStatus>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunGetArgs {
    #[serde(default)]
    run_id: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RunUpdateArgs {
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    phase: Option<lifecycle::RunPhase>,
    #[serde(default)]
    status: Option<lifecycle::RunStatus>,
    #[serde(default)]
    acceptance_criteria: Option<Vec<String>>,
    #[serde(default)]
    plan: Option<Vec<lifecycle::PlanItem>>,
    #[serde(default)]
    checklist: Option<Vec<lifecycle::ChecklistItem>>,
    #[serde(default)]
    checkpoint: Option<lifecycle::CheckpointInput>,
    #[serde(default)]
    work_remaining: Option<bool>,
    #[serde(default)]
    next_action: Option<String>,
    #[serde(default)]
    step_delta: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunResumeArgs {
    #[serde(default)]
    run_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunCancelArgs {
    #[serde(default)]
    run_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FollowupLeaseArgs {
    run_id: String,
    #[serde(default)]
    requested_nonce: Option<String>,
    #[serde(default)]
    ttl_ms: Option<u64>,
    #[serde(default)]
    delay_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanEnvelope {
    #[serde(default)]
    explanation: Option<String>,
    plan: Vec<lifecycle::PlanItem>,
}

fn default_true() -> bool {
    true
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
        workspace_base: impl AsRef<Path>,
        arg0_paths: Arg0DispatchPaths,
    ) -> anyhow::Result<Self> {
        let workspace_base = AbsolutePathBuf::from_absolute_path(workspace_base.as_ref())?;
        validate_runtime(
            &workspace_base,
            arg0_paths.codex_linux_sandbox_exe.as_deref(),
        )?;
        let environment = match arg0_paths.codex_self_exe {
            Some(self_exe) => {
                let runtime_paths = ExecServerRuntimePaths::new(
                    self_exe,
                    arg0_paths.codex_linux_sandbox_exe.clone(),
                )?;
                Environment::create(
                    std::env::var("CODEX_EXEC_SERVER_URL").ok(),
                    runtime_paths,
                    HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault),
                )?
            }
            None => {
                let linux_sandbox_exe = arg0_paths.codex_linux_sandbox_exe.clone();
                let self_exe = linux_sandbox_exe
                    .clone()
                    .or_else(|| std::env::current_exe().ok())
                    .ok_or_else(|| anyhow::anyhow!("could not determine self executable"))?;
                let runtime_paths = ExecServerRuntimePaths::new(self_exe, linux_sandbox_exe)?;
                Environment::create(
                    std::env::var("CODEX_EXEC_SERVER_URL").ok(),
                    runtime_paths,
                    HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault),
                )?
            }
        };
        let client_id =
            std::env::var("CHATCODEX_CLIENT_ID").unwrap_or_else(|_| "default".to_string());
        let lifecycle = lifecycle::LifecycleStore::open(workspace_base.as_path(), &client_id)?;
        Ok(Self {
            tools: Arc::new(tool_catalog()?),
            prompts: Arc::new(prompt_catalog()),
            harness: Arc::new(NativeHarness {
                workspace_base,
                lifecycle,
                environment,
                processes: Mutex::new(HashMap::new()),
                linux_sandbox_exe: arg0_paths.codex_linux_sandbox_exe,
            }),
        })
    }

    pub fn tools(&self) -> &[Tool] {
        self.tools.as_slice()
    }
}

impl NativeHarness {
    async fn workspace_or_error(&self) -> anyhow::Result<AbsolutePathBuf> {
        let state = self.lifecycle.snapshot()?;
        let active_project_id = state.active_project_id.clone();
        let project_id = state
            .active_run_id
            .as_deref()
            .and_then(|run_id| state.runs.get(run_id))
            .map(|run| run.project_id.clone())
            .or(active_project_id);
        let project = project_id
            .as_deref()
            .and_then(|id| state.projects.get(id))
            .ok_or_else(|| anyhow::anyhow!("Workspace not configured. Call project_create/project_select or setup_workspace(source: '<git-url>'|'sandbox') first."))?;
        AbsolutePathBuf::from_absolute_path(&project.workspace_root)
            .context("selected project workspace path is not absolute")
    }

    async fn call(&self, name: &str, arguments: Value) -> anyhow::Result<CallToolResult> {
        let mut result = match name {
            "exec_command" => self.exec_command(serde_json::from_value(arguments)?).await,
            "write_stdin" => self.write_stdin(serde_json::from_value(arguments)?).await,
            "update_plan" => self.update_plan(arguments).await,
            "apply_patch" => self.apply_patch(serde_json::from_value(arguments)?).await,
            "view_image" => self.view_image(serde_json::from_value(arguments)?).await,
            "read_file" => self.read_file(serde_json::from_value(arguments)?).await,
            "search_code" => self.search_code(serde_json::from_value(arguments)?).await,
            "setup_workspace" => {
                self.setup_workspace(serde_json::from_value(arguments)?)
                    .await
            }
            "git" => self.git_tool(serde_json::from_value(arguments)?).await,
            "git_status" => self.git_status(serde_json::from_value(arguments)?).await,
            "get_time" => self.get_time(serde_json::from_value(arguments)?).await,
            "memory_search" => {
                hindsight::memory_search(serde_json::from_value(arguments)?).await
            }
            "memory_retain" => {
                hindsight::memory_retain(serde_json::from_value(arguments)?).await
            }
            "memory_reflect" => {
                hindsight::memory_reflect(serde_json::from_value(arguments)?).await
            }
            "git_diff" => self.git_diff(serde_json::from_value(arguments)?).await,
            "git_commit" => self.git_commit(serde_json::from_value(arguments)?).await,
            "git_branch" => self.git_branch(serde_json::from_value(arguments)?).await,
            "git_checkout" => self.git_checkout(serde_json::from_value(arguments)?).await,
            "git_push" => self.git_push(serde_json::from_value(arguments)?).await,
            "list_directory" => {
                self.list_directory(serde_json::from_value(arguments)?)
                    .await
            }
            "todo" => self.todo(serde_json::from_value(arguments)?).await,
            "project_create" => {
                self.project_create(serde_json::from_value(arguments)?)
                    .await
            }
            "project_select" => {
                self.project_select(serde_json::from_value(arguments)?)
                    .await
            }
            "project_list" => self.project_list(serde_json::from_value(arguments)?).await,
            "project_get" => self.project_get(serde_json::from_value(arguments)?).await,
            "run_start" => self.run_start(serde_json::from_value(arguments)?).await,
            "run_list" => self.run_list(serde_json::from_value(arguments)?).await,
            "run_get" => self.run_get(serde_json::from_value(arguments)?).await,
            "run_update" => self.run_update(serde_json::from_value(arguments)?).await,
            "run_resume" => self.run_resume(serde_json::from_value(arguments)?).await,
            "run_cancel" => self.run_cancel(serde_json::from_value(arguments)?).await,
            "run_followup_lease" => {
                self.run_followup_lease(serde_json::from_value(arguments)?)
                    .await
            }
            _ => anyhow::bail!("unknown deterministic tool: {name}"),
        }?;
        if tool_gets_active_run_metadata(name) {
            self.attach_active_run_metadata(&mut result)?;
        }
        Ok(result)
    }

    async fn exec_command(&self, args: ExecCommandArgs) -> anyhow::Result<CallToolResult> {
        self.ensure_active_run_can("run local commands", |run| {
            run.autonomy.allow_local_commands
        })?;
        let workspace = self.workspace_or_error().await?;
        let shell_argv = vec!["bash".to_string(), "-lc".to_string(), args.cmd];
        if dangerous_command_match(&shell_argv).is_some() {
            anyhow::bail!("command rejected by the deterministic command policy");
        }

        let policy = SandboxPolicy::ReadOnly {
            network_access: true,
        };
        let permissions =
            PermissionProfile::from_legacy_sandbox_policy_for_cwd(&policy, workspace.as_path());
        let (file_system_sandbox_policy, _) = permissions.to_runtime_permissions();
        let sandbox = self.resolve_sandbox_type();
        let no_sandbox_warning = if sandbox == SandboxType::None {
            Some(
                "Warning: read-only filesystem sandbox is not available in this environment; commands MAY modify the workspace.",
            )
        } else {
            None
        };
        let mut cmd_env = HashMap::new();
        if sandbox != SandboxType::None {
            if let Some(path) = std::env::var_os("PATH") {
                cmd_env.insert("PATH".to_string(), path.to_string_lossy().into_owned());
            }
            if let Some(home) = std::env::var_os("HOME") {
                cmd_env.insert("HOME".to_string(), home.to_string_lossy().into_owned());
            }
        }
        let workspace_uri = PathUri::from_abs_path(&workspace);
        let transformed = SandboxManager::new().transform(SandboxTransformRequest {
            command: SandboxCommand {
                program: "bash".into(),
                args: shell_argv[1..].to_vec(),
                cwd: workspace_uri.clone(),
                env: cmd_env,
                managed_network: None,
                additional_permissions: None,
            },
            permissions: &permissions,
            sandbox,
            enforce_managed_network: false,
            environment_id: None,
            network: None,
            sandbox_policy_cwd: &workspace_uri,
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
                cwd: transformed.cwd,
                env_policy: None,
                env: transformed.env,
                tty: false,
                pipe_stdin: true,
                arg0: transformed.arg0,
                sandbox: None,
                enforce_managed_network: false,
                managed_network: None,
                network_proxy: None,
            })
            .await?;
        let process = started.process;
        let result = read_process(
            Arc::clone(&process),
            None,
            args.yield_time_ms.unwrap_or(DEFAULT_YIELD_MS),
        )
        .await?;
        let output = match no_sandbox_warning {
            Some(warning) => format!("{warning}\n\n{}", result.output),
            None => result.output,
        };
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
            "output": output,
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
            tokio::task::yield_now().await;
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
        let envelope: PlanEnvelope = serde_json::from_value(arguments.clone())?;
        lifecycle::validate_plan(&envelope.plan)?;
        if let Some(run) = self.lifecycle.active_run()? {
            let PlanEnvelope { plan, .. } = envelope;
            self.lifecycle.update_run(lifecycle::RunUpdate {
                run_id: Some(run.id),
                plan: Some(plan),
                ..lifecycle::RunUpdate::default()
            })?;
        } else {
            self.lifecycle.set_legacy_plan(arguments.clone())?;
        }
        Ok(text_result(arguments))
    }

    async fn apply_patch(&self, args: ApplyPatchArgs) -> anyhow::Result<CallToolResult> {
        self.ensure_active_run_can("edit files", |run| run.autonomy.allow_file_edits)?;
        let workspace = self.workspace_or_error().await?;
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: Vec::new(),
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        };
        let workspace_uri = PathUri::from_abs_path(&workspace);
        let sandbox =
            FileSystemSandboxContext::from_legacy_sandbox_policy(policy, workspace_uri.clone())?;
        let sandbox = if self.sandbox_available() {
            Some(&sandbox)
        } else {
            None
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = codex_apply_patch::apply_patch(
            &args.input,
            &workspace_uri,
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
        let workspace = self.workspace_or_error().await?;
        let policy = SandboxPolicy::ReadOnly {
            network_access: false,
        };
        let sandbox =
            FileSystemSandboxContext::from_legacy_sandbox_policy(policy, PathUri::from_abs_path(&workspace))?;
        let sandbox = if self.sandbox_available() {
            Some(&sandbox)
        } else {
            None
        };
        let fs = self.environment.get_filesystem();
        let path = resolve_workspace_path(fs.as_ref(), &workspace, &args.path, sandbox).await?;
        let data = fs.read_file(&path, sandbox).await?;
        let mime = mime_guess::from_path(path.to_path_buf())
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
        result.structured_content = Some(json!({"path": path.to_path_buf()}));
        Ok(result)
    }

    async fn read_file(&self, args: ReadFileArgs) -> anyhow::Result<CallToolResult> {
        let workspace = self.workspace_or_error().await?;
        let policy = SandboxPolicy::ReadOnly {
            network_access: false,
        };
        let sandbox =
            FileSystemSandboxContext::from_legacy_sandbox_policy(policy, PathUri::from_abs_path(&workspace))?;
        let sandbox = if self.sandbox_available() {
            Some(&sandbox)
        } else {
            None
        };
        let fs = self.environment.get_filesystem();
        let path = resolve_workspace_path(fs.as_ref(), &workspace, &args.path, sandbox).await?;
        let data = fs.read_file_text(&path, sandbox).await?;
        let lines: Vec<&str> = data.lines().collect();
        let total_lines = lines.len();
        let start = args.start_line.unwrap_or(1).saturating_sub(1);
        let end = args.end_line.unwrap_or(total_lines).min(total_lines);
        if start >= total_lines {
            anyhow::bail!(
                "start_line {} exceeds file length {}",
                args.start_line.unwrap_or(1),
                total_lines
            );
        }
        let selected = if start > 0 || end < total_lines {
            lines[start..end].join("\n")
        } else {
            data.clone()
        };
        Ok(text_result(json!({
            "path": path.to_path_buf(),
            "total_lines": total_lines,
            "start_line": start + 1,
            "end_line": end,
            "content": selected,
        })))
    }

    async fn search_code(&self, args: SearchCodeArgs) -> anyhow::Result<CallToolResult> {
        let workspace = self.workspace_or_error().await?;
        if args.query.is_empty() {
            return Ok(text_result(json!({"matches": []})));
        }
        let max = args.max_results.unwrap_or(50);
        let output = if let Some(glob) = &args.path_glob {
            let glob = glob.trim_start_matches("**/");
            let mut find_cmd = std::process::Command::new("find");
            find_cmd.arg(".");
            find_cmd.arg("-type").arg("f");
            if glob.contains('/') {
                find_cmd.arg("-path");
                find_cmd.arg(format!("./{}", glob));
            } else {
                find_cmd.arg("-name");
                find_cmd.arg(glob);
            }
            find_cmd
                .arg("-exec")
                .arg("grep")
                .arg("-nH")
                .arg("--")
                .arg(&args.query)
                .arg("{}")
                .arg("+")
                .current_dir(workspace.as_path())
                .output()
                .context("failed to run find+grep")?
        } else {
            let mut cmd = std::process::Command::new("grep");
            cmd.arg("-rn");
            cmd.arg("--");
            cmd.arg(&args.query);
            cmd.arg(".");
            cmd.current_dir(workspace.as_path());
            cmd.output().context("failed to run grep")?
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut matches = Vec::new();
        for line in stdout.lines() {
            if matches.len() >= max {
                break;
            }
            let rest = line.strip_prefix("./").unwrap_or(line);
            let mut parts = rest.splitn(3, ':');
            if let (Some(file), Some(line_str), Some(snippet)) =
                (parts.next(), parts.next(), parts.next())
            {
                if snippet == "Binary file matches" {
                    continue;
                }
                let line_no: u64 = line_str.parse().unwrap_or(0);
                matches.push(json!({
                    "path": file,
                    "line": line_no,
                    "snippet": snippet,
                }));
            }
        }
        Ok(text_result(json!({"matches": matches})))
    }

    async fn get_time(&self, _args: GetTimeArgs) -> anyhow::Result<CallToolResult> {
        use time::format_description::well_known::Rfc3339;
        let now = time::OffsetDateTime::now_utc();
        let iso8601 = now
            .format(&Rfc3339)
            .unwrap_or_else(|_| now.to_string());
        Ok(text_result(json!({
            "iso8601": iso8601,
            "unix_seconds": now.unix_timestamp(),
        })))
    }

    async fn git_status(&self, _args: GitStatusArgs) -> anyhow::Result<CallToolResult> {
        let result = self.git_raw("status --porcelain", None).await?;
        let stdout = result.0;
        let stderr = result.1;
        let exit_code = result.2;
        if exit_code != 0 && !stderr.is_empty() {
            anyhow::bail!("git status failed (exit {exit_code}): {stderr}");
        }
        let entries: Vec<Value> = stdout
            .lines()
            .filter_map(|line| {
                if line.len() < 3 {
                    return None;
                }
                let (status, path) = line.split_at(2);
                let path = path.trim();
                if path.is_empty() {
                    return None;
                }
                Some(json!({
                    "status": status.trim(),
                    "path": path,
                }))
            })
            .collect();
        Ok(text_result(json!({
            "entries": entries,
            "stderr": stderr,
        })))
    }

    async fn git_diff(&self, args: GitDiffArgs) -> anyhow::Result<CallToolResult> {
        let mut command = "diff".to_string();
        if args.staged {
            command.push_str(" --staged");
        }
        if let Some(paths) = &args.paths {
            command.push_str(" --");
            for p in paths {
                command.push(' ');
                command.push_str(&shell_escape(p));
            }
        }
        let result = self.git_raw(&command, None).await?;
        let exit_code = result.2;
        if exit_code != 0 && !result.1.is_empty() {
            anyhow::bail!("git diff failed (exit {exit_code}): {}", result.1);
        }
        Ok(text_result(json!({
            "diff": result.0,
            "stderr": result.1,
        })))
    }

    async fn list_directory(&self, args: ListDirectoryArgs) -> anyhow::Result<CallToolResult> {
        let workspace = self.workspace_or_error().await?;
        let policy = SandboxPolicy::ReadOnly {
            network_access: false,
        };
        let sandbox =
            FileSystemSandboxContext::from_legacy_sandbox_policy(policy, PathUri::from_abs_path(&workspace))?;
        let sandbox = if self.sandbox_available() {
            Some(&sandbox)
        } else {
            None
        };
        let fs = self.environment.get_filesystem();
        let path = match &args.path {
            Some(p) => resolve_workspace_path(fs.as_ref(), &workspace, p, sandbox).await?,
            None => PathUri::from_abs_path(&workspace),
        };
        let entries = fs.read_directory(&path, sandbox).await?;
        let listing: Vec<Value> = entries
            .into_iter()
            .map(|entry| {
                json!({
                    "name": entry.file_name,
                    "is_directory": entry.is_directory,
                    "is_file": entry.is_file,
                })
            })
            .collect();
        Ok(text_result(json!({
            "path": path.to_path_buf(),
            "entries": listing,
        })))
    }

    async fn todo(&self, args: TodoArgs) -> anyhow::Result<CallToolResult> {
        let active_run = self.lifecycle.active_run()?;
        let mut todo = if let Some(run) = &active_run {
            run.checklist.clone()
        } else {
            self.lifecycle.snapshot()?.legacy_todo
        };
        match args.action {
            TodoAction::Replace => {
                let mut items = Vec::new();
                for (i, input) in args.items.into_iter().enumerate() {
                    items.push(lifecycle::ChecklistItem {
                        id: input.id.unwrap_or_else(|| format!("t{}", i + 1)),
                        description: input.description.unwrap_or_default(),
                        status: match input.status.as_deref() {
                            Some("checked") => lifecycle::ChecklistStatus::Checked,
                            Some("dismissed") => lifecycle::ChecklistStatus::Dismissed,
                            _ => lifecycle::ChecklistStatus::Pending,
                        },
                    });
                }
                todo = items;
            }
            TodoAction::Update => {
                for input in args.items {
                    let id = input.id.unwrap_or_default();
                    if let Some(existing) = todo.iter_mut().find(|item| item.id == id) {
                        if let Some(desc) = input.description {
                            existing.description = desc;
                        }
                        existing.status = match input.status.as_deref() {
                            Some("checked") => lifecycle::ChecklistStatus::Checked,
                            Some("dismissed") => lifecycle::ChecklistStatus::Dismissed,
                            Some("pending") => lifecycle::ChecklistStatus::Pending,
                            _ => continue,
                        };
                    }
                }
            }
        }
        if let Some(run) = active_run {
            self.lifecycle.update_run(lifecycle::RunUpdate {
                run_id: Some(run.id),
                checklist: Some(todo.clone()),
                ..lifecycle::RunUpdate::default()
            })?;
        } else {
            self.lifecycle.set_legacy_todo(todo.clone())?;
        }
        let response: Vec<Value> = todo
            .iter()
            .map(|item| {
                let status_str = match item.status {
                    lifecycle::ChecklistStatus::Pending => "pending",
                    lifecycle::ChecklistStatus::Checked => "checked",
                    lifecycle::ChecklistStatus::Dismissed => "dismissed",
                };
                json!({
                    "id": item.id,
                    "description": item.description,
                    "status": status_str,
                })
            })
            .collect();
        let pending_count = todo
            .iter()
            .filter(|i| i.status == lifecycle::ChecklistStatus::Pending)
            .count();
        let checked_count = todo
            .iter()
            .filter(|i| i.status == lifecycle::ChecklistStatus::Checked)
            .count();
        let dismissed_count = todo
            .iter()
            .filter(|i| i.status == lifecycle::ChecklistStatus::Dismissed)
            .count();
        Ok(text_result(json!({
            "items": response,
            "summary": {
                "total": todo.len(),
                "pending": pending_count,
                "checked": checked_count,
                "dismissed": dismissed_count,
            },
            "all_done": pending_count == 0,
        })))
    }

    async fn setup_workspace(&self, args: SetupWorkspaceArgs) -> anyhow::Result<CallToolResult> {
        let workspace_result = do_setup_workspace(
            &self.workspace_base,
            self.lifecycle.client_id(),
            &args.source,
            args.name.as_deref(),
            args.ref_name.as_deref(),
            args.timeout_ms,
        )
        .await?;
        let name = args
            .name
            .as_deref()
            .map(sanitize_workspace_name)
            .transpose()?
            .unwrap_or_else(|| {
                derive_workspace_name(&args.source).unwrap_or_else(|_| "workspace".to_string())
            });
        let source = if args.source == "sandbox" {
            lifecycle::ProjectSource::Scratch
        } else {
            lifecycle::redacted_git_source(&args.source)?
        };
        let project = self.lifecycle.upsert_project(lifecycle::ProjectUpsert {
            kind: if args.source == "sandbox" {
                lifecycle::ProjectKind::Scratch
            } else {
                lifecycle::ProjectKind::Repo
            },
            name,
            workspace_root: workspace_result.workspace_root.clone(),
            source,
            select: true,
        })?;
        let response_source = match &project.project.source {
            lifecycle::ProjectSource::Git { url, .. } => url.clone(),
            _ => workspace_result.source,
        };
        Ok(text_result(json!({
            "workspace_root": workspace_result.workspace_root,
            "source": response_source,
            "action": workspace_result.action,
            "project_id": project.project.id,
        })))
    }

    async fn project_create(&self, args: ProjectCreateArgs) -> anyhow::Result<CallToolResult> {
        match args.kind.clone() {
            lifecycle::ProjectKind::Scratch => self.project_create_scratch(args).await,
            lifecycle::ProjectKind::Repo => self.project_create_repo(args).await,
            lifecycle::ProjectKind::Workspace => self.project_create_workspace(args).await,
        }
    }

    async fn project_create_scratch(
        &self,
        args: ProjectCreateArgs,
    ) -> anyhow::Result<CallToolResult> {
        let name = args.name.unwrap_or_else(|| "sandbox".to_string());
        let workspace_result = do_setup_workspace(
            &self.workspace_base,
            self.lifecycle.client_id(),
            "sandbox",
            Some(&name),
            args.timeout_ms,
        )
        .await?;
        let project = self.lifecycle.upsert_project(lifecycle::ProjectUpsert {
            kind: lifecycle::ProjectKind::Scratch,
            name: sanitize_workspace_name(&name)?,
            workspace_root: workspace_result.workspace_root,
            source: lifecycle::ProjectSource::Scratch,
            select: args.select,
        })?;
        Ok(text_result(json!({
            "project": project.project,
            "action": project.action,
            "selected": args.select,
        })))
    }

    async fn project_create_repo(&self, args: ProjectCreateArgs) -> anyhow::Result<CallToolResult> {
        let source = args
            .source
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("source is required for repo projects"))?;
        let name = args
            .name
            .as_deref()
            .map(sanitize_workspace_name)
            .transpose()?
            .unwrap_or_else(|| {
                derive_workspace_name(source).unwrap_or_else(|_| "repo".to_string())
            });
        let workspace_result = do_setup_workspace(
            &self.workspace_base,
            self.lifecycle.client_id(),
            source,
            Some(&name),
            args.timeout_ms,
        )
        .await?;
        let project = self.lifecycle.upsert_project(lifecycle::ProjectUpsert {
            kind: lifecycle::ProjectKind::Repo,
            name,
            workspace_root: workspace_result.workspace_root,
            source: lifecycle::redacted_git_source(source)?,
            select: args.select,
        })?;
        Ok(text_result(json!({
            "project": project.project,
            "action": workspace_result.action,
            "selected": args.select,
        })))
    }

    async fn project_create_workspace(
        &self,
        args: ProjectCreateArgs,
    ) -> anyhow::Result<CallToolResult> {
        let requested = args
            .path
            .or_else(|| args.source.map(PathBuf::from))
            .ok_or_else(|| anyhow::anyhow!("path or source is required for workspace projects"))?;
        let path = if requested.is_absolute() {
            requested
        } else {
            self.workspace_base.as_path().join(requested)
        };
        let canonical_base =
            std::fs::canonicalize(self.workspace_base.as_path()).with_context(|| {
                format!(
                    "failed to resolve workspace base {}",
                    self.workspace_base.as_path().display()
                )
            })?;
        let canonical = std::fs::canonicalize(&path)
            .with_context(|| format!("failed to resolve workspace {}", path.display()))?;
        if !canonical.starts_with(&canonical_base) {
            anyhow::bail!("registered workspace must be under the workspace base");
        }
        if !canonical.is_dir() {
            anyhow::bail!(
                "registered workspace is not a directory: {}",
                canonical.display()
            );
        }
        let name = args
            .name
            .as_deref()
            .map(sanitize_workspace_name)
            .transpose()?
            .or_else(|| {
                canonical
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| sanitize_workspace_name(name).ok())
            })
            .unwrap_or_else(|| "workspace".to_string());
        let project = self.lifecycle.upsert_project(lifecycle::ProjectUpsert {
            kind: lifecycle::ProjectKind::Workspace,
            name,
            workspace_root: canonical.clone(),
            source: lifecycle::workspace_source(canonical),
            select: args.select,
        })?;
        Ok(text_result(json!({
            "project": project.project,
            "action": project.action,
            "selected": args.select,
        })))
    }

    async fn project_select(&self, args: ProjectSelectArgs) -> anyhow::Result<CallToolResult> {
        let project = self.lifecycle.select_project(&args.project_id)?;
        Ok(text_result(json!({
            "project": project,
            "selected": true,
        })))
    }

    async fn project_list(&self, _args: ProjectListArgs) -> anyhow::Result<CallToolResult> {
        let snapshot = self.lifecycle.snapshot()?;
        let active_project_id = snapshot.active_project_id.clone();
        Ok(text_result(json!({
            "projects": snapshot.projects.into_values().collect::<Vec<_>>(),
            "active_project_id": active_project_id,
        })))
    }

    async fn project_get(&self, args: ProjectGetArgs) -> anyhow::Result<CallToolResult> {
        let snapshot = self.lifecycle.snapshot()?;
        let active_project_id = snapshot.active_project_id.clone();
        let project_id = args
            .project_id
            .or(active_project_id.clone())
            .ok_or_else(|| anyhow::anyhow!("project_id is required when no project is selected"))?;
        let project = snapshot
            .projects
            .get(&project_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown project: {project_id}"))?;
        Ok(text_result(json!({
            "project": project,
            "selected": active_project_id.as_deref() == Some(project_id.as_str()),
        })))
    }

    async fn run_start(&self, args: RunStartArgs) -> anyhow::Result<CallToolResult> {
        let run = self.lifecycle.start_run(lifecycle::RunStart {
            project_id: args.project_id,
            objective: args.objective,
            acceptance_criteria: args.acceptance_criteria,
            autonomy: args.autonomy,
            select: args.select,
        })?;
        Ok(run_result(run.run))
    }

    async fn run_list(&self, args: RunListArgs) -> anyhow::Result<CallToolResult> {
        let snapshot = self.lifecycle.snapshot()?;
        let active_run_id = snapshot.active_run_id.clone();
        let runs = snapshot
            .runs
            .into_values()
            .filter(|run| {
                args.project_id
                    .as_deref()
                    .is_none_or(|project_id| run.project_id == project_id)
            })
            .filter(|run| {
                args.status
                    .as_ref()
                    .is_none_or(|status| run.status == *status)
            })
            .collect::<Vec<_>>();
        Ok(text_result(json!({
            "runs": runs,
            "active_run_id": active_run_id,
        })))
    }

    async fn run_get(&self, args: RunGetArgs) -> anyhow::Result<CallToolResult> {
        let run = self.resolve_run(args.run_id.as_deref())?;
        Ok(run_result(run))
    }

    async fn run_update(&self, args: RunUpdateArgs) -> anyhow::Result<CallToolResult> {
        let update = lifecycle::RunUpdate {
            run_id: args.run_id,
            phase: args.phase,
            status: args.status,
            acceptance_criteria: args.acceptance_criteria,
            plan: args.plan,
            checklist: args.checklist,
            checkpoint: args.checkpoint,
            work_remaining: args.work_remaining,
            next_action: args.next_action,
            step_delta: args.step_delta,
        };
        let run = self.lifecycle.update_run(update)?;
        Ok(run_result(run.run))
    }

    async fn run_resume(&self, args: RunResumeArgs) -> anyhow::Result<CallToolResult> {
        let run_id = self.resolve_run_id(args.run_id.as_deref())?;
        let run = self.lifecycle.resume_run(&run_id)?;
        Ok(run_result(run.run))
    }

    async fn run_cancel(&self, args: RunCancelArgs) -> anyhow::Result<CallToolResult> {
        let run_id = self.resolve_run_id(args.run_id.as_deref())?;
        let run = self.lifecycle.cancel_run(&run_id)?;
        Ok(run_result(run.run))
    }

    async fn run_followup_lease(&self, args: FollowupLeaseArgs) -> anyhow::Result<CallToolResult> {
        let lease = self
            .lifecycle
            .acquire_followup_lease(lifecycle::FollowupLeaseRequest {
                run_id: args.run_id.clone(),
                requested_nonce: args.requested_nonce,
                now_ms: None,
                ttl_ms: args.ttl_ms,
                delay_ms: args.delay_ms,
            })?;
        let run = self.lifecycle.get_run(&args.run_id)?;
        let mut value = serde_json::to_value(lease)?;
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "run_metadata".to_string(),
                lifecycle::run_metadata(&run, lifecycle::current_time_ms()),
            );
        }
        Ok(text_result(value))
    }

    fn resolve_run_id(&self, explicit: Option<&str>) -> anyhow::Result<String> {
        if let Some(run_id) = explicit {
            return Ok(run_id.to_string());
        }
        self.lifecycle
            .snapshot()?
            .active_run_id
            .ok_or_else(|| anyhow::anyhow!("run_id is required when no run is selected"))
    }

    fn resolve_run(&self, explicit: Option<&str>) -> anyhow::Result<lifecycle::Run> {
        let run_id = self.resolve_run_id(explicit)?;
        self.lifecycle.get_run(&run_id)
    }

    fn attach_active_run_metadata(&self, result: &mut CallToolResult) -> anyhow::Result<()> {
        let Some(run) = self.lifecycle.active_run()? else {
            return Ok(());
        };
        attach_run_metadata(
            result,
            lifecycle::run_metadata(&run, lifecycle::current_time_ms()),
        )
    }

    fn ensure_active_run_can(
        &self,
        action: &str,
        allowed: impl FnOnce(&lifecycle::Run) -> bool,
    ) -> anyhow::Result<()> {
        let Some(run) = self.lifecycle.active_run()? else {
            return Ok(());
        };
        if run.status != lifecycle::RunStatus::Active {
            anyhow::bail!(
                "selected run {} is {}; cannot {} until the run is active",
                run.id,
                run.status.as_str(),
                action
            );
        }
        if !allowed(&run) {
            anyhow::bail!("selected run {} does not allow {}", run.id, action);
        }
        Ok(())
    }

    async fn git_tool(&self, args: GitToolArgs) -> anyhow::Result<CallToolResult> {
        if git_command_requires_commit_permission(&args.command) {
            self.ensure_active_run_can("run local git writes", |run| {
                run.autonomy.allow_git_commits
            })?;
        }
        let workspace = self.workspace_or_error().await?;
        let result = self
            .run_sandboxed_git(
                &workspace,
                &args.command,
                args.timeout_ms,
                GitSandboxMode::Unsandboxed,
            )
            .await?;
        // Writable git commands cannot run inside the workspace-write sandbox
        // because the upstream sandbox protects .git/ metadata under a writable
        // workspace root. They run unsandboxed with outbound subcommands blocked
        // by the parser and no declared network access.
        Ok(text_result(json!({
            "stdout": result.stdout,
            "stderr": result.stderr,
            "exit_code": result.exit_code,
        })))
    }

    async fn git_raw(
        &self,
        command: &str,
        timeout_ms: Option<u64>,
    ) -> anyhow::Result<(String, String, i32)> {
        let workspace = self.workspace_or_error().await?;
        let result = self
            .run_sandboxed_git(&workspace, command, timeout_ms, GitSandboxMode::ReadOnly)
            .await?;
        Ok((result.stdout, result.stderr, result.exit_code))
    }

    async fn git_commit(&self, args: GitCommitArgs) -> anyhow::Result<CallToolResult> {
        self.ensure_active_run_can("create git commits", |run| run.autonomy.allow_git_commits)?;
        let workspace = self.workspace_or_error().await?;
        // Ensure git has an identity in this repo so commits succeed without
        // relying on global user configuration.
        let _ = self
            .run_sandboxed_git(
                &workspace,
                "config user.email chatcodex@example.invalid",
                Some(10_000),
                GitSandboxMode::Unsandboxed,
            )
            .await;
        let _ = self
            .run_sandboxed_git(
                &workspace,
                "config user.name ChatCodex",
                Some(10_000),
                GitSandboxMode::Unsandboxed,
            )
            .await;
        let mut command = format!("commit -m {}", shell_escape(&args.message));
        if args.allow_empty {
            command.push_str(" --allow-empty");
        }
        let result = self
            .run_sandboxed_git(
                &workspace,
                &command,
                args.timeout_ms,
                GitSandboxMode::Unsandboxed,
            )
            .await?;
        Ok(text_result(json!({
            "stdout": result.stdout,
            "stderr": result.stderr,
            "exit_code": result.exit_code,
        })))
    }

    async fn git_branch(&self, args: GitBranchArgs) -> anyhow::Result<CallToolResult> {
        self.ensure_active_run_can("run local git writes", |run| run.autonomy.allow_git_commits)?;
        let workspace = self.workspace_or_error().await?;
        let mut command = format!("branch {}", shell_escape(&args.name));
        if args.force {
            command.push_str(" --force");
        }
        if let Some(start_point) = args.start_point {
            command.push(' ');
            command.push_str(&shell_escape(&start_point));
        }
        let result = self
            .run_sandboxed_git(
                &workspace,
                &command,
                args.timeout_ms,
                GitSandboxMode::Unsandboxed,
            )
            .await?;
        Ok(text_result(json!({
            "stdout": result.stdout,
            "stderr": result.stderr,
            "exit_code": result.exit_code,
        })))
    }

    async fn git_checkout(&self, args: GitCheckoutArgs) -> anyhow::Result<CallToolResult> {
        self.ensure_active_run_can("run local git writes", |run| run.autonomy.allow_git_commits)?;
        let workspace = self.workspace_or_error().await?;
        let mut command = String::from("checkout");
        if args.create_branch {
            command.push_str(" -b");
        }
        command.push(' ');
        command.push_str(&shell_escape(&args.target));
        let result = self
            .run_sandboxed_git(
                &workspace,
                &command,
                args.timeout_ms,
                GitSandboxMode::Unsandboxed,
            )
            .await?;
        Ok(text_result(json!({
            "stdout": result.stdout,
            "stderr": result.stderr,
            "exit_code": result.exit_code,
        })))
    }

    /// Push the current (or named) branch to the origin remote and,
    /// optionally, open a pull request for it.
    ///
    /// This is the only sanctioned outbound network path for git: the `git`
    /// tool deliberately blocks push/fetch/ls-remote. Authentication is
    /// provided exclusively by the server-side `CHATCODEX_GITHUB_TOKEN`
    /// environment variable — never by credentials embedded in URLs or by
    /// interactive prompts. The token is passed to git via a credential
    /// helper that reads the environment variable, so it is never written to
    /// `.git/config` or any other file the model can read.
    async fn git_push(&self, args: GitPushArgs) -> anyhow::Result<CallToolResult> {
        let workspace = self.workspace_or_error().await?;
        let token = std::env::var("CHATCODEX_GITHUB_TOKEN").ok();
        let Some(token) = token.filter(|t| !t.is_empty()) else {
            anyhow::bail!(
                "git_push is disabled: CHATCODEX_GITHUB_TOKEN is not configured on the server"
            );
        };

        // Ensure git has an identity in this repo so pushes from branches
        // without commits still behave deterministically.
        for config in [
            "config user.email chatcodex@example.invalid",
            "config user.name ChatCodex",
        ] {
            let _ = self
                .run_sandboxed_git(&workspace, config, Some(10_000), GitSandboxMode::Unsandboxed)
                .await;
        }

        // Resolve the branch to push: explicit arg, or the current branch.
        // A detached HEAD cannot be pushed under a name, so require one.
        let branch = if let Some(branch) = args.branch {
            let command = format!("checkout -B {}", shell_escape(&branch));
            let result = self
                .run_sandboxed_git(
                    &workspace,
                    &command,
                    args.timeout_ms,
                    GitSandboxMode::Unsandboxed,
                )
                .await?;
            if result.exit_code != 0 {
                anyhow::bail!(
                    "failed to switch to branch {}: {}",
                    branch,
                    result.stderr.trim()
                );
            }
            branch
        } else {
            let result = self
                .run_sandboxed_git(
                    &workspace,
                    "symbolic-ref --short HEAD",
                    Some(10_000),
                    GitSandboxMode::Unsandboxed,
                )
                .await?;
            if result.exit_code != 0 {
                anyhow::bail!(
                    "cannot push: HEAD is detached. Create a branch first (git_checkout with create_branch, or pass a branch). stderr: {}",
                    result.stderr.trim()
                );
            }
            result.stdout.trim().to_string()
        };

        let mut env = std::collections::HashMap::new();
        env.insert("CHATCODEX_GITHUB_TOKEN".to_string(), token);
        env.insert("GIT_TERMINAL_PROMPT".to_string(), "0".to_string());
        env.insert("GIT_ASKPASS".to_string(), "/bin/true".to_string());
        env.insert(
            "GCM_INTERACTIVE".to_string(),
            "Never".to_string(),
        );

        let push_argv = vec![
            "-c".to_string(),
            format!("credential.helper={}", github_credential_helper()),
            "push".to_string(),
            "-u".to_string(),
            "origin".to_string(),
            format!("HEAD:{branch}"),
        ];
        let push = run_git_command_with_env(
            workspace.as_path(),
            push_argv,
            &env,
            args.timeout_ms.or(Some(120_000)),
        )
        .await?;
        let pushed = push.exit_code == 0;

        let mut result = json!({
            "branch": branch,
            "pushed": pushed,
            "stdout": push.stdout,
            "stderr": push.stderr,
            "exit_code": push.exit_code,
        });

        if pushed && args.create_pr {
            let pr = self
                .create_pull_request(&workspace, &branch, args.base.as_deref(), &env)
                .await;
            match pr {
                Ok(url) => {
                    result["pr_url"] = json!(url);
                }
                Err(err) => {
                    result["pr_error"] = json!(err.to_string());
                }
            }
        }

        Ok(text_result(result))
    }

    /// Open a pull request for `branch` against `base` using the gh CLI.
    /// Returns the PR URL on success.
    async fn create_pull_request(
        &self,
        workspace: &AbsolutePathBuf,
        branch: &str,
        base: Option<&str>,
        env: &std::collections::HashMap<String, String>,
    ) -> anyhow::Result<String> {
        let base = match base {
            Some(base) => base.to_string(),
            None => {
                let result = run_git_command_with_env(
                    workspace.as_path(),
                    vec![
                        "symbolic-ref".to_string(),
                        "refs/remotes/origin/HEAD".to_string(),
                    ],
                    env,
                    Some(10_000),
                )
                .await?;
                if result.exit_code == 0 {
                    result
                        .stdout
                        .trim()
                        .trim_start_matches("refs/remotes/origin/")
                        .to_string()
                } else {
                    "main".to_string()
                }
            }
        };
        // After a fresh push the origin/HEAD ref may not exist yet, so fall
        // back to the symbolic default branch if resolution produced garbage.
        let base = if base.is_empty() { "main".to_string() } else { base };

        let mut gh_env = env.clone();
        gh_env.insert(
            "GH_TOKEN".to_string(),
            env.get("CHATCODEX_GITHUB_TOKEN").cloned().unwrap_or_default(),
        );

        let output = tokio::process::Command::new("gh")
            .args(["pr", "create", "--fill", "--base", &base, "--head", branch])
            .current_dir(workspace.as_path())
            .env_clear()
            .envs(&gh_env)
            .output()
            .await
            .context("failed to spawn gh; is the GitHub CLI installed in the container?")?;
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !output.status.success() {
            anyhow::bail!(
                "gh pr create failed (base={base}): {stderr}{}",
                if stdout.is_empty() { String::new() } else { format!(" stdout: {stdout}") }
            );
        }
        // gh prints the PR URL on stdout; fall back to a search if it didn't.
        if let Some(url) = stdout.lines().find(|l| l.contains("https://")) {
            return Ok(url.to_string());
        }
        anyhow::bail!("gh pr create succeeded but no URL was found in output: {stdout}");
    }

    /// Run a git command through the exec-server sandbox.
    ///
    /// ReadOnly mode applies the command sandbox. Unsandboxed mode is reserved
    /// for local git metadata writes after outbound subcommands and active-run
    /// autonomy gates have been checked.
    async fn run_sandboxed_git(
        &self,
        workspace: &AbsolutePathBuf,
        command: &str,
        timeout_ms: Option<u64>,
        mode: GitSandboxMode,
    ) -> anyhow::Result<GitCommandOutput> {
        if is_outbound_git_command(command) {
            anyhow::bail!("outbound git command blocked by policy: {command}");
        }
        let argv = shlex::split(command)
            .unwrap_or_else(|| command.split_whitespace().map(String::from).collect());
        self.run_sandboxed_git_with_args(workspace, argv, timeout_ms, mode)
            .await
    }

    async fn run_sandboxed_git_with_args(
        &self,
        workspace: &AbsolutePathBuf,
        argv: Vec<String>,
        timeout_ms: Option<u64>,
        mode: GitSandboxMode,
    ) -> anyhow::Result<GitCommandOutput> {
        let (permissions, sandbox) = match mode {
            GitSandboxMode::ReadOnly => {
                let policy = SandboxPolicy::ReadOnly {
                    network_access: false,
                };
                (
                    PermissionProfile::from_legacy_sandbox_policy_for_cwd(
                        &policy,
                        workspace.as_path(),
                    ),
                    self.resolve_sandbox_type(),
                )
            }
            GitSandboxMode::Unsandboxed => {
                // The upstream workspace-write sandbox always protects .git/
                // metadata under a writable workspace root, so git metadata
                // writes cannot be expressed there. Run git unsandboxed with
                // no declared network access; outbound subcommands are still
                // rejected by the parser.
                let policy = SandboxPolicy::ExternalSandbox {
                    network_access: codex_protocol::protocol::NetworkAccess::Restricted,
                };
                (
                    PermissionProfile::from_legacy_sandbox_policy(&policy),
                    SandboxType::None,
                )
            }
        };
        let mut cmd_env = HashMap::new();
        if sandbox != SandboxType::None {
            if let Some(path) = std::env::var_os("PATH") {
                cmd_env.insert("PATH".to_string(), path.to_string_lossy().into_owned());
            }
            if let Some(home) = std::env::var_os("HOME") {
                cmd_env.insert("HOME".to_string(), home.to_string_lossy().into_owned());
            }
        }
        let (file_system_sandbox_policy, _) = permissions.to_runtime_permissions();
        let workspace_uri = PathUri::from_abs_path(&workspace);
        let transformed = SandboxManager::new().transform(SandboxTransformRequest {
            command: SandboxCommand {
                program: "git".into(),
                args: argv,
                cwd: workspace_uri.clone(),
                env: cmd_env,
                managed_network: None,
                additional_permissions: None,
            },
            permissions: &permissions,
            sandbox,
            enforce_managed_network: false,
            environment_id: None,
            network: None,
            sandbox_policy_cwd: &workspace_uri,
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
                cwd: transformed.cwd,
                env_policy: None,
                env: transformed.env,
                tty: false,
                pipe_stdin: false,
                arg0: transformed.arg0,
                sandbox: None,
                enforce_managed_network: false,
                managed_network: None,
                network_proxy: None,
            })
            .await?;
        let yield_ms = timeout_ms.unwrap_or(30_000).min(MAX_YIELD_MS);
        let (stdout, stderr, exit_code) =
            read_process_streams(Arc::clone(&started.process), yield_ms).await?;
        Ok(GitCommandOutput {
            stdout,
            stderr,
            exit_code,
        })
    }
}

async fn read_process_streams(
    process: Arc<dyn ExecProcess>,
    yield_time_ms: u64,
) -> anyhow::Result<(String, String, i32)> {
    use codex_exec_server::ExecOutputStream;
    let deadline =
        tokio::time::Instant::now() + Duration::from_millis(yield_time_ms.min(MAX_YIELD_MS));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut next_seq: Option<u64> = None;
    let mut exit_code: Option<i32> = None;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let response = process
            .read(
                next_seq,
                Some(MAX_OUTPUT_BYTES),
                Some(remaining.as_millis().max(1) as u64),
            )
            .await?;
        for chunk in &response.chunks {
            match chunk.stream {
                ExecOutputStream::Stderr => stderr.extend(chunk.chunk.clone().into_inner()),
                _ => stdout.extend(chunk.chunk.clone().into_inner()),
            }
        }
        if response.exit_code.is_some() {
            exit_code = response.exit_code;
        }
        next_seq = Some(response.next_seq);
        if (response.exited && response.chunks.is_empty())
            || tokio::time::Instant::now() >= deadline
        {
            break;
        }
    }
    Ok((
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
        exit_code.unwrap_or(-1),
    ))
}

#[allow(unused_assignments)]
async fn read_process(
    process: Arc<dyn ExecProcess>,
    after_seq: Option<u64>,
    yield_time_ms: u64,
) -> anyhow::Result<ProcessRead> {
    let deadline =
        tokio::time::Instant::now() + Duration::from_millis(yield_time_ms.min(MAX_YIELD_MS));
    let mut output = Vec::new();
    let mut next_seq = after_seq;
    let mut exit_code: Option<i32> = None;
    let mut exited = false;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let response = process
            .read(
                next_seq,
                Some(MAX_OUTPUT_BYTES),
                Some(remaining.as_millis().max(1) as u64),
            )
            .await?;
        for chunk in &response.chunks {
            output.extend(chunk.chunk.clone().into_inner());
        }
        if response.exit_code.is_some() {
            exit_code = response.exit_code;
        }
        exited = response.exited;
        next_seq = Some(response.next_seq);
        if (response.closed && response.chunks.is_empty())
            || tokio::time::Instant::now() >= deadline
        {
            break;
        }
    }
    Ok(ProcessRead {
        output: String::from_utf8_lossy(&output).into_owned(),
        exited,
        exit_code,
        next_seq: next_seq.unwrap_or(0),
    })
}

fn platform_sandbox() -> anyhow::Result<SandboxType> {
    codex_sandboxing::get_platform_sandbox(false)
        .ok_or_else(|| anyhow::anyhow!("this platform has no supported read-only sandbox"))
}

#[cfg(target_os = "linux")]
fn user_namespaces_available() -> bool {
    match std::process::Command::new("unshare")
        .args(["-U", "true"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(status) => status.success(),
        Err(_) => true,
    }
}

impl NativeHarness {
    fn sandbox_available(&self) -> bool {
        self.resolve_sandbox_type() != SandboxType::None
    }

    fn resolve_sandbox_type(&self) -> SandboxType {
        if !(self.environment.is_remote()
            || self
                .environment
                .local_runtime_paths()
                .is_some_and(|p| p.codex_linux_sandbox_exe.is_some()))
        {
            return SandboxType::None;
        }
        let Ok(sandbox) = platform_sandbox() else {
            return SandboxType::None;
        };
        #[cfg(target_os = "linux")]
        if sandbox == SandboxType::LinuxSeccomp {
            if let Some(bwrap_path) = codex_sandboxing::find_system_bwrap_in_path() {
                let timeout = std::time::Duration::from_millis(500);
                if !codex_sandboxing::system_bwrap_has_user_namespace_access(&bwrap_path, timeout) {
                    tracing::warn!(
                        "bubblewrap user namespace access unavailable; disabling sandbox"
                    );
                    return SandboxType::None;
                }
            } else if !user_namespaces_available() {
                tracing::warn!(
                    "no bwrap on PATH and user namespaces unavailable; disabling sandbox"
                );
                return SandboxType::None;
            }
        }
        sandbox
    }
}

async fn resolve_workspace_path(
    fs: &dyn codex_exec_server::ExecutorFileSystem,
    workspace: &AbsolutePathBuf,
    requested: &Path,
    sandbox: Option<&FileSystemSandboxContext>,
) -> anyhow::Result<PathUri> {
    let path = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        workspace.as_path().join(requested)
    };
    let path = AbsolutePathBuf::from_absolute_path(path)?;
    let workspace_uri = PathUri::from_abs_path(workspace);
    let path_uri = PathUri::from_abs_path(&path);
    let canonical_workspace = fs.canonicalize(&workspace_uri, sandbox).await?;
    let canonical_path = fs.canonicalize(&path_uri, sandbox).await?;
    if !canonical_path.starts_with(&canonical_workspace) {
        anyhow::bail!("path is outside the configured workspace");
    }
    Ok(canonical_path)
}

pub(crate) fn text_result(value: Value) -> CallToolResult {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    let mut result = CallToolResult::success(vec![Content::text(text)]);
    result.structured_content = Some(value);
    result
}

fn error_result(message: String) -> CallToolResult {
    CallToolResult::error(vec![Content::text(message)])
}

fn run_result(run: lifecycle::Run) -> CallToolResult {
    let metadata = lifecycle::run_metadata(&run, lifecycle::current_time_ms());
    text_result(json!({
        "run": run,
        "run_metadata": metadata,
    }))
}

fn attach_run_metadata(result: &mut CallToolResult, metadata: Value) -> anyhow::Result<()> {
    let structured = result
        .structured_content
        .take()
        .unwrap_or_else(|| json!({}));
    let mut structured = match structured {
        Value::Object(object) => object,
        other => {
            let mut object = serde_json::Map::new();
            object.insert("value".to_string(), other);
            object
        }
    };
    structured.insert("run_metadata".to_string(), metadata.clone());
    let structured = Value::Object(structured);
    if result
        .content
        .first()
        .and_then(|content| content.as_text())
        .is_some()
    {
        let text =
            serde_json::to_string_pretty(&structured).unwrap_or_else(|_| structured.to_string());
        result.content = vec![Content::text(text)];
    }
    result.structured_content = Some(structured);

    let mut meta = result.meta.take().map(|meta| meta.0).unwrap_or_default();
    meta.insert("chatcodex/run".to_string(), metadata);
    result.meta = Some(Meta(meta));
    Ok(())
}

fn tool_gets_active_run_metadata(name: &str) -> bool {
    matches!(
        name,
        "exec_command"
            | "write_stdin"
            | "update_plan"
            | "apply_patch"
            | "view_image"
            | "read_file"
            | "search_code"
            | "setup_workspace"
            | "git"
            | "git_status"
            | "git_diff"
            | "git_commit"
            | "git_branch"
            | "git_checkout"
            | "list_directory"
            | "todo"
    )
}

#[derive(Debug, Clone, Copy)]
enum GitSandboxMode {
    ReadOnly,
    Unsandboxed,
}

#[derive(Debug)]
struct SetupWorkspaceResult {
    workspace_root: PathBuf,
    source: String,
    action: String,
}

fn is_allowed_git_scheme(scheme: &str) -> bool {
    matches!(scheme, "https" | "http" | "ssh" | "git+https" | "git+ssh")
}

fn sanitize_workspace_name(name: &str) -> anyhow::Result<String> {
    if name.is_empty() {
        anyhow::bail!("workspace name must not be empty");
    }
    if name.contains("..") || name.contains('/') || name.contains('\\') || name.starts_with('/') {
        anyhow::bail!("workspace name must not contain path separators or '..'");
    }
    let sanitized = name
        .replace(".git", "")
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    let sanitized = sanitized
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if sanitized.is_empty() {
        anyhow::bail!("workspace name is empty after sanitization");
    }
    Ok(sanitized)
}

fn derive_workspace_name(source: &str) -> anyhow::Result<String> {
    if source == "sandbox" {
        return Ok("sandbox".to_string());
    }
    let url_str = source.strip_prefix("git+").unwrap_or(source);
    let url = url_str.parse::<url::Url>().context("invalid git URL")?;
    let path = url.path();
    let basename = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("repo")
        .to_string();
    sanitize_workspace_name(&basename)
}

async fn do_setup_workspace(
    workspace_base: &AbsolutePathBuf,
    client_id: &str,
    source: &str,
    name: Option<&str>,
    ref_name: Option<&str>,
    timeout_ms: Option<u64>,
) -> anyhow::Result<SetupWorkspaceResult> {
    let name = match name {
        Some(n) => sanitize_workspace_name(n)?,
        None => derive_workspace_name(source)?,
    };
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(60_000).clamp(1_000, 120_000));

    if source == "sandbox" {
        let target = workspace_base
            .as_path()
            .join("clients")
            .join(client_id)
            .join("sandboxes")
            .join(&name);
        tokio::fs::create_dir_all(&target)
            .await
            .context("failed to create sandbox directory")?;
        let git_dir = target.join(".git");
        let existed = git_dir.exists();
        if !existed {
            run_git_command(&target, "init", Some(30_000))
                .await
                .context("failed to initialize git repo in sandbox")?;
        }
        let action = if existed {
            "existing".to_string()
        } else {
            "created".to_string()
        };
        return Ok(SetupWorkspaceResult {
            workspace_root: target,
            source: source.to_string(),
            action,
        });
    }

    let url_str = source.strip_prefix("git+").unwrap_or(source);
    let url = url_str.parse::<url::Url>().context("invalid git URL")?;
    if !is_allowed_git_scheme(url.scheme()) {
        anyhow::bail!("git URL scheme '{}' is not allowed", url.scheme());
    }
    // Reject URLs that embed credentials: git would write them verbatim into
    // `.git/config`, where the model can read them back. Private-repo access
    // must go through the server-side CHATCODEX_GITHUB_TOKEN instead.
    if !url.username().is_empty() || url.password().is_some() {
        anyhow::bail!(
            "git URL must not contain embedded credentials; configure CHATCODEX_GITHUB_TOKEN on the server and use a credential-free https URL instead"
        );
    }

    // Private GitHub repos: when the server holds a CHATCODEX_GITHUB_TOKEN,
    // authenticate the clone via an ephemeral credential helper so the token
    // is never written to disk. Public clones are untouched.
    let use_token = github_token_from_env().is_some() && is_github_host(url.host_str());
    if use_token {
        tracing::info!("setup_workspace: using CHATCODEX_GITHUB_TOKEN for {url_str}");
    }

    let target = workspace_base
        .as_path()
        .join("clients")
        .join(client_id)
        .join("repos")
        .join(&name);

    if target.exists() {
        let origin = run_git_command(&target, "remote get-url origin", Some(30_000))
            .await
            .context("failed to verify existing repo origin")?;
        if origin.exit_code != 0 {
            anyhow::bail!(
                "workspace {} exists but is not a valid git repository",
                target.display()
            );
        }
        let current_origin = origin.stdout.trim();
        if current_origin != url_str {
            anyhow::bail!(
                "workspace {} already exists and its origin remote ({}) does not match {}",
                target.display(),
                current_origin,
                url_str
            );
        }
        return Ok(SetupWorkspaceResult {
            workspace_root: target,
            source: source.to_string(),
            action: "existing".to_string(),
        });
    }

    let parent = target
        .parent()
        .context("target path has no parent directory")?;
    tokio::fs::create_dir_all(parent)
        .await
        .context("failed to create repo parent directory")?;

    let mut retries = 0;
    let max_retries = 3;
    let mut delays = [1_000, 2_000, 4_000].iter();
    loop {
        let result = run_git_command_with_args(
            parent,
            build_clone_argv(url_str, ref_name, use_token, &target),
            Some(timeout.as_millis() as u64),
        )
        .await;

        match result {
            Ok(output) if output.exit_code == 0 => {
                if use_token {
                    // Hardening: record the credential-free origin (git never
                    // saw credentials in the URL, but keep the remote clean
                    // anyway) and install the env-based credential helper so
                    // later fetch/push operations authenticate without the
                    // token ever being written to .git/config.
                    let clean = clean_git_url(&url);
                    let _ = run_git_command_with_args(
                        &target,
                        vec![
                            "remote".into(),
                            "set-url".into(),
                            "origin".into(),
                            clean.clone(),
                        ],
                        Some(30_000),
                    )
                    .await;
                    let _ = run_git_command_with_args(
                        &target,
                        vec![
                            "config".into(),
                            "credential.helper".into(),
                            github_credential_helper(),
                        ],
                        Some(30_000),
                    )
                    .await;
                }
                return Ok(SetupWorkspaceResult {
                    workspace_root: target,
                    source: source.to_string(),
                    action: "cloned".to_string(),
                });
            }
            Ok(output) => {
                if retries >= max_retries || !is_retriable_git_error(&output.stderr) {
                    anyhow::bail!(
                        "git clone failed (exit {}): {}. stderr: {}",
                        output.exit_code,
                        source,
                        output.stderr
                    );
                }
            }
            Err(error) => {
                if retries >= max_retries {
                    return Err(error.context(format!(
                        "git clone failed after {max_retries} retries: {source}"
                    )));
                }
            }
        }
        retries += 1;
        if let Some(&delay) = delays.next() {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
    }
}

fn is_retriable_git_error(stderr: &str) -> bool {
    let text = stderr.to_lowercase();
    text.contains("429")
        || ["500", "502", "503", "504", "507", "508", "509"]
            .iter()
            .any(|code| text.contains(code))
        || text.contains("timeout")
        || text.contains("timed out")
        || text.contains("connection reset")
}

#[derive(Debug)]
struct GitCommandOutput {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

async fn run_git_command(
    cwd: &Path,
    command: &str,
    timeout_ms: Option<u64>,
) -> anyhow::Result<GitCommandOutput> {
    if is_outbound_git_command(command) {
        anyhow::bail!("outbound git command blocked by policy: {command}");
    }
    let argv = shlex::split(command)
        .unwrap_or_else(|| command.split_whitespace().map(String::from).collect());
    run_git_command_with_args(cwd, argv, timeout_ms).await
}

fn is_outbound_git_command(command: &str) -> bool {
    let trimmed = command.trim();
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.is_empty() {
        return false;
    }
    match words[0] {
        "push" | "fetch" | "pull" | "clone" | "ls-remote" => return true,
        "remote" if words.len() >= 3 && (words[1] == "add" || words[1] == "set-url") => {
            return true;
        }
        "submodule" if words.iter().any(|w| w == &"update") => return true,
        _ => {}
    }
    false
}

fn git_command_requires_commit_permission(command: &str) -> bool {
    let words = shlex::split(command)
        .unwrap_or_else(|| command.split_whitespace().map(String::from).collect());
    let Some(subcommand) = words.first().map(String::as_str) else {
        return false;
    };
    match subcommand {
        "add" | "am" | "apply" | "bisect" | "branch" | "checkout" | "cherry-pick" | "clean"
        | "commit" | "config" | "merge" | "mv" | "rebase" | "reset" | "restore" | "revert"
        | "rm" | "stash" | "switch" | "tag" | "worktree" => true,
        _ => false,
    }
}

async fn run_git_command_with_args(
    cwd: &Path,
    argv: Vec<String>,
    timeout_ms: Option<u64>,
) -> anyhow::Result<GitCommandOutput> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.args(&argv).current_dir(cwd);
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(30_000).clamp(1_000, 120_000));
    let output = tokio::time::timeout(timeout, cmd.output())
        .await
        .context("git command timed out")?
        .context("failed to execute git")?;
    Ok(GitCommandOutput {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code().unwrap_or(-1),
    })
}

fn shell_escape(s: &str) -> String {
    if s.is_empty()
        || s.chars()
            .any(|c| matches!(c, ' ' | '\'' | '"' | '$' | '\\'))
    {
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        s.to_string()
    }
}

/// Build the `git clone` argv for a workspace setup.
///
/// When `use_token` is set, a credential helper is injected via `-c` so the
/// clone authenticates against GitHub without the token appearing in the URL
/// or on disk. The helper reads the token from the `CHATCODEX_GITHUB_TOKEN`
/// environment variable at call time.
fn build_clone_argv(
    source: &str,
    ref_name: Option<&str>,
    use_token: bool,
    target: &Path,
) -> Vec<String> {
    let mut argv = Vec::new();
    if use_token {
        argv.push("-c".to_string());
        argv.push(format!("credential.helper={}", github_credential_helper()));
    }
    argv.push("clone".to_string());
    if let Some(ref_name) = ref_name {
        argv.push("--branch".to_string());
        argv.push(ref_name.to_string());
    }
    argv.push("--".to_string());
    argv.push(source.to_string());
    argv.push(target.to_string_lossy().to_string());
    argv
}

/// Credential helper that feeds the server-side GitHub token to git without
/// ever writing it to disk. The helper only references the environment
/// variable name — the value lives exclusively in the daemon's environment.
fn github_credential_helper() -> String {
    "!f(){ echo \"username=x-access-token\"; echo \"password=$CHATCODEX_GITHUB_TOKEN\"; }; f"
        .to_string()
}

/// Whether the server has a non-empty CHATCODEX_GITHUB_TOKEN configured.
fn github_token_from_env() -> Option<String> {
    std::env::var("CHATCODEX_GITHUB_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
}

/// Whether the URL host is GitHub (github.com or a GitHub Enterprise host).
fn is_github_host(host: Option<&str>) -> bool {
    matches!(host, Some(h) if h == "github.com" || h.ends_with(".github.com"))
}

/// Reconstruct a git URL with any userinfo (credentials) stripped. Used to
/// keep the recorded `origin` remote free of credentials the model could read.
fn clean_git_url(url: &url::Url) -> String {
    let mut s = format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default());
    if let Some(port) = url.port() {
        s.push_str(&format!(":{port}"));
    }
    s.push_str(url.path());
    s
}

/// Run a git command with an explicit, minimal environment (no inherited
/// secrets beyond what the caller passes). Used for the sanctioned outbound
/// git_push path, where the credential env vars are injected deliberately.
async fn run_git_command_with_env(
    cwd: &Path,
    argv: Vec<String>,
    env: &std::collections::HashMap<String, String>,
    timeout_ms: Option<u64>,
) -> anyhow::Result<GitCommandOutput> {
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(60_000));
    let mut cmd = tokio::process::Command::new("git");
    cmd.args(&argv).current_dir(cwd).env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        cmd.env("PATH", path);
    }
    if let Some(home) = std::env::var_os("HOME") {
        cmd.env("HOME", home);
    }
    for (key, value) in env {
        cmd.env(key, value);
    }
    let output = tokio::time::timeout(timeout, cmd.output())
        .await
        .context("git command timed out")?
        .context("failed to execute git")?;
    Ok(GitCommandOutput {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code().unwrap_or(-1),
    })
}

fn validate_runtime(
    workspace_base: &AbsolutePathBuf,
    linux_sandbox_exe: Option<&Path>,
) -> anyhow::Result<()> {
    let bwrap_ok = linux_sandbox_exe
        .map(Path::is_file)
        .unwrap_or_else(|| codex_sandboxing::find_system_bwrap_in_path().is_some());
    if !bwrap_ok {
        anyhow::bail!(
            "Bubblewrap is unavailable: no system bwrap on PATH and no bundled \n             codex-resources/bwrap binary. Install bubblewrap in the runtime image \n             or bundle the helper."
        );
    }
    let base = workspace_base.as_path();
    if !base.exists() || !base.is_dir() {
        anyhow::bail!(
            "Workspace base {} does not exist or is not a directory. \n             Mount a persistent directory at /workspaces.",
            base.display()
        );
    }
    let test_file = base.join(".chatcodex-write-test");
    match std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&test_file)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&test_file);
        }
        Err(error) => {
            anyhow::bail!(
                "Workspace base {} is not writable: {error}. \n                 Mount a persistent directory at /workspaces.",
                base.display()
            );
        }
    }
    if let Some(path) = linux_sandbox_exe {
        tracing::info!(bwrap = %path.display(), "using bundled bubblewrap");
    } else {
        tracing::info!("using system bubblewrap");
    }
    Ok(())
}

fn tool_catalog() -> anyhow::Result<Vec<Tool>> {
    [
        (
            "exec_command",
            "Run a command in the workspace under a read-only filesystem sandbox.",
            json!({"type":"object","properties":{"cmd":{"type":"string"},"yield_time_ms":{"type":"integer","minimum":0,"maximum":30000}},"required":["cmd"],"additionalProperties":false}),
            json!({"type":"object","properties":{"output":{"type":"string"},"exit_code":{"oneOf":[{"type":"integer"},{"type":"null"}]},"session_id":{"oneOf":[{"type":"string"},{"type":"null"}]}},"required":["output","exit_code","session_id"],"additionalProperties":false}),
        ),
        (
            "write_stdin",
            "Write to or poll a running command session.",
            json!({"type":"object","properties":{"session_id":{"type":"string"},"chars":{"type":"string"},"yield_time_ms":{"type":"integer","minimum":0,"maximum":30000}},"required":["session_id"],"additionalProperties":false}),
            json!({"type":"object","properties":{"output":{"type":"string"},"exited":{"type":"boolean"},"exit_code":{"oneOf":[{"type":"integer"},{"type":"null"}]}},"required":["output","exited","exit_code"],"additionalProperties":false}),
        ),
        (
            "update_plan",
            "Replace the deterministic task plan.",
            json!({"type":"object","properties":{"explanation":{"type":"string"},"plan":{"type":"array","items":{"type":"object","properties":{"step":{"type":"string"},"status":{"enum":["pending","in_progress","completed"]}},"required":["step","status"],"additionalProperties":false}}},"required":["plan"],"additionalProperties":false}),
            json!({"type":"object","properties":{"explanation":{"type":"string"},"plan":{"type":"array","items":{"type":"object","properties":{"step":{"type":"string"},"status":{"enum":["pending","in_progress","completed"]}},"required":["step","status"],"additionalProperties":false}}},"required":["plan"],"additionalProperties":false}),
        ),
        (
            "apply_patch",
            "Apply a structured patch inside the workspace.",
            json!({"type":"object","properties":{"input":{"type":"string"}},"required":["input"],"additionalProperties":false}),
            json!({"type":"object","properties":{"result":{"type":"string"}},"required":["result"],"additionalProperties":false}),
        ),
        (
            "view_image",
            "Read an image located inside the workspace.",
            json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}),
            json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}),
        ),
        (
            "read_file",
            "Read a file from the workspace, optionally selecting a line range.",
            json!({"type":"object","properties":{"path":{"type":"string"},"start_line":{"type":"integer","minimum":1},"end_line":{"type":"integer","minimum":1}},"required":["path"],"additionalProperties":false}),
            json!({"type":"object","properties":{"path":{"type":"string"},"total_lines":{"type":"integer"},"start_line":{"type":"integer"},"end_line":{"type":"integer"},"content":{"type":"string"}},"required":["path","total_lines","start_line","end_line","content"],"additionalProperties":false}),
        ),
        (
            "search_code",
            "Search workspace source files for a text pattern using grep.",
            json!({"type":"object","properties":{"query":{"type":"string"},"path_glob":{"type":"string"},"max_results":{"type":"integer","minimum":1,"maximum":500}},"required":["query"],"additionalProperties":false}),
            json!({"type":"object","properties":{"matches":{"type":"array","items":{"type":"object","properties":{"path":{"type":"string"},"line":{"type":"integer"},"snippet":{"type":"string"}},"required":["path","line","snippet"],"additionalProperties":false}}},"required":["matches"],"additionalProperties":false}),
        ),
        (
            "setup_workspace",
            "Prepare the workspace for this client. Provide a git URL to clone a repository, or pass 'sandbox' for a persistent scratch directory with an initialized git repo. Private GitHub repositories are supported when the server has CHATCODEX_GITHUB_TOKEN configured: use a credential-free https://github.com/... URL and the clone is authenticated server-side. Pass \"ref\" (a branch, tag, or commit) to check out a specific ref after clone. URLs containing embedded credentials are rejected.",
            json!({"type":"object","properties":{"source":{"type":"string"},"name":{"type":"string"},"ref":{"type":"string"},"timeout_ms":{"type":"integer","minimum":1000,"maximum":120000}},"required":["source"],"additionalProperties":false}),
            json!({"type":"object","properties":{"workspace_root":{"type":"string"},"source":{"type":"string"},"action":{"type":"string"},"project_id":{"type":"string"}},"required":["workspace_root","source","action","project_id"],"additionalProperties":false}),
        ),
        (
            "git",
            "Run a local-only git command in the workspace. Outbound network operations (push, fetch, pull, clone, ls-remote, remote add/set-url, submodule update --init) are blocked by policy. Writable git commands run unsandboxed with declared network access restricted; read-only git_status and git_diff remain sandboxed.",
            json!({"type":"object","properties":{"command":{"type":"string"},"timeout_ms":{"type":"integer","minimum":1000,"maximum":120000}},"required":["command"],"additionalProperties":false}),
            json!({"type":"object","properties":{"stdout":{"type":"string"},"stderr":{"type":"string"},"exit_code":{"type":"integer"}},"required":["stdout","stderr","exit_code"],"additionalProperties":false}),
        ),
        (
            "git_status",
            "Show workspace status (git status --porcelain). Runs in the read-only sandbox.",
            json!({"type":"object","properties":{},"additionalProperties":false}),
            json!({"type":"object","properties":{"entries":{"type":"array","items":{"type":"object","properties":{"status":{"type":"string"},"path":{"type":"string"}},"required":["status","path"],"additionalProperties":false}},"stderr":{"type":"string"}},"required":["entries","stderr"],"additionalProperties":false}),
        ),
        (
            "get_time",
            "Return the current UTC time as ISO 8601 and unix seconds. Works even before a workspace is configured; useful for timestamps in commits, commit messages, and time-sensitive decisions.",
            json!({"type":"object","properties":{},"additionalProperties":false}),
            json!({"type":"object","properties":{"iso8601":{"type":"string"},"unix_seconds":{"type":"integer"}},"required":["iso8601","unix_seconds"],"additionalProperties":false}),
        ),
        (
            "memory_search",
            "Search persistent project memory (Hindsight) for facts, decisions, and context relevant to a query. Returns matched memory records with their text. Use before asking questions the workspace may not answer, and to pick up context from previous sessions. Requires the server to have CHATCODEX_HINDSIGHT_URL configured.",
            json!({"type":"object","properties":{"query":{"type":"string"},"budget":{"type":"string","enum":["low","mid","high"]},"tags":{"type":"array","items":{"type":"string"}},"max_tokens":{"type":"integer","minimum":1}},"required":["query"],"additionalProperties":false}),
            json!({"type":"object","properties":{"query":{"type":"string"},"results":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"text":{"type":"string"},"context":{"type":"string"},"occurred_start":{"type":"string"}},"additionalProperties":true}}},"required":["query","results"],"additionalProperties":false}),
        ),
        (
            "memory_retain",
            "Store a fact in persistent project memory (Hindsight). Use for durable knowledge that future sessions should remember: project conventions, user preferences, architectural decisions, and lessons learned. Content should be a self-contained declarative statement. Requires the server to have CHATCODEX_HINDSIGHT_URL configured.",
            json!({"type":"object","properties":{"content":{"type":"string"},"context":{"type":"string"},"tags":{"type":"array","items":{"type":"string"}}},"required":["content"],"additionalProperties":false}),
            json!({"type":"object","properties":{"stored":{"type":"boolean"},"content":{"type":"string"}},"required":["stored","content"],"additionalProperties":false}),
        ),
        (
            "memory_reflect",
            "Ask the Hindsight memory service to synthesize an answer across stored memories (runs on Hindsight's own model, not ChatGPT). Use for cross-session synthesis: 'what decisions did we make about X', 'summarize what we know about Y'. Requires the server to have CHATCODEX_HINDSIGHT_URL configured.",
            json!({"type":"object","properties":{"query":{"type":"string"},"budget":{"type":"string","enum":["low","mid","high"]},"tags":{"type":"array","items":{"type":"string"}},"max_tokens":{"type":"integer","minimum":1}},"required":["query"],"additionalProperties":false}),
            json!({"type":"object","properties":{"reflection":{"type":"string"}},"required":["reflection"],"additionalProperties":false}),
        ),
        (
            "git_diff",
            "Show workspace diff (git diff). Runs in the read-only sandbox.",
            json!({"type":"object","properties":{"paths":{"type":"array","items":{"type":"string"}},"staged":{"type":"boolean"}},"additionalProperties":false}),
            json!({"type":"object","properties":{"diff":{"type":"string"},"stderr":{"type":"string"}},"required":["diff","stderr"],"additionalProperties":false}),
        ),
        (
            "git_commit",
            "Create a git commit in the workspace. Runs unsandboxed with outbound network operations blocked; use only for local metadata writes.",
            json!({"type":"object","properties":{"message":{"type":"string"},"allow_empty":{"type":"boolean"},"timeout_ms":{"type":"integer","minimum":1000,"maximum":120000}},"required":["message"],"additionalProperties":false}),
            json!({"type":"object","properties":{"stdout":{"type":"string"},"stderr":{"type":"string"},"exit_code":{"type":"integer"}},"required":["stdout","stderr","exit_code"],"additionalProperties":false}),
        ),
        (
            "git_branch",
            "Create or move a git branch in the workspace. Runs unsandboxed with outbound network operations blocked; use only for local metadata writes.",
            json!({"type":"object","properties":{"name":{"type":"string"},"start_point":{"type":"string"},"force":{"type":"boolean"},"timeout_ms":{"type":"integer","minimum":1000,"maximum":120000}},"required":["name"],"additionalProperties":false}),
            json!({"type":"object","properties":{"stdout":{"type":"string"},"stderr":{"type":"string"},"exit_code":{"type":"integer"}},"required":["stdout","stderr","exit_code"],"additionalProperties":false}),
        ),
        (
            "git_checkout",
            "Switch branches in the workspace. Runs unsandboxed with outbound network operations blocked; use only for local metadata writes.",
            json!({"type":"object","properties":{"target":{"type":"string"},"create_branch":{"type":"boolean"},"timeout_ms":{"type":"integer","minimum":1000,"maximum":120000}},"required":["target"],"additionalProperties":false}),
            json!({"type":"object","properties":{"stdout":{"type":"string"},"stderr":{"type":"string"},"exit_code":{"type":"integer"}},"required":["stdout","stderr","exit_code"],"additionalProperties":false}),
        ),
        (
            "git_push",
            "Push the current (or named) branch to the origin remote, and optionally open a pull request for it. This is the only sanctioned outbound git operation: authentication uses the server-side CHATCODEX_GITHUB_TOKEN (the token is never exposed to the model or written to disk). Requires a cloned repository with a github.com (or GitHub Enterprise) origin. Set create_pr to true to open a pull request via the gh CLI; base defaults to the remote default branch.",
            json!({"type":"object","properties":{"branch":{"type":"string"},"create_pr":{"type":"boolean"},"base":{"type":"string"},"timeout_ms":{"type":"integer","minimum":1000,"maximum":120000}},"additionalProperties":false}),
            json!({"type":"object","properties":{"branch":{"type":"string"},"pushed":{"type":"boolean"},"stdout":{"type":"string"},"stderr":{"type":"string"},"exit_code":{"type":"integer"},"pr_url":{"type":"string"},"pr_error":{"type":"string"}},"required":["branch","pushed","stdout","stderr","exit_code"],"additionalProperties":false}),
        ),
        (
            "list_directory",
            "List entries in a workspace directory.",
            json!({"type":"object","properties":{"path":{"type":"string"}},"additionalProperties":false}),
            json!({"type":"object","properties":{"path":{"type":"string"},"entries":{"type":"array","items":{"type":"object","properties":{"name":{"type":"string"},"is_directory":{"type":"boolean"},"is_file":{"type":"boolean"}},"required":["name","is_directory","is_file"],"additionalProperties":false}}},"required":["path","entries"],"additionalProperties":false}),
        ),
        (
            "project_create",
            "Create or register a persistent project and optionally select it. Scratch projects create a persistent git-initialized sandbox; repo projects clone or reuse a git remote; workspace projects register an existing directory beneath the workspace base.",
            json!({"type":"object","properties":{"kind":{"type":"string","enum":["repo","workspace","scratch"]},"name":{"type":"string"},"source":{"type":"string"},"path":{"type":"string"},"select":{"type":"boolean","default":true},"timeout_ms":{"type":"integer","minimum":1000,"maximum":120000}},"required":["kind"],"additionalProperties":false}),
            json!({"type":"object","properties":{"project": project_schema(),"action":{"type":"string"},"selected":{"type":"boolean"}},"required":["project","action","selected"],"additionalProperties":false}),
        ),
        (
            "project_select",
            "Select a persistent project as the default workspace context. Selecting a project clears any selected run from another project.",
            json!({"type":"object","properties":{"project_id":{"type":"string"}},"required":["project_id"],"additionalProperties":false}),
            json!({"type":"object","properties":{"project": project_schema(),"selected":{"type":"boolean"}},"required":["project","selected"],"additionalProperties":false}),
        ),
        (
            "project_list",
            "List persistent projects for the current CHATCODEX_CLIENT_ID namespace.",
            json!({"type":"object","properties":{},"additionalProperties":false}),
            json!({"type":"object","properties":{"projects":{"type":"array","items": project_schema()},"active_project_id":{"oneOf":[{"type":"string"},{"type":"null"}]}},"required":["projects","active_project_id"],"additionalProperties":false}),
        ),
        (
            "project_get",
            "Get a persistent project by id, or the selected project when project_id is omitted.",
            json!({"type":"object","properties":{"project_id":{"type":"string"}},"additionalProperties":false}),
            json!({"type":"object","properties":{"project": project_schema(),"selected":{"type":"boolean"}},"required":["project","selected"],"additionalProperties":false}),
        ),
        (
            "run_start",
            "Start a persistent coding run for a selected project. ChatGPT owns reasoning; this only records deterministic run state and selects it.",
            json!({"type":"object","properties":{"project_id":{"type":"string"},"objective":{"type":"string"},"acceptance_criteria":{"type":"array","items":{"type":"string"}},"autonomy": autonomy_schema(),"select":{"type":"boolean","default":true}},"required":["objective"],"additionalProperties":false}),
            run_result_schema(),
        ),
        (
            "run_list",
            "List persistent runs, optionally filtered by project_id or lifecycle status.",
            json!({"type":"object","properties":{"project_id":{"type":"string"},"status":{"type":"string","enum":["active","paused","blocked","awaiting_approval","completed","cancelled"]}},"additionalProperties":false}),
            json!({"type":"object","properties":{"runs":{"type":"array","items": run_schema()},"active_run_id":{"oneOf":[{"type":"string"},{"type":"null"}]}},"required":["runs","active_run_id"],"additionalProperties":false}),
        ),
        (
            "run_get",
            "Get a persistent run by id, or the selected run when run_id is omitted.",
            json!({"type":"object","properties":{"run_id":{"type":"string"}},"additionalProperties":false}),
            run_result_schema(),
        ),
        (
            "run_update",
            "Deterministically update run phase, status, plan, checklist, checkpoints, work_remaining, next_action, or step counters. Invalid transitions and limit overruns are rejected server-side.",
            json!({"type":"object","properties":{"run_id":{"type":"string"},"phase":{"type":"string","enum":["inspect","plan","execute","verify"]},"status":{"type":"string","enum":["active","paused","blocked","awaiting_approval","completed","cancelled"]},"acceptance_criteria":{"type":"array","items":{"type":"string"}},"plan":{"type":"array","items": plan_item_schema()},"checklist":{"type":"array","items": checklist_item_schema()},"checkpoint":{"type":"object","properties":{"message":{"type":"string"}},"required":["message"],"additionalProperties":false},"work_remaining":{"type":"boolean"},"next_action":{"type":"string"},"step_delta":{"type":"integer","minimum":0}},"additionalProperties":false}),
            run_result_schema(),
        ),
        (
            "run_resume",
            "Resume and select a non-terminal persistent run after ChatGPT receives a follow-up or the user explicitly asks to continue. Respects run limits and never starts an agent loop.",
            json!({"type":"object","properties":{"run_id":{"type":"string"}},"additionalProperties":false}),
            run_result_schema(),
        ),
        (
            "run_cancel",
            "Cancel a non-completed persistent run and clear any continuation lease.",
            json!({"type":"object","properties":{"run_id":{"type":"string"}},"additionalProperties":false}),
            run_result_schema(),
        ),
        (
            "run_followup_lease",
            "Acquire a duplicate-safe continuation lease for the run-status component. Grants only for active runs with work remaining and available autonomy limits.",
            json!({"type":"object","properties":{"run_id":{"type":"string"},"requested_nonce":{"type":"string"},"ttl_ms":{"type":"integer","minimum":1000,"maximum":300000},"delay_ms":{"type":"integer","minimum":0,"maximum":30000}},"required":["run_id"],"additionalProperties":false}),
            followup_lease_schema(),
        ),
        (
            "todo",
            "Manage a TODO checklist. Use `action: \"replace\"` to set the full list, `action: \"update\"` to check off or dismiss items by id. Returns the current state with summary and `all_done` flag. After each step, check if all_done is true; if not, continue working on remaining pending items.",
            json!({"type":"object","properties":{"items":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"description":{"type":"string"},"status":{"type":"string","enum":["pending","checked","dismissed"]}},"required":[],"additionalProperties":false}},"action":{"type":"string","enum":["replace","update"],"default":"replace"}},"required":["items"],"additionalProperties":false}),
            json!({"type":"object","properties":{"items":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"description":{"type":"string"},"status":{"type":"string","enum":["pending","checked","dismissed"]}},"required":["id","description","status"],"additionalProperties":false}},"summary":{"type":"object","properties":{"total":{"type":"integer"},"pending":{"type":"integer"},"checked":{"type":"integer"},"dismissed":{"type":"integer"}},"required":["total","pending","checked","dismissed"],"additionalProperties":false},"all_done":{"type":"boolean"}},"required":["items","summary","all_done"],"additionalProperties":false}),
        ),
    ]
    .into_iter()
    .map(|(name, description, schema, output_schema)| {
        let output_schema = if tool_gets_active_run_metadata(name) {
            add_optional_run_metadata_to_schema(output_schema)
        } else {
            output_schema
        };
        let tool = Tool::new(
            Cow::Borrowed(name),
            Cow::Borrowed(description),
            Arc::new(serde_json::from_value::<JsonObject>(schema)?),
        )
        .with_raw_output_schema(Arc::new(serde_json::from_value::<JsonObject>(output_schema)?))
        .with_annotations(tool_annotations(name)?);
        let tool = if let Some(meta) = app_resources::tool_meta(name) {
            tool.with_meta(meta)
        } else {
            tool
        };
        Ok(tool)
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
        "read_file" => ToolAnnotations::with_title("Read file")
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        "search_code" => ToolAnnotations::with_title("Search code")
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        "git_status" => ToolAnnotations::with_title("Git status")
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        "get_time" => ToolAnnotations::with_title("Get current time")
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(true),
        "memory_search" => ToolAnnotations::with_title("Search memory")
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(true),
        "memory_retain" => ToolAnnotations::with_title("Store memory")
            .read_only(false)
            .destructive(false)
            .idempotent(true)
            .open_world(true),
        "memory_reflect" => ToolAnnotations::with_title("Reflect over memory")
            .read_only(true)
            .destructive(false)
            .idempotent(false)
            .open_world(true),
        "git_diff" => ToolAnnotations::with_title("Git diff")
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        "setup_workspace" => ToolAnnotations::with_title("Setup workspace")
            .read_only(false)
            .destructive(false)
            .idempotent(false)
            .open_world(true),
        "git" => ToolAnnotations::with_title("Run git command")
            .read_only(true)
            .destructive(false)
            .idempotent(false)
            .open_world(false),
        "git_commit" => ToolAnnotations::with_title("Git commit")
            .read_only(false)
            .destructive(false)
            .idempotent(false)
            .open_world(false),
        "git_branch" => ToolAnnotations::with_title("Git branch")
            .read_only(false)
            .destructive(false)
            .idempotent(false)
            .open_world(false),
        "git_checkout" => ToolAnnotations::with_title("Git checkout")
            .read_only(false)
            .destructive(false)
            .idempotent(false)
            .open_world(false),
        "git_push" => ToolAnnotations::with_title("Git push")
            .read_only(false)
            .destructive(false)
            .idempotent(false)
            .open_world(true),
        "list_directory" => ToolAnnotations::with_title("List directory")
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        "todo" => ToolAnnotations::with_title("TODO checklist")
            .read_only(false)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        "project_create" => ToolAnnotations::with_title("Create project")
            .read_only(false)
            .destructive(false)
            .idempotent(true)
            .open_world(true),
        "project_select" => ToolAnnotations::with_title("Select project")
            .read_only(false)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        "project_list" => ToolAnnotations::with_title("List projects")
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        "project_get" => ToolAnnotations::with_title("Get project")
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        "run_start" => ToolAnnotations::with_title("Start run")
            .read_only(false)
            .destructive(false)
            .idempotent(false)
            .open_world(false),
        "run_list" => ToolAnnotations::with_title("List runs")
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        "run_get" => ToolAnnotations::with_title("Get run")
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        "run_update" => ToolAnnotations::with_title("Update run")
            .read_only(false)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        "run_resume" => ToolAnnotations::with_title("Resume run")
            .read_only(false)
            .destructive(false)
            .idempotent(false)
            .open_world(false),
        "run_cancel" => ToolAnnotations::with_title("Cancel run")
            .read_only(false)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        "run_followup_lease" => ToolAnnotations::with_title("Acquire follow-up lease")
            .read_only(false)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        _ => anyhow::bail!("missing annotation policy for {name}"),
    })
}

fn add_optional_run_metadata_to_schema(mut schema: Value) -> Value {
    if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        properties.insert("run_metadata".to_string(), run_metadata_schema());
    }
    schema
}

fn project_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": {"type": "string"},
            "name": {"type": "string"},
            "kind": {"type": "string", "enum": ["repo", "workspace", "scratch"]},
            "workspace_root": {"type": "string"},
            "source": {"type": "object"},
            "created_at_ms": {"type": "integer"},
            "updated_at_ms": {"type": "integer"}
        },
        "required": ["id", "name", "kind", "workspace_root", "source", "created_at_ms", "updated_at_ms"],
        "additionalProperties": false
    })
}

fn run_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": {"type": "string"},
            "project_id": {"type": "string"},
            "objective": {"type": "string"},
            "acceptance_criteria": {"type": "array", "items": {"type": "string"}},
            "phase": {"type": "string", "enum": ["inspect", "plan", "execute", "verify"]},
            "status": {"type": "string", "enum": ["active", "paused", "blocked", "awaiting_approval", "completed", "cancelled"]},
            "plan": {"type": "array", "items": plan_item_schema()},
            "checklist": {"type": "array", "items": checklist_item_schema()},
            "checkpoints": {"type": "array", "items": {"type": "object"}},
            "autonomy": autonomy_schema(),
            "counters": {"type": "object"},
            "continuation": {"type": "object"},
            "work_remaining": {"type": "boolean"},
            "next_action": {"type": "string"},
            "created_at_ms": {"type": "integer"},
            "updated_at_ms": {"type": "integer"},
            "started_at_ms": {"type": "integer"},
            "completed_at_ms": {"oneOf": [{"type": "integer"}, {"type": "null"}]},
            "cancelled_at_ms": {"oneOf": [{"type": "integer"}, {"type": "null"}]}
        },
        "required": ["id", "project_id", "objective", "acceptance_criteria", "phase", "status", "plan", "checklist", "checkpoints", "autonomy", "counters", "continuation", "work_remaining", "next_action", "created_at_ms", "updated_at_ms", "started_at_ms"],
        "additionalProperties": false
    })
}

fn run_result_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "run": run_schema(),
            "run_metadata": run_metadata_schema()
        },
        "required": ["run", "run_metadata"],
        "additionalProperties": false
    })
}

fn plan_item_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "step": {"type": "string"},
            "status": {"type": "string", "enum": ["pending", "in_progress", "completed"]}
        },
        "required": ["step", "status"],
        "additionalProperties": false
    })
}

fn checklist_item_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": {"type": "string"},
            "description": {"type": "string"},
            "status": {"type": "string", "enum": ["pending", "checked", "dismissed"]}
        },
        "required": ["id", "description", "status"],
        "additionalProperties": false
    })
}

fn autonomy_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "max_turns": {"type": "integer", "minimum": 0},
            "max_runtime_seconds": {"type": "integer", "minimum": 0},
            "max_steps": {"type": "integer", "minimum": 0},
            "allow_local_commands": {"type": "boolean"},
            "allow_file_edits": {"type": "boolean"},
            "allow_git_commits": {"type": "boolean"}
        },
        "required": [],
        "additionalProperties": false
    })
}

fn run_metadata_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "run_id": {"type": "string"},
            "project_id": {"type": "string"},
            "phase": {"type": "string", "enum": ["inspect", "plan", "execute", "verify"]},
            "status": {"type": "string", "enum": ["active", "paused", "blocked", "awaiting_approval", "completed", "cancelled"]},
            "work_remaining": {"type": "boolean"},
            "next_action": {"type": "string"},
            "limits": {"type": "object"},
            "lease": {"type": "object"}
        },
        "required": ["run_id", "project_id", "phase", "status", "work_remaining", "next_action", "limits", "lease"],
        "additionalProperties": false
    })
}

fn followup_lease_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "run_id": {"type": "string"},
            "granted": {"type": "boolean"},
            "duplicate": {"type": "boolean"},
            "nonce": {"oneOf": [{"type": "string"}, {"type": "null"}]},
            "acquired_at_ms": {"oneOf": [{"type": "integer"}, {"type": "null"}]},
            "expires_at_ms": {"oneOf": [{"type": "integer"}, {"type": "null"}]},
            "delay_ms": {"type": "integer"},
            "max_turns": {"type": "integer"},
            "max_runtime_seconds": {"type": "integer"},
            "max_steps": {"type": "integer"},
            "reason": {"oneOf": [{"type": "string"}, {"type": "null"}]},
            "run_metadata": run_metadata_schema()
        },
        "required": ["run_id", "granted", "duplicate", "delay_ms", "max_turns", "max_runtime_seconds", "max_steps", "run_metadata"],
        "additionalProperties": false
    })
}

fn prompt_catalog() -> Vec<Prompt> {
    vec![
        Prompt::new(
            "read-file-guide",
            Some("How to read files in the workspace"),
            None,
        )
        .with_title("Reading Files Guide"),
        Prompt::new(
            "search-code-guide",
            Some("How to search workspace source code"),
            None,
        )
        .with_title("Code Search Guide"),
        Prompt::new(
            "apply-patch-guide",
            Some("How to apply patches to workspace files"),
            None,
        )
        .with_title("Applying Patches Guide"),
        Prompt::new(
            "run-command-guide",
            Some("How to run commands in the workspace sandbox"),
            None,
        )
        .with_title("Running Commands Guide"),
        Prompt::new(
            "workspace-overview-guide",
            Some("How to explore and understand the workspace structure"),
            None,
        )
        .with_title("Workspace Overview Guide"),
        Prompt::new(
            "git-operations-guide",
            Some("How to inspect git status and diffs"),
            None,
        )
        .with_title("Git Operations Guide"),
        Prompt::new(
            "task-completion-guide",
            Some("How to complete tasks thoroughly using the TODO checklist"),
            None,
        )
        .with_title("Task Completion Guide"),
    ]
}

fn get_prompt_content(name: &str) -> Option<GetPromptResult> {
    match name {
        "read-file-guide" => Some(GetPromptResult::new(vec![
            PromptMessage::new_text(
                PromptMessageRole::User,
                "How do I read files in the workspace?",
            ),
            PromptMessage::new_text(
                PromptMessageRole::Assistant,
                r#"# Reading Files in ChatCodex

Use the `read_file` tool to read files from the workspace. This tool is preferred over `exec_command cat <file>` because it is deterministic, respects the filesystem sandbox, and supports line-range selection.

## Basic usage

```
read_file(path: "src/main.rs")
```

## Reading a specific line range

```
read_file(path: "src/main.rs", start_line: 10, end_line: 50)
```

## Notes

- Paths are relative to the workspace root.
- Absolute paths are allowed only if they resolve inside the workspace.
- The tool returns the file content along with total_lines, start_line, and end_line metadata.
- For binary or image files, use `view_image` instead.
- For very large files, always specify a line range to avoid excessive output."#,
            ),
        ])),

        "search-code-guide" => Some(GetPromptResult::new(vec![
            PromptMessage::new_text(
                PromptMessageRole::User,
                "How do I search for code in the workspace?",
            ),
            PromptMessage::new_text(
                PromptMessageRole::Assistant,
                r#"# Searching Code in ChatCodex

Use the `search_code` tool to find text patterns across workspace files. It uses `grep -rn` under the hood and is preferred over `exec_command grep ...` because it is deterministic and structured.

## Basic usage

```
search_code(query: "fn main")
```

## With file type filter

```
search_code(query: "TODO", path_glob: "*.rs")
```

## Limiting results

```
search_code(query: "impl", max_results: 20)
```

## Notes

- The query is a plain-text grep pattern (not a regex by default, though grep patterns apply).
- `path_glob` filters by filename pattern (e.g., `*.rs`, `*.py`, `*.{ts,js}`).
- `max_results` defaults to 50 and caps at 500.
- Binary file matches are automatically skipped.
- Results include path, line number, and the matching snippet."#,
            ),
        ])),

        "apply-patch-guide" => Some(GetPromptResult::new(vec![
            PromptMessage::new_text(
                PromptMessageRole::User,
                "How do I apply patches to workspace files?",
            ),
            PromptMessage::new_text(
                PromptMessageRole::Assistant,
                r#"# Applying Patches in ChatCodex

Use the `apply_patch` tool for all workspace file modifications. This is the only write path for workspace files — do not use `exec_command` with sed, echo, or other file-writing commands.

## Patch format

The patch format uses structured markers:

```
*** Begin Patch
*** Add File: path/to/new_file.txt
+file content here
*** End Patch
```

```
*** Edit File: path/to/existing.txt
-old line
+new line
```

```
*** Remove File: path/to/obsolete.txt
```

## Notes

- Always inspect the file first with `read_file` before patching.
- Multiple operations can be combined in a single patch.
- The tool returns the patch result and any error messages.
- Patches are applied atomically where possible."#,
            ),
        ])),

        "run-command-guide" => Some(GetPromptResult::new(vec![
            PromptMessage::new_text(
                PromptMessageRole::User,
                "How do I run commands in the workspace?",
            ),
            PromptMessage::new_text(
                PromptMessageRole::Assistant,
                r#"# Running Commands in ChatCodex

Use the `exec_command` tool to run shell commands. Commands execute in a read-only filesystem sandbox with network access enabled.

## Basic usage

```
exec_command(cmd: "ls -la")
```

## Running with a yield timeout

```
exec_command(cmd: "cargo build", yield_time_ms: 30000)
```

## Interactive sessions

For long-running or interactive commands, use `exec_command` first, then `write_stdin` to interact with the session:

```
exec_command(cmd: "python3")
write_stdin(session_id: "abc-123", chars: "print('hello')\n")
```

## Notes

- Dangerous commands (rm -rf, sudo, etc.) are rejected by policy.
- The sandbox is read-only — use `apply_patch` for file writes.
- Prefer `mise exec -- <command>` for toolchains.
- Use `uv` for Python environments.
- System package installation and privilege escalation are forbidden."#,
            ),
        ])),

        "workspace-overview-guide" => Some(GetPromptResult::new(vec![
            PromptMessage::new_text(
                PromptMessageRole::User,
                "How do I explore the workspace structure?",
            ),
            PromptMessage::new_text(
                PromptMessageRole::Assistant,
                r#"# Workspace Overview in ChatCodex

Use `list_directory` to explore the workspace directory structure, and `read_file` to inspect individual files.

## List the workspace root

```
list_directory()
```

## List a subdirectory

```
list_directory(path: "src")
```

## Explore recursively

Use `exec_command` with `find` or `tree` for recursive listings:

```
exec_command(cmd: "find . -type f | head -50")
```

## Notes

- `list_directory` returns entries with name, is_directory, and is_file.
- All paths are relative to the workspace root.
- Use `search_code` to find files by content.
- Use `git_status` to see which files have been modified."#,
            ),
        ])),

        "git-operations-guide" => Some(GetPromptResult::new(vec![
            PromptMessage::new_text(
                PromptMessageRole::User,
                "How do I inspect git state in the workspace?",
            ),
            PromptMessage::new_text(
                PromptMessageRole::Assistant,
                r#"# Git Operations in ChatCodex

Use `git_status` and `git_diff` to inspect the repository state without running arbitrary git commands.

## Check working tree status

```
git_status()
```

Returns a structured list of changed files with their status codes.

## View unstaged diff

```
git_diff()
```

## View staged diff

```
git_diff(staged: true)
```

## Diff specific paths

```
git_diff(paths: ["src/main.rs"])
```

## Notes

- These tools are preferred over `exec_command git ...` because they return structured results.
- For committing, branching, and other write operations, use `exec_command` with git commands.
- Always check `git_status` before applying patches to understand the current state."#,
            ),
        ])),

        "task-completion-guide" => Some(GetPromptResult::new(vec![
            PromptMessage::new_text(
                PromptMessageRole::User,
                "How do I make sure I complete the entire task?",
            ),
            PromptMessage::new_text(
                PromptMessageRole::Assistant,
                r#"# Task Completion in ChatCodex

Follow this protocol to ensure you complete tasks thoroughly:

## 1. Plan with TODO

Break the user's request into concrete, actionable items:

```
todo(items: [
  {id: "t1", description: "Inspect project structure and understand layout"},
  {id: "t2", description: "Implement feature X in module Y"},
  {id: "t3", description: "Add tests for feature X"},
  {id: "t4", description: "Run tests and verify everything passes"},
], action: "replace")
```

## 2. Execute and check off

Work through items one at a time. After completing each:

```
todo(items: [{id: "t1", status: "checked"}], action: "update")
```

The tool returns `all_done: false` while items remain. Keep going.

## 3. Handle blockers

If an item turns out to be unnecessary or blocked, dismiss it:

```
todo(items: [{id: "t3", status: "dismissed"}], action: "update")
```

## 4. Verify before finishing

Once all_done is true, do a final verification step (run tests, check output).

## Golden rule

Do NOT stop working until `all_done` is `true`. If the user's request has multiple parts, every part must be addressed. Only stop when all items are checked or dismissed."#,
            ),
        ])),

        _ => None,
    }
}

impl ServerHandler for NativeHarnessMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .enable_resources()
                .build(),
        )
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

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListPromptsResult, McpError>> + Send + '_ {
        let prompts = Arc::clone(&self.prompts);
        async move {
            Ok(ListPromptsResult {
                prompts: (*prompts).clone(),
                next_cursor: None,
                meta: None,
            })
        }
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> impl std::future::Future<Output = Result<GetPromptResult, McpError>> + Send + '_ {
        let result = get_prompt_content(&request.name);
        async move { result.ok_or_else(|| McpError::invalid_params("unknown prompt", None)) }
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        let tools = Arc::clone(&self.tools);
        async move {
            let resources: Vec<rmcp::model::Resource> = tools
                .iter()
                .map(|tool| {
                    RawResource::new(format!("tool:///{}", tool.name), tool.name.to_string())
                        .with_description(tool.description.clone().unwrap_or_default())
                        .no_annotation()
                })
                .collect();
            let mut resources = resources;
            resources.push(app_resources::run_status_resource());
            Ok(ListResourcesResult {
                resources,
                next_cursor: None,
                meta: None,
            })
        }
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourceTemplatesResult, McpError>> + Send + '_
    {
        async move {
            Ok(ListResourceTemplatesResult {
                resource_templates: vec![],
                next_cursor: None,
                meta: None,
            })
        }
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let uri = &request.uri;
        if uri == app_resources::RUN_STATUS_RESOURCE_URI {
            return Ok(ReadResourceResult::new(vec![
                app_resources::run_status_resource_contents(),
            ]));
        }
        if let Some(tool_name) = uri.strip_prefix("tool:///") {
            let tools = self.tools.as_slice();
            if let Some(tool) = tools.iter().find(|t| t.name == tool_name) {
                let content = serde_json::to_string_pretty(tool).unwrap_or_default();
                return Ok(ReadResourceResult::new(vec![ResourceContents::text(
                    content, uri,
                )]));
            }
        }
        Err(McpError::resource_not_found(
            "resource_not_found",
            Some(json!({ "uri": uri })),
        ))
    }
}

pub async fn http_router(arg0_paths: Arg0DispatchPaths) -> anyhow::Result<Router> {
    let auth_state = chatcodex_oauth::AuthState::from_env()?;
    http_router_with_state(arg0_paths, auth_state).await
}

pub async fn http_router_with_state(
    arg0_paths: Arg0DispatchPaths,
    auth_state: chatcodex_oauth::AuthState,
) -> anyhow::Result<Router> {
    let service = NativeHarnessMcp::new_with_arg0_paths(arg0_paths).await?;
    let public_url = http::Uri::try_from(auth_state.config().public_base_url.as_str())
        .context("invalid CHATCODEX_PUBLIC_BASE_URL")?;
    let public_host = public_url
        .host()
        .ok_or_else(|| anyhow::anyhow!("CHATCODEX_PUBLIC_BASE_URL must have a host"))?
        .to_string();
    let mut mcp_config = StreamableHttpServerConfig::default();
    mcp_config.allowed_hosts.push(public_host);
    let mcp_service = StreamableHttpService::new(
        move || Ok(service.clone()),
        Arc::new(LocalSessionManager::default()),
        mcp_config,
    );
    static PROMETHEUS_HANDLE: std::sync::OnceLock<metrics_exporter_prometheus::PrometheusHandle> =
        std::sync::OnceLock::new();
    let prometheus_layer = PrometheusMetricLayer::new();
    let prometheus_handle = PROMETHEUS_HANDLE
        .get_or_init(|| {
            let (_layer, handle) = PrometheusMetricLayer::pair();
            handle
        })
        .clone();

    Ok(Router::new()
        .route(
            "/metrics",
            get(move || {
                let prometheus_handle = prometheus_handle.clone();
                async move { prometheus_handle.render() }
            }),
        )
        .route("/healthz", get(|| async { Json(json!({"status":"ok"})) }))
        .merge(
            Router::new()
                .route(
                    "/.well-known/oauth-authorization-server",
                    get(chatcodex_oauth::well_known::oauth_authorization_server),
                )
                .route(
                    "/.well-known/oauth-protected-resource",
                    get(chatcodex_oauth::well_known::oauth_protected_resource),
                )
                .route(
                    "/.well-known/jwks.json",
                    get(chatcodex_oauth::well_known::jwks),
                )
                .route(
                    "/oauth/authorize",
                    get(chatcodex_oauth::authorize::authorize),
                )
                .route(
                    "/oauth/authorize/decide",
                    axum::routing::post(chatcodex_oauth::authorize::decide),
                )
                .route(
                    "/oauth/register",
                    axum::routing::post(chatcodex_oauth::clients::register),
                )
                .route(
                    "/oauth/token",
                    axum::routing::post(chatcodex_oauth::token::token).layer(
                        axum::middleware::from_fn_with_state(
                            chatcodex_oauth::ratelimit::RateLimiter::new(10, 60),
                            chatcodex_oauth::ratelimit::rate_limit_token,
                        ),
                    ),
                )
                .route(
                    "/oauth/introspect",
                    axum::routing::post(chatcodex_oauth::token::introspect),
                )
                .route(
                    "/oauth/revoke",
                    axum::routing::post(chatcodex_oauth::token::revoke),
                )
                .with_state(auth_state.clone()),
        )
        .nest(
            "/mcp",
            Router::new().fallback_service(mcp_service).layer(
                axum::middleware::from_fn_with_state(
                    auth_state,
                    chatcodex_oauth::middleware::require_bearer,
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
#[allow(clippy::unwrap_used, clippy::expect_used)]
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
                "view_image",
                "read_file",
                "search_code",
                "setup_workspace",
                "git",
                "git_status",
                "get_time",
                "memory_search",
                "memory_retain",
                "memory_reflect",
                "git_diff",
                "git_commit",
                "git_branch",
                "git_checkout",
                "git_push",
                "list_directory",
                "project_create",
                "project_select",
                "project_list",
                "project_get",
                "run_start",
                "run_list",
                "run_get",
                "run_update",
                "run_resume",
                "run_cancel",
                "run_followup_lease",
                "todo",
            ]
        );
        assert!(server.tools().iter().all(|tool| tool.annotations.is_some()));
        assert!(
            server
                .tools()
                .iter()
                .all(|tool| tool.output_schema.is_some())
        );
    }

    #[tokio::test]
    async fn get_time_returns_valid_utc() {
        let workspace = tempfile::tempdir().expect("workspace");
        let server = NativeHarnessMcp::new_for_paths(
            workspace.path(),
            codex_arg0::Arg0DispatchPaths::default(),
        )
        .await
        .expect("server");

        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("before epoch")
            .as_secs() as i64;
        let result = server
            .harness
            .call("get_time", serde_json::json!({}))
            .await
            .expect("get_time call");
        assert!(
            result.is_error != Some(true),
            "get_time reported an error: {result:?}"
        );
        let parsed = result
            .structured_content
            .as_ref()
            .expect("structured content");
        let iso = parsed["iso8601"].as_str().expect("iso8601 string");
        let unix = parsed["unix_seconds"].as_i64().expect("unix_seconds int");
        let after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after epoch")
            .as_secs() as i64;
        assert!(
            unix >= before && unix <= after,
            "unix_seconds {unix} outside [{before}, {after}]"
        );
        // ISO 8601 must be UTC (Z suffix) and round-trip to the same instant.
        assert!(iso.ends_with('Z'), "iso8601 should be UTC: {iso}");
        let parsed_back = time::OffsetDateTime::parse(
            iso,
            &time::format_description::well_known::Rfc3339,
        )
        .expect("rfc3339 parse");
        assert_eq!(parsed_back.unix_timestamp(), unix, "iso/unix mismatch");
    }

    #[tokio::test]
    async fn catalog_output_schemas_match_structured_results() {
        let workspace = tempfile::tempdir().expect("workspace");
        let server = NativeHarnessMcp::new_for_paths(
            workspace.path(),
            codex_arg0::Arg0DispatchPaths::default(),
        )
        .await
        .expect("server");

        let view_image = server
            .tools()
            .iter()
            .find(|tool| tool.name == "view_image")
            .expect("view_image tool");
        let output_schema = view_image.output_schema.as_ref().expect("output schema");
        assert_eq!(
            output_schema.get("required"),
            Some(&serde_json::json!(["path"]))
        );
        assert_eq!(
            output_schema
                .get("properties")
                .and_then(serde_json::Value::as_object)
                .map(|properties| properties.keys().cloned().collect::<Vec<_>>()),
            Some(vec!["path".to_string(), "run_metadata".to_string()])
        );

        for tool in server.tools() {
            let output_schema = tool.output_schema.as_ref().expect("output schema");
            assert_eq!(
                output_schema.get("type"),
                Some(&serde_json::json!("object")),
                "{} output schema must describe an object",
                tool.name
            );
            assert_eq!(
                output_schema.get("additionalProperties"),
                Some(&serde_json::json!(false)),
                "{} output schema must reject undeclared fields",
                tool.name
            );
        }
    }

    #[tokio::test]
    async fn catalog_exposes_run_status_component_metadata() {
        let workspace = tempfile::tempdir().expect("workspace");
        let server = NativeHarnessMcp::new_for_paths(
            workspace.path(),
            codex_arg0::Arg0DispatchPaths::default(),
        )
        .await
        .expect("server");
        let run_get = server
            .tools()
            .iter()
            .find(|tool| tool.name == "run_get")
            .expect("run_get tool");
        let meta = run_get.meta.as_ref().expect("run_get meta");
        assert_eq!(
            meta.0["openai/outputTemplate"],
            serde_json::json!(super::app_resources::RUN_STATUS_RESOURCE_URI)
        );
        let lease = server
            .tools()
            .iter()
            .find(|tool| tool.name == "run_followup_lease")
            .expect("run_followup_lease tool");
        let meta = lease.meta.as_ref().expect("lease tool meta");
        assert_eq!(meta.0["openai/widgetAccessible"], serde_json::json!(true));
        assert!(meta.0.get("openai/outputTemplate").is_none());
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
        assert!(codex_shell_command::is_dangerous_command::dangerous_command_match(&command).is_some());
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
        server
            .harness
            .call("setup_workspace", serde_json::json!({"source": "sandbox"}))
            .await
            .expect("setup workspace");
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
            std::fs::read_to_string(
                workspace
                    .path()
                    .join("clients")
                    .join("default")
                    .join("sandboxes")
                    .join("sandbox")
                    .join("proof.txt")
            )
            .expect("patched file"),
            "patched\n"
        );
    }

    #[tokio::test]
    async fn todo_manages_checklist() {
        let workspace = tempfile::tempdir().expect("workspace");
        let server = NativeHarnessMcp::new_for_paths(
            workspace.path(),
            codex_arg0::Arg0DispatchPaths::default(),
        )
        .await
        .expect("server");
        // Replace with items
        let result = server
            .harness
            .call(
                "todo",
                serde_json::json!({
                    "items": [
                        {"id": "t1", "description": "First task"},
                        {"id": "t2", "description": "Second task"},
                    ],
                    "action": "replace",
                }),
            )
            .await
            .expect("todo replace");
        assert_eq!(result.is_error, Some(false), "{result:?}");
        let parsed: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(parsed["summary"]["pending"], 2);
        assert_eq!(parsed["summary"]["total"], 2);
        assert_eq!(parsed["all_done"], false);

        // Check off one item
        let result = server
            .harness
            .call(
                "todo",
                serde_json::json!({
                    "items": [{"id": "t1", "status": "checked"}],
                    "action": "update",
                }),
            )
            .await
            .expect("todo check");
        let parsed: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(parsed["summary"]["pending"], 1);
        assert_eq!(parsed["summary"]["checked"], 1);
        assert_eq!(parsed["all_done"], false);

        // Dismiss the other
        let result = server
            .harness
            .call(
                "todo",
                serde_json::json!({
                    "items": [{"id": "t2", "status": "dismissed"}],
                    "action": "update",
                }),
            )
            .await
            .expect("todo dismiss");
        let parsed: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(parsed["summary"]["pending"], 0);
        assert_eq!(parsed["summary"]["dismissed"], 1);
        assert_eq!(parsed["all_done"], true);
    }

    #[tokio::test]
    async fn active_run_metadata_is_added_to_coding_tool_results() {
        let workspace = tempfile::tempdir().expect("workspace");
        let server = NativeHarnessMcp::new_for_paths(
            workspace.path(),
            codex_arg0::Arg0DispatchPaths::default(),
        )
        .await
        .expect("server");
        let setup = server
            .harness
            .call("setup_workspace", serde_json::json!({"source": "sandbox"}))
            .await
            .expect("setup workspace");
        let setup_json: serde_json::Value =
            serde_json::from_str(setup.content[0].as_text().unwrap().text.as_str()).unwrap();
        let project_id = setup_json["project_id"].as_str().unwrap().to_string();
        let run = server
            .harness
            .call(
                "run_start",
                serde_json::json!({
                    "project_id": project_id,
                    "objective": "prove metadata",
                    "acceptance_criteria": ["metadata is present"]
                }),
            )
            .await
            .expect("run start");
        let run_json: serde_json::Value =
            serde_json::from_str(run.content[0].as_text().unwrap().text.as_str()).unwrap();
        let run_id = run_json["run"]["id"].as_str().unwrap().to_string();

        let result = server
            .harness
            .call("list_directory", serde_json::json!({}))
            .await
            .expect("list directory");
        let parsed: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(parsed["run_metadata"]["run_id"], run_id);
        assert_eq!(parsed["run_metadata"]["status"], "active");
        let meta = result.meta.expect("tool result meta");
        assert_eq!(meta.0["chatcodex/run"]["run_id"], run_id);
    }

    #[tokio::test]
    async fn legacy_setup_workspace_keeps_workspace_tools_available_without_run() {
        let workspace = tempfile::tempdir().expect("workspace");
        let server = NativeHarnessMcp::new_for_paths(
            workspace.path(),
            codex_arg0::Arg0DispatchPaths::default(),
        )
        .await
        .expect("server");
        let setup = server
            .harness
            .call("setup_workspace", serde_json::json!({"source": "sandbox"}))
            .await
            .expect("setup workspace");
        let setup_json: serde_json::Value =
            serde_json::from_str(setup.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert!(setup_json["project_id"].is_string());

        let result = server
            .harness
            .call("list_directory", serde_json::json!({}))
            .await
            .expect("list directory");
        let parsed: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert!(parsed["entries"].is_array());
        assert!(parsed.get("run_metadata").is_none());
    }

    #[tokio::test]
    async fn selected_run_survives_harness_restart() {
        let workspace = tempfile::tempdir().expect("workspace");
        let first = NativeHarnessMcp::new_for_paths(
            workspace.path(),
            codex_arg0::Arg0DispatchPaths::default(),
        )
        .await
        .expect("first server");
        let setup = first
            .harness
            .call("setup_workspace", serde_json::json!({"source": "sandbox"}))
            .await
            .expect("setup workspace");
        let setup_json: serde_json::Value =
            serde_json::from_str(setup.content[0].as_text().unwrap().text.as_str()).unwrap();
        first
            .harness
            .call(
                "run_start",
                serde_json::json!({
                    "project_id": setup_json["project_id"].as_str().unwrap(),
                    "objective": "persist through restart"
                }),
            )
            .await
            .expect("start run");

        let second = NativeHarnessMcp::new_for_paths(
            workspace.path(),
            codex_arg0::Arg0DispatchPaths::default(),
        )
        .await
        .expect("second server");
        let result = second
            .harness
            .call("run_get", serde_json::json!({}))
            .await
            .expect("get selected run after restart");
        let parsed: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(parsed["run"]["objective"], "persist through restart");
    }

    #[tokio::test]
    async fn prompt_catalog_has_expected_prompts() {
        let workspace = tempfile::tempdir().expect("workspace");
        let server = NativeHarnessMcp::new_for_paths(
            workspace.path(),
            codex_arg0::Arg0DispatchPaths::default(),
        )
        .await
        .expect("server");
        let names: Vec<&str> = server.prompts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "read-file-guide",
                "search-code-guide",
                "apply-patch-guide",
                "run-command-guide",
                "workspace-overview-guide",
                "git-operations-guide",
                "task-completion-guide",
            ]
        );
    }

    #[tokio::test]
    async fn get_prompt_returns_content_for_known_prompts() {
        for name in &[
            "read-file-guide",
            "search-code-guide",
            "apply-patch-guide",
            "run-command-guide",
            "workspace-overview-guide",
            "git-operations-guide",
            "task-completion-guide",
        ] {
            let result = super::get_prompt_content(name);
            assert!(result.is_some(), "prompt {name} should have content");
            let result = result.unwrap();
            assert!(
                !result.messages.is_empty(),
                "prompt {name} should have messages"
            );
        }
    }

    #[tokio::test]
    async fn get_prompt_returns_none_for_unknown() {
        assert!(super::get_prompt_content("nonexistent-prompt").is_none());
    }

    #[tokio::test]
    async fn git_tool_is_sandboxed_and_separate_stderr() {
        let workspace = tempfile::tempdir().expect("workspace");
        let mut arg0_paths = codex_arg0::Arg0DispatchPaths::default();
        let self_exe = std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/debug/chatcodex-mcp-server"
        ));
        arg0_paths.codex_self_exe = Some(self_exe.clone());
        arg0_paths.codex_linux_sandbox_exe = Some(self_exe);
        let server = NativeHarnessMcp::new_for_paths(workspace.path(), arg0_paths)
            .await
            .expect("server");
        server
            .harness
            .call("setup_workspace", serde_json::json!({"source": "sandbox"}))
            .await
            .expect("setup workspace");

        // status runs through the exec-server sandbox and returns structured fields.
        let result = server
            .harness
            .call("git", serde_json::json!({"command": "status --short"}))
            .await
            .expect("git status via sandbox");
        assert_eq!(result.is_error, Some(false), "{result:?}");
        let parsed: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert!(parsed.get("stdout").is_some());
        assert!(parsed.get("stderr").is_some());
        assert!(parsed.get("exit_code").is_some());
    }

    #[tokio::test]
    async fn git_commit_and_branch_update_local_repo() {
        let workspace = tempfile::tempdir().expect("workspace");
        let mut arg0_paths = codex_arg0::Arg0DispatchPaths::default();
        let self_exe = std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/debug/chatcodex-mcp-server"
        ));
        arg0_paths.codex_self_exe = Some(self_exe.clone());
        arg0_paths.codex_linux_sandbox_exe = Some(self_exe);
        let server = NativeHarnessMcp::new_for_paths(workspace.path(), arg0_paths)
            .await
            .expect("server");
        server
            .harness
            .call("setup_workspace", serde_json::json!({"source": "sandbox"}))
            .await
            .expect("setup workspace");

        // Create a file via apply_patch so there is something to commit.
        let patch_result = server
            .harness
            .call(
                "apply_patch",
                serde_json::json!({
                    "input": "*** Begin Patch
*** Add File: hello.txt
+world
*** End Patch
"
                }),
            )
            .await
            .expect("apply patch");
        assert_eq!(patch_result.is_error, Some(false), "{patch_result:?}");

        // Stage and commit through the writable git tools.
        let result = server
            .harness
            .call("git", serde_json::json!({"command": "add hello.txt"}))
            .await
            .expect("git add");
        assert_eq!(result.is_error, Some(false), "{result:?}");
        let parsed: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(parsed["exit_code"], 0, "git add failed: {parsed}");

        let result = server
            .harness
            .call(
                "git_commit",
                serde_json::json!({"message": "initial commit"}),
            )
            .await
            .expect("git commit");
        assert_eq!(result.is_error, Some(false), "{result:?}");
        let parsed: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(parsed["exit_code"], 0, "commit failed: {parsed}");

        // Create and checkout a new branch.
        let result = server
            .harness
            .call("git_branch", serde_json::json!({"name": "feature"}))
            .await
            .expect("git branch");
        assert_eq!(result.is_error, Some(false), "{result:?}");
        let parsed: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(parsed["exit_code"], 0, "branch failed: {parsed}");

        let result = server
            .harness
            .call("git_checkout", serde_json::json!({"target": "feature"}))
            .await
            .expect("git checkout");
        assert_eq!(result.is_error, Some(false), "{result:?}");
        let parsed: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(parsed["exit_code"], 0, "checkout failed: {parsed}");

        // Verify the current branch.
        let result = server
            .harness
            .call(
                "git",
                serde_json::json!({"command": "branch --show-current"}),
            )
            .await
            .expect("git branch --show-current");
        let parsed: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(parsed["stdout"].as_str().unwrap().trim(), "feature");
    }

    #[tokio::test]
    async fn setup_workspace_sandbox_initializes_git() {
        let workspace = tempfile::tempdir().expect("workspace");
        let server = NativeHarnessMcp::new_for_paths(
            workspace.path(),
            codex_arg0::Arg0DispatchPaths::default(),
        )
        .await
        .expect("server");

        let result = server
            .harness
            .call("setup_workspace", serde_json::json!({"source": "sandbox"}))
            .await
            .expect("setup workspace");
        assert_eq!(result.is_error, Some(false), "{result:?}");
        let parsed: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(parsed["action"], "created");
        assert!(
            parsed["workspace_root"]
                .as_str()
                .unwrap()
                .contains("sandboxes")
        );
        assert!(
            std::path::Path::new(parsed["workspace_root"].as_str().unwrap())
                .join(".git")
                .exists(),
            "sandbox should be a git repo"
        );

        // Second setup of the same sandbox is idempotent and reports existing.
        let result = server
            .harness
            .call("setup_workspace", serde_json::json!({"source": "sandbox"}))
            .await
            .expect("setup workspace idempotent");
        let parsed: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(parsed["action"], "existing");
    }

    #[tokio::test]
    async fn tools_require_setup_workspace_first() {
        let workspace = tempfile::tempdir().expect("workspace");
        let server = NativeHarnessMcp::new_for_paths(
            workspace.path(),
            codex_arg0::Arg0DispatchPaths::default(),
        )
        .await
        .expect("server");

        let cases: [(&str, serde_json::Value); 6] = [
            ("read_file", serde_json::json!({"path": "x.txt"})),
            ("apply_patch", serde_json::json!({"input": ""})),
            ("exec_command", serde_json::json!({"cmd": "echo x"})),
            ("git_status", serde_json::json!({})),
            ("git_diff", serde_json::json!({})),
            ("git", serde_json::json!({"command": "status"})),
        ];
        for (tool, args) in cases {
            let result = server.harness.call(tool, args).await;
            assert!(
                result.is_err() || result.as_ref().unwrap().is_error == Some(true),
                "{tool} should fail before setup_workspace: {result:?}"
            );
            let err_text = match &result {
                Ok(r) => r.content[0].as_text().unwrap().text.clone(),
                Err(e) => e.to_string(),
            };
            assert!(
                err_text.contains("Workspace not configured"),
                "{tool} error should mention workspace not configured: {err_text}"
            );
        }
    }

    #[tokio::test]
    async fn git_status_delegates_to_git_tool() {
        let workspace = tempfile::tempdir().expect("workspace");
        let mut arg0_paths = codex_arg0::Arg0DispatchPaths::default();
        let self_exe = std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/debug/chatcodex-mcp-server"
        ));
        arg0_paths.codex_self_exe = Some(self_exe.clone());
        arg0_paths.codex_linux_sandbox_exe = Some(self_exe);
        let server = NativeHarnessMcp::new_for_paths(workspace.path(), arg0_paths)
            .await
            .expect("server");

        server
            .harness
            .call("setup_workspace", serde_json::json!({"source": "sandbox"}))
            .await
            .expect("setup workspace");

        let result = server
            .harness
            .call("git_status", serde_json::json!({}))
            .await
            .expect("git_status");
        assert_eq!(result.is_error, Some(false), "{result:?}");
        let parsed: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert!(parsed["entries"].is_array());
    }

    #[tokio::test]
    async fn git_tool_blocks_push() {
        let workspace = tempfile::tempdir().expect("workspace");
        let server = NativeHarnessMcp::new_for_paths(
            workspace.path(),
            codex_arg0::Arg0DispatchPaths::default(),
        )
        .await
        .expect("server");

        server
            .harness
            .call("setup_workspace", serde_json::json!({"source": "sandbox"}))
            .await
            .expect("setup workspace");

        let blocked = [
            "push origin main",
            "fetch origin",
            "pull origin main",
            "clone https://example.com/repo.git",
            "ls-remote origin",
            "remote add origin https://example.com/repo.git",
            "remote set-url origin https://example.com/repo.git",
            "submodule update --init",
        ];
        for command in blocked {
            let result = server
                .harness
                .call("git", serde_json::json!({"command": command}))
                .await;
            assert!(
                result.is_err() || result.as_ref().unwrap().is_error == Some(true),
                "git {command} should be blocked: {result:?}"
            );
            let err_text = match &result {
                Ok(r) => r.content[0].as_text().unwrap().text.clone(),
                Err(e) => e.to_string(),
            };
            assert!(
                err_text.contains("blocked by policy"),
                "git {command} error should mention policy: {err_text}"
            );
        }
    }

    #[test]
    fn allowed_git_schemes() {
        for scheme in ["https", "http", "ssh", "git+https", "git+ssh"] {
            let source = format!("{scheme}://example.com/repo.git");
            let stripped = source.strip_prefix("git+").unwrap_or(&source);
            let url = stripped.parse::<url::Url>().unwrap();
            assert!(
                super::is_allowed_git_scheme(url.scheme()),
                "{scheme} should be allowed"
            );
        }
    }

    #[test]
    fn disallowed_git_schemes() {
        for scheme in ["file", "ftp", "s3", "data"] {
            let source = format!("{scheme}://example.com/repo.git");
            let stripped = source.strip_prefix("git+").unwrap_or(&source);
            let url = stripped.parse::<url::Url>().unwrap();
            assert!(
                !super::is_allowed_git_scheme(url.scheme()),
                "{scheme} should be rejected"
            );
        }
    }

    #[test]
    fn workspace_name_sanitization() {
        assert_eq!(super::sanitize_workspace_name("repo.git").unwrap(), "repo");
        assert_eq!(
            super::sanitize_workspace_name("my repo").unwrap(),
            "my-repo"
        );
        assert_eq!(
            super::sanitize_workspace_name("a/../b")
                .unwrap_err()
                .to_string()
                .contains(".."),
            true
        );
        assert!(super::sanitize_workspace_name("").is_err());
    }

    #[test]
    fn derive_workspace_name_from_url() {
        assert_eq!(
            super::derive_workspace_name("https://github.com/owner/repo.git").unwrap(),
            "repo"
        );
        assert_eq!(
            super::derive_workspace_name("git+ssh://git@github.com/owner/repo.git").unwrap(),
            "repo"
        );
        assert_eq!(super::derive_workspace_name("sandbox").unwrap(), "sandbox");
    }

    #[test]
    fn outbound_git_command_detection() {
        assert!(super::is_outbound_git_command("push origin main"));
        assert!(super::is_outbound_git_command("fetch"));
        assert!(super::is_outbound_git_command("pull origin main"));
        assert!(super::is_outbound_git_command(
            "clone https://example.com/repo"
        ));
        assert!(super::is_outbound_git_command("ls-remote origin"));
        assert!(super::is_outbound_git_command(
            "remote add origin https://example.com"
        ));
        assert!(super::is_outbound_git_command(
            "remote set-url origin https://example.com"
        ));
        assert!(super::is_outbound_git_command("submodule update --init"));

        assert!(!super::is_outbound_git_command("status"));
        assert!(!super::is_outbound_git_command("log --oneline -5"));
        assert!(!super::is_outbound_git_command("diff --staged"));
        assert!(!super::is_outbound_git_command("remote -v"));
    }

    #[test]
    fn clone_argv_building() {
        let target = std::path::Path::new("/tmp/workspaces/repo");
        // Plain public clone.
        assert_eq!(
            super::build_clone_argv(
                "https://github.com/owner/repo.git",
                None,
                false,
                target
            ),
            vec![
                "clone".to_string(),
                "--".to_string(),
                "https://github.com/owner/repo.git".to_string(),
                "/tmp/workspaces/repo".to_string(),
            ]
        );
        // Clone at a specific ref.
        let with_ref = super::build_clone_argv(
            "https://github.com/owner/repo.git",
            Some("feature/xyz"),
            false,
            target,
        );
        assert_eq!(with_ref[1], "--branch");
        assert_eq!(with_ref[2], "feature/xyz");
        // Authenticated clone: -c credential.helper must precede `clone`.
        let authed =
            super::build_clone_argv("https://github.com/owner/repo.git", None, true, target);
        assert_eq!(authed[0], "-c");
        assert!(authed[1].starts_with("credential.helper="));
        assert_eq!(authed[2], "clone");
        // The helper value must reference the env var, never an inline secret.
        assert!(authed[1].contains("CHATCODEX_GITHUB_TOKEN"));
    }

    #[test]
    fn credential_helper_references_env_only() {
        let helper = super::github_credential_helper();
        assert!(helper.contains("$CHATCODEX_GITHUB_TOKEN"));
        // Must not contain any plausible secret value.
        for leak in ["ghp_", "github_pat_", "password=ghp"] {
            assert!(!helper.contains(leak), "helper leaked {leak}: {helper}");
        }
    }

    #[test]
    fn github_host_detection() {
        assert!(super::is_github_host(Some("github.com")));
        assert!(super::is_github_host(Some("ghe.example.github.com")));
        assert!(!super::is_github_host(Some("gitlab.com")));
        assert!(!super::is_github_host(Some("notgithub.com")));
        assert!(!super::is_github_host(None));
    }

    #[test]
    fn clean_git_url_strips_credentials() {
        let url = url::Url::parse("https://user:secret@github.com/owner/repo.git").unwrap();
        let cleaned = super::clean_git_url(&url);
        assert_eq!(cleaned, "https://github.com/owner/repo.git");
        assert!(!cleaned.contains("secret"));

        let url = url::Url::parse("https://github.com/owner/repo.git").unwrap();
        assert_eq!(
            super::clean_git_url(&url),
            "https://github.com/owner/repo.git"
        );
    }

    #[test]
    fn embedded_credentials_are_detectable() {
        // Mirrors the guard in do_setup_workspace: URLs carrying userinfo
        // must be rejected so git never persists credentials to .git/config.
        let bad = url::Url::parse("https://token@github.com/owner/repo.git").unwrap();
        assert!(!bad.username().is_empty() || bad.password().is_some());
        let bad2 = url::Url::parse("https://user:pass@github.com/owner/repo.git").unwrap();
        assert!(!bad2.username().is_empty() || bad2.password().is_some());
        let good = url::Url::parse("https://github.com/owner/repo.git").unwrap();
        assert!(good.username().is_empty() && good.password().is_none());
    }

    #[test]
    fn git_write_classifier_gates_mutating_local_commands() {
        for command in [
            "add src/lib.rs",
            "branch feature",
            "checkout -b feature",
            "commit -m hi",
            "config user.name ChatCodex",
            "merge main",
            "rebase main",
            "reset --hard HEAD",
            "stash push",
            "tag v1",
        ] {
            assert!(
                super::git_command_requires_commit_permission(command),
                "{command}"
            );
        }

        for command in [
            "status",
            "diff",
            "log --oneline",
            "show HEAD",
            "rev-parse HEAD",
        ] {
            assert!(
                !super::git_command_requires_commit_permission(command),
                "{command}"
            );
        }
    }

    #[tokio::test]
    async fn workspace_path_construction() {
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
                "setup_workspace",
                serde_json::json!({
                    "source": "sandbox",
                    "name": "my-sandbox"
                }),
            )
            .await
            .expect("setup workspace");
        let parsed: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        let root = parsed["workspace_root"].as_str().unwrap();
        assert!(root.contains("clients/default/sandboxes/my-sandbox"));
    }
}
