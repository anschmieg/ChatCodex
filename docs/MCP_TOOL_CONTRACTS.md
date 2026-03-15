# MCP tool contracts

These are the public tools exposed to ChatGPT.

## codex_prepare_run

Input:
- `workspaceId: string`
- `userGoal: string`
- `focusPaths?: string[]`
- `mode?: "plan" | "refresh" | "repair" | "review"`
- `policy?: PolicyProfileInput` (Milestone 8) — optional per-run policy configuration

`PolicyProfileInput` fields (all optional, omitted fields use defaults):
- `patchEditThreshold?: number` — max edits in one patch before approval (default: `5`)
- `deleteRequiresApproval?: boolean` — whether file deletion always gates (default: `true`)
- `sensitivePathRequiresApproval?: boolean` — whether sensitive path edits always gate (default: `true`)
- `outsideFocusRequiresApproval?: boolean` — whether out-of-focus edits gate when focus is set (default: `true`)
- `extraSafeMakeTargets?: string[]` — additional make targets that may run without approval (normalised to lowercase)

Returns structured content:
- `runId`
- `objective`
- `assistantBrief`
- `constraints`
- `plan`
- `currentStep`
- `recommendedNextAction`
- `recommendedTool`
- `status`
- `effectivePolicy` (Milestone 8) — the resolved active `RunPolicy` that will govern this run:
  - `patchEditThreshold`, `deleteRequiresApproval`, `sensitivePathRequiresApproval`, `outsideFocusRequiresApproval`, `extraSafeMakeTargets`, `focusPaths`

## get_workspace_summary

Input:
- `workspaceId: string`
- `focusPaths?: string[]`

Returns:
- root info
- detected language/tooling
- dirty files
- likely commands
- relevant paths

## read_file

Input:
- `runId`
- `path`
- `startLine?`
- `endLine?`
- `purpose?`

Returns:
- file content
- range metadata
- updated run state summary

## git_status

Input:
- `runId`

Returns:
- branch
- dirty files
- untracked files
- ahead/behind if available

## search_code

Input:
- `runId`
- `query`
- `pathGlob?`
- `maxResults?`

Returns:
- ranked matches
- snippets
- updated run-state summary

## apply_patch

Input:
- `runId`
- `edits[]`

Each edit:
- `path`
- `operation`
- `startLine?`
- `endLine?`
- `oldText?`
- `newText`
- `anchorText?`
- `reason`

### Approval policy

Before applying the patch, a deterministic policy is evaluated using the run's effective `RunPolicy` (Milestone 8).
The patch is gated (approval required) if any of the following hold:
- Any edit has `operation: "delete"` (file deletion) and `deleteRequiresApproval` is `true`
- More than `patchEditThreshold` edits in a single request (default: 5)
- Any path matches a sensitive pattern (`.env`, `.ssh`, `.git/`, `id_rsa`, etc.) and `sensitivePathRequiresApproval` is `true`
- Any path is outside the run's declared `focusPaths` (when non-empty) and `outsideFocusRequiresApproval` is `true`

If approval is required, the result includes `approvalRequired` with the
pending approval details and the patch is **not** applied.

### Returns

- `changedFiles` — list of affected file paths (empty if approval required)
- `diffStats` — summary of additions/deletions (empty if approval required)
- `approvalRequired?` — pending approval object if the patch was gated
- updated run-state summary

## run_tests

Execute a whitelisted test command in the workspace.

### Input

- `runId`: string — Run ID from codex_prepare_run
- `scope`: string — **Semantic test scope**. Accepted values:
  - **Framework names** (explicit): `"cargo"`, `"npm"`, `"pytest"`, `"make"`
  - **Semantic labels** (auto-resolved): `"unit"`, `"integration"`, `"all"`
- `target?`: string — Specific test target (e.g., test name, file path)
- `reason`: string — Why tests are being run (required for audit trail)

### Scope Resolution

1. If `scope` is a framework name, use it directly
2. If `scope` is a semantic label, detect framework via workspace files:
   - `Cargo.toml` exists → "cargo"
   - `package.json` exists → "npm"
   - `setup.py` or `pyproject.toml` exists → "pytest"
   - `Makefile` exists → "make"
