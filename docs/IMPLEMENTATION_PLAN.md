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

## Milestone 11: deterministic run reopening and post-finalization continuation controls ✅

Add explicit and deterministic lifecycle continuation controls so ChatGPT can reopen
previously finalized runs without introducing backend autonomy.

### New protocol types

- `ReopenMetadata` struct: `reason`, `reopenedAt`, `reopenedFromOutcomeKind`, `reopenCount`
- `RunReopenParams`: `runId`, `reason`
- `RunReopenResult`: `runId`, `status`, `reopenedFromOutcomeKind`, `reopenCount`, `reopenedAt`, `recommendedNextAction`, `recommendedTool`
- `reopen_metadata: Option<ReopenMetadata>` field added to `RunState`, `RunRefreshResult`, `RunGetResult`
- `reopen_count: Option<u32>` field added to `RunSummary`

### New daemon method

- `run.reopen` — reopen a finalized run for deterministic continuation
  - Only finalized runs (`finalized:completed`, `finalized:failed`, `finalized:abandoned`) may be reopened
  - Active, prepared, or awaiting-approval runs are rejected
  - Transitions status back to `"active"`, clears `finalized_outcome`
  - Persists `ReopenMetadata` in SQLite; increments `reopen_count` on successive reopens
  - Appends `run_reopened` audit trail entry
  - Returns deterministic guidance; no autonomous follow-up

### New MCP tool

- `reopen_run` — thin gateway to `run.reopen`; not a coarse autonomous tool

### SQLite migration

- Adds `reopen_metadata TEXT` column to `runs` table
- Backward compatible (NULL for runs never reopened on older databases)

### TypeScript gateway

- `ReopenRunInput` Zod schema in `schemas.ts`
- `tools.ts` registers `reopen_run`

Acceptance:
- ✅ finalized runs can be explicitly reopened with a structured reason
- ✅ only finalized runs are reopenable; active/prepared runs are rejected
- ✅ reopening is recorded in the audit trail as `run_reopened`
- ✅ reopen metadata (reason, timestamp, source outcome kind, reopen count) persists in SQLite
- ✅ reopen metadata is visible in `run.get`, `run.refresh`, `runs.list`
- ✅ TypeScript remains thin
- ✅ SQLite migration is backward compatible
- ✅ no autonomous continuation

## Milestone 12: deterministic run supersession and replacement lineage ✅

Add explicit and deterministic run supersession so ChatGPT can replace a finalized run with
a new successor run while preserving clear, auditable lineage between them.

### New protocol types

- `RunSupersedeParams`: `runId`, `newUserGoal?`, `reason`
- `RunSupersedeResult`: `originalRunId`, `successorRunId`, `supersededAt`, `successorStatus`, `recommendedNextAction`, `recommendedTool`
- Lineage fields added to `RunState`, `RunGetResult`, `RunSummary`:
  - `supersedes_run_id: Option<String>` — the run this run supersedes (set on successor)
  - `superseded_by_run_id: Option<String>` — the run that superseded this run (set on original)
  - `supersession_reason: Option<String>` — human-readable reason for supersession (set on both)
  - `superseded_at: Option<String>` — ISO 8601 timestamp of supersession (set on both)

### New daemon method

- `run.supersede` — create a successor run that explicitly replaces a finalized run
  - Only finalized runs may be superseded; active/prepared/awaiting-approval runs are rejected
  - Supersession creates a new run in `"prepared"` status (does not reactivate the original)
  - Original run marked with `superseded_by_run_id`; status remains finalized (history preserved)
  - Successor run carries `supersedes_run_id` pointing to the original
  - Successor inherits workspace, focus paths, and policy profile from original
  - Successor starts with an empty plan; no autonomous work is triggered
  - Appends `run_superseded` (original) and `run_created_from_supersession` (successor) audit entries
  - Returns deterministic guidance; no autonomous follow-up

### New MCP tool

- `supersede_run` — thin gateway to `run.supersede`; not a coarse autonomous tool

### SQLite migration

- Adds `supersedes_run_id TEXT`, `superseded_by_run_id TEXT`, `supersession_reason TEXT`, `superseded_at TEXT` columns to `runs` table
- Backward compatible (NULL for all existing rows on older databases)

### TypeScript gateway

