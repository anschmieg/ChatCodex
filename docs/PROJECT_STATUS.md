# Project Status

## Overview

This repository implements a **deterministic coding harness control plane for ChatGPT**.

The architecture ensures ChatGPT is the only LLM in the stack. The backend is purely deterministic—no model calls, no agent loops, no autonomous reasoning.

## Architecture

```
ChatGPT-hosted model
→ MCP server (TypeScript)
→ internal JSON-RPC
→ deterministic Rust harness daemon
→ filesystem / git / patch / tests / approvals / sandbox
```

### Key Principles

1. **ChatGPT is the only LLM** — no backend model SDKs or API calls
2. **Deterministic backend** — all logic is rule-based and predictable
3. **Fine-grained tools** — no coarse autonomous operations
4. **Server-side policy enforcement** — approvals and restrictions are backend-owned
5. **Thin TypeScript gateway** — validation, mapping, and formatting only

## Completed Milestones

### Milestone 0: Bootstrap and Design Freeze
- Created AGENTS.md, copilot-instructions.md, and docs/
- Established architecture constraints and no-hidden-agent invariants
- Defined tool contracts and public/internal surfaces

### Milestone 1: Deterministic Rust Daemon Skeleton
- Created `deterministic-protocol`, `deterministic-core`, `deterministic-daemon` crates
- Implemented request/response types and run-state schema
- Added SQLite persistence with `/healthz` and `/rpc` endpoints
- Handlers: `run.prepare`, `workspace.summary`, `file.read`, `git.status`

### Milestone 2: MCP Gateway Skeleton
- Created `apps/chatgpt-mcp` TypeScript project
- Implemented MCP server bootstrap and tool registration
- Added daemon client and initial tool mappings

### Milestone 3: Minimal End-to-End Coding Loop
- Added handlers: `code.search`, `patch.apply`, `tests.run`, `git.diff`
- Added MCP tools: `search_code`, `apply_patch`, `run_tests`, `show_diff`
- Verified: prepare → inspect → patch → test → diff works end-to-end

### Milestone 3.1: Reliability and Contract Hardening
- Added GitHub workflow for milestone-scoped CI
- Implemented invariant checks for forbidden methods and tools
- Added response envelope pattern for consistent API shape
- Refined tool contracts and scope parameters

### Milestone 4: Deterministic Control-Plane Statefulness
- Expanded run-state model with `completedSteps`, `pendingSteps`, `lastAction`, etc.
- Added statuses: `prepared`, `active`, `blocked`, `awaiting_approval`, `done`, `failed`
- New internal methods: `run.refresh`, `run.replan`, `approval.resolve`
- New MCP tools: `refresh_run_state`, `replan_run`, `approve_action`
- Added SQLite `approvals` table and approval plumbing

### Milestone 4.1: SQLite Schema Migration Compatibility
- Implemented automatic schema migration using `ALTER TABLE ADD COLUMN`
- Added backward compatibility for older databases (Milestone 3 → 4)
- Safe deterministic defaults for new columns

### Milestone 5: Approval Policy Hardening
- Added deterministic approval policy layer (`approval_policy.rs`)
- Patch policy: gates deletes, large patches (>5 edits), sensitive paths, out-of-focus edits
- Test-run policy: gates non-standard make targets
- Added `focus_paths` and `policy_rationale` fields to support policy decisions
- Updated SQLite schema and migration for Milestone 5 columns

### Milestone 6: Deterministic Action Resumption and Retry Guidance
- Added `RetryableAction` model to protocol types with kind, summary, payload, validity, recommendation
- Extended `RunState` with `retryableAction` for persisted retry metadata
- Extended `RunRefreshResult`, `RunReplanResult`, `ApprovalResolveResult` with retryable action state
- When `patch.apply` or `tests.run` is blocked by approval policy, a retryable action is recorded
- On approval: retryable action is marked recommended; `recommendedTool` points to the blocked action's tool
- On denial: retryable action is invalidated; recommended next tool shifts to `replan_run`
- On replan with failure context: stale retryable actions are invalidated deterministically
- On replan without failure: valid retryable actions are preserved
- `replanDelta` field emitted by `run.replan` for concise change description
- Refresh surfaces retryable action metadata and warns on staleness
- SQLite migration adds `retryable_action` column with backward compatibility
- No new public MCP tools; no new internal daemon methods
- No autonomous continuation—ChatGPT must still invoke the next tool explicitly

