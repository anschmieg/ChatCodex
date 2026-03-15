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

## Milestone 4: deterministic control-plane statefulness

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
- expanded state persists in SQLite
- refresh returns consistent snapshots
- replan deterministically updates plan
- approval resolution works end to end
- no hidden agent loop
- TypeScript remains thin

## Out of scope for this first task

- approvals UI
- widgets
- OAuth
- advanced replanning
- worktrees
- background orchestration
- any agent-owned runtime