- `SupersedeRunInput` Zod schema in `schemas.ts`: `runId`, `newUserGoal?` (max 500 chars), `reason` (min 1, max 500 chars)
- `tools.ts` registers `supersede_run`

Acceptance:
- ✅ ChatGPT can explicitly supersede a finalized run with a successor run
- ✅ only finalized runs are supersedable; active/prepared runs are rejected deterministically
- ✅ supersession is recorded in the audit trail on both original and successor runs
- ✅ lineage metadata persists in SQLite with backward-compatible migration
- ✅ original run remains preserved, inspectable, and carries superseded_by_run_id
- ✅ successor run carries supersedes_run_id and starts in "prepared" status
- ✅ lineage visible in `run.get`, `runs.list`
- ✅ TypeScript remains thin
- ✅ no autonomous continuation; no coarse autonomous tools

## Milestone 13: deterministic run archiving and retention controls ✅

**Goal:** Let ChatGPT explicitly archive eligible runs so they remain preserved and inspectable while being distinguishable from the active working set.

**Rust (deterministic-protocol / deterministic-core / deterministic-daemon):**

- ✅ `ArchiveMetadata` struct added to protocol types (`reason`, `archived_at`)
- ✅ `archive_metadata: Option<ArchiveMetadata>` field on `RunState`
- ✅ `RunArchiveParams` / `RunArchiveResult` added to protocol types
- ✅ `include_archived` / `archived_only` added to `RunsListParams`
- ✅ `is_archived`, `archive_reason`, `archived_at` fields on `RunSummary`
- ✅ `archive_metadata` field on `RunGetResult`
- ✅ `Method::RunArchive` (`run.archive`) added to methods enum
- ✅ `deterministic_core::run_archive` module implements eligibility rules and archive logic
- ✅ Only finalized runs may be archived; active/prepared/awaiting-approval runs and already-archived runs are rejected
- ✅ Archiving does not execute work, reopen, supersede, or continue the run
- ✅ Archive metadata is appended to run state and persisted
- ✅ `run_archived` audit entry is appended with archive reason
- ✅ `handle_run_archive` in daemon handlers dispatches archive operation
- ✅ `handle_runs_list` passes `include_archived` / `archived_only` through to persistence
- ✅ `handle_run_get` exposes `archive_metadata` in `RunGetResult`
- ✅ SQLite persistence: `is_archived` and `archive_metadata` columns added with safe migration
- ✅ `list_runs` in persistence supports archive filtering: default excludes archived, `include_archived=true` includes all, `archived_only=true` returns only archived
- ✅ `RunSummary` carries archive fields from persistence query

**TypeScript (MCP gateway):**

- ✅ `ArchiveRunInput` Zod schema added to `schemas.ts` (`runId`, `reason` 1–500 chars)
- ✅ `ListRunsInput` extended with `includeArchived` and `archivedOnly` optional booleans
- ✅ `archive_run` added to `REGISTERED_TOOL_NAMES`
- ✅ `archive_run` tool registered: validates inputs, calls `run.archive`, returns result
- ✅ `list_runs` tool updated to pass `includeArchived` and `archivedOnly` to daemon
- ✅ TypeScript remains thin: validation + mapping + daemon calls only

**Tests:**

- ✅ Core: archiving completed/failed runs, eligibility rejection for active/prepared/awaiting-approval, already-archived, archive metadata roundtrip
- ✅ Daemon handlers: M13 tests for all archive scenarios including audit trail, run.get visibility, list filtering
- ✅ Persistence: archive metadata roundtrip, defaults to None, list filtering (default excludes, include_archived, archived_only), summary fields, migration from M12 schema
- ✅ TypeScript: `ArchiveRunInput` schema validation, `ListRunsInput` archive filter schema, no-hidden-agent regression, exact registry test updated

---

## Milestone 14: Deterministic Run Unarchiving and Archive Restoration Controls ✅

**Protocol (`deterministic-protocol`):**

- ✅ `UnarchiveMetadata` struct: `reason`, `unarchived_at`
- ✅ `RunUnarchiveParams` struct: `run_id`, `reason`
- ✅ `RunUnarchiveResult` struct: `run_id`, `status`, `unarchived_at`, `reason`, `message`
- ✅ `unarchive_metadata: Option<UnarchiveMetadata>` field added to `RunState`
- ✅ `unarchive_reason`, `unarchived_at` fields added to `RunSummary`
- ✅ `unarchive_metadata` field added to `RunGetResult`
- ✅ `Method::RunUnarchive` (`run.unarchive`) added to methods enum

