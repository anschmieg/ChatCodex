/**
 * MCP tool registration for the deterministic ChatGPT control plane.
 *
 * Each tool:
 *  1. Validates inputs via Zod schema
 *  2. Maps to the daemon JSON-RPC method
 *  3. Formats the response for MCP
 *
 * No core logic lives here.
 */

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { DaemonClient } from "./daemon-client.js";
import {
  CodexPrepareRunInput,
  GetWorkspaceSummaryInput,
  ReadFileInput,
  GitStatusInput,
  SearchCodeInput,
  ApplyPatchInput,
  RunTestsInput,
  ShowDiffInput,
  RefreshRunStateInput,
  ReplanRunInput,
  ApproveActionInput,
  ListRunsInput,
  GetRunStateInput,
  GetRunHistoryInput,
  PreviewPatchPolicyInput,
  PreviewTestPolicyInput,
  FinalizeRunInput,
  ReopenRunInput,
  SupersedeRunInput,
  ArchiveRunInput,
  UnarchiveRunInput,
  AnnotateRunInput,
  PinRunInput,
  UnpinRunInput,
  SnoozeRunInput,
  UnsnoozeRunInput,
  SetRunPriorityInput,
  AssignRunOwnerInput,
  SetRunDueDateInput,
  GetQueueOverviewInput,
  CreateQueueViewInput,
  UpdateQueueViewInput,
  DeleteQueueViewInput,
  GetQueueViewInput,
  ListQueueViewsInput,
} from "./schemas.js";

/**
 * Strings that must NEVER appear as tool names.
 * Checked at registration time and in tests.
 */
export const FORBIDDEN_TOOL_NAMES = [
  "continue_run",
  "resume_codex_thread",
  "fix_end_to_end",
  "agent_step",
  "turn_start",
  "codex_reply",
  "codex",
  "resume_thread",
] as const;

/**
 * The set of tool names we actually register.
 * Exported so tests can inspect it.
 */
export const REGISTERED_TOOL_NAMES = [
  "codex_prepare_run",
  "get_workspace_summary",
  "read_file",
  "git_status",
  "search_code",
  "apply_patch",
  "run_tests",
  "show_diff",
  "refresh_run_state",
  "replan_run",
  "approve_action",
  // Milestone 7: read-only history and state inspection
  "list_runs",
  "get_run_state",
  "get_run_history",
  // Milestone 9: deterministic preflight / preview (read-only)
  "preview_patch_policy",
  "preview_test_policy",
  // Milestone 10: deterministic run finalization
  "finalize_run",
  // Milestone 11: deterministic run reopening
  "reopen_run",
  // Milestone 12: deterministic run supersession
  "supersede_run",
  // Milestone 13: deterministic run archiving
  "archive_run",
  // Milestone 14: deterministic run unarchiving
  "unarchive_run",
  // Milestone 15: deterministic run labeling / annotation
  "annotate_run",
  // Milestone 16: deterministic run pinning
  "pin_run",
  "unpin_run",
  // Milestone 17: deterministic run snoozing
  "snooze_run",
  "unsnooze_run",
  // Milestone 18: deterministic run priority
  "set_run_priority",
  // Milestone 19: deterministic run ownership
  "assign_run_owner",
  // Milestone 20: deterministic run due dates
  "set_run_due_date",
] as const;

