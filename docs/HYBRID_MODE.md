# Hybrid Mode Guide

Hybrid mode gives ChatGPT a second LLM ("the worker") to explore implementation
approaches in parallel. ChatGPT remains the orchestrator — it reviews the worker's
proposed edits and approves or rejects them before any filesystem mutation occurs.

---

## How It Works

```
ChatGPT (orchestrator)
  └─→ hybrid.worker.prepare  → creates a worker run (prepared)
  └─→ hybrid.worker.start   → worker LLM generates proposed patches
  └─→ hybrid.worker.get    → ChatGPT reads the worker's proposed_edits
  └─→ hybrid.patch.submit  → ChatGPT asks for approval to apply patches
  └─→ approval.resolve     → ChatGPT approves → patch is applied to workspace
```

The worker never writes files directly. All mutations go through ChatGPT's
`patch.apply` tool, gated by the approval chain. ChatGPT must explicitly
approve before any change lands.

---

## Enabling Hybrid Mode

```sh
# Required
CHATCODEX_HYBRID_ENABLED=true
CHATCODEX_HYBRID_PROVIDER_URL=https://your-worker-llm.com/v1
CHATCODEX_HYBRID_MODEL=gpt-4.1

# Optional — needed when the endpoint requires authentication
OPENAI_API_KEY=sk-...   # or whichever env var the provider needs
```

Or pass them inline when starting the daemon:

```sh
CHATCODEX_HYBRID_ENABLED=true \
CHATCODEX_HYBRID_PROVIDER_URL=http://localhost:11434/v1 \
CHATCODEX_HYBRID_MODEL=qwen2.5-coder \
cargo run -p deterministic-daemon
```

---

## Environment Variables

### Required

| Variable | Example | Description |
|---|---|---|
| `CHATCODEX_HYBRID_ENABLED` | `true` | Must be `true` to enable |
| `CHATCODEX_HYBRID_PROVIDER_URL` | `http://localhost:11434/v1` | Base URL of the worker LLM (OpenAI-compatible) |
| `CHATCODEX_HYBRID_MODEL` | `gpt-4.1` | Model name the worker endpoint accepts |

### Optional

| Variable | Default | Description |
|---|---|---|
| `CHATCODEX_HYBRID_API_KEY_ENV` | *(none)* | Name of an env var holding the API key (e.g., `OPENAI_API_KEY`) |
| `CHATCODEX_HYBRID_TIMEOUT_SECONDS` | `120` | Seconds before a worker call times out |
| `CHATCODEX_HYBRID_MAX_OUTPUT_TOKENS` | `8000` | Max tokens in the worker response |
| `CHATCODEX_HYBRID_TEMPERATURE` | `0.2` | Sampling temperature for the worker |

### Legacy/Alternative Names (still supported)

The daemon also reads these env var names (Phase 4/5 naming):

| Variable | Maps to |
|---|---|
| `CHATCODEX_HYBRID_PROVIDER_BASE_URL` | `CHATCODEX_HYBRID_PROVIDER_URL` |
| `CHATCODEX_HYBRID_WORKER_MODEL` | `CHATCODEX_HYBRID_MODEL` |
| `CHATCODEX_HYBRID_WORKER_API_KEY` | set via `CHATCODEX_HYBRID_API_KEY_ENV` |

---

## Provider Examples

### OpenAI (API)

```sh
CHATCODEX_HYBRID_ENABLED=true
CHATCODEX_HYBRID_PROVIDER_URL=https://api.openai.com/v1
CHATCODEX_HYBRID_MODEL=gpt-4.1
OPENAI_API_KEY=sk-...
```

### Anthropic (via OpenAI-compatible proxy or direct)

```sh
CHATCODEX_HYBRID_ENABLED=true
CHATCODEX_HYBRID_PROVIDER_URL=https://api.anthropic.com/v1
CHATCODEX_HYBRID_MODEL=claude-3-5-sonnet-20250620
ANTHROPIC_API_KEY=sk-ant-...
```

### Ollama (local)

```sh
CHATCODEX_HYBRID_ENABLED=true
CHATCODEX_HYBRID_PROVIDER_URL=http://localhost:11434/v1
CHATCODEX_HYBRID_MODEL=qwen2.5-coder
# No API key needed for localhost Ollama
```

To start Ollama:

```sh
ollama pull qwen2.5-coder
ollama serve  # starts on localhost:11434
```

### LM Studio

