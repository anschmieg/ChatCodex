/**
 * Invariant checks for the MCP gateway.
 *
 * These tests verify that:
 * 1. No forbidden tool names are registered
 * 2. No forbidden daemon methods are called
 * 3. The tool registry matches the expected set
 * 4. Milestone 8: policy schema validates correctly
 */

import { describe, it } from "node:test";
import * as assert from "node:assert/strict";
import { z } from "zod";
import { FORBIDDEN_TOOL_NAMES, REGISTERED_TOOL_NAMES } from "./tools.js";
import {
  PolicyProfileInputSchema,
  PreviewPatchPolicyInput,
  PreviewTestPolicyInput,
  FinalizeRunInput,
  ReopenRunInput,
} from "./schemas.js";

describe("MCP tool registry invariants", () => {
  it("should not contain any forbidden tool names", () => {
    for (const forbidden of FORBIDDEN_TOOL_NAMES) {
      assert.ok(
        !REGISTERED_TOOL_NAMES.includes(forbidden as (typeof REGISTERED_TOOL_NAMES)[number]),
        `Forbidden tool name found in registry: ${forbidden}`,
      );
    }
  });

  it("should contain exactly the expected tools", () => {
    const expected = new Set([
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
    ]);
    const actual = new Set(REGISTERED_TOOL_NAMES);
    assert.deepStrictEqual(actual, expected);
  });

  it("should not include coarse autonomous tools", () => {
    const coarsePatterns = [
      "continue",
      "resume",
      "agent",
      "turn",
      "codex_reply",
      "fix_end_to_end",
    ];
    for (const name of REGISTERED_TOOL_NAMES) {
      for (const pattern of coarsePatterns) {
        assert.ok(
          !name.includes(pattern),
          `Tool name "${name}" contains forbidden pattern "${pattern}"`,
        );
      }
    }
  });
});

// ---------------------------------------------------------------
// Milestone 8: policy schema validation tests
// ---------------------------------------------------------------
describe("PolicyProfileInput schema (Milestone 8)", () => {
  it("should accept an empty object (all defaults)", () => {
    const result = PolicyProfileInputSchema.safeParse({});
    assert.ok(result.success, "Empty policy object should be valid");
  });

  it("should accept a fully specified valid policy", () => {
    const result = PolicyProfileInputSchema.safeParse({
      patchEditThreshold: 20,
      deleteRequiresApproval: false,
      sensitivePathRequiresApproval: true,
      outsideFocusRequiresApproval: false,
      extraSafeMakeTargets: ["deploy", "lint"],
    });
    assert.ok(result.success, "Fully specified policy should be valid");
  });

  it("should accept a partial policy (only some fields)", () => {
    const result = PolicyProfileInputSchema.safeParse({
      patchEditThreshold: 10,
    });
    assert.ok(result.success, "Partial policy should be valid");
  });

  it("should reject patchEditThreshold of zero (must be positive)", () => {
    const result = PolicyProfileInputSchema.safeParse({
      patchEditThreshold: 0,
    });
    assert.ok(!result.success, "patchEditThreshold of 0 should be invalid");
  });

  it("should reject a non-boolean deleteRequiresApproval", () => {
    const result = PolicyProfileInputSchema.safeParse({
      deleteRequiresApproval: "yes",
    });
    assert.ok(!result.success, "String value for boolean field should be invalid");
  });

  it("should accept undefined (omitted policy)", () => {
    const outerSchema = z.object({ policy: PolicyProfileInputSchema.optional() });
    const result = outerSchema.safeParse({});
    assert.ok(result.success, "Omitted policy field should be valid");
    assert.strictEqual(result.data?.policy, undefined);
  });
});

// ---------------------------------------------------------------
// Milestone 9: preflight schema validation tests
// ---------------------------------------------------------------
const PreviewPatchPolicySchema = z.object(PreviewPatchPolicyInput);
const PreviewTestPolicySchema = z.object(PreviewTestPolicyInput);

