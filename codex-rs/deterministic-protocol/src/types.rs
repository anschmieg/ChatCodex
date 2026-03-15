//! Shared request / response DTOs for the deterministic daemon.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// JSON-RPC envelope
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Response envelope (inside the JSON-RPC result field)
//
// Every successful daemon response wraps the handler result in this
// envelope so the MCP gateway has a consistent shape to rely on.
// See docs/INTERNAL_RPC.md for the canonical specification.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseEnvelope {
    pub ok: bool,
    pub result: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_state: Option<RunState>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub audit_id: String,
}

// ---------------------------------------------------------------------------
// run.prepare
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPrepareParams {
    pub workspace_id: String,
    pub user_goal: String,
    #[serde(default)]
    pub focus_paths: Vec<String>,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPrepareResult {
    pub run_id: String,
    pub objective: String,
    pub assistant_brief: String,
    pub constraints: Vec<String>,
    pub status: String,
    pub plan: Vec<String>,
    pub current_step: usize,
    pub recommended_next_action: String,
    pub recommended_tool: String,
}

// ---------------------------------------------------------------------------
// workspace.summary
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSummaryParams {
    pub workspace_id: String,
    #[serde(default)]
    pub focus_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSummaryResult {
    pub root: String,
    pub detected_languages: Vec<String>,
    pub dirty_files: Vec<String>,
    pub relevant_paths: Vec<String>,
}

// ---------------------------------------------------------------------------
// file.read
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileReadParams {
    pub run_id: String,
    pub path: String,
    #[serde(default)]
    pub start_line: Option<u64>,
    #[serde(default)]
    pub end_line: Option<u64>,
    #[serde(default)]
    pub purpose: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileReadResult {
    pub path: String,
    pub content: String,
    pub start_line: u64,
    pub end_line: u64,
    pub total_lines: u64,
}

// ---------------------------------------------------------------------------
// git.status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusParams {
    pub run_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusResult {
    pub branch: String,
    pub dirty_files: Vec<String>,
    pub untracked_files: Vec<String>,
}

// ---------------------------------------------------------------------------
// code.search
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeSearchParams {
    pub run_id: String,
    pub query: String,
    #[serde(default)]
    pub path_glob: Option<String>,
    #[serde(default)]
    pub max_results: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeSearchMatch {
    pub path: String,
    pub line: u64,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeSearchResult {
    pub matches: Vec<CodeSearchMatch>,
}

// ---------------------------------------------------------------------------
// patch.apply
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchEdit {
    pub path: String,
    pub operation: String,
    #[serde(default)]
    pub start_line: Option<u64>,
    #[serde(default)]
    pub end_line: Option<u64>,
    #[serde(default)]
    pub old_text: Option<String>,
    pub new_text: String,
    #[serde(default)]
    pub anchor_text: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchApplyParams {
    pub run_id: String,
    pub edits: Vec<PatchEdit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchApplyResult {
    pub changed_files: Vec<String>,
    pub diff_stats: String,
    /// When set, the patch was NOT applied — an approval is required first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_required: Option<PendingApproval>,
}

// ---------------------------------------------------------------------------
// tests.run
//
// `scope` is a semantic string — not limited to specific framework
// names.  The daemon resolves the scope to a concrete command
// deterministically (e.g. by inspecting workspace tooling).  Well-known
// values include "unit", "integration", "all", "cargo", "npm", etc.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestsRunParams {
    pub run_id: String,
    pub scope: String,
    #[serde(default)]
    pub target: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestsRunResult {
    pub resolved_command: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub summary: String,
    /// When set, the test was NOT run — an approval is required first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_required: Option<PendingApproval>,
}

// ---------------------------------------------------------------------------
// git.diff
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffParams {
    pub run_id: String,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffResult {
    pub changed_files: Vec<String>,
    pub diff_summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch_text: Option<String>,
}

// ---------------------------------------------------------------------------
// run.refresh
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRefreshParams {
    pub run_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRefreshResult {
    pub run_id: String,
    pub status: String,
    pub current_step: usize,
    pub completed_steps: Vec<String>,
    pub pending_steps: Vec<String>,
    pub last_action: Option<String>,
    pub last_observation: Option<String>,
    pub recommended_next_action: Option<String>,
    pub recommended_tool: Option<String>,
    pub pending_approvals: Vec<PendingApproval>,
    pub latest_diff_summary: Option<String>,
    pub latest_test_result: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// run.replan
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReplanParams {
    pub run_id: String,
    pub reason: String,
    #[serde(default)]
    pub new_evidence: Vec<String>,
    #[serde(default)]
    pub failure_context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReplanResult {
    pub run_id: String,
    pub status: String,
    pub current_step: usize,
    pub pending_steps: Vec<String>,
    pub recommended_next_action: String,
    pub recommended_tool: String,
    pub replan_summary: String,
}

// ---------------------------------------------------------------------------
// approval.resolve
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalResolveParams {
    pub run_id: String,
    pub approval_id: String,
    pub decision: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalResolveResult {
    pub approval_id: String,
    pub run_id: String,
    pub decision: String,
    pub status: String,
    pub summary: String,
    /// Guidance on what to do next after the approval decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_next_action: Option<String>,
    /// Recommended MCP tool to invoke next.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_tool: Option<String>,
}

// ---------------------------------------------------------------------------
// Pending approval
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingApproval {
    pub approval_id: String,
    pub run_id: String,
    pub action_description: String,
    pub risk_reason: String,
    /// The specific policy rule that triggered this approval.
    #[serde(default)]
    pub policy_rationale: String,
    pub status: String,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Run state (persisted in SQLite)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunState {
    pub run_id: String,
    pub workspace_id: String,
    pub user_goal: String,
    pub status: String,
    pub plan: Vec<String>,
    pub current_step: usize,
    pub completed_steps: Vec<String>,
    pub pending_steps: Vec<String>,
    pub last_action: Option<String>,
    pub last_observation: Option<String>,
    pub recommended_next_action: Option<String>,
    pub recommended_tool: Option<String>,
    pub latest_diff_summary: Option<String>,
    pub latest_test_result: Option<String>,
    /// Focus paths declared at run-prepare time, used for approval policy.
    #[serde(default)]
    pub focus_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}
