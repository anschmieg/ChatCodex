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
  SupersedeRunInput,
  ArchiveRunInput,
  UnarchiveRunInput,
  ListRunsInput,
  AnnotateRunInput,
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
      // Milestone 12: deterministic run supersession
      "supersede_run",
      // Milestone 13: deterministic run archiving
      "archive_run",
      // Milestone 14: deterministic run unarchiving
      "unarchive_run",
      // Milestone 15: deterministic run labeling / annotation
      "annotate_run",
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

// ---------------------------------------------------------------
// Milestone 12: SupersedeRunInput schema validation tests
// ---------------------------------------------------------------
describe("SupersedeRunInput schema (Milestone 12)", () => {
  it("should accept a minimal supersede request (no new goal)", () => {
    const schema = z.object(SupersedeRunInput);
    const result = schema.safeParse({
      runId: "run-orig",
      reason: "Scope changed after completion",
    });
    assert.ok(result.success, "Minimal supersede request should be valid");
  });

  it("should accept a supersede request with a new goal", () => {
    const schema = z.object(SupersedeRunInput);
    const result = schema.safeParse({
      runId: "run-orig",
      newUserGoal: "Fix the same bug with a better approach",
      reason: "Previous approach failed",
    });
    assert.ok(result.success, "Supersede request with new goal should be valid");
  });

  it("should reject a missing runId", () => {
    const schema = z.object(SupersedeRunInput);
    const result = schema.safeParse({ reason: "some reason" });
    assert.ok(!result.success, "Missing runId should fail validation");
  });

  it("should reject a missing reason", () => {
    const schema = z.object(SupersedeRunInput);
    const result = schema.safeParse({ runId: "run-orig" });
    assert.ok(!result.success, "Missing reason should fail validation");
  });

  it("should reject an empty reason", () => {
    const schema = z.object(SupersedeRunInput);
    const result = schema.safeParse({ runId: "run-orig", reason: "" });
    assert.ok(!result.success, "Empty reason should fail validation (min 1)");
  });

  it("should reject a reason exceeding 500 characters", () => {
    const schema = z.object(SupersedeRunInput);
    const result = schema.safeParse({
      runId: "run-orig",
      reason: "x".repeat(501),
    });
    assert.ok(!result.success, "Reason exceeding 500 chars should fail validation");
  });

  it("should accept a reason of exactly 500 characters", () => {
    const schema = z.object(SupersedeRunInput);
    const result = schema.safeParse({
      runId: "run-orig",
      reason: "x".repeat(500),
    });
    assert.ok(result.success, "Reason of exactly 500 chars should be valid");
  });

  it("should reject a newUserGoal exceeding 500 characters", () => {
    const schema = z.object(SupersedeRunInput);
    const result = schema.safeParse({
      runId: "run-orig",
      reason: "valid reason",
      newUserGoal: "x".repeat(501),
    });
    assert.ok(!result.success, "newUserGoal exceeding 500 chars should fail validation");
  });

  it("should accept omitted newUserGoal (inherits from original)", () => {
    const schema = z.object(SupersedeRunInput);
    const result = schema.safeParse({ runId: "run-orig", reason: "reason" });
    assert.ok(result.success, "Omitted newUserGoal should be valid (optional)");
    assert.strictEqual(result.data?.newUserGoal, undefined);
  });
});

describe("No-hidden-agent regression (Milestone 12)", () => {
  it("supersede_run should be registered as a lifecycle tool", () => {
    assert.ok(
      REGISTERED_TOOL_NAMES.includes("supersede_run" as (typeof REGISTERED_TOOL_NAMES)[number]),
      "supersede_run must be in the tool registry",
    );
  });

  it("supersede_run is not an autonomous continuation tool", () => {
    const coarsePatterns = ["continue", "resume", "agent", "turn", "codex_reply", "fix_end"];
    for (const pattern of coarsePatterns) {
      assert.ok(
        !"supersede_run".includes(pattern),
        `"supersede_run" must not contain autonomous pattern "${pattern}"`,
      );
    }
  });

  it("daemon method run.supersede is not a forbidden agent-runtime method", () => {
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
        method !== "run.supersede",
        `run.supersede must not be a forbidden agent-runtime method`,
      );
    }
  });
});

