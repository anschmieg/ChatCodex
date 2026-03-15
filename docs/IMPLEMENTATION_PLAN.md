# Implementation plan

## Milestone 0: bootstrap and design freeze

Create:
- `AGENTS.md`
- `.github/copilot-instructions.md`
- `.github/instructions/rust.instructions.md`
- `.github/instructions/typescript.instructions.md`
- docs in `docs/`

Acceptance:
- architecture is explicit
- no-hidden-agent rule is documented
- tool contracts are fixed

## Milestone 1: deterministic Rust daemon skeleton

Create crates:
- `codex-rs/deterministic-protocol`
- `codex-rs/deterministic-core`
- `codex-rs/deterministic-daemon`

Implement:
- request/response types
- run-state schema
- SQLite store
- `/healthz`
- `/rpc`
- handlers for:
  - `run.prepare`
  - `workspace.summary`
  - `file.read`
  - `git.status`

Acceptance:
- daemon builds
- basic RPC calls work
- state persists

## Milestone 2: MCP gateway skeleton

Create:
- `apps/chatgpt-mcp`

Implement:
- MCP server bootstrap
- tool registration
- daemon client
- tools for:
  - `codex_prepare_run`
  - `get_workspace_summary`
  - `read_file`
  - `git_status`

Acceptance:
- gateway builds
- tools call daemon correctly

## Milestone 3: minimal end-to-end coding loop

Implement:
- Rust daemon handlers and core logic for:
  - `code.search`
  - `patch.apply`
  - `tests.run`
  - `git.diff`

Add MCP tools:
- `search_code`
- `apply_patch`
- `run_tests`
- `show_diff`

Acceptance:
- prepare -> inspect -> patch -> test -> diff works end to end on a sample workspace
- no hidden-agent violations
- TypeScript remains thin

## Milestone 4: deterministic control-plane statefulness ✅

Implement stateful deterministic orchestration:

### Expanded run-state model

Extend `RunState` with:
- `completedSteps`, `pendingSteps`
- `lastAction`, `lastObservation`
- `recommendedNextAction`, `recommendedTool`
- `latestDiffSummary`, `latestTestResult`
- `warnings`
- status values: `prepared`, `active`, `blocked`, `awaiting_approval`, `done`, `failed`

### New internal daemon methods

- `run.refresh` — return an updated run-state snapshot (read-only)
- `run.replan` — deterministic rule-based replanning
- `approval.resolve` — resolve pending approvals

### New public MCP tools

- `refresh_run_state`
- `replan_run`
- `approve_action`

### Approval plumbing

- SQLite `approvals` table for pending approval state
- Deterministic state transitions (approve → unblock, deny → block)
- Policy hooks for risky operations

Acceptance:
- ✅ expanded state persists in SQLite
- ✅ refresh returns consistent snapshots
- ✅ replan deterministically updates plan
- ✅ approval resolution works end to end
- ✅ no hidden agent loop
- ✅ TypeScript remains thin

## Milestone 4.1: SQLite schema migration compatibility ✅

Add automatic schema migration for backward compatibility:

- `ALTER TABLE ADD COLUMN` migration for older databases
- Safe deterministic defaults for new columns
- Idempotent migration (safe to run multiple times)

Acceptance:
- ✅ daemon starts with Milestone 3-era databases
- ✅ `run.prepare` succeeds against upgraded old DB
- ✅ migration is deterministic and idempotent

## Milestone 5: approval policy hardening ✅

Implement deterministic approval policy layer:

### Patch policy

Gate patches requiring approval if:
- Any edit has `operation: "delete"` (file deletion)
- More than 5 edits in a single request (large patch)
- Any path matches sensitive pattern (`.env`, `.ssh/`, `.git/`, `id_rsa`, etc.)
- Any path is outside the run's declared `focusPaths`

### Test-run policy

Gate test runs requiring approval if:
- `scope` is `"make"` and `target` is not a standard safe target

### Implementation

- `approval_policy.rs` with rule-based evaluation
- `focus_paths` field in run state
- `policy_rationale` field in pending approvals
- SQLite schema migration for Milestone 5 columns