### Milestone 7: Deterministic Run History, Audit Trail, and State Inspection
- Added three new read-only protocol types: `RunSummary`, `RunGetResult`, `RunHistoryEntry` and associated params/result structs
- New internal daemon methods: `runs.list`, `run.get`, `run.history`
- New public MCP tools: `list_runs`, `get_run_state`, `get_run_history` (all read-only)
- Added `audit_trail` SQLite table to persist key run events; migration adds it to older databases
- Key events recorded: run prepared, refresh performed, replan performed, approval created, approval resolved, patch applied, tests run
- `list_runs` supports limit, workspace, and status filters
- `run.get` returns the full authoritative run state with pending approvals, retryable action, and recommendations
- `run.history` returns the audit trail for a run (newest first, configurable limit)
- 13 new Rust persistence tests; TypeScript invariants test updated
- Architecture invariants maintained: no model calls, no autonomous tools, deterministic only

### Milestone 8: Deterministic Policy Configuration and Per-Run Execution Constraints
- Added `RunPolicy` struct to `deterministic-protocol`: `patchEditThreshold`, `deleteRequiresApproval`, `sensitivePathRequiresApproval`, `outsideFocusRequiresApproval`, `extraSafeMakeTargets`, `focusPaths`
- Added `RunPolicyInput` struct for optional partial policy input at prepare time; missing fields fall back to defaults
- `RunPrepareParams` accepts an optional `policy: RunPolicyInput` field
- `RunPrepareResult`, `RunRefreshResult`, and `RunGetResult` now include `effectivePolicy: RunPolicy`
- `RunState` persists the active `policyProfile: RunPolicy` in SQLite (`policy_profile` TEXT column)
- Approval policy (`approval_policy.rs`) uses per-run `RunPolicy` instead of hardcoded constants
- `focusPaths` are always copied into `RunPolicy.focusPaths` for backward compatibility
- `extraSafeMakeTargets` are normalised to lowercase at validation time
- SQLite migration M7→M8 adds `policy_profile TEXT NOT NULL DEFAULT '{}'`; older runs get default policy
- TypeScript `schemas.ts` exports `PolicyProfileInputSchema` (Zod) and `CodexPrepareRunInput` now includes `policy`
- `tools.ts` passes `policy` through to `run.prepare`
- 3 new Rust persistence tests (default, custom, migration); 6 TypeScript policy schema tests
- No new public MCP tools; no new internal daemon methods
- No backend model calls; no autonomous continuation

### Milestone 9: Deterministic Operation Preflight and Approval Preview
- Added `PreflightDecision` enum (`proceed` | `requires_approval`) to `deterministic-protocol`
- Added `PreflightResult` struct (shared result for both preflight methods): `decision`, `actionSummary?`, `riskReason?`, `policyRationale?`, `effectivePolicy`
- Added `PatchPreflightParams` and `TestsPreflightParams` to `deterministic-protocol`
- Added `patch.preflight` and `tests.preflight` daemon methods (read-only, no state mutation)
- Handlers reuse existing `evaluate_patch` / `evaluate_test_run` policy logic (no duplication)
- Added `preview_patch_policy` and `preview_test_policy` MCP tools in TypeScript
- TypeScript schemas: `PreviewPatchPolicyInput` and `PreviewTestPolicyInput` (Zod validated)
- 10 new Rust handler tests (proceed + requires-approval + no-mutation cases for both preflight methods, plus method registry)
- 8 new TypeScript tests (schema validation + no-hidden-agent regression)
- No backend model calls; no autonomous continuation; no state mutation from preview calls

