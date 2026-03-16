/**
 * Zod schemas for MCP tool inputs.
 *
 * These schemas are the **only** input validation layer in the MCP
 * gateway.  They must mirror the daemon's expected parameters.
 */

import { z } from "zod";

// ---------------------------------------------------------------
// PolicyProfileInput — per-run policy configuration (Milestone 8)
// ---------------------------------------------------------------
export const PolicyProfileInputSchema = z
  .object({
    patchEditThreshold: z
      .number()
      .int()
      .positive()
      .optional()
      .describe(
        "Maximum edits in a single patch before approval is required (default: 5)",
      ),
    deleteRequiresApproval: z
      .boolean()
      .optional()
      .describe(
        "Whether file deletion always requires approval (default: true)",
      ),
    sensitivePathRequiresApproval: z
      .boolean()
      .optional()
      .describe(
        "Whether edits to sensitive file paths always require approval (default: true)",
      ),
    outsideFocusRequiresApproval: z
      .boolean()
      .optional()
      .describe(
        "Whether edits outside declared focus paths require approval when focus is non-empty (default: true)",
      ),
    extraSafeMakeTargets: z
      .array(z.string())
      .optional()
      .describe(
        "Additional make targets that may run without approval beyond the built-in safe list",
      ),
  })
  .describe("Optional per-run policy configuration");

// ---------------------------------------------------------------
// codex_prepare_run
// ---------------------------------------------------------------
export const CodexPrepareRunInput = {
  workspaceId: z.string().describe("Absolute path to the workspace root"),
  userGoal: z.string().describe("User's coding goal"),
  focusPaths: z
    .array(z.string())
    .optional()
    .describe("Optional paths to focus on"),
  mode: z
    .enum(["plan", "refresh", "repair", "review"])
    .optional()
    .describe("Run mode"),
  policy: PolicyProfileInputSchema.optional().describe(
    "Optional per-run policy configuration. When omitted the daemon uses deterministic defaults.",
  ),
};

// ---------------------------------------------------------------
// get_workspace_summary
// ---------------------------------------------------------------
export const GetWorkspaceSummaryInput = {
  workspaceId: z.string().describe("Absolute path to the workspace root"),
  focusPaths: z
    .array(z.string())
    .optional()
    .describe("Optional paths to focus on"),
};

// ---------------------------------------------------------------
// read_file
// ---------------------------------------------------------------
export const ReadFileInput = {
  runId: z.string().describe("Run ID from codex_prepare_run"),
  path: z.string().describe("Relative path within workspace"),
  startLine: z
    .number()
    .int()
    .positive()
    .optional()
    .describe("Start line (1-indexed)"),
  endLine: z
    .number()
    .int()
    .positive()
    .optional()
    .describe("End line (1-indexed, inclusive)"),
  purpose: z
    .string()
    .optional()
    .describe("Why this file is being read (for audit trail)"),
};

// ---------------------------------------------------------------
// git_status
// ---------------------------------------------------------------
export const GitStatusInput = {
  runId: z.string().describe("Run ID from codex_prepare_run"),
};

// ---------------------------------------------------------------
// search_code
// ---------------------------------------------------------------
export const SearchCodeInput = {
  runId: z.string().describe("Run ID from codex_prepare_run"),
  query: z.string().describe("Text or regex to search for"),
  pathGlob: z.string().optional().describe("File glob pattern"),
  maxResults: z.number().int().positive().optional().describe("Max results"),
};

// ---------------------------------------------------------------
// apply_patch
// ---------------------------------------------------------------
const PatchEditSchema = z.object({
  path: z.string(),
  operation: z.enum(["create", "replace", "delete"]),
  startLine: z.number().int().optional(),
  endLine: z.number().int().optional(),
  oldText: z.string().optional(),
  newText: z.string(),
  anchorText: z
    .string()
    .optional()
    .describe("Context text to anchor the edit location"),
  reason: z.string().optional().describe("Why this edit is being made"),
});

export const ApplyPatchInput = {
  runId: z.string().describe("Run ID from codex_prepare_run"),
  edits: z.array(PatchEditSchema).describe("Edits to apply"),
};

// ---------------------------------------------------------------
// run_tests
//
// `scope` is a semantic string.  The daemon resolves it to a
// concrete command deterministically.  Well-known values include
// framework names ("cargo", "npm", "pytest", "make") and semantic
// labels ("unit", "integration", "all").
// ---------------------------------------------------------------
export const RunTestsInput = {
  runId: z.string().describe("Run ID from codex_prepare_run"),
  scope: z
    .string()
    .describe(
      "Test scope — a framework name (cargo, npm, pytest, make) or a semantic label (unit, integration, all)",
    ),
  target: z.string().optional().describe("Specific test target within scope"),
  reason: z.string().describe("Reason for running tests"),
};

// ---------------------------------------------------------------
// show_diff
// ---------------------------------------------------------------
export const ShowDiffInput = {
  runId: z.string().describe("Run ID from codex_prepare_run"),
  paths: z.array(z.string()).optional().describe("Paths to diff"),
  format: z
    .enum(["summary", "patch"])
    .optional()
    .describe("Output format"),
};

