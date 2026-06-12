//! Model-free access to the deterministic Codex tool harness.

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use crate::codex::Session;
use crate::codex::TurnContext;
use crate::config::ConfigBuilder;
use crate::config::ConfigOverrides;
use crate::protocol::AskForApproval;
use crate::tools::ToolRouter;
use crate::tools::context::SharedTurnDiffTracker;
use crate::tools::context::ToolPayload;
use crate::tools::router::ToolCall;
use crate::tools::router::ToolRouterParams;
use crate::tools::spec::ToolsConfig;
use crate::tools::spec::build_specs_with_discoverable_tools;
use crate::turn_diff_tracker::TurnDiffTracker;
use codex_protocol::config_types::SandboxMode;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::ResponseInputItem;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;

/// A native Codex tool definition ready to map onto an MCP tool.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HarnessToolSpec {
    pub name: String,
    pub definition: Value,
    pub supports_parallel_tool_calls: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HarnessToolResult {
    pub content: Vec<Value>,
    pub structured_content: Option<Value>,
    pub is_error: Option<bool>,
}

struct NativeHarnessInner {
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    router: ToolRouter,
    tracker: SharedTurnDiffTracker,
    next_call_id: AtomicU64,
}

#[derive(Clone)]
pub struct NativeHarness {
    inner: Arc<NativeHarnessInner>,
}

impl NativeHarness {
    pub async fn new(workspace: impl AsRef<Path>) -> anyhow::Result<Self> {
        let data_dir = std::env::var_os("CHATCODEX_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/data"));
        Self::new_with_data_dir(workspace, data_dir).await
    }

    pub async fn new_with_data_dir(
        workspace: impl AsRef<Path>,
        data_dir: impl AsRef<Path>,
    ) -> anyhow::Result<Self> {
        Self::new_with_runtime_paths(workspace, data_dir, None, None).await
    }

    pub async fn new_with_runtime_paths(
        workspace: impl AsRef<Path>,
        data_dir: impl AsRef<Path>,
        codex_linux_sandbox_exe: Option<PathBuf>,
        main_execve_wrapper_exe: Option<PathBuf>,
    ) -> anyhow::Result<Self> {
        Self::new_with_sandbox(
            workspace,
            data_dir,
            SandboxMode::WorkspaceWrite,
            codex_linux_sandbox_exe,
            main_execve_wrapper_exe,
        )
        .await
    }

    async fn new_with_sandbox(
        workspace: impl AsRef<Path>,
        data_dir: impl AsRef<Path>,
        sandbox_mode: SandboxMode,
        codex_linux_sandbox_exe: Option<PathBuf>,
        main_execve_wrapper_exe: Option<PathBuf>,
    ) -> anyhow::Result<Self> {
        let workspace = std::fs::canonicalize(workspace.as_ref())?;
        let data_dir = data_dir.as_ref().join("codex");
        std::fs::create_dir_all(&data_dir)?;
        let config = ConfigBuilder::default()
            .codex_home(data_dir)
            .cli_overrides(vec![
                (
                    "features.use_legacy_landlock".to_string(),
                    toml::Value::Boolean(true),
                ),
                (
                    "sandbox_workspace_write.network_access".to_string(),
                    toml::Value::Boolean(true),
                ),
            ])
            .harness_overrides(ConfigOverrides {
                cwd: Some(workspace),
                approval_policy: Some(AskForApproval::Never),
                sandbox_mode: Some(sandbox_mode),
                codex_linux_sandbox_exe,
                main_execve_wrapper_exe,
                ephemeral: Some(false),
                ..ConfigOverrides::default()
            })
            .build()
            .await?;
        let (session, turn, events) = Session::new_native_harness(Arc::new(config)).await?;
        tokio::spawn(async move { while events.recv().await.is_ok() {} });
        let router = ToolRouter::from_config(
            &ToolsConfig::native_harness(),
            ToolRouterParams {
                mcp_tools: None,
                app_tools: None,
                discoverable_tools: None,
                dynamic_tools: &[],
            },
        );

        Ok(Self {
            inner: Arc::new(NativeHarnessInner {
                session,
                turn,
                router,
                tracker: Arc::new(tokio::sync::Mutex::new(TurnDiffTracker::new())),
                next_call_id: AtomicU64::new(1),
            }),
        })
    }

    pub async fn call(
        &self,
        tool_name: &str,
        arguments: Value,
    ) -> anyhow::Result<HarnessToolResult> {
        anyhow::ensure!(
            arguments.is_object(),
            "tool arguments for {tool_name} must be a JSON object"
        );
        let call_id = format!(
            "mcp-{}",
            self.inner.next_call_id.fetch_add(1, Ordering::Relaxed)
        );
        let (response, structured_content) = self
            .inner
            .router
            .dispatch_native_harness_call(
                Arc::clone(&self.inner.session),
                Arc::clone(&self.inner.turn),
                Arc::clone(&self.inner.tracker),
                ToolCall {
                    tool_name: tool_name.to_string(),
                    tool_namespace: None,
                    call_id,
                    payload: ToolPayload::Function {
                        arguments: serde_json::to_string(&arguments)?,
                    },
                },
            )
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;

        Ok(harness_result(response, structured_content))
    }
}