### Milestone 11: Deterministic Run Reopening and Post-Finalization Continuation Controls
- Added `ReopenMetadata` struct to `deterministic-protocol`: `reason`, `reopenedAt`, `reopenedFromOutcomeKind`, `reopenCount`
- Added `RunReopenParams` and `RunReopenResult` to `deterministic-protocol`
- Added `reopen_metadata: Option<ReopenMetadata>` to `RunState`, `RunRefreshResult`, and `RunGetResult`
- Added `reopen_count: Option<u32>` to `RunSummary` for concise run listings
- Added `run.reopen` internal daemon method with deterministic lifecycle rules:
  - Only finalized runs may be reopened; active/prepared/awaiting-approval runs are rejected
  - Status is reset to `"active"` and `finalized_outcome` is cleared
  - Reopen metadata persists; `reopen_count` increments on each successive reopen
  - Reopening appends a `run_reopened` entry to the audit trail
  - No autonomous follow-up work is triggered
- Added `reopen_run` MCP tool in TypeScript (lifecycle tool, not a coarse autonomous tool)
- TypeScript schema: `ReopenRunInput` (Zod validated) — `runId`, `reason` (required, 1–500 chars)
- SQLite migration adds `reopen_metadata TEXT` column with backward compatibility (NULL default)
- Reopened runs expose authoritative continuation metadata in `run.get`, `run.refresh`, `runs.list`
- 9 new Rust handler tests (completed/failed/abandoned reopen, active rejection, unknown run, audit, persistence, run.get)
- 5 new Rust persistence tests (null for fresh run, roundtrip, increment, migration safety, list_runs)
- 9 new TypeScript tests (6 schema validation + 3 no-hidden-agent regression)
- No backend model calls; no autonomous continuation; no coarse tools introduced
- Added `RunOutcome` struct to `deterministic-protocol`: `outcomeKind`, `summary`, `reason?`, `finalizedAt`
- Added `RunFinalizeParams` and `RunFinalizeResult` to `deterministic-protocol`
- Added `VALID_OUTCOME_KINDS` constant: `["completed", "failed", "abandoned"]`
- Added `finalized_outcome: Option<RunOutcome>` to `RunState`, `RunRefreshResult`, and `RunGetResult`
- Added `outcome_kind: Option<String>` to `RunSummary` for concise run listings
- Added `run.finalize` internal daemon method with deterministic lifecycle rules:
  - `outcome_kind` must be one of `completed`, `failed`, `abandoned`
  - A run that is already finalized cannot be finalized again
  - Run status is set to `finalized:<outcome_kind>`
  - Finalization appends a `run_finalized` entry to the audit trail
  - No autonomous follow-up work is triggered
- Added `finalize_run` MCP tool in TypeScript (lifecycle tool, not a coarse autonomous tool)
- TypeScript schema: `FinalizeRunInput` (Zod validated) — `runId`, `outcomeKind`, `summary`, `reason?`
- SQLite migration adds `outcome_kind TEXT` and `finalized_outcome TEXT` columns with backward compatibility
- Runs can now be inspected as active or finalized with authoritative closure metadata
- 5 new Rust core tests (completed, failed, abandoned, invalid kind, duplicate finalization)
- 18 new Rust daemon/handler tests (finalize paths, persistence roundtrip, audit trail, migration, registry)
- 10 new TypeScript tests (8 schema validation + 2 no-hidden-agent regression)
- No backend model calls; no autonomous continuation; no coarse tools introduced

### Milestone 12: Deterministic Run Supersession and Replacement Lineage
- Added `supersedes_run_id`, `superseded_by_run_id`, `supersession_reason`, `superseded_at` fields to `RunState` (all `Option<String>`, Milestone 12)
- Added the same lineage fields to `RunGetResult` for direct inspection
- Added `supersedes_run_id` and `superseded_by_run_id` to `RunSummary` for concise run listings
- Added `RunSupersedeParams` and `RunSupersedeResult` to `deterministic-protocol`
- Added `run.supersede` internal daemon method with deterministic lifecycle rules:
  - Only finalized runs (`finalized:completed`, `finalized:failed`, `finalized:abandoned`) may be superseded
  - Active, prepared, or awaiting-approval runs are rejected deterministically
  - Supersession creates a new successor run in `"prepared"` status
  - Original run is marked with `superseded_by_run_id` (status remains finalized; history and plan preserved)
  - Successor run carries `supersedes_run_id` pointing to the original
  - Both runs share `supersession_reason` and `superseded_at` timestamp
  - Successor inherits workspace, focus paths, and policy profile from original
  - Successor starts with an empty plan (clean slate for ChatGPT to replan)
  - Supersession appends `run_superseded` (original) and `run_created_from_supersession` (successor) audit entries
  - No autonomous follow-up work is triggered
