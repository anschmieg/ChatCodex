# MCP tool contracts

These are the public tools exposed to ChatGPT.

## codex_prepare_run

Input:
- `workspaceId: string`
- `userGoal: string`
- `focusPaths?: string[]`
- `mode?: "plan" | "refresh" | "repair" | "review"`

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

Before applying the patch, a deterministic policy is evaluated.
The patch is gated (approval required) if any of the following hold:
- Any edit has `operation: "delete"` (file deletion)
- More than 5 edits in a single request (large patch)
- Any path matches a sensitive pattern (`.env`, `.ssh`, `.git/`, `id_rsa`, etc.)
- Any path is outside the run's declared `focusPaths` (when non-empty)

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

Before running the test, a deterministic policy is evaluated.
The test is gated (approval required) if:
- `scope` is `"make"` and `target` is not a standard safe target
  (`test`, `check`, `lint`, `build`, `clean`, `all`, `verify`, `fmt`, `format`)

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
- `warnings`

### Behavior

- Merges persisted state with live workspace facts (e.g. current git diff)
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

### Behavior

- Rule-based replanning only — no LLM calls
- If failure context is provided, inserts a recovery step
- Updates recommended next action and tool based on pending steps
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

### Behavior

- `"approve"` unblocks the run if no more pending approvals remain
  - Recommends retrying the previously gated action
- `"deny"` blocks the run
  - Recommends replanning via `replan_run`
- Multiple pending approvals are handled predictably: each is resolved independently
- Persists decision to SQLite
- Does not trigger any autonomous continuation

## Forbidden public tools

Do not expose:
- `continue_run`
- `resume_codex_thread`
- `fix_end_to_end`
- `agent_step`
- `turn_start`
- `codex_reply`
