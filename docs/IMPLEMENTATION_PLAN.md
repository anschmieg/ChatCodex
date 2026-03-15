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

## Potential future milestones

If extending the project, likely next steps:

### Milestone 7: enhanced policy
- User-configurable approval thresholds
- More granular policy rules
- Policy configuration persistence

### Milestone 8: run history
- Persistence of completed runs
- Searchable run history
- Run comparison and diff

### Milestone 9: workspace templates
- Predefined workspace configurations
- Template sharing
- Project scaffolding
