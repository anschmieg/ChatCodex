/**
 * Zod schemas for MCP tool inputs.
 *
 * These schemas are the **only** input validation layer in the MCP
 * gateway.  They must mirror the daemon's expected parameters.
 */

import { z } from "zod";

// ---------------------------------------------------------------
// PolicyProfile (Milestone 8)
// Optional per-run policy configuration.
// TypeScript validates structure only — policy logic stays in Rust.
// ---------------------------------------------------------------
export const PolicyProfileInput = z.object({
  patchEditThreshold: z
    .number()
    .int()
    .min(1)
    .optional()
    .describe(
      "Max edits in a single patch before approval is required (default: 5)",
    ),
  deleteRequiresApproval: z
    .boolean()
    .optional()
    .describe("Whether file deletions require approval (default: true)"),
  sensitivePathRequiresApproval: z
    .boolean()
    .optional()
    .describe(
      "Whether edits to sensitive paths require approval (default: true)",
    ),
  outsideFocusRequiresApproval: z
    .boolean()
    .optional()
    .describe(
      "Whether edits outside focus paths require approval (default: true)",
    ),
  extraSafeMakeTargets: z
    .array(z.string().min(1))
    .optional()
    .describe("Additional make targets considered safe beyond the built-in list"),
  focusPaths: z
    .array(z.string())
    .optional()
    .describe("Focus paths for this run (overrides top-level focusPaths)"),
});

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
  // Optional per-run policy configuration (Milestone 8).
  // If omitted, deterministic defaults are applied.
  policy: PolicyProfileInput.optional().describe(
    "Optional per-run policy configuration. Deterministic defaults apply if omitted.",
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
// list_runs  (Milestone 7)
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