fn harness_result(response: ResponseInputItem, structured_content: Value) -> HarnessToolResult {
    match response {
        ResponseInputItem::FunctionCallOutput { output, .. }
        | ResponseInputItem::CustomToolCallOutput { output, .. } => HarnessToolResult {
            content: output_content(output.body),
            structured_content: structured_content.is_object().then_some(structured_content),
            is_error: output.success.map(|success| !success),
        },
        other => HarnessToolResult {
            content: vec![json!({
                "type": "text",
                "text": serde_json::to_string(&other).unwrap_or_else(|error| error.to_string())
            })],
            structured_content: structured_content.is_object().then_some(structured_content),
            is_error: Some(false),
        },
    }
}

fn output_content(body: FunctionCallOutputBody) -> Vec<Value> {
    match body {
        FunctionCallOutputBody::Text(text) => vec![json!({ "type": "text", "text": text })],
        FunctionCallOutputBody::ContentItems(items) => items
            .into_iter()
            .map(|item| match item {
                FunctionCallOutputContentItem::InputText { text } => {
                    json!({ "type": "text", "text": text })
                }
                FunctionCallOutputContentItem::InputImage { image_url, .. } => {
                    json!({ "type": "text", "text": image_url })
                }
            })
            .collect(),
    }
}

/// Build the deterministic model-facing catalog without constructing a model
/// client or starting a Codex turn.
pub fn native_tool_catalog() -> Result<Vec<HarnessToolSpec>, serde_json::Error> {
    let (specs, _) =
        build_specs_with_discoverable_tools(&ToolsConfig::native_harness(), None, None, None, &[])
            .build();

    specs
        .into_iter()
        .map(|configured| {
            let name = configured.spec.name().to_string();
            let definition = serde_json::to_value(configured.spec)?;
            Ok(HarnessToolSpec {
                name,
                definition,
                supports_parallel_tool_calls: configured.supports_parallel_tool_calls,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::NativeHarness;
    use super::native_tool_catalog;
    use codex_protocol::config_types::SandboxMode;
    use crate::features::Feature;
    use serde_json::Value;
    use serde_json::json;

    #[test]
    fn native_catalog_uses_codex_tool_specs_without_agent_tools() {
        let catalog = native_tool_catalog().expect("native tool catalog");
        let names = catalog
            .iter()
            .map(|tool| tool.name.as_str())
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

        let apply_patch = catalog
            .iter()
            .find(|tool| tool.name == "apply_patch")
            .expect("apply_patch tool");
        assert_eq!(
            apply_patch.definition["type"],
            Value::String("function".into())
        );
        assert_eq!(
            apply_patch.definition["parameters"]["required"],
            serde_json::json!(["input"])
        );

        for forbidden in [
            "codex",
            "codex-reply",
            "spawn_agent",
            "send_input",
            "resume_agent",
            "wait",
            "close_agent",
        ] {
            assert!(
                !names.contains(&forbidden),
                "{forbidden} must not be exposed"
            );
        }
    }

    #[tokio::test]
    async fn native_harness_dispatches_exec_command_through_codex_handler() {
        let workspace = tempfile::tempdir().expect("workspace");
        let data = tempfile::tempdir().expect("data");
        let harness = NativeHarness::new_with_sandbox(
            workspace.path(),
            data.path(),
            SandboxMode::DangerFullAccess,
            None,
            None,
        )
        .await
        .expect("native harness");

        let result = harness
            .call(
                "exec_command",
                json!({
                    "cmd": "printf native-dispatch",
                    "yield_time_ms": 1000
                }),
            )
            .await
            .expect("exec_command result");

        assert_eq!(result.is_error, Some(false), "{result:?}");
        assert!(
            result
                .structured_content
                .as_ref()
                .and_then(|value| value["output"].as_str())
                .is_some_and(|output| output.contains("native-dispatch"))
        );
    }

    #[tokio::test]
    async fn native_harness_uses_legacy_landlock_for_container_compatibility() {
        let workspace = tempfile::tempdir().expect("workspace");
        let data = tempfile::tempdir().expect("data");
        let harness =
            NativeHarness::new_with_runtime_paths(workspace.path(), data.path(), None, None)
                .await
                .expect("native harness");

        assert!(
            harness
                .inner
                .turn
                .features
                .enabled(Feature::UseLegacyLandlock)
        );
    }

    #[tokio::test]
    async fn native_harness_preserves_exec_sessions_for_write_stdin() {
        let workspace = tempfile::tempdir().expect("workspace");
        let data = tempfile::tempdir().expect("data");
        let harness = NativeHarness::new_with_sandbox(
            workspace.path(),
            data.path(),
            SandboxMode::DangerFullAccess,
            None,
            None,
        )
        .await
        .expect("native harness");

        let started = harness
            .call(
                "exec_command",
                json!({
                    "cmd": "sleep 1; printf session-complete",
                    "yield_time_ms": 250
                }),
            )
            .await
            .expect("exec_command result");
        let session_id = started
            .structured_content
            .as_ref()
            .and_then(|value| value["session_id"].as_i64())
            .expect("running session id");

        let completed = harness
            .call(
                "write_stdin",
                json!({
                    "session_id": session_id,
                    "chars": "",
                    "yield_time_ms": 2000
                }),
            )
            .await
            .expect("write_stdin result");

        assert_eq!(completed.is_error, Some(false), "{completed:?}");
        assert!(
            completed
                .structured_content
                .as_ref()
                .and_then(|value| value["output"].as_str())
                .is_some_and(|output| output.contains("session-complete")),
            "{completed:?}"
        );
    }

    #[tokio::test]
    async fn native_harness_dispatches_function_apply_patch_to_codex_executable() {
        let workspace = tempfile::tempdir().expect("workspace");
        let data = tempfile::tempdir().expect("data");
        let harness = NativeHarness::new_with_sandbox(
            workspace.path(),
            data.path(),
            SandboxMode::DangerFullAccess,
            None,
            None,
        )
        .await
        .expect("native harness");

        let result = harness
            .call(
                "apply_patch",
                json!({
                    "input": "*** Begin Patch\n*** Add File: hello.txt\n+hello from native patch\n*** End Patch\n"
                }),
            )
            .await
            .expect("apply_patch result");

        assert_eq!(result.is_error, Some(true));
        assert!(
            result.content[0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("codex-run-as-apply-patch")),
            "{result:?}"
        );
    }
}