**Core (`deterministic-core`):**

- ✅ `deterministic_core::run_unarchive` module: `unarchive()` function enforces all lifecycle rules
- ✅ Only archived runs (with `archive_metadata`) may be unarchived
- ✅ Non-archived runs are rejected with a clear error
- ✅ Already-unarchived runs are rejected (idempotent-safe)
- ✅ Unarchiving does not execute work, does not reopen, does not change status
- ✅ Finalized outcome, plan, completed steps, and lineage metadata are preserved
- ✅ Original `archive_metadata` remains intact after unarchiving
- ✅ `unarchive_metadata` is set on the run state after unarchiving

**Daemon (`deterministic-daemon`):**

- ✅ `handle_run_unarchive` handler added and dispatched from `Method::RunUnarchive`
- ✅ Audit entry `run_unarchived` appended with unarchive reason
- ✅ `handle_run_get` exposes `unarchive_metadata` in `RunGetResult`
- ✅ SQLite persistence: `unarchive_metadata` column added with safe migration
- ✅ `is_archived` flag: a run is considered archived only if `archive_metadata` is set AND `unarchive_metadata` is not set
- ✅ Unarchived runs return to default list visibility (is_archived = 0)
- ✅ `archived_only=true` excludes unarchived runs
- ✅ `RunSummary` carries `unarchive_reason` and `unarchived_at` fields from persistence query

**TypeScript (MCP gateway):**

- ✅ `UnarchiveRunInput` Zod schema added to `schemas.ts` (`runId`, `reason` 1–500 chars)
- ✅ `unarchive_run` added to `REGISTERED_TOOL_NAMES`
- ✅ `unarchive_run` tool registered: validates inputs, calls `run.unarchive`, returns result
- ✅ TypeScript remains thin: validation + mapping + daemon calls only

**Tests:**

- ✅ Core: unarchiving completed/failed/abandoned runs, rejecting non-archived and already-unarchived runs, preserving history and lineage, status unchanged, no reopen
- ✅ Daemon handlers: M14 tests for unarchive completed/failed, rejection for non-archived/unknown, audit trail, list restoration, archived_only exclusion, run.get visibility, persistence roundtrip
- ✅ Persistence: unarchive metadata roundtrip, defaults to None, restored run in default list, excluded from archived_only, summary fields, migration from M13 schema
- ✅ TypeScript: `UnarchiveRunInput` schema validation, no-hidden-agent regression, exact registry test updated

---

## Milestone 15: deterministic run labeling and annotation metadata ✅

**Goal:** Let ChatGPT explicitly attach organization metadata (labels and an operator note) to a run.

**Protocol (`deterministic-protocol`):**

- ✅ `RunAnnotation` struct: `labels: Vec<String>`, `operator_note: Option<String>`
- ✅ `LABEL_MAX_LEN = 64`, `LABEL_MAX_COUNT = 16`, `OPERATOR_NOTE_MAX_LEN = 1000` constants
- ✅ `RunAnnotateParams` struct: `run_id`, `labels: Option<Vec<String>>`, `operator_note: Option<String>`
- ✅ `RunAnnotateResult` struct: `run_id`, `status`, `annotation: RunAnnotation`, `updated_at`, `message`
- ✅ `annotation: Option<RunAnnotation>` field on `RunState` and `RunGetResult`
- ✅ `labels`, `operator_note` fields on `RunSummary`
- ✅ `label_filter: Option<String>` added to `RunsListParams`
- ✅ `Method::RunAnnotate` (`run.annotate`) added to methods enum

**Core (`deterministic-core`):**

- ✅ `deterministic_core::run_annotate` module: `normalize_labels()` and `annotate()` functions
- ✅ Labels are trimmed, lowercased, deduplicated (first wins), and sorted deterministically
- ✅ Each label: non-empty, `[a-z0-9_-]` only, bounded to `LABEL_MAX_LEN`
- ✅ Total label count bounded to `LABEL_MAX_COUNT`
- ✅ Operator note bounded to `OPERATOR_NOTE_MAX_LEN`
- ✅ At least one of `labels` or `operator_note` must be provided
- ✅ Annotating does not execute work, change status, plan, or any lifecycle field

