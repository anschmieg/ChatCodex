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
    /// Structured final outcome if this run has been explicitly finalized (Milestone 10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalized_outcome: Option<RunOutcome>,
    /// Reopen lineage metadata if this run has been reopened (Milestone 11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reopen_metadata: Option<ReopenMetadata>,
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
// Run finalization outcome (Milestone 10)
// ---------------------------------------------------------------------------

/// Structured final outcome record for a closed run.
///
/// Persisted in SQLite alongside the run state when ChatGPT explicitly
/// finalizes a run.  The record is read-only after creation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunOutcome {
    /// Disposition: `"completed"`, `"failed"`, or `"abandoned"`.
    pub outcome_kind: String,
    /// Short deterministic summary of what was accomplished or why the run ended.
    pub summary: String,
    /// Optional reason for failure or abandonment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// ISO 8601 timestamp of when finalization occurred.
    pub finalized_at: String,
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
    /// Structured final outcome if this run has been explicitly finalized (Milestone 10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalized_outcome: Option<RunOutcome>,
    /// Reopen lineage metadata if this run has been reopened one or more times (Milestone 11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reopen_metadata: Option<ReopenMetadata>,
    /// The run ID that this run supersedes, if any (Milestone 12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes_run_id: Option<String>,
    /// The run ID that superseded this run, if any (Milestone 12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by_run_id: Option<String>,
    /// Human-readable reason this run was superseded or is superseding another (Milestone 12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersession_reason: Option<String>,
    /// ISO 8601 timestamp of when the supersession occurred (Milestone 12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_at: Option<String>,
    /// Archive metadata if this run has been explicitly archived (Milestone 13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_metadata: Option<ArchiveMetadata>,
    /// Unarchive (restoration) metadata if this run has been explicitly unarchived (Milestone 14).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unarchive_metadata: Option<UnarchiveMetadata>,
    /// Organization metadata: labels and optional operator note (Milestone 15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation: Option<RunAnnotation>,
    /// Pin metadata if this run has been explicitly pinned (Milestone 16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_metadata: Option<PinMetadata>,
    /// Snooze metadata if this run has been explicitly snoozed (Milestone 17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snooze_metadata: Option<SnoozeMetadata>,
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
    /// Final disposition if the run has been explicitly finalized (Milestone 10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome_kind: Option<String>,
    /// Number of times this run has been reopened (Milestone 11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reopen_count: Option<u32>,
    /// Run ID superseded by this run, if any (Milestone 12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes_run_id: Option<String>,
    /// Run ID that superseded this run, if any (Milestone 12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by_run_id: Option<String>,
    /// Whether this run has been explicitly archived (Milestone 13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_archived: Option<bool>,
    /// Archive reason if the run has been archived (Milestone 13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_reason: Option<String>,
    /// ISO 8601 timestamp of when this run was archived (Milestone 13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
    /// Unarchive reason if this run has been explicitly unarchived (Milestone 14).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unarchive_reason: Option<String>,
    /// ISO 8601 timestamp of when this run was unarchived (Milestone 14).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unarchived_at: Option<String>,
    /// Labels for this run (Milestone 15).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Operator note for this run (Milestone 15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_note: Option<String>,
    /// Whether this run is currently pinned (Milestone 16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_pinned: Option<bool>,
    /// Pin reason if the run is pinned (Milestone 16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_reason: Option<String>,
    /// ISO 8601 timestamp of when this run was pinned (Milestone 16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_at: Option<String>,
    /// Whether this run is currently snoozed (Milestone 17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_snoozed: Option<bool>,
    /// Snooze reason if the run is snoozed (Milestone 17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snooze_reason: Option<String>,
    /// ISO 8601 timestamp of when this run was snoozed (Milestone 17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_at: Option<String>,
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
    /// When true, archived runs are included alongside non-archived runs (Milestone 13).
    /// Default: false (archived runs are excluded).
    #[serde(default)]
    pub include_archived: Option<bool>,
    /// When true, return only archived runs (Milestone 13).
    /// Takes precedence over `include_archived`.
    #[serde(default)]
    pub archived_only: Option<bool>,
    /// Filter by exact normalized label (Milestone 15).
    /// When set, only runs that carry this label are returned.
    #[serde(default)]
    pub label: Option<String>,
    /// When true, return only pinned runs (Milestone 16).
    #[serde(default)]
    pub pinned_only: Option<bool>,
    /// When true, snoozed runs are included alongside non-snoozed runs (Milestone 17).
    /// Default: false (snoozed runs are excluded).
    #[serde(default)]
    pub include_snoozed: Option<bool>,
    /// When true, return only snoozed runs (Milestone 17).
    /// Takes precedence over `include_snoozed`.
    #[serde(default)]
    pub snoozed_only: Option<bool>,
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
    /// Structured final outcome if this run has been explicitly finalized (Milestone 10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalized_outcome: Option<RunOutcome>,
    /// Reopen lineage metadata if this run has been reopened (Milestone 11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reopen_metadata: Option<ReopenMetadata>,
    /// The run ID this run supersedes, if any (Milestone 12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes_run_id: Option<String>,
    /// The run ID that superseded this run, if any (Milestone 12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by_run_id: Option<String>,
    /// Human-readable reason for the supersession (Milestone 12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersession_reason: Option<String>,
    /// ISO 8601 timestamp of when supersession occurred (Milestone 12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_at: Option<String>,
    /// Archive metadata if this run has been explicitly archived (Milestone 13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_metadata: Option<ArchiveMetadata>,
    /// Unarchive (restoration) metadata if this run has been explicitly unarchived (Milestone 14).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unarchive_metadata: Option<UnarchiveMetadata>,
    /// Organization metadata: labels and optional operator note (Milestone 15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation: Option<RunAnnotation>,
    /// Pin metadata if this run has been explicitly pinned (Milestone 16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_metadata: Option<PinMetadata>,
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