// ---------------------------------------------------------------
// refresh_run_state
// ---------------------------------------------------------------
export const RefreshRunStateInput = {
  runId: z.string().describe("Run ID from codex_prepare_run"),
};

// ---------------------------------------------------------------
// replan_run
// ---------------------------------------------------------------
export const ReplanRunInput = {
  runId: z.string().describe("Run ID from codex_prepare_run"),
  reason: z.string().describe("Why the run needs replanning"),
  newEvidence: z
    .array(z.string())
    .optional()
    .describe("New evidence or observations"),
  failureContext: z
    .string()
    .optional()
    .describe("Error or failure context that triggered replanning"),
};

// ---------------------------------------------------------------
// approve_action
// ---------------------------------------------------------------
export const ApproveActionInput = {
  runId: z.string().describe("Run ID from codex_prepare_run"),
  approvalId: z.string().describe("Approval ID to resolve"),
  decision: z
    .enum(["approve", "deny"])
    .describe("Whether to approve or deny the pending action"),
  reason: z
    .string()
    .optional()
    .describe("Reason for the decision"),
};

// ---------------------------------------------------------------
// list_runs  (Milestone 7; extended in Milestone 13, 15)
// ---------------------------------------------------------------
export const ListRunsInput = {
  limit: z
    .number()
    .int()
    .positive()
    .max(100)
    .optional()
    .describe("Maximum number of runs to return (default: 20, max: 100)"),
  workspaceId: z
    .string()
    .optional()
    .describe("Filter by workspace path"),
  status: z
    .string()
    .optional()
    .describe("Filter by run status (e.g. active, done, blocked)"),
  // Milestone 13: archive filtering
  includeArchived: z
    .boolean()
    .optional()
    .describe(
      "When true, include archived runs alongside non-archived runs in the results. Default: false (archived runs are excluded).",
    ),
  archivedOnly: z
    .boolean()
    .optional()
    .describe(
      "When true, return only archived runs. Takes precedence over includeArchived.",
    ),
  // Milestone 15: label filtering
  label: z
    .string()
    .optional()
    .describe(
      "Filter by exact normalized label. When provided, only runs that carry this label are returned.",
    ),
  // Milestone 16: pinned_only filter
  pinnedOnly: z
    .boolean()
    .optional()
    .describe("When true, return only pinned runs."),
  // Milestone 17: snooze filtering
  includeSnoozed: z
    .boolean()
    .optional()
    .describe(
      "When true, include snoozed runs alongside non-snoozed runs in the results. Default: false (snoozed runs are excluded).",
    ),
  snoozedOnly: z
    .boolean()
    .optional()
    .describe(
      "When true, return only snoozed runs. Takes precedence over includeSnoozed.",
    ),
};

// ---------------------------------------------------------------
// get_run_state  (Milestone 7)
// ---------------------------------------------------------------
export const GetRunStateInput = {
  runId: z.string().describe("Run ID to inspect"),
};

// ---------------------------------------------------------------
// get_run_history  (Milestone 7)
// ---------------------------------------------------------------
export const GetRunHistoryInput = {
  runId: z.string().describe("Run ID to retrieve audit history for"),
  limit: z
    .number()
    .int()
    .positive()
    .max(200)
    .optional()
    .describe("Maximum number of entries to return (default: 50, max: 200)"),
};

// ---------------------------------------------------------------
// preview_patch_policy  (Milestone 9)
//
// Mirrors apply_patch inputs but is strictly read-only.
// ---------------------------------------------------------------
export const PreviewPatchPolicyInput = {
  runId: z.string().describe("Run ID from codex_prepare_run"),
  edits: z.array(PatchEditSchema).describe("Proposed edits to evaluate (not applied)"),
};

// ---------------------------------------------------------------
// preview_test_policy  (Milestone 9)
//
// Mirrors run_tests inputs but is strictly read-only.
// ---------------------------------------------------------------
export const PreviewTestPolicyInput = {
  runId: z.string().describe("Run ID from codex_prepare_run"),
  scope: z
    .string()
    .describe(
      "Test scope — a framework name (cargo, npm, pytest, make) or a semantic label",
    ),
  target: z.string().optional().describe("Specific test target within scope"),
  reason: z.string().optional().describe("Why this test run is being evaluated"),
};

// ---------------------------------------------------------------
// finalize_run  (Milestone 10)
//
// Explicitly closes a run with a structured outcome record.
// No autonomous work is triggered.
// ---------------------------------------------------------------
export const FinalizeRunInput = {
  runId: z.string().describe("Run ID from codex_prepare_run"),
  outcomeKind: z
    .enum(["completed", "failed", "abandoned"])
    .describe(
      "Final disposition of the run: 'completed', 'failed', or 'abandoned'",
    ),
  summary: z
    .string()
    .max(500)
    .describe(
      "Short deterministic summary of what was accomplished or why the run ended",
    ),
  reason: z
    .string()
    .optional()
    .describe(
      "Optional reason, typically for 'failed' or 'abandoned' runs",
    ),
};