3. If no framework detected, return error

### Validation

- `scope` must be non-empty and a supported value
- `reason` must be non-empty (for audit trail)
- Scope matching is case-insensitive

### Returns

- `resolvedCommand`: string — The actual command that was executed (empty if approval required)
- `exitCode`: number — Exit code from the test command (-1 if approval required)
- `stdout`: string — Standard output (truncated to 4096 chars)
- `stderr`: string — Standard error (truncated to 4096 chars)
- `summary`: string — Human-readable summary of results
- `approvalRequired?` — Pending approval object if the test was gated

### Approval policy

Before running the test, a deterministic policy is evaluated using the run's effective `RunPolicy` (Milestone 8).
The test is gated (approval required) if:
- `scope` is `"make"` and `target` is not a standard safe target
  (`test`, `check`, `lint`, `build`, `clean`, `all`, `verify`, `fmt`, `format`)
  and not in the run's `extraSafeMakeTargets` list

If approval is required, the result includes `approvalRequired` with the
pending approval details and the test is **not** executed.

### Errors

- Returns error for unsupported scope values
- Returns error if workspace framework cannot be auto-detected
- Returns error if test command fails to execute

## show_diff

Input:
- `runId`
- `paths?: string[]`
- `format?: "summary" | "patch"`

Returns:
- changed files
- diff summary
- optionally patch text
- updated run-state summary

## refresh_run_state

Refresh and return the current run state snapshot.  This is a read-only
operation: it does not trigger actions or perform LLM reasoning.

### Input

- `runId`: string — Run ID from codex_prepare_run

### Returns

- `runId`
- `status` — one of `prepared`, `active`, `blocked`, `awaiting_approval`, `done`, `failed`
- `currentStep`
- `completedSteps`
- `pendingSteps`
- `lastAction`
- `lastObservation`
- `recommendedNextAction`
- `recommendedTool`
- `pendingApprovals` — list of pending approval objects
- `latestDiffSummary`
- `latestTestResult`
- `retryableAction?` — retryable action metadata (Milestone 6), including:
  - `kind` — `"patch.apply"` or `"tests.run"`
  - `summary` — human-readable description
  - `isValid` — whether retry is still valid
  - `isRecommended` — whether retry is the recommended next step
  - `invalidationReason?` — why the action is no longer valid
  - `recommendedTool` — MCP tool to invoke for retry
- `effectivePolicy` (Milestone 8) — the active `RunPolicy` for this run
- `warnings`

### Behavior

- Merges persisted state with live workspace facts (e.g. current git diff)
- Surfaces retryable action metadata for resumption guidance
- Warns if a retryable action is stale or invalidated
- Does not mutate state or trigger any actions
- Does not call any LLM

## replan_run

Deterministically replan the run based on new evidence or failure context.

### Input

- `runId`: string — Run ID from codex_prepare_run
- `reason`: string — Why the run needs replanning
- `newEvidence?: string[]` — New evidence or observations
- `failureContext?: string` — Error or failure context that triggered replanning

### Returns

- `runId`
- `status`
- `currentStep`
- `pendingSteps`
- `recommendedNextAction`
- `recommendedTool`
- `replanSummary`
- `retryableAction?` — retryable action state after replanning (Milestone 6)
- `replanDelta?` — concise description of what changed during replanning (Milestone 6)

### Behavior

- Rule-based replanning only — no LLM calls
- If failure context is provided, inserts a recovery step and invalidates
  any stale retryable action
- If no failure context, preserves valid retryable actions
- Updates recommended next action and tool based on pending steps
- Emits a concise `replanDelta` describing what changed
- Persists updated state to SQLite

## approve_action

Resolve a pending approval (approve or deny a risky action).

### Input

- `runId`: string — Run ID from codex_prepare_run
- `approvalId`: string — Approval ID to resolve
- `decision`: `"approve"` | `"deny"`
- `reason?: string` — Reason for the decision

### Returns

- `approvalId`
- `runId`
- `decision`
- `status` — resulting run status after resolution
- `summary`
- `recommendedNextAction?` — suggested next step after resolution
- `recommendedTool?` — suggested MCP tool after resolution
- `retryableAction?` — retryable action state after the decision (Milestone 6)