// ---------------------------------------------------------------------------
// Preflight / preview  (Milestone 9)
// ---------------------------------------------------------------------------

/// Outcome of a deterministic preflight evaluation.
///
/// Returned by `patch.preflight` and `tests.preflight`.  The value is
/// read-only — no state is modified when computing it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightDecision {
    /// The operation would proceed immediately under the current policy.
    Proceed,
    /// The operation would be gated and require explicit approval.
    RequiresApproval,
}

/// Shared result model for a preflight policy evaluation.
///
/// Compact, explicit, and reusable for both `patch.preflight` and
/// `tests.preflight`.  All optional fields are `None` when
/// `decision == Proceed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightResult {
    /// Whether the operation would proceed or require approval.
    pub decision: PreflightDecision,
    /// Human-readable summary of the proposed action (present when gated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_summary: Option<String>,
    /// Why the operation is considered risky (present when gated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_reason: Option<String>,
    /// Which policy rule would trigger the gate (present when gated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_rationale: Option<String>,
    /// The effective policy profile used for this evaluation.
    pub effective_policy: RunPolicy,
}

// ---------------------------------------------------------------------------
// patch.preflight  (Milestone 9)
// ---------------------------------------------------------------------------

/// Parameters for `patch.preflight` — mirrors `PatchApplyParams` but is
/// strictly read-only: no files are modified.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchPreflightParams {
    pub run_id: String,
    pub edits: Vec<PatchEdit>,
}

// ---------------------------------------------------------------------------
// tests.preflight  (Milestone 9)
// ---------------------------------------------------------------------------

/// Parameters for `tests.preflight` — mirrors `TestsRunParams` but is
/// strictly read-only: no tests are executed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestsPreflightParams {
    pub run_id: String,
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// ---------------------------------------------------------------------------
// run.reopen  (Milestone 11)
// ---------------------------------------------------------------------------

/// Compact metadata recorded each time a finalized run is reopened.
///
/// Persisted in SQLite alongside run state.  Provides a compact audit
/// trail of reopening lineage without duplicating the full outcome record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReopenMetadata {
    /// Human-readable reason ChatGPT supplied for reopening.
    pub reason: String,
    /// ISO 8601 timestamp of when the most recent reopen occurred.
    pub reopened_at: String,
    /// `outcome_kind` of the finalized outcome that was cleared by this reopen.
    pub reopened_from_outcome_kind: String,
    /// How many times this run has been reopened in total (starts at 1).
    pub reopen_count: u32,
}