**Daemon (`deterministic-daemon`):**

- ✅ `handle_run_annotate` handler dispatched from `Method::RunAnnotate`
- ✅ Audit entry `run_annotated` appended with label count and note presence
- ✅ `handle_run_get` exposes `annotation` in `RunGetResult`
- ✅ SQLite persistence: `annotation` TEXT column added with safe migration
- ✅ `handle_runs_list` supports `label_filter` (exact post-query Rust filtering)
- ✅ `RunSummary` carries `labels` and `operator_note` fields from persistence

**TypeScript (MCP gateway):**

- ✅ `AnnotateRunInput` Zod schema added to `schemas.ts` (`runId`, `labels?: string[]`, `operatorNote?: string`)
- ✅ `ListRunsInput` extended with `labelFilter?: string` optional field
- ✅ `annotate_run` added to `REGISTERED_TOOL_NAMES`
- ✅ `annotate_run` tool registered: validates inputs, calls `run.annotate`, returns result
- ✅ TypeScript remains thin: validation + mapping + daemon calls only

**Tests:**

- ✅ Core: label normalization, deduplication, sort, invalid character rejection, length bounds, operator note update, at-least-one requirement
- ✅ Daemon handlers: M15 tests for annotating runs, label filter in list, audit trail, run.get visibility
- ✅ Persistence: annotation roundtrip, defaults to None, label filter in list, summary fields, migration
- ✅ TypeScript: `AnnotateRunInput` schema validation, `ListRunsInput` label filter, no-hidden-agent regression, exact registry test updated

---

## Milestone 16: deterministic run pinning and working-set prioritization ✅

**Goal:** Let ChatGPT explicitly pin runs to mark them as prominent in the visible working set.

**Protocol (`deterministic-protocol`):**

- ✅ `PinMetadata` struct: `reason`, `pinned_at`
- ✅ `PIN_REASON_MAX_LEN = 500` constant
- ✅ `RunPinParams` struct: `run_id`, `reason`
- ✅ `RunPinResult` struct: `run_id`, `status`, `pinned_at`, `reason`, `message`
- ✅ `RunUnpinParams` struct: `run_id`, `reason`
- ✅ `RunUnpinResult` struct: `run_id`, `status`, `unpinned_at`, `reason`, `message`
- ✅ `pin_metadata: Option<PinMetadata>` field on `RunState` and `RunGetResult`
- ✅ `is_pinned`, `pin_reason`, `pinned_at` fields on `RunSummary`
- ✅ `pinned_only: Option<bool>` added to `RunsListParams`
- ✅ `Method::RunPin` (`run.pin`) and `Method::RunUnpin` (`run.unpin`) added to methods enum

**Core (`deterministic-core`):**

- ✅ `deterministic_core::run_pin` module: `pin()` function
- ✅ `deterministic_core::run_unpin` module: `unpin()` function
- ✅ Any run (regardless of status) may be pinned; re-pinning replaces metadata (idempotent)
- ✅ Only pinned runs may be unpinned; non-pinned runs are rejected
- ✅ Pinning/unpinning does not execute work, change status, plan, or any lifecycle field

**Daemon (`deterministic-daemon`):**

- ✅ `handle_run_pin` / `handle_run_unpin` handlers dispatched from respective methods
- ✅ Audit entries `run_pinned` / `run_unpinned` appended with reason
- ✅ SQLite persistence: `pin_metadata` TEXT column added with safe migration
- ✅ `is_pinned` flag: set when `pin_metadata` is present
- ✅ Pinned runs sort first in the default `list_runs` ordering
- ✅ `pinned_only=true` filter returns only currently pinned runs
- ✅ `RunSummary` carries pin fields from persistence

**TypeScript (MCP gateway):**

- ✅ `PinRunInput` / `UnpinRunInput` Zod schemas added to `schemas.ts`
- ✅ `ListRunsInput` extended with `pinnedOnly?: boolean`
- ✅ `pin_run`, `unpin_run` added to `REGISTERED_TOOL_NAMES`
- ✅ Tools registered: validate inputs, call daemon, return results
- ✅ TypeScript remains thin: validation + mapping + daemon calls only

**Tests:**