Acceptance:
- ✅ patch policy gates risky operations
- ✅ test-run policy gates non-standard make targets
- ✅ policy decisions are deterministic
- ✅ policy rationale is captured and returned

## Milestone 6: deterministic action resumption ✅

Add structured retryable action metadata to enable deterministic resumption after policy blocks:

### Retryable action model

- `RetryableAction` type with `kind`, `summary`, `payload`, `validity`, `recommendation`
- Recorded when `patch.apply` or `tests.run` is blocked by approval policy
- Updated on approval resolution (validated/invalidated)
- Preserved or invalidated on replan based on context

### Run state extensions

- `retryableAction` field in `RunState`
- `replanDelta` field for concise change description
- Refresh surfaces retryable action metadata and warns on staleness

### SQLite migration

- Adds `retryable_action` column with backward compatibility
- Safe defaults for existing databases

Acceptance:
- ✅ retryable action recorded on policy block
- ✅ approval resolution updates retryable action state
- ✅ replan preserves valid retryable actions, invalidates stale ones
- ✅ refresh surfaces retryable action with staleness warnings
- ✅ no new public tools or daemon methods needed
- ✅ no autonomous continuation—ChatGPT still invokes next tool explicitly

## Milestone 7: deterministic run history, audit trail, and state inspection ✅

Add read-only visibility into prior runs and recent state transitions:

### Run listing

- `runs.list` daemon method → `list_runs` MCP tool
- Returns `RunSummary` items with run ID, workspace, goal, status, step counts, timestamps
- Supports limit (default 20, max 100), workspace filter, status filter

### Run state inspection

- `run.get` daemon method → `get_run_state` MCP tool
- Returns `RunGetResult` with full run state, pending approvals, retryable action, diff/test metadata, recommendations

### Audit trail

- `run.history` daemon method → `get_run_history` MCP tool
- Returns `RunHistoryEntry` list (newest first, configurable limit up to 200)
- Key events recorded: `run_prepared`, `refresh_performed`, `replan_performed`, `approval_created`, `approval_resolved`, `patch_applied`, `tests_run`
- Backed by `audit_trail` SQLite table

### SQLite migration

- Adds `audit_trail` table to new databases and migrates older databases (`CREATE TABLE IF NOT EXISTS`)
- Backward compatible with Milestone 6 and earlier databases

Acceptance:
- ✅ prior runs can be listed deterministically
- ✅ authoritative run state can be inspected directly
- ✅ lightweight audit trail persisted and retrievable
- ✅ all new tools are read-only (no autonomous operations)
- ✅ no model/provider SDKs added
- ✅ TypeScript remains thin

## Milestone 8: deterministic policy configuration and per-run execution constraints ✅

Add structured, inspectable, per-run policy profiles:

### Per-run policy profile

- `RunPolicy` struct: `patchEditThreshold`, `deleteRequiresApproval`, `sensitivePathRequiresApproval`, `outsideFocusRequiresApproval`, `extraSafeMakeTargets`, `focusPaths`
- `RunPolicyInput` for optional partial input at prepare time (missing fields → defaults)
- `RunState.policyProfile` persisted in SQLite

### Policy-aware run preparation

- `RunPrepareParams.policy: Option<RunPolicyInput>` — pass custom constraints at run creation
- `RunPrepareResult.effectivePolicy` — daemon returns the resolved active policy
- `focusPaths` always copied into `RunPolicy.focusPaths` for backward compatibility
- `extraSafeMakeTargets` normalised to lowercase at validation time

### Policy-aware approval evaluation

- `approval_policy.rs` reads thresholds and flags from the per-run `RunPolicy` instead of hardcoded constants
- All rules remain deterministic; no LLM reasoning involved

### Policy surfacing in responses

- `RunRefreshResult.effectivePolicy` — policy visible on every refresh
- `RunGetResult.effectivePolicy` — policy visible on direct run inspection

### SQLite migration

- Adds `policy_profile TEXT NOT NULL DEFAULT '{}'` column to `runs` table
- Existing runs get `RunPolicy::default()` on upgrade

