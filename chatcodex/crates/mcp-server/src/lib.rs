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
use rmcp::model::GetPromptRequestParams;
use rmcp::model::GetPromptResult;
use rmcp::model::JsonObject;
use rmcp::model::ListPromptsResult;
use rmcp::model::AnnotateAble;
use rmcp::model::ListResourcesResult;
use rmcp::model::ListResourceTemplatesResult;
use rmcp::model::ListToolsResult;
use rmcp::model::PaginatedRequestParams;
use rmcp::model::RawResource;
use rmcp::model::ReadResourceRequestParams;
use rmcp::model::ReadResourceResult;
use rmcp::model::ResourceContents;
use rmcp::model::Prompt;
use rmcp::model::PromptMessage;
use rmcp::model::PromptMessageRole;
use rmcp::model::ServerCapabilities;
use rmcp::model::ServerInfo;
use rmcp::model::Tool;
use rmcp::model::ToolAnnotations;
use rmcp::transport::StreamableHttpServerConfig;
use rmcp::transport::StreamableHttpService;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json::json;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info_span;
use uuid::Uuid;

const SERVER_INSTRUCTIONS: &str = "\
# ChatCodex — Deterministic Coding Harness

You are connected to a deterministic coding harness. Your goal is to complete the \
user's task end-to-end. Plan your work, execute each step, verify results, and do not \
stop until every item on your TODO checklist is checked off or explicitly dismissed.

## Golden rules
1. Commands execute in a read-only filesystem sandbox — use `apply_patch` for ALL writes.
2. Inspect manifests and understand the project before modifying anything.
3. Prefer `mise exec -- <command>` for toolchains and `uv` for Python.
4. System package installation and privilege escalation are FORBIDDEN.
5. After each step, check your TODO list. If items remain, continue working.
6. Stop only when ALL items are checked off or dismissed — or when you need user input.

## Tool usage
- `exec_command` / `write_stdin` — run commands, start interactive sessions
- `apply_patch` — the ONLY workspace write path
- `read_file` / `search_code` / `list_directory` — inspect the workspace
- `git_status` / `git_diff` — inspect git state
- `update_plan` / `todo` — track progress
- `view_image` — view images in the workspace
- `update_plan` — track the current plan step
- `todo` — manage a persistent checklist

## Completion protocol
1. **Plan**: Break the task into concrete TODO items using the `todo` tool.
2. **Execute**: Work through items one at a time. After each, check it off with `todo(items: [{id: \"...\", status: \"checked\"}], action: \"update\")`.
3. **Verify**: After all items are done, verify the result works.
4. **Finish**: Only stop once your TODO list is empty (all checked or dismissed).";

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
    current_workspace: RwLock<Option<AbsolutePathBuf>>,
    environment: Environment,
    processes: Mutex<HashMap<ProcessId, ProcessSession>>,
    plan: Mutex<Value>,
    todo: Mutex<Vec<TodoItem>>,
    linux_sandbox_exe: Option<PathBuf>,
    client_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TodoItem {
    id: String,
    description: String,
    status: TodoStatus,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum TodoStatus {
    Pending,
    Checked,
    Dismissed,
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
                Environment::create(std::env::var("CODEX_EXEC_SERVER_URL").ok(), runtime_paths)?
            }
            None => {
                let linux_sandbox_exe = arg0_paths.codex_linux_sandbox_exe.clone();
                let self_exe = linux_sandbox_exe
                    .clone()
                    .or_else(|| std::env::current_exe().ok())
                    .ok_or_else(|| anyhow::anyhow!("could not determine self executable"))?;
                let runtime_paths = ExecServerRuntimePaths::new(self_exe, linux_sandbox_exe)?;
                Environment::create(std::env::var("CODEX_EXEC_SERVER_URL").ok(), runtime_paths)?
            }
        };
        let client_id =
            std::env::var("CHATCODEX_CLIENT_ID").unwrap_or_else(|_| "default".to_string());
        Ok(Self {
            tools: Arc::new(tool_catalog()?),
            prompts: Arc::new(prompt_catalog()),
            harness: Arc::new(NativeHarness {
                workspace_base,
                current_workspace: RwLock::new(None),
                environment,
                processes: Mutex::new(HashMap::new()),
                plan: Mutex::new(json!({"plan": []})),
                todo: Mutex::new(Vec::new()),
                linux_sandbox_exe: arg0_paths.codex_linux_sandbox_exe,
                client_id,
            }),
        })
    }

    pub fn tools(&self) -> &[Tool] {
        self.tools.as_slice()
    }
}

impl NativeHarness {
    async fn workspace_or_error(&self) -> anyhow::Result<AbsolutePathBuf> {
        let workspace = self.current_workspace.read().await;
        workspace
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Workspace not configured. Call setup_workspace(source: '<git-url>') or setup_workspace(source: 'sandbox') first."))
    }

