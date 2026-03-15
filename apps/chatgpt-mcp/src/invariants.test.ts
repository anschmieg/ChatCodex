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
import { PolicyProfileInputSchema } from "./schemas.js";

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