- ✅ Core: pinning any status run, re-pin replaces metadata, unpin pinned run, reject unpin of non-pinned
- ✅ Daemon handlers: M16 tests for pin/unpin operations, pinned_only filter, audit trail, persistence
- ✅ Persistence: pin metadata roundtrip, defaults to None, pinned_only filter, summary fields, migration
- ✅ TypeScript: schema validation, no-hidden-agent regression, exact registry test updated

---

## Milestone 17: deterministic run snoozing and deferred visibility ✅

**Goal:** Let ChatGPT explicitly snooze runs to temporarily defer them out of the default visible working set without archiving.

**Protocol (`deterministic-protocol`):**

- ✅ `SnoozeMetadata` struct: `reason`, `snoozed_at`
- ✅ `SNOOZE_REASON_MAX_LEN = 500` constant
- ✅ `RunSnoozeParams` struct: `run_id`, `reason`
- ✅ `RunSnoozeResult` struct: `run_id`, `status`, `snoozed_at`, `reason`, `message`
- ✅ `RunUnsnoozeParams` struct: `run_id`, `reason`
- ✅ `RunUnsnoozeResult` struct: `run_id`, `status`, `unsnoozed_at`, `reason`, `message`
- ✅ `snooze_metadata: Option<SnoozeMetadata>` field on `RunState`
- ✅ `is_snoozed`, `snooze_reason`, `snoozed_at` fields on `RunSummary`
- ✅ `include_snoozed` / `snoozed_only` added to `RunsListParams`
- ✅ `Method::RunSnooze` (`run.snooze`) and `Method::RunUnsnooze` (`run.unsnooze`) added to methods enum

**Core (`deterministic-core`):**

- ✅ `deterministic_core::run_snooze` module: `snooze()` function
- ✅ `deterministic_core::run_unsnooze` module: `unsnooze()` function
- ✅ Any run (regardless of status) may be snoozed; re-snoozing replaces metadata
- ✅ Only snoozed runs may be unsnoozed; non-snoozed runs are rejected
- ✅ Snoozing/unsnoozing does not execute work, change status, plan, or any lifecycle field

**Daemon (`deterministic-daemon`):**

- ✅ `handle_run_snooze` / `handle_run_unsnooze` handlers dispatched from respective methods
- ✅ Audit entries `run_snoozed` / `run_unsnoozed` appended with reason
- ✅ SQLite persistence: `snooze_metadata` TEXT column added with safe migration
- ✅ `is_snoozed` flag: set when `snooze_metadata` is present
- ✅ `list_runs` excludes snoozed runs by default; `include_snoozed=true` includes them; `snoozed_only=true` returns only snoozed
- ✅ `RunSummary` carries snooze fields from persistence

**TypeScript (MCP gateway):**

- ✅ `SnoozeRunInput` / `UnsnoozeRunInput` Zod schemas added to `schemas.ts` (reason 1–500 chars)
- ✅ `ListRunsInput` extended with `includeSnoozed?: boolean`, `snoozedOnly?: boolean`
- ✅ `snooze_run`, `unsnooze_run` added to `REGISTERED_TOOL_NAMES`
- ✅ Tools registered: validate inputs, call daemon, return results
- ✅ TypeScript remains thin: validation + mapping + daemon calls only

**Tests:**

- ✅ Core: snoozing any status run, re-snooze replaces metadata, unsnooze snoozed run, reject unsnooze of non-snoozed
- ✅ Daemon handlers: M17 tests for snooze/unsnooze operations, filtering, audit trail, persistence
- ✅ Persistence: snooze metadata roundtrip, list filtering (default excludes, include_snoozed, snoozed_only), migration
- ✅ TypeScript: schema validation, ListRunsInput snooze filter, no-hidden-agent regression, registry test updated

---

## Milestone 18: deterministic run priority levels and queue ordering ✅

**Goal:** Let ChatGPT explicitly classify runs by urgency within the visible working set.

**Protocol (`deterministic-protocol`):**

