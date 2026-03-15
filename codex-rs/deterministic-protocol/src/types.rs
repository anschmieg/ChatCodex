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
// RunPolicy — deterministic per-run execution constraints (Milestone 8)
// ---------------------------------------------------------------------------

/// Deterministic per-run policy profile.
///
/// Captures the active execution constraints for a run.  When omitted at
/// prepare time the backend applies deterministic defaults that match the
/// pre-Milestone-8 behaviour.  Persisted alongside run state in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunPolicy {
    /// Maximum number of edits allowed in a single patch before approval
    /// is required.  Default: 5.
    pub patch_edit_threshold: usize,
    /// If true, any file-delete operation always requires approval.
    /// Default: true.
    pub delete_requires_approval: bool,
    /// If true, edits to paths that match a sensitive-file pattern always
    /// require approval.  Default: true.
    pub sensitive_path_requires_approval: bool,
    /// If true, edits to paths outside the declared `focusPaths` require
    /// approval (when `focusPaths` is non-empty).  Default: true.
    pub outside_focus_requires_approval: bool,
    /// Additional `make` targets (beyond the built-in safe list) that may
    /// run without approval.  Values are normalised to lowercase.
    #[serde(default)]
    pub extra_safe_make_targets: Vec<String>,
    /// Focus paths for this run — copied from `RunPrepareParams.focusPaths`
    /// for backward compatibility.  Evaluated by approval policy when
    /// `outsideFocusRequiresApproval` is true.
    #[serde(default)]
    pub focus_paths: Vec<String>,
}

impl Default for RunPolicy {
    fn default() -> Self {
        Self {
            patch_edit_threshold: 5,
            delete_requires_approval: true,
            sensitive_path_requires_approval: true,
            outside_focus_requires_approval: true,
            extra_safe_make_targets: vec![],
            focus_paths: vec![],
        }
    }
}

/// Optional policy configuration accepted at run-prepare time.
///
/// All fields are optional — omitted fields fall back to `RunPolicy` defaults.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RunPolicyInput {
    #[serde(default)]
    pub patch_edit_threshold: Option<usize>,
    #[serde(default)]
    pub delete_requires_approval: Option<bool>,
    #[serde(default)]
    pub sensitive_path_requires_approval: Option<bool>,
    #[serde(default)]
    pub outside_focus_requires_approval: Option<bool>,
    #[serde(default)]
    pub extra_safe_make_targets: Option<Vec<String>>,
}