```sh
CHATCODEX_HYBRID_ENABLED=true
CHATCODEX_HYBRID_PROVIDER_URL=http://localhost:1234/v1
CHATCODEX_HYBRID_MODEL=meta-llama-3.1-8b-instruct
```

---

## MCP Tools for Hybrid Mode

Once the daemon is running with hybrid mode enabled, ChatGPT calls these tools
(in this approximate flow):

### 1. `hybrid_prepare_worker_run`

Prepares a worker run. ChatGPT provides:
- `runId` — the orchestrator run ID
- `taskGoal` — what the worker should do
- `focusPaths` — files the worker should focus on
- `contextFiles` — specific line ranges to make available to the worker

**Returns:** `workerRunId`, `status: "prepared"`, recommended next tool.

### 2. `hybrid_start_worker_run`

Starts the worker LLM call. Pass the `workerRunId` from step 1.

**Returns:** `status: "succeeded" | "failed"`, `summary`, `proposedEdits`.
On failure, includes a `failureMessage` with the reason.

### 3. `hybrid_get_worker_run`

Reads the current state of a worker run. Pass `workerRunId`.

**Returns:** Full `HybridWorkerRun` object including `status`, `proposedEdits`,
`summary`, `failureMessage`.

### 4. `hybrid_submit_proposed_patches`

Submits selected edits from a succeeded worker for approval. ChatGPT passes:
- `runId` — orchestrator run ID
- `workerRunId` — worker run ID
- `patchIndices` — which of the worker's `proposedEdits` to submit (0-based indices)

**Returns:** `approvalIds` — approval records that must be resolved before
`patch.apply` can succeed.

On approval (`approval.resolve` with `decision: "approve"`), the run's
`retryableAction` is set with `skipPolicy: true`, so the next `patch.apply`
call bypasses the approval policy and applies the changes directly.

### 5. `hybrid_cancel_worker_run`

Cancels a running or prepared worker. Pass `workerRunId` and `reason`.

### 6. `hybrid_list_worker_runs`

Lists all worker runs for an orchestrator run. Pass `runId` and optional `status` filter.

---

## Minimal End-to-End Example

A single ChatGPT prompt that exercises the full flow:

```
Use the hybrid_prepare_worker_run tool:
  runId: <any active run ID>
  taskGoal: "replace all occurrences of 'foo' with 'bar' in src/"
  contextFiles: []

Then use hybrid_start_worker_run with the workerRunId you received.

Then use hybrid_get_worker_run to read the worker's proposed_edits.

If proposed_edits is not empty, use hybrid_submit_proposed_patches
with patchIndices: [0, 1, ...] (all of them).

Then use approval_resolve with decision: "approve" for each approval ID.

Then use patch_apply with the edits from the worker.
```

---

## Architecture Notes

- **No hidden agent loops**: The daemon is fully deterministic. The worker LLM
  runs once per `hybrid.worker.start` call and returns. The daemon never
  calls the worker autonomously.
- **Policy bypass is approval-gated**: `skipPolicy: true` in `patch.apply` is
  only set when `approval.resolve` was called with `decision: "approve"`.
  Workers can never apply patches directly.
- **Concurrency**: Multiple worker runs can be active simultaneously for the
  same parent run (Phase 11). Use `hybrid_list_worker_runs` to track them.

---

## Testing Hybrid Mode Without a Real Worker

The daemon fails gracefully when no worker LLM is reachable:

```sh
# Point to a port with no server → connection-refused error
CHATCODEX_HYBRID_ENABLED=true \
CHATCODEX_HYBRID_PROVIDER_URL=http://127.0.0.1:19997 \
CHATCODEX_HYBRID_MODEL=mock \
cargo run -p deterministic-daemon
```

`hybrid.worker.start` will return `status: "failed"` with a connection error.
This lets you test the orchestration flow (prepare → start → get → cancel → list)
without a real model.

---

## Troubleshooting

**`hybrid.worker.start` returns `status: "failed"` with "connection refused"**

The worker LLM endpoint is not reachable. Check:
- Is Ollama running? (`ollama serve`)
- Is the URL correct? (must include `/v1` for OpenAI-compatible endpoints)
- Is there a firewall blocking the port?

**`hybrid.patch.submit` fails with "succeeded"**

The worker has not completed successfully. Check `hybrid.worker.get` to see
the actual status and `failureMessage`.

**ChatGPT doesn't see the hybrid tools**

The daemon must be started with `CHATCODEX_HYBRID_ENABLED=true`. Check the
daemon log for "hybrid mode enabled".