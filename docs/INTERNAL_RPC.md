# Internal RPC

The MCP gateway talks to the Rust daemon over internal JSON-RPC over HTTP.

## Endpoint

- `POST /rpc`
- `GET /healthz`

## Envelope

Request:
```json
{
  "jsonrpc": "2.0",
  "id": "req_123",
  "method": "run.prepare",
  "params": {}
}
````

Response:

```json
{
  "jsonrpc": "2.0",
  "id": "req_123",
  "result": {
    "ok": true,
    "result": {},
    "run_state": {},
    "warnings": [],
    "audit_id": "aud_123"
  }
}
```

## Methods

### Milestone 1–3 methods

* `run.prepare`
* `workspace.summary`
* `file.read`
* `git.status`
* `code.search`
* `patch.apply`
* `tests.run`
* `git.diff`

### Milestone 4 methods

* `run.refresh`
* `run.replan`
* `approval.resolve`

### Milestone 7 methods (read-only)

* `runs.list`
* `run.get`
* `run.history`

### Milestone 8 changes

No new daemon methods. Existing methods extended:

* `run.prepare` — accepts optional `policy` input; returns `effectivePolicy`
* `run.refresh` — returns `effectivePolicy`
* `run.get` — returns `effectivePolicy`

### Milestone 9 methods (read-only preflight)

* `patch.preflight`
* `tests.preflight`

### Milestone 10 methods

* `run.finalize` — close a run with a structured outcome record

#### `run.finalize` params

```json
{
  "runId": "run_abc",
  "outcomeKind": "completed",
  "summary": "All steps completed successfully",
  "reason": null
}
```

Valid `outcomeKind` values: `"completed"`, `"failed"`, `"abandoned"`.

Returns `RunFinalizeResult`:
- `runId`
- `outcomeKind`
- `finalizedAt` — ISO 8601 timestamp
- `status` — e.g. `"finalized:completed"`
- `recommendedNextAction` — deterministic guidance string

### Milestone 11 methods

* `run.reopen` — reopen a previously finalized run for deterministic continuation

Only finalized runs (`finalized:completed`, `finalized:failed`, `finalized:abandoned`) may be reopened.
Active, prepared, or awaiting-approval runs cannot be reopened.
Reopening does not execute work; it transitions the run back to `"active"` status,
persists compact reopen metadata, and appends a `run_reopened` audit entry.

#### `run.reopen` params

```json
{
  "runId": "run_abc",
  "reason": "Found another bug after the run was marked completed"
}
```

`reason` is required (min 1 character) for auditability.

Returns `RunReopenResult`:
- `runId`
- `status` — `"active"` after a successful reopen
- `reopenedFromOutcomeKind` — the outcome kind that was cleared (e.g. `"completed"`)
- `reopenCount` — total number of times this run has been reopened
- `reopenedAt` — ISO 8601 timestamp
- `recommendedNextAction` — deterministic guidance string
- `recommendedTool` — `"refresh_run_state"` (always)

## Forbidden internal methods

Do not implement or surface:

* `turn.start`
* `turn.steer`
* `review.start`
* `agent.step`
* `run.continue`
* any method that implies backend-owned reasoning

## Method details

### run.prepare

Compile a deterministic run brief and initialize run state.

Params:
```json
{
  "workspaceId": "string",
  "userGoal": "string",
  "focusPaths": ["string"],
  "mode": "plan | refresh | repair | review",
  "policy": {
    "patchEditThreshold": 5,
    "deleteRequiresApproval": true,
    "sensitivePathRequiresApproval": true,
    "outsideFocusRequiresApproval": true,
    "extraSafeMakeTargets": []
  }
}
```

All `policy` fields are optional. Omitted fields use deterministic defaults
(matching pre-Milestone-8 behavior). `focusPaths` is always copied into the
effective policy's `focusPaths` field for backward compatibility.
`extraSafeMakeTargets` values are normalised to lowercase.

Returns: run brief + run ID + plan + `effectivePolicy` — the resolved active
`RunPolicy` that will govern this run (Milestone 8).

### run.refresh

Return an updated run-state snapshot.  Merges persisted state with
live workspace facts (git status, diff summary).  Read-only — does
not trigger actions or perform LLM reasoning.

Params: `{ runId: string }`

Returns: full run-state snapshot including `pendingApprovals`,
`latestDiffSummary`, `latestTestResult`, `retryableAction`, `warnings`,
and `effectivePolicy` (Milestone 8) — the active `RunPolicy` for this run.

Milestone 6: if a retryable action is persisted and invalid/stale,
`warnings` will include a staleness note.

### run.replan

Deterministically recompute the run plan.  Inputs include a reason,
optional new evidence strings, and optional failure context.  The
backend applies rule-based logic to update `pendingSteps`,
`recommendedNextAction`, and `recommendedTool`.

Params: `{ runId, reason, newEvidence?, failureContext? }`

Returns: updated plan fields + `replanSummary` + `retryableAction` +
`replanDelta`.

Milestone 6: if failure context is provided, any valid retryable
action is invalidated.  If no failure context, valid retryable
actions are preserved (but may be un-recommended).  `replanDelta`
describes what changed.

### approval.resolve

Resolve a pending approval.  Decision must be `"approve"` or
`"deny"`.  Approving the last pending approval unblocks the run;
denying any approval blocks the run.

Params: `{ runId, approvalId, decision, reason? }`

Returns: resolution summary + resulting run status +
`recommendedNextAction` + `recommendedTool` + `retryableAction`.

After approve (last pending): if a valid retryable action exists,
marks it recommended and points `recommendedTool` at the action's
tool.  If the retryable action is stale, recommends replanning.
After deny: invalidates the retryable action and recommends replanning
via `replan_run`.

### workspace.summary

Return a deterministic summary of the workspace.

### file.read

Read file contents or a bounded line range.

### git.status

Return working tree status.

### code.search

Return ranked text/symbol matches with snippets.

### patch.apply

Apply a validated patch within policy boundaries.

Before executing the patch, a deterministic approval policy is evaluated
using the run's effective `RunPolicy` (Milestone 8).
If the policy determines the patch is risky (e.g. file deletion with
`deleteRequiresApproval`, patch exceeds `patchEditThreshold`, sensitive
file path with `sensitivePathRequiresApproval`, or outside focus paths
with `outsideFocusRequiresApproval`), the handler creates a pending
approval and returns the result with `approvalRequired` set instead of
applying the patch.

Milestone 6: when a patch is blocked by policy, a `retryableAction`
record is persisted in the run state so ChatGPT can later identify
what to retry after approval.

### tests.run

Resolve and run a canonical test command.

Before executing the test, a deterministic approval policy is evaluated
using the run's effective `RunPolicy` (Milestone 8).
If the policy determines the command is risky (e.g. non-standard make
target not in `extraSafeMakeTargets`), the handler creates a pending
approval and returns the result with `approvalRequired` set instead of
running the test.

Milestone 6: when a test run is blocked by policy, a `retryableAction`
record is persisted in the run state so ChatGPT can later identify
what to retry after approval.

### git.diff

Return diff summary or patch text.

### runs.list (Milestone 7)

List known runs. Read-only.

Params: `{ limit?, workspaceId?, status? }`

Returns: `{ runs: RunSummary[], count }` ordered by `updatedAt` descending.

### run.get (Milestone 7)

Get the authoritative current state of a specific run. Read-only.

Params: `{ runId }`

Returns: `{ runState, pendingApprovals, retryableAction?, latestDiffSummary?, latestTestResult?, recommendedNextAction?, recommendedTool?, effectivePolicy, warnings }`

Milestone 8: `effectivePolicy` contains the active `RunPolicy` for the run.
`runState` also includes `policyProfile` with the same values.

### run.history (Milestone 7)

Get the audit trail entries for a run. Read-only.

Params: `{ runId, limit? }`

Returns: `{ runId, entries: RunHistoryEntry[], count }` ordered newest-first.

Events recorded: `run_prepared`, `refresh_performed`, `replan_performed`, `approval_created`, `approval_resolved`, `patch_applied`, `tests_run`.

### patch.preflight (Milestone 9)

Evaluate a proposed patch against the run's effective policy without applying
any changes.  Read-only — no files, approvals, run state, or audit trail are
modified.

Params: `{ runId, edits: PatchEdit[] }`

Returns: `PreflightResult` — `{ decision, actionSummary?, riskReason?, policyRationale?, effectivePolicy }`

`decision` is `"proceed"` or `"requires_approval"`.

### tests.preflight (Milestone 9)

Evaluate a proposed test run against the run's effective policy without
executing any tests.  Read-only — no commands are executed and no state is
mutated.

Params: `{ runId, scope, target?, reason? }`

Returns: `PreflightResult` — same shape as `patch.preflight`.
