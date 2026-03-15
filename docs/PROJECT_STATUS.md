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

## Current Implementation Surface

### Public MCP Tools (14)

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
| `list_runs` | List known runs with status and metadata (read-only) |
| `get_run_state` | Get authoritative current state of a run (read-only) |
| `get_run_history` | Get audit trail of key events for a run (read-only) |

### Internal Daemon Methods (14)

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
| `runs.list` | List runs (read-only) |
| `run.get` | Get full run state with approvals and retryable action (read-only) |
| `run.history` | Get audit trail entries for a run (read-only) |

### Run State Model

**Statuses:** `prepared`, `active`, `blocked`, `awaiting_approval`, `done`, `failed`

**Fields:**
- `runId`, `workspaceId`, `userGoal`, `status`, `plan`
- `currentStep`, `completedSteps`, `pendingSteps`
- `lastAction`, `lastObservation`
- `recommendedNextAction`, `recommendedTool`
- `focusPaths` (Milestone 5)
- `latestDiffSummary`, `latestTestResult`
- `retryableAction` (Milestone 6) — structured metadata for the last gated/failed action
- `warnings`
- `createdAt`, `updatedAt`

### Approval Policy (Milestone 5)

**Patch gates:**
- File deletion operations
- Patches with >5 edits
- Sensitive file paths (`.env`, `.ssh/`, `.git/`, `id_rsa`, etc.)
- Paths outside declared `focusPaths`

**Test-run gates:**
- `make` scope with non-standard targets (not in: `test`, `check`, `lint`, `build`, `clean`, `all`, `verify`, `fmt`, `format`)

## Verified

- ✅ 129 Rust tests pass (deterministic-protocol, deterministic-core, deterministic-daemon)
- ✅ 3 TypeScript tests pass (MCP gateway invariants)
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

1. **Enhanced policy** — more granular approval rules, user-configurable policies
2. **Run history** — persistence of completed runs, searchable history
3. **Workspace templates** — predefined workspace configurations
4. **Multi-workspace** — support for runs spanning multiple workspaces

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