// ---------------------------------------------------------------
// Milestone 13: ArchiveRunInput schema validation
// ---------------------------------------------------------------
describe("ArchiveRunInput schema (Milestone 13)", () => {
  it("should accept a valid archive request", () => {
    const schema = z.object(ArchiveRunInput);
    const result = schema.safeParse({ runId: "run-abc", reason: "Archiving completed run" });
    assert.ok(result.success, "Valid archive request should pass validation");
  });

  it("should reject missing runId", () => {
    const schema = z.object(ArchiveRunInput);
    const result = schema.safeParse({ reason: "reason" });
    assert.ok(!result.success, "Missing runId should fail validation");
  });

  it("should reject missing reason", () => {
    const schema = z.object(ArchiveRunInput);
    const result = schema.safeParse({ runId: "run-abc" });
    assert.ok(!result.success, "Missing reason should fail validation");
  });

  it("should reject an empty reason", () => {
    const schema = z.object(ArchiveRunInput);
    const result = schema.safeParse({ runId: "run-abc", reason: "" });
    assert.ok(!result.success, "Empty reason should fail validation (min 1)");
  });

  it("should reject a reason exceeding 500 characters", () => {
    const schema = z.object(ArchiveRunInput);
    const result = schema.safeParse({ runId: "run-abc", reason: "x".repeat(501) });
    assert.ok(!result.success, "Reason exceeding 500 chars should fail validation");
  });

  it("should accept a reason of exactly 500 characters", () => {
    const schema = z.object(ArchiveRunInput);
    const result = schema.safeParse({ runId: "run-abc", reason: "x".repeat(500) });
    assert.ok(result.success, "Reason of exactly 500 chars should be valid");
  });
});

// ---------------------------------------------------------------
// Milestone 13: ListRunsInput archive filtering schema
// ---------------------------------------------------------------
describe("ListRunsInput archive filtering (Milestone 13)", () => {
  it("should accept includeArchived=true", () => {
    const schema = z.object(ListRunsInput);
    const result = schema.safeParse({ includeArchived: true });
    assert.ok(result.success, "includeArchived=true should be valid");
    assert.strictEqual(result.data?.includeArchived, true);
  });

  it("should accept archivedOnly=true", () => {
    const schema = z.object(ListRunsInput);
    const result = schema.safeParse({ archivedOnly: true });
    assert.ok(result.success, "archivedOnly=true should be valid");
    assert.strictEqual(result.data?.archivedOnly, true);
  });

  it("should accept both flags together", () => {
    const schema = z.object(ListRunsInput);
    const result = schema.safeParse({ includeArchived: true, archivedOnly: true });
    assert.ok(result.success, "Both flags together should be valid schema-wise");
  });

  it("should default both flags to undefined when omitted", () => {
    const schema = z.object(ListRunsInput);
    const result = schema.safeParse({});
    assert.ok(result.success, "Empty input should be valid");
    assert.strictEqual(result.data?.includeArchived, undefined);
    assert.strictEqual(result.data?.archivedOnly, undefined);
  });

  it("should reject non-boolean includeArchived", () => {
    const schema = z.object(ListRunsInput);
    const result = schema.safeParse({ includeArchived: "yes" });
    assert.ok(!result.success, "Non-boolean includeArchived should fail validation");
  });
});

// ---------------------------------------------------------------
// Milestone 13: No-hidden-agent regression for archive_run
// ---------------------------------------------------------------
describe("No-hidden-agent regression (Milestone 13)", () => {
  it("archive_run should be registered as a lifecycle tool", () => {
    assert.ok(
      REGISTERED_TOOL_NAMES.includes("archive_run" as (typeof REGISTERED_TOOL_NAMES)[number]),
      "archive_run must be in the tool registry",
    );
  });

  it("archive_run is not an autonomous continuation tool", () => {
    const coarsePatterns = ["continue", "resume", "agent", "turn", "codex_reply", "fix_end"];
    for (const pattern of coarsePatterns) {
      assert.ok(
        !"archive_run".includes(pattern),
        `"archive_run" must not contain autonomous pattern "${pattern}"`,
      );
    }
  });

  it("daemon method run.archive is not a forbidden agent-runtime method", () => {
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
        method !== "run.archive",
        `run.archive must not be a forbidden agent-runtime method`,
      );
    }
  });
});