- Added `supersede_run` MCP tool in TypeScript (lifecycle tool, not a coarse autonomous tool)
- TypeScript schema: `SupersedeRunInput` (Zod validated) — `runId`, `newUserGoal?` (max 500 chars, optional), `reason` (required, 1–500 chars)
- SQLite migration adds `supersedes_run_id TEXT`, `superseded_by_run_id TEXT`, `supersession_reason TEXT`, `superseded_at TEXT` columns with backward compatibility (NULL default)
- Lineage metadata is visible in `run.get`, `runs.list`, and audit trail entries
- 12 new Rust core tests (completed/failed/abandoned supersession, active/prepared rejection, workspace/policy inheritance, history preservation, empty plan start, goal fallback, successor ID format)
- 8 new Rust handler tests (create successor, custom goal, rejection, unknown run, audit trail, run.get lineage)
- 5 new Rust persistence tests (roundtrip, default null, list_runs lineage, M12 migration)
- 12 new TypeScript tests (9 schema validation + 3 no-hidden-agent regression)
- No backend model calls; no autonomous continuation; no coarse tools introduced

### Milestone 13: Deterministic Run Archiving and Retention Controls
- Added `ArchiveMetadata` struct to `deterministic-protocol` (`reason`, `archived_at`)
- Added `archive_metadata: Option<ArchiveMetadata>` field to `RunState`
- Added `RunArchiveParams` and `RunArchiveResult` to `deterministic-protocol`
- Added `include_archived` and `archived_only` fields to `RunsListParams`
- Added `is_archived`, `archive_reason`, `archived_at` fields to `RunSummary`
- Added `archive_metadata` field to `RunGetResult`
- Added `Method::RunArchive` (`run.archive`) to the daemon method enum
- Added `deterministic_core::run_archive` module with deterministic eligibility rules:
  - Only finalized runs (`finalized:completed`, `finalized:failed`, `finalized:abandoned`) may be archived
  - Active, prepared, or awaiting-approval runs are rejected deterministically
  - Already-archived runs are rejected idempotently (not silently)
  - Archiving does not execute work, reopen, supersede, or continue the run
  - Archive metadata is applied to run state and persisted
- Added `handle_run_archive` in daemon handlers
- Updated `handle_runs_list` to pass `include_archived` / `archived_only` through to persistence
- Updated `handle_run_get` to expose `archive_metadata` in `RunGetResult`
- SQLite: added `is_archived INTEGER DEFAULT 0` and `archive_metadata TEXT` columns with backward-compatible migration (M13)
- `list_runs` persistence function updated: default excludes archived; `include_archived=true` includes all; `archived_only=true` returns only archived
- `RunSummary` populated with `is_archived`, `archive_reason`, `archived_at` from persistence query
- `run_archived` audit entry appended with archive reason on successful archive
- Added `ArchiveRunInput` Zod schema in TypeScript (`runId`, `reason` 1–500 chars)
- Extended `ListRunsInput` Zod schema with `includeArchived` and `archivedOnly` optional booleans
- Added `archive_run` to `REGISTERED_TOOL_NAMES` and registered the MCP tool
- Updated `list_runs` tool to pass archive filtering params to daemon
- TypeScript remains thin: validation + mapping + daemon calls only
- 11 new Rust core tests (archive completed/failed/abandoned, eligibility rejection, already-archived, metadata roundtrip, audit entry)
- 11 new Rust handler tests (archive completed/failed, rejections for active/prepared/unknown, audit trail, run.get visibility, list filtering: default/includeArchived/archivedOnly)
- 10 new Rust persistence tests (metadata roundtrip, default None, list filtering, summary fields, M13 migration)
- 14 new TypeScript tests (6 schema validation + 5 list filtering + 3 no-hidden-agent regression)
- No backend model calls; no autonomous continuation; no coarse tools introduced

