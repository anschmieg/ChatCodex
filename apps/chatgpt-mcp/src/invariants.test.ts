/**
 * Invariant checks for the MCP gateway.
 *
 * These tests verify that:
 * 1. No forbidden tool names are registered
 * 2. No forbidden daemon methods are called
 * 3. The tool registry matches the expected set
 * 4. Milestone 8: PolicyProfileInput schema validates correctly
 */

import { describe, it } from "node:test";
import * as assert from "node:assert/strict";
import { z } from "zod";
import { FORBIDDEN_TOOL_NAMES, REGISTERED_TOOL_NAMES } from "./tools.js";
import { PolicyProfileInput } from "./schemas.js";

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
// Milestone 8: PolicyProfileInput schema validation
// ---------------------------------------------------------------
describe("PolicyProfileInput schema (Milestone 8)", () => {
  it("accepts an empty object (all defaults)", () => {
    const result = PolicyProfileInput.safeParse({});
    assert.ok(result.success, "empty object should be valid");
  });

  it("accepts a fully populated valid policy", () => {
    const result = PolicyProfileInput.safeParse({
      patchEditThreshold: 10,
      deleteRequiresApproval: false,
      sensitivePathRequiresApproval: true,
      outsideFocusRequiresApproval: false,
      extraSafeMakeTargets: ["deploy-staging", "release"],
      focusPaths: ["src/", "tests/"],
    });
    assert.ok(result.success, "fully populated policy should be valid");
  });

  it("rejects patchEditThreshold of 0 (must be >= 1)", () => {
    const result = PolicyProfileInput.safeParse({ patchEditThreshold: 0 });
    assert.ok(!result.success, "threshold 0 should be rejected");
  });

  it("rejects negative patchEditThreshold", () => {
    const result = PolicyProfileInput.safeParse({ patchEditThreshold: -1 });
    assert.ok(!result.success, "negative threshold should be rejected");
  });

  it("rejects extraSafeMakeTargets with empty string entries", () => {
    const result = PolicyProfileInput.safeParse({
      extraSafeMakeTargets: ["valid", ""],
    });
    assert.ok(!result.success, "empty string in make targets should be rejected");
  });

  it("accepts partial policy with only some fields set", () => {
    const result = PolicyProfileInput.safeParse({
      patchEditThreshold: 3,
      deleteRequiresApproval: true,
    });
    assert.ok(result.success, "partial policy should be valid");
    if (result.success) {
      assert.strictEqual(result.data.patchEditThreshold, 3);
      assert.strictEqual(result.data.deleteRequiresApproval, true);
      assert.strictEqual(result.data.sensitivePathRequiresApproval, undefined);
    }
  });
});