/// Parameters for `run.reopen` — explicit deterministic run continuation.
///
/// Only finalized runs (`completed`, `failed`, `abandoned`) may be reopened.
/// Reopening is deterministic and audited; it does not itself execute work.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReopenParams {
    pub run_id: String,
    /// Human-readable reason for reopening (required for auditability).
    pub reason: String,
}

/// Result of `run.reopen`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReopenResult {
    pub run_id: String,
    /// New run status after reopening (e.g. `"active"`).
    pub status: String,
    /// Outcome kind from which the run was reopened (e.g. `"completed"`).
    pub reopened_from_outcome_kind: String,
    /// Total number of times this run has been reopened.
    pub reopen_count: u32,
    /// ISO 8601 timestamp of when the reopen occurred.
    pub reopened_at: String,
    /// Deterministic guidance on what to do next.
    pub recommended_next_action: String,
    /// Recommended MCP tool to invoke next.
    pub recommended_tool: String,
}

// ---------------------------------------------------------------------------
// run.finalize  (Milestone 10)
// ---------------------------------------------------------------------------

/// Valid closure kinds for run finalization.
///
/// The backend rejects any value not in this set.
pub const VALID_OUTCOME_KINDS: &[&str] = &["completed", "failed", "abandoned"];

/// Parameters for `run.finalize` — explicit deterministic run closure.
///
/// Calling this method permanently closes the run with a structured outcome
/// record.  A run that is already finalized cannot be finalized again.
/// No autonomous work is triggered as a result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunFinalizeParams {
    pub run_id: String,
    /// Must be one of: `"completed"`, `"failed"`, `"abandoned"`.
    pub outcome_kind: String,
    /// Short deterministic summary (recommended ≤ 200 characters).
    pub summary: String,
    /// Optional reason, typically for `"failed"` or `"abandoned"` runs.
    #[serde(default)]
    pub reason: Option<String>,
}

/// Result of `run.finalize`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunFinalizeResult {
    pub run_id: String,
    pub outcome_kind: String,
    pub finalized_at: String,
    /// The new run status after finalization (e.g. `"finalized:completed"`).
    pub status: String,
    /// Deterministic guidance on what to do next (varies by outcome_kind).
    pub recommended_next_action: String,
}

// ---------------------------------------------------------------------------
// run.supersede  (Milestone 12)
// ---------------------------------------------------------------------------

/// Parameters for `run.supersede` — create a successor run that explicitly
/// replaces a prior run.
///
/// The original run must be finalized before it can be superseded.  Supersession
/// is deterministic, audited, and does not execute work.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSupersedeParams {
    /// ID of the finalized run to supersede.
    pub run_id: String,
    /// Goal for the new successor run.  When omitted, the original run's goal
    /// is inherited by the successor.
    #[serde(default)]
    pub new_user_goal: Option<String>,
    /// Human-readable reason for supersession (required for auditability).
    pub reason: String,
}

/// Result of `run.supersede`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSupersedeResult {
    /// ID of the run that was superseded (now carries `superseded_by_run_id`).
    pub original_run_id: String,
    /// ID of the newly created successor run (carries `supersedes_run_id`).
    pub successor_run_id: String,
    /// ISO 8601 timestamp of when the supersession occurred.
    pub superseded_at: String,
    /// Status of the new successor run after creation.
    pub successor_status: String,
    /// Deterministic guidance on what to do next.
    pub recommended_next_action: String,
    /// Recommended MCP tool to invoke next.
    pub recommended_tool: String,
}

// ---------------------------------------------------------------------------
// run.archive  (Milestone 13)
// ---------------------------------------------------------------------------

/// Compact archive metadata recorded when a run is explicitly archived.
///
/// Persisted in SQLite alongside run state.  Provides an auditable record
/// of archival without duplicating the full run state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveMetadata {
    /// Human-readable reason supplied by ChatGPT for archiving.
    pub reason: String,
    /// ISO 8601 timestamp of when the run was archived.
    pub archived_at: String,
}

/// Parameters for `run.archive` — explicit deterministic run archiving.
///
/// Only finalized runs (`completed`, `failed`, `abandoned`) may be archived.
/// Active, prepared, or awaiting-approval runs are rejected.
/// Archiving is deterministic and audited; it does not execute work.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunArchiveParams {
    /// ID of the run to archive.
    pub run_id: String,
    /// Human-readable reason for archiving (required for auditability).
    pub reason: String,
}