### Milestone 14: Deterministic Run Unarchiving and Archive Restoration Controls
- Added `UnarchiveMetadata` struct to `deterministic-protocol` (`reason`, `unarchived_at`)
- Added `unarchive_metadata: Option<UnarchiveMetadata>` field to `RunState`
- Added `RunUnarchiveParams` and `RunUnarchiveResult` to `deterministic-protocol`
- Added `unarchive_reason`, `unarchived_at` fields to `RunSummary`
- Added `unarchive_metadata` field to `RunGetResult`
- Added `Method::RunUnarchive` (`run.unarchive`) to the daemon method enum
- Added `deterministic_core::run_unarchive` module with deterministic eligibility rules:
  - Only archived runs (with `archive_metadata`) may be unarchived
  - Non-archived runs are rejected deterministically
  - Already-unarchived runs are rejected
  - Unarchiving does not execute work, reopen, or change the finalized outcome
  - Original `archive_metadata` remains intact; `unarchive_metadata` is set on the run state
- Added `handle_run_unarchive` in daemon handlers
- Updated `handle_run_get` to expose `unarchive_metadata` in `RunGetResult`
- SQLite: added `unarchive_metadata TEXT` column with backward-compatible migration (M14)
- `is_archived` flag: a run is archived only if `archive_metadata` is set AND `unarchive_metadata` is not set
- After unarchiving, the run returns to the default `list_runs` visible set
- `archived_only=true` excludes unarchived (restored) runs
- `RunSummary` populated with `unarchive_reason`, `unarchived_at` from persistence query
- `run_unarchived` audit entry appended with unarchive reason on successful unarchive
- Added `UnarchiveRunInput` Zod schema in TypeScript (`runId`, `reason` 1–500 chars)
- Added `unarchive_run` to `REGISTERED_TOOL_NAMES` and registered the MCP tool
- TypeScript remains thin: validation + mapping + daemon calls only
- 7 new Rust core tests (unarchive completed/failed/abandoned, non-archived rejection, already-unarchived rejection, status unchanged, finalized outcome preserved)
- 9 new Rust handler tests (unarchive completed/failed, rejection for non-archived/unknown, audit trail, default list restoration, archived_only exclusion, run.get visibility, persistence roundtrip)
- 6 new Rust persistence tests (unarchive metadata roundtrip, default None, restored run in default list, excluded from archived_only, summary fields, M14 migration)
- 9 new TypeScript tests (6 schema validation + 3 no-hidden-agent regression)
- No backend model calls; no autonomous continuation; no coarse tools introduced

### Milestone 15: Deterministic Run Labeling and Operator-Visible Organization Metadata
- Added `RunAnnotation` struct to `deterministic-protocol` (`labels: Vec<String>`, `operator_note: Option<String>`)
- Added `annotation: Option<RunAnnotation>` field to `RunState`
- Added `labels: Vec<String>` and `operator_note: Option<String>` fields to `RunSummary`
- Added `annotation` field to `RunGetResult`
- Added `Method::RunAnnotate` (`run.annotate`) to the daemon method enum
- Added constants: `LABEL_MAX_LEN = 64`, `LABEL_MAX_COUNT = 16`, `OPERATOR_NOTE_MAX_LEN = 1000`
- Added `RunAnnotateParams` / `RunAnnotateResult` structs
- Added `label: Option<String>` filter field to `RunsListParams`
- Added `deterministic_core::run_annotate` module with deterministic rules:
  - Labels normalized to lowercase, deduplicated, sorted
  - Label validation: max 64 chars, only `[a-z0-9_-]`, at most 16 per run
  - Operator note bounded to 1000 chars; empty string clears the note
  - Requires at least one of `labels` or `operator_note`
  - Does not execute work, replan, reopen, finalize, archive, unarchive, or supersede
- Added `handle_run_annotate` in daemon handlers
- Updated `handle_run_get` to expose `annotation` in `RunGetResult`
- Updated `handle_runs_list` to pass label filter to persistence; post-query in-Rust label filtering
- SQLite: added `annotation TEXT` column with backward-compatible migration (M15)
- `run_annotated` audit entry appended with labels and note_updated flag on successful annotation
- Added `annotate_run` to `REGISTERED_TOOL_NAMES` and registered the MCP tool
- Added `AnnotateRunInput` Zod schema; extended `ListRunsInput` with `label?: string`
- 23 new Rust tests (core: normalization, deduplication, invalid character rejection, note validation, empty params rejection, persistence roundtrip, status unchanged; handler: labels, note, normalization, persistence, audit, run.get visibility, runs.list visibility, label filter, status unchanged, empty rejection, invalid label rejection; persistence: annotation roundtrip, default None, list filter by label, summary carries annotation, M15 migration)
- 17 new TypeScript tests (12 schema validation + 2 list label field + 3 no-hidden-agent regression)
- No backend model calls; no autonomous continuation; no coarse tools introduced