// ---------------------------------------------------------------
// reopen_run  (Milestone 11)
//
// Reopens a previously finalized run for deterministic continuation.
// Only finalized runs may be reopened.  No autonomous work is triggered.
// ---------------------------------------------------------------
export const ReopenRunInput = {
  runId: z.string().describe("Run ID of the finalized run to reopen"),
  reason: z
    .string()
    .min(1)
    .max(500)
    // 500-char limit matches `FinalizeRunInput.summary` for consistency
    // and fits within a single SQLite TEXT field / audit log entry.
    .describe(
      "Human-readable reason for reopening the run (required for auditability)",
    ),
};

// ---------------------------------------------------------------
// supersede_run  (Milestone 12)
//
// Creates a new successor run that explicitly replaces a finalized run.
// The original run remains preserved with its full audit history.
// Only finalized runs may be superseded.  No autonomous work is triggered.
// ---------------------------------------------------------------
export const SupersedeRunInput = {
  runId: z
    .string()
    .describe(
      "Run ID of the finalized run to supersede (must be finalized: completed, failed, or abandoned)",
    ),
  newUserGoal: z
    .string()
    .max(500)
    .optional()
    .describe(
      "Goal for the successor run. When omitted the original run's goal is inherited.",
    ),
  reason: z
    .string()
    .min(1)
    .max(500)
    .describe(
      "Human-readable reason for supersession (required for auditability)",
    ),
};

// ---------------------------------------------------------------
// archive_run  (Milestone 13)
//
// Explicitly archives a finalized run so it remains preserved and
// inspectable but is excluded from the default active run listing.
// Only finalized runs (completed, failed, abandoned) may be archived.
// Archiving is deterministic and audited; it does not execute work.
// ---------------------------------------------------------------
export const ArchiveRunInput = {
  runId: z
    .string()
    .describe(
      "Run ID of the finalized run to archive (must be finalized: completed, failed, or abandoned)",
    ),
  reason: z
    .string()
    .min(1)
    .max(500)
    .describe(
      "Human-readable reason for archiving (required for auditability)",
    ),
};

// Milestone 14: unarchive_run
export const UnarchiveRunInput = {
  runId: z
    .string()
    .describe(
      "Run ID of the archived run to unarchive (must be archived)",
    ),
  reason: z
    .string()
    .min(1)
    .max(500)
    .describe(
      "Human-readable reason for unarchiving (required for auditability)",
    ),
};

// ---------------------------------------------------------------
// annotate_run  (Milestone 15)
//
// Explicitly annotates a run with compact organization metadata:
// one or more labels and/or an optional operator note.
// This operation is deterministic, persisted, and audited.
// It does not execute work, replan, reopen, finalize, archive,
// unarchive, or supersede the run.
// At least one of `labels` or `operatorNote` must be provided.
// ---------------------------------------------------------------
export const AnnotateRunInput = {
  runId: z
    .string()
    .describe("Run ID to annotate"),
  labels: z
    .array(
      z
        .string()
        .min(1)
        .max(64)
        .regex(
          /^[a-z0-9_-]+$/,
          "Labels must contain only lowercase ASCII letters, digits, hyphens, or underscores",
        ),
    )
    .max(16)
    .optional()
    .describe(
      "Compact normalized labels to set on the run. Replaces any existing labels. " +
        "At most 16 labels, each at most 64 characters, lowercase alphanumeric/hyphens/underscores.",
    ),
  operatorNote: z
    .string()
    .max(1000)
    .optional()
    .describe(
      "Optional concise operator note (max 1000 characters). Pass an empty string to clear. " +
        "Non-semantic organization metadata only.",
    ),
};

// ---------------------------------------------------------------
// PinRunInput / UnpinRunInput  (Milestone 16)
// ---------------------------------------------------------------

/** Input schema for `pin_run`. */
export const PinRunInput = {
  runId: z.string().describe("Run ID to pin"),
  reason: z
    .string()
    .min(1)
    .max(500)
    .describe(
      "Human-readable reason for pinning (required for auditability, max 500 characters).",
    ),
};

/** Input schema for `unpin_run`. */
export const UnpinRunInput = {
  runId: z.string().describe("Run ID to unpin"),
  reason: z
    .string()
    .min(1)
    .max(500)
    .describe(
      "Human-readable reason for unpinning (required for auditability, max 500 characters).",
    ),
};

// ---------------------------------------------------------------
// SnoozeRunInput / UnsnoozeRunInput  (Milestone 17)
// ---------------------------------------------------------------

/** Input schema for `snooze_run`. */
export const SnoozeRunInput = {
  runId: z.string().describe("Run ID to snooze"),
  reason: z
    .string()
    .min(1)
    .max(500)
    .describe(
      "Human-readable reason for snoozing (required for auditability, max 500 characters).",
    ),
};

/** Input schema for `unsnooze_run`. */
export const UnsnoozeRunInput = {
  runId: z.string().describe("Run ID to unsnooze (must currently be snoozed)"),
  reason: z
    .string()
    .min(1)
    .max(500)
    .describe(
      "Human-readable reason for unsnoozing (required for auditability, max 500 characters).",
    ),
};