/// Result of `run.archive`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunArchiveResult {
    /// ID of the run that was archived.
    pub run_id: String,
    /// Current status of the run (e.g. `"finalized:completed"`).
    pub status: String,
    /// ISO 8601 timestamp of when the run was archived.
    pub archived_at: String,
    /// Human-readable reason supplied for archiving.
    pub reason: String,
    /// Confirmation message.
    pub message: String,
}

// ---------------------------------------------------------------------------
// run.annotate  (Milestone 15)
// ---------------------------------------------------------------------------

/// Compact deterministic organization metadata attached to a run.
///
/// Labels are normalized to lowercase, deduplicated, and sorted.
/// The operator note is a free-text annotation with no semantic meaning
/// to the backend — it is purely organizational metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RunAnnotation {
    /// Zero or more lowercase normalized labels/tags for this run.
    /// Each label is bounded to `LABEL_MAX_LEN` characters.
    /// The total number of labels is bounded to `LABEL_MAX_COUNT`.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Optional operator note — concise free-text annotation for organization.
    /// Bounded to `OPERATOR_NOTE_MAX_LEN` characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_note: Option<String>,
}

/// Maximum length of a single label (characters).
pub const LABEL_MAX_LEN: usize = 64;
/// Maximum number of labels on a single run.
pub const LABEL_MAX_COUNT: usize = 16;
/// Maximum length of the operator note (characters).
pub const OPERATOR_NOTE_MAX_LEN: usize = 1000;

/// Parameters for `run.annotate` — update organization metadata on a run.
///
/// This operation is purely organizational:
/// - it does not execute work
/// - it does not refresh, replan, reopen, finalize, archive, unarchive, or
///   supersede the run
/// - labels and note are persisted and visible in read surfaces
/// - an audit entry is appended
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunAnnotateParams {
    pub run_id: String,
    /// Full replacement label set (normalized at validation time).
    /// When provided, replaces the existing label set entirely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<String>>,
    /// Full replacement operator note.
    /// When provided, replaces any existing note.
    /// Pass an empty string to clear the note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_note: Option<String>,
}

/// Result of `run.annotate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunAnnotateResult {
    /// ID of the annotated run.
    pub run_id: String,
    /// The annotation after applying this update.
    pub annotation: RunAnnotation,
    /// Confirmation message.
    pub message: String,
}

// ---------------------------------------------------------------------------
// run.unarchive  (Milestone 14)
// ---------------------------------------------------------------------------

/// Compact unarchive (restoration) metadata recorded when a run is explicitly unarchived.
///
/// Persisted in SQLite alongside run state.  Provides an auditable record of
/// restoration without duplicating the full run state.  The original
/// `archive_metadata` remains intact for historical inspection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UnarchiveMetadata {
    /// Human-readable reason supplied by ChatGPT for unarchiving.
    pub reason: String,
    /// ISO 8601 timestamp of when the run was unarchived.
    pub unarchived_at: String,
}

/// Parameters for `run.unarchive` — explicit deterministic run unarchiving.
///
/// Only archived runs may be unarchived.
/// Non-archived runs are rejected.
/// Unarchiving is deterministic and audited; it does not execute work.
/// Unarchiving does not reopen the run or change its finalized outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunUnarchiveParams {
    /// ID of the archived run to unarchive.
    pub run_id: String,
    /// Human-readable reason for unarchiving (required for auditability).
    pub reason: String,
}

/// Result of `run.unarchive`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunUnarchiveResult {
    /// ID of the run that was unarchived.
    pub run_id: String,
    /// Current status of the run (unchanged, e.g. `"finalized:completed"`).
    pub status: String,
    /// ISO 8601 timestamp of when the run was unarchived.
    pub unarchived_at: String,
    /// Human-readable reason supplied for unarchiving.
    pub reason: String,
    /// Confirmation message.
    pub message: String,
}

// ---------------------------------------------------------------------------
// run.pin / run.unpin  (Milestone 16)
// ---------------------------------------------------------------------------

/// Maximum length of a pin reason (characters).
pub const PIN_REASON_MAX_LEN: usize = 500;