- ✅ `RunPriority` enum: `Low`, `Normal`, `High`, `Critical` (serialized as `"low"`, `"normal"`, `"high"`, `"critical"`)
- ✅ `PRIORITY_REASON_MAX_LEN = 500` constant
- ✅ `RunSetPriorityParams` struct: `run_id`, `priority: RunPriority`, `reason`
- ✅ `RunSetPriorityResult` struct: `run_id`, `status`, `previous_priority`, `priority`, `reason`, `updated_at`, `message`
- ✅ `priority: RunPriority` field on `RunState`, `RunSummary`, and `RunGetResult` (default: `Normal`)
- ✅ `priority_filter: Option<RunPriority>` added to `RunsListParams`
- ✅ `sort_by_priority: Option<bool>` added to `RunsListParams`
- ✅ `Method::RunSetPriority` (`run.set_priority`) added to methods enum

**Core (`deterministic-core`):**

- ✅ `deterministic_core::run_set_priority` module: `set_priority()` function
- ✅ Any run (regardless of status) may have priority updated
- ✅ Reason must be non-empty and ≤ `PRIORITY_REASON_MAX_LEN`
- ✅ Setting priority does not execute work, change status, plan, or any lifecycle field
- ✅ Previous and new priority recorded in result for audit visibility

**Daemon (`deterministic-daemon`):**

- ✅ `handle_run_set_priority` handler dispatched from `Method::RunSetPriority`
- ✅ Audit entry `run_priority_set` appended with previous and new priority
- ✅ SQLite persistence: `priority` TEXT column added with safe migration (defaults to `"normal"`)
- ✅ `list_runs` supports `priority_filter` (exact match) and `sort_by_priority` (high→low, ties broken by pinned-first/updated_at)
- ✅ `RunSummary` carries `priority` field from persistence

**TypeScript (MCP gateway):**

- ✅ `SetRunPriorityInput` Zod schema: `runId`, `priority` enum `["critical","high","normal","low"]`, `reason`
- ✅ `ListRunsInput` extended with `priorityFilter?: string`, `sortByPriority?: boolean`
- ✅ `set_run_priority` added to `REGISTERED_TOOL_NAMES`
- ✅ Tool registered: validates inputs, calls `run.set_priority`, returns result
- ✅ TypeScript remains thin: validation + mapping + daemon calls only

**Tests:**

- ✅ Core: setting each priority level, reason validation, status unchanged, previous/new priority in result
- ✅ Daemon handlers: M18 tests for priority set/update, priority_filter, sort_by_priority, audit trail
- ✅ Persistence: priority roundtrip, default Normal, filter and sort tests, migration from M17 schema
- ✅ TypeScript: `SetRunPriorityInput` schema accepts valid levels, rejects unknown, registry check

---

## Milestone 19: deterministic run ownership assignment ✅

**Goal:** Let ChatGPT explicitly assign ownership and coordination metadata to runs.

**Protocol (`deterministic-protocol`):**

- ✅ `ASSIGNEE_MAX_LEN = 64`, `OWNERSHIP_NOTE_MAX_LEN = 500` constants
- ✅ `RunAssignOwnerParams` struct: `run_id`, `assignee: Option<String>` (nullable to clear), `ownership_note: Option<String>`
- ✅ `RunAssignOwnerResult` struct: `run_id`, `status`, `previous_assignee`, `assignee`, `ownership_note`, `updated_at`, `message`
- ✅ `assignee: Option<String>` and `ownership_note: Option<String>` fields on `RunState`
- ✅ `assignee: Option<String>` on `RunSummary` and `RunGetResult`
- ✅ `assignee_filter: Option<String>` added to `RunsListParams`
- ✅ `Method::RunAssignOwner` (`run.assign_owner`) added to methods enum

**Core (`deterministic-core`):**

- ✅ `deterministic_core::run_assign_owner` module: `normalize_assignee()` and `assign_owner()` functions
- ✅ Assignee normalized: trimmed, lowercased, `[a-z0-9._-]` only, bounded to `ASSIGNEE_MAX_LEN`
- ✅ `assignee=null` clears ownership
- ✅ Ownership note bounded to `OWNERSHIP_NOTE_MAX_LEN`
- ✅ At least one of `assignee` or `ownership_note` must be provided
- ✅ Assigning does not execute work, change status, plan, or any lifecycle field

**Daemon (`deterministic-daemon`):**

- ✅ `handle_run_assign_owner` handler dispatched from `Method::RunAssignOwner`
- ✅ Audit entry `run_owner_assigned` (or `run_owner_cleared`) appended
- ✅ SQLite persistence: `assignee` and `ownership_note` TEXT columns added with safe migration
- ✅ `list_runs` supports `assignee_filter` (exact normalized match)
- ✅ `RunSummary` carries `assignee` field from persistence