// ---------------------------------------------------------------
// Milestone 14: UnarchiveRunInput schema validation
// ---------------------------------------------------------------
describe("UnarchiveRunInput schema (Milestone 14)", () => {
  it("should accept a valid unarchive request", () => {
    const schema = z.object(UnarchiveRunInput);
    const result = schema.safeParse({ runId: "run-xyz", reason: "Restoring for follow-up inspection" });
    assert.ok(result.success, "Valid unarchive request should pass validation");
  });

  it("should reject missing runId", () => {
    const schema = z.object(UnarchiveRunInput);
    const result = schema.safeParse({ reason: "reason" });
    assert.ok(!result.success, "Missing runId should fail validation");
  });

  it("should reject missing reason", () => {
    const schema = z.object(UnarchiveRunInput);
    const result = schema.safeParse({ runId: "run-xyz" });
    assert.ok(!result.success, "Missing reason should fail validation");
  });

  it("should reject an empty reason", () => {
    const schema = z.object(UnarchiveRunInput);
    const result = schema.safeParse({ runId: "run-xyz", reason: "" });
    assert.ok(!result.success, "Empty reason should fail validation (min 1)");
  });

  it("should reject a reason exceeding 500 characters", () => {
    const schema = z.object(UnarchiveRunInput);
    const result = schema.safeParse({ runId: "run-xyz", reason: "x".repeat(501) });
    assert.ok(!result.success, "Reason exceeding 500 chars should fail validation");
  });

  it("should accept a reason of exactly 500 characters", () => {
    const schema = z.object(UnarchiveRunInput);
    const result = schema.safeParse({ runId: "run-xyz", reason: "x".repeat(500) });
    assert.ok(result.success, "Reason of exactly 500 chars should be valid");
  });
});

// ---------------------------------------------------------------
// Milestone 14: No-hidden-agent regression for unarchive_run
// ---------------------------------------------------------------
describe("No-hidden-agent regression (Milestone 14)", () => {
  it("unarchive_run should be registered as a lifecycle tool", () => {
    assert.ok(
      REGISTERED_TOOL_NAMES.includes("unarchive_run" as (typeof REGISTERED_TOOL_NAMES)[number]),
      "unarchive_run must be in the tool registry",
    );
  });

  it("unarchive_run is not an autonomous continuation tool", () => {
    const coarsePatterns = ["continue", "resume", "agent", "turn", "codex_reply", "fix_end"];
    for (const pattern of coarsePatterns) {
      assert.ok(
        !"unarchive_run".includes(pattern),
        `"unarchive_run" must not contain autonomous pattern "${pattern}"`,
      );
    }
  });

  it("daemon method run.unarchive is not a forbidden agent-runtime method", () => {
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
        method !== "run.unarchive",
        `run.unarchive must not be a forbidden agent-runtime method`,
      );
    }
  });
});