/// Compact pin metadata recorded when a run is explicitly pinned.
///
/// Persisted in SQLite alongside run state.  Provides an auditable record of
/// the pin action without duplicating the full run state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PinMetadata {
    /// Human-readable reason supplied by ChatGPT for pinning.
    pub reason: String,
    /// ISO 8601 timestamp of when the run was pinned.
    pub pinned_at: String,
}

/// Parameters for `run.pin` — explicit deterministic run pinning.
///
/// Pinning is deterministic, explicit, and audited.  It updates only the
/// pin metadata and does not execute work, change status, refresh, replan,
/// reopen, finalize, archive, unarchive, or supersede the run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPinParams {
    /// ID of the run to pin.
    pub run_id: String,
    /// Human-readable reason for pinning (required for auditability).
    pub reason: String,
}

/// Result of `run.pin`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPinResult {
    /// ID of the run that was pinned.
    pub run_id: String,
    /// Current status of the run (unchanged).
    pub status: String,
    /// ISO 8601 timestamp of when the run was pinned.
    pub pinned_at: String,
    /// Human-readable reason supplied for pinning.
    pub reason: String,
    /// Confirmation message.
    pub message: String,
}

/// Parameters for `run.unpin` — explicit deterministic run unpinning.
///
/// Unpinning is deterministic, explicit, and audited.  It updates only the
/// pin metadata and does not execute work, change status, or affect any other
/// lifecycle field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunUnpinParams {
    /// ID of the pinned run to unpin.
    pub run_id: String,
    /// Human-readable reason for unpinning (required for auditability).
    pub reason: String,
}

/// Result of `run.unpin`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunUnpinResult {
    /// ID of the run that was unpinned.
    pub run_id: String,
    /// Current status of the run (unchanged).
    pub status: String,
    /// Confirmation message.
    pub message: String,
}

// ---------------------------------------------------------------------------
// run.snooze / run.unsnooze  (Milestone 17)
// ---------------------------------------------------------------------------

/// Maximum length of a snooze reason (characters).
pub const SNOOZE_REASON_MAX_LEN: usize = 500;

/// Compact snooze metadata recorded when a run is explicitly snoozed.
///
/// Persisted in SQLite alongside run state.  Provides an auditable record of
/// the snooze action without duplicating the full run state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SnoozeMetadata {
    /// Human-readable reason supplied by ChatGPT for snoozing.
    pub reason: String,
    /// ISO 8601 timestamp of when the run was snoozed.
    pub snoozed_at: String,
}

/// Parameters for `run.snooze` — explicit deterministic run snoozing.
///
/// Snoozing is deterministic, explicit, and audited.  It updates only the
/// snooze metadata and does not execute work, change status, refresh, replan,
/// reopen, finalize, archive, unarchive, or supersede the run.
/// Snoozed runs are excluded from the default `runs.list` result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSnoozeParams {
    /// ID of the run to snooze.
    pub run_id: String,
    /// Human-readable reason for snoozing (required for auditability).
    pub reason: String,
}

/// Result of `run.snooze`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSnoozeResult {
    /// ID of the run that was snoozed.
    pub run_id: String,
    /// Current status of the run (unchanged).
    pub status: String,
    /// ISO 8601 timestamp of when the run was snoozed.
    pub snoozed_at: String,
    /// Human-readable reason supplied for snoozing.
    pub reason: String,
    /// Confirmation message.
    pub message: String,
}

/// Parameters for `run.unsnooze` — explicit deterministic run unsnoozing.
///
/// Unsnoozing is deterministic, explicit, and audited.  It clears the snooze
/// metadata only and does not execute work, change status, refresh, replan,
/// reopen, finalize, archive, unarchive, or supersede the run.
/// Only snoozed runs may be unsnoozed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunUnsnoozeParams {
    /// ID of the snoozed run to unsnooze.
    pub run_id: String,
    /// Human-readable reason for unsnoozing (required for auditability).
    pub reason: String,
}

/// Result of `run.unsnooze`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunUnsnoozeResult {
    /// ID of the run that was unsnoozed.
    pub run_id: String,
    /// Current status of the run (unchanged).
    pub status: String,
    /// Confirmation message.
    pub message: String,
}