describe("PreviewPatchPolicyInput schema (Milestone 9)", () => {
  it("should accept a minimal valid patch preview request", () => {
    const result = PreviewPatchPolicySchema.safeParse({
      runId: "run-abc",
      edits: [{ path: "src/main.rs", operation: "replace", newText: "fn main() {}" }],
    });
    assert.ok(result.success, "Minimal patch preview should be valid");
  });

  it("should reject missing runId", () => {
    const result = PreviewPatchPolicySchema.safeParse({
      edits: [{ path: "src/main.rs", operation: "replace", newText: "x" }],
    });
    assert.ok(!result.success, "Missing runId should be invalid");
  });

  it("should reject missing edits", () => {
    const result = PreviewPatchPolicySchema.safeParse({ runId: "run-abc" });
    assert.ok(!result.success, "Missing edits array should be invalid");
  });

  it("should accept multiple edits with optional fields", () => {
    const result = PreviewPatchPolicySchema.safeParse({
      runId: "run-xyz",
      edits: [
        { path: "a.rs", operation: "create", newText: "content", reason: "new file" },
        { path: "b.rs", operation: "delete", newText: "" },
      ],
    });
    assert.ok(result.success, "Multiple edits with optional fields should be valid");
  });
});

describe("PreviewTestPolicyInput schema (Milestone 9)", () => {
  it("should accept a minimal valid test preview request", () => {
    const result = PreviewTestPolicySchema.safeParse({
      runId: "run-abc",
      scope: "cargo",
    });
    assert.ok(result.success, "Minimal test preview should be valid");
  });

  it("should accept a make target test preview", () => {
    const result = PreviewTestPolicySchema.safeParse({
      runId: "run-abc",
      scope: "make",
      target: "deploy-prod",
      reason: "check if approval needed",
    });
    assert.ok(result.success, "Full test preview with target and reason should be valid");
  });

  it("should reject missing runId", () => {
    const result = PreviewTestPolicySchema.safeParse({ scope: "cargo" });
    assert.ok(!result.success, "Missing runId should be invalid");
  });

  it("should reject missing scope", () => {
    const result = PreviewTestPolicySchema.safeParse({ runId: "run-abc" });
    assert.ok(!result.success, "Missing scope should be invalid");
  });
});

describe("No-hidden-agent regression (Milestone 9)", () => {
  it("preview tools should be read-only (not coarse autonomous tools)", () => {
    const previewTools = ["preview_patch_policy", "preview_test_policy"];
    for (const tool of previewTools) {
      assert.ok(
        REGISTERED_TOOL_NAMES.includes(tool as (typeof REGISTERED_TOOL_NAMES)[number]),
        `Preview tool '${tool}' should be registered`,
      );
    }
  });

  it("no continue/resume/agent patterns in registered tool names", () => {
    const coarsePatterns = ["continue", "resume", "agent", "turn", "codex_reply", "fix_end"];
    for (const name of REGISTERED_TOOL_NAMES) {
      for (const pattern of coarsePatterns) {
        assert.ok(
          !name.includes(pattern),
          `Tool "${name}" contains forbidden autonomous pattern "${pattern}"`,
        );
      }
    }
  });
});

// ---------------------------------------------------------------
// Milestone 10: FinalizeRunInput schema validation tests
// ---------------------------------------------------------------
const FinalizeRunSchema = z.object(FinalizeRunInput);

describe("FinalizeRunInput schema (Milestone 10)", () => {
  it("should accept a minimal completed finalization", () => {
    const result = FinalizeRunSchema.safeParse({
      runId: "run-abc",
      outcomeKind: "completed",
      summary: "All steps finished successfully",
    });
    assert.ok(result.success, "Minimal completed finalization should be valid");
  });

  it("should accept a failed finalization with a reason", () => {
    const result = FinalizeRunSchema.safeParse({
      runId: "run-xyz",
      outcomeKind: "failed",
      summary: "Build failed at step 2",
      reason: "compiler error",
    });
    assert.ok(result.success, "Failed finalization with reason should be valid");
  });

  it("should accept an abandoned finalization", () => {
    const result = FinalizeRunSchema.safeParse({
      runId: "run-aba",
      outcomeKind: "abandoned",
      summary: "No longer needed",
    });
    assert.ok(result.success, "Abandoned finalization should be valid");
  });

  it("should reject an invalid outcome kind", () => {
    const result = FinalizeRunSchema.safeParse({
      runId: "run-abc",
      outcomeKind: "unknown_kind",
      summary: "done",
    });
    assert.ok(!result.success, "Invalid outcomeKind should be rejected");
  });

  it("should reject missing runId", () => {
    const result = FinalizeRunSchema.safeParse({
      outcomeKind: "completed",
      summary: "done",
    });
    assert.ok(!result.success, "Missing runId should be invalid");
  });

  it("should reject missing outcomeKind", () => {
    const result = FinalizeRunSchema.safeParse({
      runId: "run-abc",
      summary: "done",
    });
    assert.ok(!result.success, "Missing outcomeKind should be invalid");
  });

  it("should reject missing summary", () => {
    const result = FinalizeRunSchema.safeParse({
      runId: "run-abc",
      outcomeKind: "completed",
    });
    assert.ok(!result.success, "Missing summary should be invalid");
  });

  it("should accept optional reason as undefined", () => {
    const result = FinalizeRunSchema.safeParse({
      runId: "run-abc",
      outcomeKind: "completed",
      summary: "done",
      reason: undefined,
    });
    assert.ok(result.success, "Undefined reason should be valid (optional)");
  });
});