impl RunPolicyInput {
    /// Merge with defaults derived from `focus_paths` to produce an effective
    /// `RunPolicy`.  `focus_paths` is always taken from the top-level prepare
    /// params for backward compatibility.
    pub fn into_policy(self, focus_paths: Vec<String>) -> RunPolicy {
        let defaults = RunPolicy::default();
        RunPolicy {
            patch_edit_threshold: self
                .patch_edit_threshold
                .unwrap_or(defaults.patch_edit_threshold),
            delete_requires_approval: self
                .delete_requires_approval
                .unwrap_or(defaults.delete_requires_approval),
            sensitive_path_requires_approval: self
                .sensitive_path_requires_approval
                .unwrap_or(defaults.sensitive_path_requires_approval),
            outside_focus_requires_approval: self
                .outside_focus_requires_approval
                .unwrap_or(defaults.outside_focus_requires_approval),
            extra_safe_make_targets: self
                .extra_safe_make_targets
                .unwrap_or_default()
                .into_iter()
                .map(|t| t.to_lowercase())
                .collect(),
            focus_paths,
        }
    }
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
    /// Optional per-run policy configuration (Milestone 8).
    /// When omitted the daemon uses deterministic defaults.
    #[serde(default)]
    pub policy: Option<RunPolicyInput>,
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
    /// The effective policy profile that will govern this run (Milestone 8).
    pub effective_policy: RunPolicy,
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
    /// Retryable action metadata for resumption guidance (Milestone 6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable_action: Option<RetryableAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// The effective policy profile governing this run (Milestone 8).
    pub effective_policy: RunPolicy,
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
    /// Retryable action state after replanning (Milestone 6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable_action: Option<RetryableAction>,
    /// Concise delta describing what changed during replanning (Milestone 6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replan_delta: Option<String>,
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
    /// Retryable action state after the decision (Milestone 6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable_action: Option<RetryableAction>,
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
// Retryable action (persisted in RunState, Milestone 6)
// ---------------------------------------------------------------------------

/// A structured representation of a gated or failed action that ChatGPT
/// may retry after approval, or should avoid after denial/replanning.
///
/// This is purely deterministic metadata — the backend never auto-retries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryableAction {
    /// Action kind: `"patch.apply"` or `"tests.run"`.
    pub kind: String,
    /// Human-readable summary of what the action does.
    pub summary: String,
    /// Normalized action payload (JSON string of the original request).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
    /// Why this action became retryable (e.g. "blocked by approval policy").
    pub retryable_reason: String,
    /// Whether retrying this action is still valid.
    pub is_valid: bool,
    /// Whether retrying this action is the recommended next step.
    pub is_recommended: bool,
    /// If the action is no longer valid, why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalidation_reason: Option<String>,
    /// The MCP tool to invoke if retrying.
    pub recommended_tool: String,
    /// When this retryable record was created.
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
    /// The last action that was gated or failed and may be retryable (Milestone 6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable_action: Option<RetryableAction>,
    /// Per-run policy profile governing approval decisions (Milestone 8).
    #[serde(default)]
    pub policy_profile: RunPolicy,
    pub created_at: String,
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// runs.list  (Milestone 7)
// ---------------------------------------------------------------------------

/// Compact summary of a run for listing purposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub run_id: String,
    pub workspace_id: String,
    pub user_goal: String,
    pub status: String,
    pub current_step: usize,
    pub total_steps: usize,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunsListParams {
    /// Maximum number of runs to return (default: 20, max: 100).
    #[serde(default)]
    pub limit: Option<usize>,
    /// Filter by workspace ID (optional).
    #[serde(default)]
    pub workspace_id: Option<String>,
    /// Filter by status (optional).
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunsListResult {
    pub runs: Vec<RunSummary>,
    /// Number of runs returned (may be less than total if limit was applied).
    pub count: usize,
}

// ---------------------------------------------------------------------------
// run.get  (Milestone 7)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunGetParams {
    pub run_id: String,
}

/// Full authoritative current state of a run for direct inspection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunGetResult {
    pub run_state: RunState,
    pub pending_approvals: Vec<PendingApproval>,
    /// Retryable action metadata (from RunState).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable_action: Option<RetryableAction>,
    /// Latest diff summary if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_diff_summary: Option<String>,
    /// Latest test result if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_test_result: Option<String>,
    /// Recommended next action (forwarded from RunState).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_next_action: Option<String>,
    /// Recommended MCP tool (forwarded from RunState).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_tool: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// The effective policy profile governing this run (Milestone 8).
    pub effective_policy: RunPolicy,
}

// ---------------------------------------------------------------------------
// run.history  (Milestone 7)
// ---------------------------------------------------------------------------

/// A single audit-trail entry for a run event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunHistoryEntry {
    pub entry_id: String,
    pub run_id: String,
    /// Event kind (e.g. "run_prepared", "patch_applied", "tests_run", ...).
    pub event_kind: String,
    /// Short human-readable description of what happened.
    pub summary: String,
    /// Optional structured metadata (JSON string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<String>,
    pub occurred_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunHistoryParams {
    pub run_id: String,
    /// Maximum number of entries to return (default: 50, max: 200).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunHistoryResult {
    pub run_id: String,
    pub entries: Vec<RunHistoryEntry>,
    /// Number of entries returned (may be less than total if limit was applied).
    pub count: usize,
}
