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

### run.refresh

Return an updated run-state snapshot.  Merges persisted state with
live workspace facts (git status, diff summary).  Read-only — does
not trigger actions or perform LLM reasoning.

Params: `{ runId: string }`

Returns: full run-state snapshot including `pendingApprovals`,
`latestDiffSummary`, `latestTestResult`, `retryableAction`, `warnings`.

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

Before executing the patch, a deterministic approval policy is evaluated.
If the policy determines the patch is risky (e.g. file deletion,
large patch, sensitive file path, outside focus paths), the handler
creates a pending approval and returns the result with
`approvalRequired` set instead of applying the patch.

Milestone 6: when a patch is blocked by policy, a `retryableAction`
record is persisted in the run state so ChatGPT can later identify
what to retry after approval.

### tests.run

Resolve and run a canonical test command.

Before executing the test, a deterministic approval policy is evaluated.
If the policy determines the command is risky (e.g. non-standard make
target), the handler creates a pending approval and returns the result
with `approvalRequired` set instead of running the test.

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

Returns: `{ runState, pendingApprovals, retryableAction?, latestDiffSummary?, latestTestResult?, recommendedNextAction?, recommendedTool?, warnings }`

### run.history (Milestone 7)

Get the audit trail entries for a run. Read-only.

Params: `{ runId, limit? }`

Returns: `{ runId, entries: RunHistoryEntry[], count }` ordered newest-first.

Events recorded: `run_prepared`, `refresh_performed`, `replan_performed`, `approval_created`, `approval_resolved`, `patch_applied`, `tests_run`.