### TypeScript gateway

- `PolicyProfileInputSchema` Zod schema in `schemas.ts`
- `CodexPrepareRunInput` includes optional `policy` field
- `tools.ts` passes `policy` to `run.prepare`

Acceptance:
- ✅ each run has an explicit effective policy profile
- ✅ default policy matches pre-Milestone-8 behavior
- ✅ custom policy validated deterministically
- ✅ approval decisions use per-run policy
- ✅ policy rationale remains explicit and audit-friendly
- ✅ no autonomous continuation
- ✅ TypeScript remains thin
- ✅ SQLite migration is backward compatible

## Milestone 9: deterministic operation preflight and approval preview ✅

Add read-only policy evaluation before performing patch or test operations.

### New protocol types

- `PreflightDecision` enum: `proceed` | `requires_approval`
- `PreflightResult` struct: `decision`, `actionSummary?`, `riskReason?`, `policyRationale?`, `effectivePolicy`
- `PatchPreflightParams`, `TestsPreflightParams`

### New daemon methods (read-only, no state mutation)

- `patch.preflight` — evaluate patch approval policy without applying changes
- `tests.preflight` — evaluate test approval policy without executing tests

### New MCP tools

- `preview_patch_policy` — inspect whether a proposed patch would require approval
- `preview_test_policy` — inspect whether a proposed test run would require approval

### TypeScript gateway

- `PreviewPatchPolicyInput` and `PreviewTestPolicyInput` Zod schemas in `schemas.ts`
- `tools.ts` registers both preview tools

Acceptance:
- ✅ preflight methods reuse existing policy evaluation logic
- ✅ no state mutation from preview calls
- ✅ `decision` field is always deterministic
- ✅ TypeScript remains thin
- ✅ no model/provider SDKs added

## Milestone 10: deterministic run finalization, outcome recording, and closure ✅

Add explicit and deterministic lifecycle closure so runs have a clean ending.

### New protocol types

- `RunOutcome` struct: `outcomeKind`, `summary`, `reason?`, `finalizedAt`
- `RunFinalizeParams`: `runId`, `outcomeKind`, `summary`, `reason?`
- `RunFinalizeResult`: `runId`, `outcomeKind`, `finalizedAt`, `status`, `recommendedNextAction`
- `finalizedOutcome: Option<RunOutcome>` field added to `RunState`, `RunRefreshResult`, `RunGetResult`
- `outcomeKind: Option<String>` field added to `RunSummary`

### New daemon method

- `run.finalize` — close a run with a structured outcome record
  - Validates `outcomeKind` must be `completed`, `failed`, or `abandoned`
  - Rejects if run is already finalized
  - Sets `status` to `finalized:<outcomeKind>`
  - Persists `RunOutcome` in SQLite
  - Appends `run_finalized` audit trail entry
  - Returns deterministic guidance; no autonomous follow-up

### New MCP tool

- `finalize_run` — thin gateway to `run.finalize`; not a coarse autonomous tool

### SQLite migration

- Adds `outcome_kind TEXT` and `finalized_outcome TEXT` columns to `runs` table
- Backward compatible (NULL for unfinalized runs on older databases)

### TypeScript gateway

- `FinalizeRunInput` Zod schema in `schemas.ts`
- `tools.ts` registers `finalize_run`

Acceptance:
- ✅ runs can be explicitly finalized with a structured outcome record
- ✅ outcome_kind must be `completed`, `failed`, or `abandoned`
- ✅ double finalization is rejected deterministically
- ✅ finalization is recorded in the audit trail
- ✅ closed runs expose outcome metadata in `run.get`, `run.refresh`, `runs.list`
- ✅ TypeScript remains thin
- ✅ SQLite migration is backward compatible
- ✅ no autonomous continuation

## Out of scope

These are intentionally not implemented:

- approvals UI (backend plumbing only)
- widgets
- OAuth
- advanced replanning with LLM assistance
- worktrees
- background orchestration
- any agent-owned runtime
- `run_command` / `command.exec` (not implemented, not needed)