    async fn call(&self, name: &str, arguments: Value) -> anyhow::Result<CallToolResult> {
        match name {
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
            "git_diff" => self.git_diff(serde_json::from_value(arguments)?).await,
            "git_commit" => self.git_commit(serde_json::from_value(arguments)?).await,
            "git_branch" => self.git_branch(serde_json::from_value(arguments)?).await,
            "git_checkout" => self.git_checkout(serde_json::from_value(arguments)?).await,
            "list_directory" => {
                self.list_directory(serde_json::from_value(arguments)?)
                    .await
            }
            "todo" => self.todo(serde_json::from_value(arguments)?).await,
            _ => anyhow::bail!("unknown deterministic tool: {name}"),
        }
    }

    async fn exec_command(&self, args: ExecCommandArgs) -> anyhow::Result<CallToolResult> {
        let workspace = self.workspace_or_error().await?;
        let shell_argv = vec!["bash".to_string(), "-lc".to_string(), args.cmd];
        if command_might_be_dangerous(&shell_argv) {
            anyhow::bail!("command rejected by the deterministic command policy");
        }

        let policy = SandboxPolicy::ReadOnly {
            network_access: true,
        };
        let permissions =
            PermissionProfile::from_legacy_sandbox_policy_for_cwd(&policy, workspace.as_path());
        let (file_system_sandbox_policy, _) = permissions.to_runtime_permissions();
        let sandbox = self.resolve_sandbox_type();
        let mut cmd_env = HashMap::new();
        if sandbox != SandboxType::None {
            if let Some(path) = std::env::var_os("PATH") {
                cmd_env.insert("PATH".to_string(), path.to_string_lossy().into_owned());
            }
            if let Some(home) = std::env::var_os("HOME") {
                cmd_env.insert("HOME".to_string(), home.to_string_lossy().into_owned());
            }
        }
        let transformed = SandboxManager::new().transform(SandboxTransformRequest {
            command: SandboxCommand {
                program: "bash".into(),
                args: shell_argv[1..].to_vec(),
                cwd: workspace.clone(),
                env: cmd_env,
                additional_permissions: None,
            },
            permissions: &permissions,
            sandbox,
            enforce_managed_network: false,
            network: None,
            sandbox_policy_cwd: workspace.as_path(),
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
        let workspace = self.workspace_or_error().await?;
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: Vec::new(),
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        };
        let sandbox =
            FileSystemSandboxContext::from_legacy_sandbox_policy(policy, workspace.clone());
        let sandbox = if self.environment.is_remote()
            || self
                .environment
                .local_runtime_paths()
                .is_some_and(|p| p.codex_linux_sandbox_exe.is_some())
        {
            Some(&sandbox)
        } else {
            None
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = codex_apply_patch::apply_patch(
            &args.input,
            &workspace,
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
            FileSystemSandboxContext::from_legacy_sandbox_policy(policy, workspace.clone());
        let sandbox = if self.environment.is_remote()
            || self
                .environment
                .local_runtime_paths()
                .is_some_and(|p| p.codex_linux_sandbox_exe.is_some())
        {
            Some(&sandbox)
        } else {
            None
        };
        let fs = self.environment.get_filesystem();
        let path = resolve_workspace_path(fs.as_ref(), &workspace, &args.path, sandbox).await?;
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

    async fn read_file(&self, args: ReadFileArgs) -> anyhow::Result<CallToolResult> {
        let workspace = self.workspace_or_error().await?;
        let policy = SandboxPolicy::ReadOnly {
            network_access: false,
        };
        let sandbox =
            FileSystemSandboxContext::from_legacy_sandbox_policy(policy, workspace.clone());
        let sandbox = if self.environment.is_remote()
            || self
                .environment
                .local_runtime_paths()
                .is_some_and(|p| p.codex_linux_sandbox_exe.is_some())
        {
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
            "path": path.as_path(),
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
        let mut cmd = std::process::Command::new("grep");
        cmd.arg("-rn");
        if let Some(glob) = &args.path_glob {
            cmd.arg("--include");
            cmd.arg(glob);
        }
        cmd.arg("--");
        cmd.arg(&args.query);
        cmd.arg(".");
        cmd.current_dir(workspace.as_path());
        let output = cmd.output().context("failed to run grep")?;
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
            FileSystemSandboxContext::from_legacy_sandbox_policy(policy, workspace.clone());
        let sandbox = if self.environment.is_remote()
            || self
                .environment
                .local_runtime_paths()
                .is_some_and(|p| p.codex_linux_sandbox_exe.is_some())
        {
            Some(&sandbox)
        } else {
            None
        };
        let fs = self.environment.get_filesystem();
        let path = match &args.path {
            Some(p) => resolve_workspace_path(fs.as_ref(), &workspace, p, sandbox).await?,
            None => workspace.clone(),
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
            "path": path.as_path(),
            "entries": listing,
        })))
    }

    async fn todo(&self, args: TodoArgs) -> anyhow::Result<CallToolResult> {
        let mut todo = self.todo.lock().await;
        match args.action {
            TodoAction::Replace => {
                let mut items = Vec::new();
                for (i, input) in args.items.into_iter().enumerate() {
                    items.push(TodoItem {
                        id: input.id.unwrap_or_else(|| format!("t{}", i + 1)),
                        description: input.description.unwrap_or_default(),
                        status: match input.status.as_deref() {
                            Some("checked") => TodoStatus::Checked,
                            Some("dismissed") => TodoStatus::Dismissed,
                            _ => TodoStatus::Pending,
                        },
                    });
                }
                *todo = items;
            }
            TodoAction::Update => {
                for input in args.items {
                    let id = input.id.unwrap_or_default();
                    if let Some(existing) = todo.iter_mut().find(|item| item.id == id) {
                        if let Some(desc) = input.description {
                            existing.description = desc;
                        }
                        existing.status = match input.status.as_deref() {
                            Some("checked") => TodoStatus::Checked,
                            Some("dismissed") => TodoStatus::Dismissed,
                            Some("pending") => TodoStatus::Pending,
                            _ => continue,
                        };
                    }
                }
            }
        }
        let response: Vec<Value> = todo
            .iter()
            .map(|item| {
                let status_str = match item.status {
                    TodoStatus::Pending => "pending",
                    TodoStatus::Checked => "checked",
                    TodoStatus::Dismissed => "dismissed",
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
            .filter(|i| i.status == TodoStatus::Pending)
            .count();
        let checked_count = todo
            .iter()
            .filter(|i| i.status == TodoStatus::Checked)
            .count();
        let dismissed_count = todo
            .iter()
            .filter(|i| i.status == TodoStatus::Dismissed)
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
            &self.client_id,
            &args.source,
            args.name.as_deref(),
            args.timeout_ms,
        )
        .await?;
        let mut guard = self.current_workspace.write().await;
        *guard = Some(AbsolutePathBuf::from_absolute_path(
            &workspace_result.workspace_root,
        )?);
        Ok(text_result(json!({
            "workspace_root": workspace_result.workspace_root,
            "source": workspace_result.source,
            "action": workspace_result.action,
        })))
    }

    async fn git_tool(&self, args: GitToolArgs) -> anyhow::Result<CallToolResult> {
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

    /// Run a git command through the exec-server sandbox.
    ///
    /// ReadOnly mode applies the same filesystem policy as .
    /// WorkspaceWrite mode applies the same policy as  so that
    /// local git metadata writes (commit, branch, checkout) are allowed.
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
        let transformed = SandboxManager::new().transform(SandboxTransformRequest {
            command: SandboxCommand {
                program: "git".into(),
                args: argv,
                cwd: workspace.clone(),
                env: cmd_env,
                additional_permissions: None,
            },
            permissions: &permissions,
            sandbox,
            enforce_managed_network: false,
            network: None,
            sandbox_policy_cwd: workspace.as_path(),
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
                pipe_stdin: false,
                arg0: transformed.arg0,
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
        if (response.exited && response.chunks.is_empty())
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
                if !codex_sandboxing::system_bwrap_has_user_namespace_access(
                    &bwrap_path,
                    timeout,
                ) {
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
            vec![
                "clone".to_string(),
                source.to_string(),
                target.to_string_lossy().to_string(),
            ],
            Some(timeout.as_millis() as u64),
        )
        .await;

        match result {
            Ok(output) if output.exit_code == 0 => {
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
        format!("'{}'", s.replace('\'', "'\''"))
    } else {
        s.to_string()
    }
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
            "Prepare the workspace for this client. Provide a git URL to clone a repository, or pass 'sandbox' for a persistent scratch directory with an initialized git repo.",
            json!({"type":"object","properties":{"source":{"type":"string"},"name":{"type":"string"},"timeout_ms":{"type":"integer","minimum":1000,"maximum":120000}},"required":["source"],"additionalProperties":false}),
            json!({"type":"object","properties":{"workspace_root":{"type":"string"},"source":{"type":"string"},"action":{"type":"string"}},"required":["workspace_root","source","action"],"additionalProperties":false}),
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
            "list_directory",
            "List entries in a workspace directory.",
            json!({"type":"object","properties":{"path":{"type":"string"}},"additionalProperties":false}),
            json!({"type":"object","properties":{"path":{"type":"string"},"entries":{"type":"array","items":{"type":"object","properties":{"name":{"type":"string"},"is_directory":{"type":"boolean"},"is_file":{"type":"boolean"}},"required":["name","is_directory","is_file"],"additionalProperties":false}}},"required":["path","entries"],"additionalProperties":false}),
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
        Ok(Tool::new(
            Cow::Borrowed(name),
            Cow::Borrowed(description),
            Arc::new(serde_json::from_value::<JsonObject>(schema)?),
        )
        .with_raw_output_schema(Arc::new(serde_json::from_value::<JsonObject>(output_schema)?))
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
        _ => anyhow::bail!("missing annotation policy for {name}"),
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
                    RawResource::new(
                        format!("tool:///{}", tool.name),
                        tool.name.to_string(),
                    )
                    .with_description(tool.description.clone().unwrap_or_default())
                    .no_annotation()
                })
                .collect();
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
    ) -> impl std::future::Future<Output = Result<ListResourceTemplatesResult, McpError>> + Send + '_ {
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
                "git_diff",
                "git_commit",
                "git_branch",
                "git_checkout",
                "list_directory",
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
            Some(vec!["path".to_string()])
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