// ---------------------------------------------------------------
// Milestone 15: AnnotateRunInput schema
// ---------------------------------------------------------------
describe("AnnotateRunInput schema (Milestone 15)", () => {
  it("should accept labels-only annotation", () => {
    const schema = z.object(AnnotateRunInput);
    const result = schema.safeParse({ runId: "run-xyz", labels: ["auth", "ci"] });
    assert.ok(result.success, "Labels-only annotation should pass validation");
  });

  it("should accept operatorNote-only annotation", () => {
    const schema = z.object(AnnotateRunInput);
    const result = schema.safeParse({ runId: "run-xyz", operatorNote: "tracking regression" });
    assert.ok(result.success, "operatorNote-only annotation should pass validation");
  });

  it("should accept both labels and operatorNote", () => {
    const schema = z.object(AnnotateRunInput);
    const result = schema.safeParse({ runId: "run-xyz", labels: ["auth"], operatorNote: "note" });
    assert.ok(result.success, "labels+operatorNote should pass validation");
  });

  it("should reject a label with spaces", () => {
    const schema = z.object(AnnotateRunInput);
    const result = schema.safeParse({ runId: "run-xyz", labels: ["bad label"] });
    assert.ok(!result.success, "Label with spaces should fail validation");
  });

  it("should reject a label with uppercase", () => {
    const schema = z.object(AnnotateRunInput);
    const result = schema.safeParse({ runId: "run-xyz", labels: ["Auth"] });
    assert.ok(!result.success, "Label with uppercase should fail validation");
  });

  it("should reject a label exceeding 64 characters", () => {
    const schema = z.object(AnnotateRunInput);
    const result = schema.safeParse({ runId: "run-xyz", labels: ["a".repeat(65)] });
    assert.ok(!result.success, "Label exceeding 64 chars should fail validation");
  });

  it("should accept a label of exactly 64 characters", () => {
    const schema = z.object(AnnotateRunInput);
    const result = schema.safeParse({ runId: "run-xyz", labels: ["a".repeat(64)] });
    assert.ok(result.success, "Label of exactly 64 chars should be valid");
  });

  it("should reject more than 16 labels", () => {
    const schema = z.object(AnnotateRunInput);
    const labels = Array.from({ length: 17 }, (_, i) => `label${i}`);
    const result = schema.safeParse({ runId: "run-xyz", labels });
    assert.ok(!result.success, "More than 16 labels should fail validation");
  });

  it("should accept exactly 16 labels", () => {
    const schema = z.object(AnnotateRunInput);
    const labels = Array.from({ length: 16 }, (_, i) => `label${i}`);
    const result = schema.safeParse({ runId: "run-xyz", labels });
    assert.ok(result.success, "Exactly 16 labels should be valid");
  });

  it("should reject operatorNote exceeding 1000 characters", () => {
    const schema = z.object(AnnotateRunInput);
    const result = schema.safeParse({ runId: "run-xyz", operatorNote: "x".repeat(1001) });
    assert.ok(!result.success, "operatorNote exceeding 1000 chars should fail validation");
  });

  it("should accept operatorNote of exactly 1000 characters", () => {
    const schema = z.object(AnnotateRunInput);
    const result = schema.safeParse({ runId: "run-xyz", operatorNote: "x".repeat(1000) });
    assert.ok(result.success, "operatorNote of exactly 1000 chars should be valid");
  });

  it("should reject missing runId", () => {
    const schema = z.object(AnnotateRunInput);
    const result = schema.safeParse({ labels: ["auth"] });
    assert.ok(!result.success, "Missing runId should fail validation");
  });
});

// ---------------------------------------------------------------
// Milestone 15: list_runs label filter schema
// ---------------------------------------------------------------
describe("ListRunsInput label field (Milestone 15)", () => {
  it("should accept a label filter", () => {
    const schema = z.object(ListRunsInput);
    const result = schema.safeParse({ label: "auth" });
    assert.ok(result.success, "label filter should be accepted");
  });

  it("should accept an absent label filter", () => {
    const schema = z.object(ListRunsInput);
    const result = schema.safeParse({});
    assert.ok(result.success, "absent label filter should be accepted");
  });
});

// ---------------------------------------------------------------
// Milestone 15: No-hidden-agent regression for annotate_run
// ---------------------------------------------------------------
describe("No-hidden-agent regression (Milestone 15)", () => {
  it("annotate_run should be registered as a lifecycle tool", () => {
    assert.ok(
      REGISTERED_TOOL_NAMES.includes("annotate_run" as (typeof REGISTERED_TOOL_NAMES)[number]),
      "annotate_run must be in the tool registry",
    );
  });

  it("annotate_run is not an autonomous continuation tool", () => {
    const coarsePatterns = ["continue", "resume", "agent", "turn", "codex_reply", "fix_end"];
    for (const pattern of coarsePatterns) {
      assert.ok(
        !"annotate_run".includes(pattern),
        `"annotate_run" must not contain autonomous pattern "${pattern}"`,
      );
    }
  });

  it("daemon method run.annotate is not a forbidden agent-runtime method", () => {
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
        method !== "run.annotate",
        `run.annotate must not be a forbidden agent-runtime method`,
      );
    }
  });
});
