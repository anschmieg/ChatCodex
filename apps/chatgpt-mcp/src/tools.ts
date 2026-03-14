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
}