## Current Implementation Surface

### Public MCP Tools (22)

| Tool | Description |
|------|-------------|
| `codex_prepare_run` | Initialize a deterministic coding run |
| `get_workspace_summary` | Get workspace overview and detected tooling |
| `read_file` | Read file contents with optional line ranges |
| `git_status` | Get working tree status |
| `search_code` | Search for text/symbol matches |
| `apply_patch` | Apply validated patches (policy-gated) |
| `run_tests` | Execute whitelisted test commands (policy-gated) |
| `show_diff` | Get diff summary or patch text |
| `refresh_run_state` | Read-only run state snapshot |
| `replan_run` | Deterministic rule-based replanning |
| `approve_action` | Resolve pending approvals |
| `list_runs` | List known runs with status and metadata; supports `includeArchived` / `archivedOnly` / `label` filtering (read-only) |
| `get_run_state` | Get authoritative current state of a run (read-only) |
| `get_run_history` | Get audit trail of key events for a run (read-only) |
| `preview_patch_policy` | Preview patch policy decision without applying changes (read-only, Milestone 9) |
| `preview_test_policy` | Preview test-run policy decision without executing tests (read-only, Milestone 9) |
| `finalize_run` | Explicitly close a run with a structured outcome record (Milestone 10) |
| `reopen_run` | Reopen a finalized run for deterministic continuation (Milestone 11) |
| `supersede_run` | Create a successor run that explicitly replaces a finalized run with preserved lineage (Milestone 12) |
| `archive_run` | Explicitly archive a finalized run so it remains preserved but out of the active working set (Milestone 13) |
| `unarchive_run` | Explicitly unarchive (restore) an archived run back to the default active working set (Milestone 14) |
| `annotate_run` | Explicitly annotate a run with labels and/or operator note for organization (Milestone 15) |

### Internal Daemon Methods (22)

| Method | Description |
|--------|-------------|
| `run.prepare` | Initialize run state |
| `run.refresh` | Return updated run-state snapshot |
| `run.replan` | Deterministic replanning |
| `workspace.summary` | Workspace overview |
| `file.read` | Read file contents |
| `git.status` | Working tree status |
| `code.search` | Text/symbol search |
| `patch.apply` | Apply patches with policy checks |
| `tests.run` | Run tests with policy checks |
| `git.diff` | Diff summary/patch |
| `approval.resolve` | Resolve pending approvals |
| `runs.list` | List runs; supports `include_archived` / `archived_only` / `label` filtering (read-only) |
| `run.get` | Get full run state with approvals, retryable action, archive/unarchive/annotation metadata (read-only) |
| `run.history` | Get audit trail entries for a run (read-only) |
| `patch.preflight` | Evaluate patch policy without applying changes (read-only, Milestone 9) |
| `tests.preflight` | Evaluate test-run policy without executing tests (read-only, Milestone 9) |
| `run.finalize` | Close a run with structured outcome record (Milestone 10) |
| `run.reopen` | Reopen a finalized run for deterministic continuation (Milestone 11) |
| `run.supersede` | Create a successor run that supersedes a finalized run with preserved lineage (Milestone 12) |
| `run.archive` | Archive a finalized run for retention with audit trail (Milestone 13) |
| `run.unarchive` | Unarchive (restore) an archived run back to the default active working set (Milestone 14) |
| `run.annotate` | Annotate a run with labels and/or operator note; audited, persisted, deterministic (Milestone 15) |

### Run State Model

**Statuses:** `prepared`, `active`, `blocked`, `awaiting_approval`, `done`, `failed`, `finalized:completed`, `finalized:failed`, `finalized:abandoned`