### Behavior

- `"approve"` unblocks the run if no more pending approvals remain
  - If a valid retryable action exists, marks it recommended and points
    `recommendedTool` at the action's tool (e.g. `apply_patch`, `run_tests`)
  - If the retryable action is stale, recommends replanning instead
  - The action is never auto-retried — ChatGPT must invoke the next tool
- `"deny"` blocks the run
  - Invalidates the retryable action so it is no longer recommended
  - Recommends replanning via `replan_run`
- Multiple pending approvals are handled predictably: each is resolved independently
- Persists decision to SQLite
- Does not trigger any autonomous continuation

---

## list_runs (Milestone 7)

Read-only listing of known runs.

### Input

- `limit?: number` — Maximum runs to return (default 20, max 100)
- `workspaceId?: string` — Filter by workspace path
- `status?: string` — Filter by run status

### Returns

- `runs: RunSummary[]` — array of compact run summaries
  - `runId`, `workspaceId`, `userGoal`, `status`, `currentStep`, `totalSteps`, `createdAt`, `updatedAt`
- `count: number` — number of runs returned

### Behavior

- Read-only; does not modify any run state
- Results ordered by `updatedAt` descending (most recently modified first)

---

## get_run_state (Milestone 7)

Get the authoritative current state of a run.

### Input

- `runId`: string — Run ID to inspect

### Returns

- `runState` — full RunState (includes `policyProfile` field)
- `pendingApprovals` — current pending approvals
- `retryableAction?` — retryable action metadata if present
- `latestDiffSummary?` — latest diff summary
- `latestTestResult?` — latest test result
- `recommendedNextAction?` — current recommendation
- `recommendedTool?` — recommended MCP tool
- `effectivePolicy` (Milestone 8) — the active `RunPolicy` for this run
- `warnings[]` — active warnings

### Behavior

- Read-only; does not modify any run state
- Returns the same fields as `refresh_run_state` but without triggering a refresh operation

---

## get_run_history (Milestone 7)

Get the audit trail of key events for a run.

### Input

- `runId`: string — Run ID to retrieve history for
- `limit?: number` — Maximum entries to return (default 50, max 200)

### Returns

- `runId`
- `entries: RunHistoryEntry[]` — audit trail entries (newest first)
  - `entryId`, `runId`, `eventKind`, `summary`, `metadata?`, `occurredAt`
- `count: number` — number of entries returned

### Behavior

- Read-only; does not modify any run state
- Events include: `run_prepared`, `refresh_performed`, `replan_performed`, `approval_created`, `approval_resolved`, `patch_applied`, `tests_run`

## preview_patch_policy (Milestone 9)

Preview the policy decision for a proposed patch without applying any changes.

### Input

- `runId`: string — Run ID from `codex_prepare_run`
- `edits`: `PatchEdit[]` — Proposed edits to evaluate (identical structure to `apply_patch`)

### Returns

- `decision`: `"proceed"` | `"requires_approval"` — what would happen
- `actionSummary?`: string — human-readable summary (when gated)
- `riskReason?`: string — why the operation is considered risky (when gated)
- `policyRationale?`: string — which policy rule would trigger (when gated)
- `effectivePolicy`: `RunPolicy` — the policy profile used for evaluation

### Behavior

- Strictly read-only: no files modified, no approvals created, no audit entries written
- Reuses the same deterministic `evaluate_patch` logic as `apply_patch`

## preview_test_policy (Milestone 9)

Preview the policy decision for a proposed test run without executing tests.

### Input

- `runId`: string — Run ID from `codex_prepare_run`
- `scope`: string — test scope (e.g. `cargo`, `npm`, `make`)
- `target?`: string — specific test target within scope
- `reason?`: string — why the test run is being evaluated

### Returns

Same shape as `preview_patch_policy`.

### Behavior

- Strictly read-only: no tests executed, no state modified
- Reuses the same deterministic `evaluate_test_run` logic as `run_tests`

## Forbidden public tools

Do not expose:
- `continue_run`
- `resume_codex_thread`
- `fix_end_to_end`
- `agent_step`
- `turn_start`
- `codex_reply`