export function registerTools(server: McpServer, client: DaemonClient): void {
  // ---- codex_prepare_run ----
  server.tool(
    "codex_prepare_run",
    "Initialize a deterministic coding run",
    CodexPrepareRunInput,
    async (params) => {
      const result = await client.call("run.prepare", {
        workspaceId: params.workspaceId,
        userGoal: params.userGoal,
        focusPaths: params.focusPaths ?? [],
        mode: params.mode,
        policy: params.policy,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- get_workspace_summary ----
  server.tool(
    "get_workspace_summary",
    "Get a deterministic summary of the workspace",
    GetWorkspaceSummaryInput,
    async (params) => {
      const result = await client.call("workspace.summary", {
        workspaceId: params.workspaceId,
        focusPaths: params.focusPaths ?? [],
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- read_file ----
  server.tool(
    "read_file",
    "Read file contents from the workspace",
    ReadFileInput,
    async (params) => {
      const result = await client.call("file.read", {
        runId: params.runId,
        path: params.path,
        startLine: params.startLine,
        endLine: params.endLine,
        purpose: params.purpose,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- git_status ----
  server.tool(
    "git_status",
    "Get git working tree status",
    GitStatusInput,
    async (params) => {
      const result = await client.call("git.status", {
        runId: params.runId,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- search_code ----
  server.tool(
    "search_code",
    "Search for text matches in the workspace",
    SearchCodeInput,
    async (params) => {
      const result = await client.call("code.search", {
        runId: params.runId,
        query: params.query,
        pathGlob: params.pathGlob,
        maxResults: params.maxResults,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- apply_patch ----
  server.tool(
    "apply_patch",
    "Apply file edits to the workspace (all file writes go through here)",
    ApplyPatchInput,
    async (params) => {
      const result = await client.call("patch.apply", {
        runId: params.runId,
        edits: params.edits,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- run_tests ----
  server.tool(
    "run_tests",
    "Execute a whitelisted test command in the workspace",
    RunTestsInput,
    async (params) => {
      const result = await client.call("tests.run", {
        runId: params.runId,
        scope: params.scope,
        target: params.target,
        reason: params.reason,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- show_diff ----
  server.tool(
    "show_diff",
    "Show git diff for the workspace",
    ShowDiffInput,
    async (params) => {
      const result = await client.call("git.diff", {
        runId: params.runId,
        paths: params.paths ?? [],
        format: params.format,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- refresh_run_state ----
  server.tool(
    "refresh_run_state",
    "Refresh and return the current run state snapshot (read-only, no side effects)",
    RefreshRunStateInput,
    async (params) => {
      const result = await client.call("run.refresh", {
        runId: params.runId,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- replan_run ----
  server.tool(
    "replan_run",
    "Deterministically replan the run based on new evidence or failure context",
    ReplanRunInput,
    async (params) => {
      const result = await client.call("run.replan", {
        runId: params.runId,
        reason: params.reason,
        newEvidence: params.newEvidence ?? [],
        failureContext: params.failureContext,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- approve_action ----
  server.tool(
    "approve_action",
    "Resolve a pending approval (approve or deny a risky action)",
    ApproveActionInput,
    async (params) => {
      const result = await client.call("approval.resolve", {
        runId: params.runId,
        approvalId: params.approvalId,
        decision: params.decision,
        reason: params.reason,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- list_runs (Milestone 7; extended in Milestone 13, 15, 16, 17, 20, 23) ----
  server.tool(
    "list_runs",
    "List known runs with status and metadata (read-only). Supports archive filtering via optional parameters: set includeArchived=true to include archived runs, or archivedOnly=true to return only archived runs. Use label= to filter by exact normalized label. Use pinnedOnly=true to return only pinned runs. Pinned runs appear first by default. Snoozed runs are excluded by default; use includeSnoozed=true to include them or snoozedOnly=true to return only snoozed runs. Use dueOnOrBefore=YYYY-MM-DD to filter by due date. Use sortByDueDate=true to sort ascending by due date (soonest first; undated runs last). Use blockingOnly=true to return only runs that are blocking other runs. Use blockingRunCountAtLeast=N to return only runs blocking at least N other runs.",
    ListRunsInput,
    async (params) => {
      const result = await client.call("runs.list", {
        limit: params.limit,
        workspaceId: params.workspaceId,
        status: params.status,
        includeArchived: params.includeArchived,
        archivedOnly: params.archivedOnly,
        label: params.label,
        pinnedOnly: params.pinnedOnly,
        includeSnoozed: params.includeSnoozed,
        snoozedOnly: params.snoozedOnly,
        dueOnOrBefore: params.dueOnOrBefore,
        sortByDueDate: params.sortByDueDate,
        blockingOnly: params.blockingOnly,
        blockingRunCountAtLeast: params.blockingRunCountAtLeast,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- get_run_state (Milestone 7) ----
  server.tool(
    "get_run_state",
    "Get the authoritative current state of a run including pending approvals and retryable actions (read-only)",
    GetRunStateInput,
    async (params) => {
      const result = await client.call("run.get", {
        runId: params.runId,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- get_run_history (Milestone 7) ----
  server.tool(
    "get_run_history",
    "Get the audit trail of key events for a run (read-only)",
    GetRunHistoryInput,
    async (params) => {
      const result = await client.call("run.history", {
        runId: params.runId,
        limit: params.limit,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- preview_patch_policy (Milestone 9) ----
  server.tool(
    "preview_patch_policy",
    "Preview the policy decision for a proposed patch without applying any changes (read-only)",
    PreviewPatchPolicyInput,
    async (params) => {
      const result = await client.call("patch.preflight", {
        runId: params.runId,
        edits: params.edits,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- preview_test_policy (Milestone 9) ----
  server.tool(
    "preview_test_policy",
    "Preview the policy decision for a proposed test run without executing tests (read-only)",
    PreviewTestPolicyInput,
    async (params) => {
      const result = await client.call("tests.preflight", {
        runId: params.runId,
        scope: params.scope,
        target: params.target,
        reason: params.reason,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- finalize_run (Milestone 10) ----
  server.tool(
    "finalize_run",
    "Explicitly finalize a run with a structured outcome record (completed, failed, or abandoned). Persists final outcome. No autonomous work is triggered.",
    FinalizeRunInput,
    async (params) => {
      const result = await client.call("run.finalize", {
        runId: params.runId,
        outcomeKind: params.outcomeKind,
        summary: params.summary,
        reason: params.reason,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- reopen_run (Milestone 11) ----
  server.tool(
    "reopen_run",
    "Reopen a previously finalized run (completed, failed, or abandoned) for deterministic continuation. Requires an explicit reason for auditability. Reopening does not execute work; it transitions the run back to active status and records reopen metadata. Active or prepared runs cannot be reopened.",
    ReopenRunInput,
    async (params) => {
      const result = await client.call("run.reopen", {
        runId: params.runId,
        reason: params.reason,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- supersede_run (Milestone 12) ----
  server.tool(
    "supersede_run",
    "Create a new successor run that explicitly replaces a finalized run. The original run remains preserved with its full audit history and plan. Only finalized runs (completed, failed, or abandoned) may be superseded. Supersession does not execute work; it creates a successor run in 'prepared' status and records lineage metadata on both runs.",
    SupersedeRunInput,
    async (params) => {
      const result = await client.call("run.supersede", {
        runId: params.runId,
        newUserGoal: params.newUserGoal,
        reason: params.reason,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- archive_run (Milestone 13) ----
  server.tool(
    "archive_run",
    "Explicitly archive a finalized run so it remains preserved and inspectable but is excluded from the default active run listing. Only finalized runs (completed, failed, or abandoned) may be archived. Archiving is deterministic and audited; it does not execute work or reopen the run. Archived runs can still be read via get_run_state, get_run_history, and list_runs (with includeArchived=true or archivedOnly=true).",
    ArchiveRunInput,
    async (params) => {
      const result = await client.call("run.archive", {
        runId: params.runId,
        reason: params.reason,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- unarchive_run (Milestone 14) ----
  server.tool(
    "unarchive_run",
    "Explicitly unarchive (restore) an archived run so it returns to the default active run listing. Only archived runs may be unarchived; non-archived runs are rejected. Unarchiving is deterministic and audited; it does not execute work, reopen the run, or change the finalized outcome. The original archive metadata remains intact for historical inspection alongside the new unarchive metadata.",
    UnarchiveRunInput,
    async (params) => {
      const result = await client.call("run.unarchive", {
        runId: params.runId,
        reason: params.reason,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- annotate_run (Milestone 15) ----
  server.tool(
    "annotate_run",
    "Explicitly annotate a run with compact organization metadata: zero or more labels and/or an optional operator note. " +
      "Labels are normalized to lowercase, deduplicated, and sorted. " +
      "At least one of labels or operatorNote must be provided. " +
      "Annotating does not execute work, replan, reopen, finalize, archive, unarchive, or supersede the run. " +
      "Metadata is persisted and visible in get_run_state and list_runs. " +
      "Use label filtering in list_runs to retrieve runs by label.",
    AnnotateRunInput,
    async (params) => {
      const result = await client.call("run.annotate", {
        runId: params.runId,
        labels: params.labels,
        operatorNote: params.operatorNote,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- pin_run (Milestone 16) ----
  server.tool(
    "pin_run",
    "Explicitly pin a run to keep it prominent in the working set. " +
      "Pinning is deterministic and audited. " +
      "It updates only pin metadata and does not execute work, change status, replan, reopen, finalize, archive, unarchive, or supersede the run. " +
      "Pinned runs appear first in list_runs by default. " +
      "Use pinnedOnly=true in list_runs to return only pinned runs.",
    PinRunInput,
    async (params) => {
      const result = await client.call("run.pin", {
        runId: params.runId,
        reason: params.reason,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- unpin_run (Milestone 16) ----
  server.tool(
    "unpin_run",
    "Explicitly unpin a previously pinned run to remove it from the prominent working-set position. " +
      "Unpinning is deterministic and audited. " +
      "It clears pin metadata only and does not execute work, change status, replan, reopen, finalize, archive, unarchive, or supersede the run. " +
      "Only pinned runs can be unpinned.",
    UnpinRunInput,
    async (params) => {
      const result = await client.call("run.unpin", {
        runId: params.runId,
        reason: params.reason,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- snooze_run (Milestone 17) ----
  server.tool(
    "snooze_run",
    "Explicitly snooze a run to temporarily defer it out of the default visible working set without archiving it. " +
      "Snoozing is deterministic and audited. " +
      "It updates only snooze metadata and does not execute work, change lifecycle status, replan, reopen, finalize, archive, unarchive, or supersede the run. " +
      "Snoozed runs are excluded from list_runs by default; use includeSnoozed=true or snoozedOnly=true to include them. " +
      "Any run regardless of lifecycle status may be snoozed. Re-snoozing a snoozed run replaces the existing snooze metadata.",
    SnoozeRunInput,
    async (params) => {
      const result = await client.call("run.snooze", {
        runId: params.runId,
        reason: params.reason,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- unsnooze_run (Milestone 17) ----
  server.tool(
    "unsnooze_run",
    "Explicitly unsnooze a snoozed run to restore it to the default visible working set. " +
      "Unsnoozing is deterministic and audited. " +
      "It clears snooze metadata only and does not execute work, change lifecycle status, replan, reopen, finalize, archive, unarchive, or supersede the run. " +
      "Only snoozed runs can be unsnoozed; non-snoozed runs are rejected. " +
      "After unsnoozing, the run reappears in the default list_runs result.",
    UnsnoozeRunInput,
    async (params) => {
      const result = await client.call("run.unsnooze", {
        runId: params.runId,
        reason: params.reason,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- set_run_priority (Milestone 18) ----
  server.tool(
    "set_run_priority",
    "Explicitly set the priority level of a run. " +
      "Valid levels are: critical, high, normal, low. " +
      "Priority is deterministic and audited. " +
      "It changes only priority metadata and does not execute work, change lifecycle status, replan, reopen, finalize, archive, unarchive, snooze, or supersede the run.",
    SetRunPriorityInput,
    async (params) => {
      const result = await client.call("run.set_priority", {
        runId: params.runId,
        priority: params.priority,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- assign_run_owner (Milestone 19) ----
  server.tool(
    "assign_run_owner",
    "Explicitly assign or clear the owner (assignee) and optional ownership note of a run. " +
      "Ownership assignment is deterministic and audited. " +
      "It changes only owner/note metadata and does not execute work, change lifecycle status, replan, reopen, finalize, archive, unarchive, snooze, or supersede the run. " +
      "Pass assignee=null to clear the assignee. Pass ownershipNote=null to clear the note.",
    AssignRunOwnerInput,
    async (params) => {
      const result = await client.call("run.assign_owner", {
        runId: params.runId,
        assignee: params.assignee,
        ownershipNote: params.ownershipNote,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---- set_run_due_date (Milestone 20) ----
  server.tool(
    "set_run_due_date",
    "Explicitly set or clear the due date of a run. " +
      "The due date is an ISO YYYY-MM-DD string with no time-of-day or timezone semantics. " +
      "Due-date assignment is deterministic and audited. " +
      "It changes only due-date metadata and does not execute work, change lifecycle status, replan, reopen, finalize, archive, unarchive, snooze, prioritize, or supersede the run. " +
      "Pass dueDate=null to clear the due date.",
    SetRunDueDateInput,
    async (params) => {
      const result = await client.call("run.set_due_date", {
        runId: params.runId,
        dueDate: params.dueDate,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // runs.overview (Milestone 24/28)
  server.tool(
    "get_run_queue_overview",
    "Get a deterministic queue overview summary with compact counts: " +
      "total visible runs, ready, blocked, deferred, done, attention, urgent, overdue, stale, pinned, assigned vs unassigned. " +
      "This is a read-only inspection operation that derives summary counts from existing run state without mutating anything.",
    GetQueueOverviewInput,
    async (params) => {
      const result = await client.call("runs.overview", {
        workspaceId: params.workspaceId,
        includeArchived: params.includeArchived,
        includeSnoozed: params.includeSnoozed,
        today: params.today,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  // ---------------------------------------------------------------
  // Queue View CRUD (Milestone 29)
  // ---------------------------------------------------------------

  server.tool(
    "create_queue_view",
    "Create a saved queue view with deterministic filter/sort configuration. " +
      "The view can be applied to runs.list or runs.overview to reuse common queue slices. " +
      "Names must be unique (case-insensitive).",
    CreateQueueViewInput,
    async (params) => {
      const result = await client.call("queue_view.create", {
        name: params.name,
        description: params.description,
        filters: params.filters,
        sort: params.sort,
        limit: params.limit,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  server.tool(
    "update_queue_view",
    "Update a saved queue view. Only provided fields are updated; others remain unchanged. " +
      "Name uniqueness is enforced on update.",
    UpdateQueueViewInput,
    async (params) => {
      const result = await client.call("queue_view.update", {
        viewId: params.viewId,
        name: params.name,
        description: params.description,
        filters: params.filters,
        sort: params.sort,
        limit: params.limit,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  server.tool(
    "delete_queue_view",
    "Delete a saved queue view by ID.",
    DeleteQueueViewInput,
    async (params) => {
      const result = await client.call("queue_view.delete", {
        viewId: params.viewId,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  server.tool(
    "get_queue_view",
    "Get a saved queue view definition by ID.",
    GetQueueViewInput,
    async (params) => {
      const result = await client.call("queue_view.get", {
        viewId: params.viewId,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );

  server.tool(
    "list_queue_views",
    "List all saved queue views, optionally filtered by name.",
    ListQueueViewsInput,
    async (params) => {
      const result = await client.call("queue_view.list", {
        nameContains: params.nameContains,
      });
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    },
  );
}
