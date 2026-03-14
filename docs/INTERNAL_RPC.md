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

## Methods for the first slice

* `run.prepare`
* `workspace.summary`
* `file.read`
* `git.status`
* `code.search`
* `patch.apply`
* `tests.run`
* `git.diff`

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