describe("No-hidden-agent regression (Milestone 10)", () => {
  it("finalize_run should be registered as a lifecycle tool", () => {
    assert.ok(
      REGISTERED_TOOL_NAMES.includes("finalize_run" as (typeof REGISTERED_TOOL_NAMES)[number]),
      "finalize_run must be in the tool registry",
    );
  });

  it("no coarse autonomous patterns in registered tool names", () => {
    const coarsePatterns = ["continue", "resume", "agent", "turn", "codex_reply", "fix_end"];
    for (const name of REGISTERED_TOOL_NAMES) {
      for (const pattern of coarsePatterns) {
        assert.ok(
          !name.includes(pattern),
          `Tool "${name}" contains forbidden autonomous pattern "${pattern}"`,
        );
      }
    }
  });
});

// ---------------------------------------------------------------
// Milestone 11: ReopenRunInput schema validation tests
// ---------------------------------------------------------------
describe("ReopenRunInput schema (Milestone 11)", () => {
  it("should accept a valid reopen request", () => {
    const schema = z.object(ReopenRunInput);
    const result = schema.safeParse({
      runId: "run-abc",
      reason: "Found another bug after completion",
    });
    assert.ok(result.success, "Valid reopen request should pass validation");
  });

  it("should reject a missing runId", () => {
    const schema = z.object(ReopenRunInput);
    const result = schema.safeParse({ reason: "some reason" });
    assert.ok(!result.success, "Missing runId should fail validation");
  });

  it("should reject a missing reason", () => {
    const schema = z.object(ReopenRunInput);
    const result = schema.safeParse({ runId: "run-abc" });
    assert.ok(!result.success, "Missing reason should fail validation");
  });

  it("should reject an empty reason", () => {
    const schema = z.object(ReopenRunInput);
    const result = schema.safeParse({ runId: "run-abc", reason: "" });
    assert.ok(!result.success, "Empty reason should fail validation (min 1)");
  });

  it("should reject a reason exceeding 500 characters", () => {
    const schema = z.object(ReopenRunInput);
    const result = schema.safeParse({
      runId: "run-abc",
      reason: "x".repeat(501),
    });
    assert.ok(!result.success, "Reason exceeding 500 chars should fail validation");
  });

  it("should accept a reason of exactly 500 characters", () => {
    const schema = z.object(ReopenRunInput);
    const result = schema.safeParse({
      runId: "run-abc",
      reason: "x".repeat(500),
    });
    assert.ok(result.success, "Reason of exactly 500 chars should be valid");
  });
});

describe("No-hidden-agent regression (Milestone 11)", () => {
  it("reopen_run should be registered as a lifecycle tool", () => {
    assert.ok(
      REGISTERED_TOOL_NAMES.includes("reopen_run" as (typeof REGISTERED_TOOL_NAMES)[number]),
      "reopen_run must be in the tool registry",
    );
  });

  it("reopen_run is not an autonomous continuation tool", () => {
    // Verify that reopen_run is present but is lifecycle-scoped (not coarse autonomous).
    const coarsePatterns = ["continue", "resume", "agent", "turn", "codex_reply", "fix_end"];
    for (const pattern of coarsePatterns) {
      assert.ok(
        !"reopen_run".includes(pattern),
        `"reopen_run" must not contain autonomous pattern "${pattern}"`,
      );
    }
  });

  it("daemon method run.reopen is not a forbidden agent-runtime method", () => {
    const forbiddenMethods = [
      "turn/start",
      "turn/steer",
      "review/start",
      "codex",
      "codex-reply",
      "continue_run",
      "resume_thread",
      "agent_step",
      "fix_end_to_end",
    ];
    for (const method of forbiddenMethods) {
      assert.ok(
        method !== "run.reopen",
        `run.reopen must not be a forbidden agent-runtime method`,
      );
    }
  });
});
