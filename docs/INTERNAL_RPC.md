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
`latestDiffSummary`, `latestTestResult`, `warnings`.

### run.replan

Deterministically recompute the run plan.  Inputs include a reason,
optional new evidence strings, and optional failure context.  The
backend applies rule-based logic to update `pendingSteps`,
`recommendedNextAction`, and `recommendedTool`.

Params: `{ runId, reason, newEvidence?, failureContext? }`

Returns: updated plan fields + `replanSummary`.

### approval.resolve

Resolve a pending approval.  Decision must be `"approve"` or
`"deny"`.  Approving the last pending approval unblocks the run;
denying any approval blocks the run.

Params: `{ runId, approvalId, decision, reason? }`

Returns: resolution summary + resulting run status.

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

### tests.run

Resolve and run a canonical test command.

### git.diff

Return diff summary or patch text.