**TypeScript (MCP gateway):**

- ✅ `AssignRunOwnerInput` Zod schema: `runId`, `assignee?: string | null`, `ownershipNote?: string`
- ✅ `ListRunsInput` extended with `assigneeFilter?: string`
- ✅ `assign_run_owner` added to `REGISTERED_TOOL_NAMES`
- ✅ Tool registered: validates inputs, calls `run.assign_owner`, returns result
- ✅ TypeScript remains thin: validation + mapping + daemon calls only

**Tests:**

- ✅ Core: assign owner, clear owner, normalize assignee, note update, invalid characters rejected
- ✅ Daemon handlers: M19 tests for owner assignment, assignee_filter, audit trail
- ✅ Persistence: assignee roundtrip, clear, filter, migration from M18 schema
- ✅ TypeScript: `AssignRunOwnerInput` schema accepts assignee and null-clear, registry check

---

## Milestone 20: deterministic run due dates and deadline scheduling metadata ✅

**Goal:** Let ChatGPT explicitly track target dates for runs without introducing backend autonomy.

**Protocol (`deterministic-protocol`):**

- ✅ `due_date: Option<String>` (ISO `YYYY-MM-DD`) field on `RunState`, `RunSummary`, and `RunGetResult`
- ✅ `RunSetDueDateParams` struct: `run_id`, `due_date: Option<Option<String>>` (outer `None` = no-op, inner `None` = clear)
- ✅ `RunSetDueDateResult` struct: `run_id`, `status`, `previous_due_date`, `due_date`, `updated_at`, `message`
- ✅ `due_on_or_before: Option<String>` and `sort_by_due_date: Option<bool>` added to `RunsListParams`
- ✅ `Method::RunSetDueDate` (`run.set_due_date`) added to methods enum

**Core (`deterministic-core`):**

- ✅ `deterministic_core::run_set_due_date` module: `validate_due_date()` and `set_due_date()` functions
- ✅ `validate_due_date`: exactly 10 chars, `YYYY-MM-DD` format, digit positions, `-` separators, month 01–12, day 01–31
- ✅ `set_due_date`: set, replace, or clear due date; preserves all other state
- ✅ Setting due date does not execute work, change status, plan, or any lifecycle field
- ✅ Previous and new due date recorded in result for audit visibility

**Daemon (`deterministic-daemon`):**

- ✅ `handle_run_set_due_date` handler dispatched from `Method::RunSetDueDate`
- ✅ Audit entry `run_due_date_set` appended with previous and new due date
- ✅ SQLite persistence: `due_date` TEXT column added with safe migration
- ✅ `list_runs` supports `due_on_or_before` filter (lexicographic ISO date comparison)
- ✅ `list_runs` supports `sort_by_due_date=true` (soonest first; runs without due date sort last)
- ✅ `RunSummary` carries `due_date` field from persistence
- ✅ `handle_run_get` exposes `due_date` in `RunGetResult`

**TypeScript (MCP gateway):**

- ✅ `SetRunDueDateInput` Zod schema: `runId`, `dueDate?: string | null` (regex `^\d{4}-\d{2}-\d{2}$`, nullable to clear)
- ✅ `ListRunsInput` extended with `dueOnOrBefore?: string`, `sortByDueDate?: boolean`
- ✅ `set_run_due_date` added to `REGISTERED_TOOL_NAMES`
- ✅ Tool registered: validates inputs, calls `run.set_due_date`, returns result
- ✅ TypeScript remains thin: validation + mapping + daemon calls only

**Tests:**

- ✅ Core: valid date accepted, trimming, invalid length/separators/digits/month/day rejected, set/replace/clear, status unchanged, message content
- ✅ Daemon handlers: M20 tests for set date, persist and reload, clear date, invalid format rejection, list filtering, sort by due date, audit trail
- ✅ Persistence: due date roundtrip, clear roundtrip, list summary carries due_date, migration from M19 schema
- ✅ TypeScript: `SetRunDueDateInput` schema accepts valid date/null/absent, rejects malformed; `ListRunsInput` dueOnOrBefore/sortByDueDate; registry check; no-hidden-agent regression

---

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