**Fields:**
- `runId`, `workspaceId`, `userGoal`, `status`, `plan`
- `currentStep`, `completedSteps`, `pendingSteps`
- `lastAction`, `lastObservation`
- `recommendedNextAction`, `recommendedTool`
- `focusPaths` (Milestone 5)
- `latestDiffSummary`, `latestTestResult`
- `retryableAction` (Milestone 6) — structured metadata for the last gated/failed action
- `policyProfile` (Milestone 8) — effective `RunPolicy` governing the run
- `finalizedOutcome` (Milestone 10) — structured outcome record for closed runs
- `reopenMetadata` (Milestone 11) — compact reopen lineage metadata for reopened runs
- `supersedes_run_id` / `superseded_by_run_id` / `supersession_reason` / `superseded_at` (Milestone 12) — supersession lineage
- `archiveMetadata` (Milestone 13) — compact archive metadata (`reason`, `archivedAt`) for archived runs
- `unarchiveMetadata` (Milestone 14) — compact unarchive metadata (`reason`, `unarchivedAt`) for restored runs
- `annotation` (Milestone 15) — compact organization metadata: `labels: Vec<String>` and `operator_note: Option<String>`
- `warnings`
- `createdAt`, `updatedAt`

### Approval Policy (Milestones 5 & 8)

Policy knobs are now taken from the per-run `RunPolicy` profile (Milestone 8).

**Patch gates:**
- File deletion operations (when `deleteRequiresApproval` is true; default: true)
- Patches with more than `patchEditThreshold` edits (default: 5)
- Sensitive file paths (`.env`, `.ssh/`, `.git/`, `id_rsa`, etc.) when `sensitivePathRequiresApproval` is true (default: true)
- Paths outside declared `focusPaths` when `outsideFocusRequiresApproval` is true and focus is non-empty (default: true)

**Test-run gates:**
- `make` scope with non-standard targets (not in the built-in safe list or `extraSafeMakeTargets`)

**Default built-in safe make targets:** `test`, `check`, `lint`, `build`, `clean`, `all`, `verify`, `fmt`, `format`

### Per-Run Policy Profile (Milestone 8)

`RunPolicy` fields:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `patchEditThreshold` | `usize` | `5` | Max edits in one patch before approval |
| `deleteRequiresApproval` | `bool` | `true` | Whether file deletion always gates |
| `sensitivePathRequiresApproval` | `bool` | `true` | Whether sensitive paths always gate |
| `outsideFocusRequiresApproval` | `bool` | `true` | Whether out-of-focus edits gate (when focus is set) |
| `extraSafeMakeTargets` | `Vec<String>` | `[]` | Additional safe make targets (normalised to lowercase) |
| `focusPaths` | `Vec<String>` | `[]` | Copied from prepare params for backward compatibility |

## Verified

- ✅ 309 Rust tests pass (175 core + 132 daemon + 2 protocol)
- ✅ 90 TypeScript tests pass
- ✅ Clippy clean
- ✅ No forbidden methods or tools registered
- ✅ No model SDK dependencies in deterministic crates
- ✅ CI workflow validates all invariants

## Pending / Out of Scope

These are intentionally not implemented and not planned for the current phase:

- Approvals UI (backend plumbing only)
- Widgets or visual components
- OAuth or external authentication
- Advanced replanning with LLM assistance
- Worktree orchestration
- Background orchestration
- Any agent-owned runtime
- `run_command` / `command.exec` (not implemented, not needed)

## Next Likely Direction

If extending the project, likely next milestones would be:

1. **Policy amendment** — allow ChatGPT to tighten or relax constraints mid-run
2. **Workspace templates** — predefined workspace configurations
3. **Multi-workspace** — support for runs spanning multiple workspaces

## Repository Structure

```
codex-rs/
  deterministic-protocol/  # Shared types and method names
  deterministic-core/    # Deterministic logic and policy
  deterministic-daemon/  # HTTP JSON-RPC transport, SQLite persistence

apps/chatgpt-mcp/        # TypeScript MCP gateway

docs/                    # Architecture and contract documentation
.github/workflows/       # CI for deterministic subsystem
```

## Development Verification

See [DEVELOPMENT.md](./DEVELOPMENT.md) for local verification commands.
