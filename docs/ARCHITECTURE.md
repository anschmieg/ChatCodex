# Architecture

## Objective

Build a deterministic coding harness control plane that lets ChatGPT behave like it is operating in a Codex-like environment, with an optional dual-mode architecture:

- **Deterministic mode** (default): ChatGPT is the only LLM. The backend is purely deterministic — no model calls, no agent loops, no autonomous continuation.
- **Hybrid mode** (opt-in): ChatGPT orchestrates bounded worker LLM runs via a configured OpenAI-compatible provider. Workers return proposed patches only; all actual file writes go through `apply_patch`.

## Absolute rule

- **Deterministic mode:** The only LLM in the stack is ChatGPT.
- **Hybrid mode:** ChatGPT orchestrates bounded worker runs; hybrid workers are never orchestrators.

In both modes, all actual file changes go through `apply_patch` and all actual test execution goes through `run_tests`.

## Forbidden architecture

ChatGPT -> MCP tool -> Codex/OpenCode/Goose/other harness continues its own internal agent loop

This is forbidden even if the transport is MCP, ACP, JSON-RPC, or HTTP.

In hybrid mode, a worker LLM must never be treated as a primary orchestrator — it may only produce proposed patches that ChatGPT reviews and applies.

## Required architecture

```
User in ChatGPT
  → MCP server we own
  → internal JSON-RPC
  → deterministic Rust harness daemon
  → filesystem / git / patch / tests / approvals / sandbox

Hybrid mode extension (opt-in):
  → optional OpenAI-compatible LLM provider (for bounded worker runs)
  → workers return proposed patches only
  → ChatGPT reviews and applies via apply_patch
```

v1: OpenAI-compatible HTTP only (covers OpenAI, Ollama OpenAI-compatible endpoints, LM Studio, etc.). Anthropic not implemented in v1.

## Why fork upstream Codex

We are preserving deterministic harness semantics from upstream Codex where useful:
- workspace concepts
- instruction layering
- sandbox ideas
- diff and patch mechanics
- approvals and state concepts

We are **not** preserving:
- model ownership
- turn generation
- review generation
- Codex-as-agent APIs

## Repository structure

- `codex-rs/`
  - upstream crates remain present
  - add:
    - `deterministic-protocol`
    - `deterministic-core`
    - `deterministic-daemon`

- `apps/chatgpt-mcp/`
  - TypeScript MCP gateway

## Rust crates

### deterministic-protocol
Shared method names and DTOs.

### deterministic-core
Deterministic logic:
- instruction compilation
- run-state transitions
- workspace summaries
- suspect file ranking
- policy enforcement
- patch validation
- test command resolution
- **hybrid provider config** (optional OpenAI-compatible HTTP client)
- **hybrid worker execution** (bounded, returns proposed patches only)

### deterministic-daemon
- HTTP JSON-RPC transport
- SQLite persistence with automatic schema migration
- handler wiring
- health endpoint

#### SQLite persistence

The daemon stores run state in a local SQLite database (`runs.db`). The persistence layer automatically migrates older databases to the current schema using `ALTER TABLE ADD COLUMN` for backward compatibility. This allows the daemon to start and operate correctly even when an older database is present. Missing columns are added with safe deterministic defaults (empty JSON arrays `[]` for list fields, `NULL` for optional fields).

## TypeScript MCP gateway

Thin gateway:
- tool registration
- Zod schemas
- daemon client
- response formatting

No repo logic belongs here.

## Public MCP tools

Deterministic mode tools (always available):
- `codex_prepare_run` — Initialize a coding run with goal and plan
- `refresh_run_state` — Read-only run state snapshot
- `replan_run` — Deterministic rule-based replanning
- `approve_action` — Resolve pending approvals
- `get_workspace_summary` — Workspace overview and detected tooling
- `read_file` — Read file contents with optional line ranges
- `search_code` — Text/symbol search with snippets
- `apply_patch` — Apply patches (gates: delete, >5 edits, sensitive paths, out-of-focus)
- `run_tests` — Execute whitelisted test commands (gates: non-standard make targets)
- `show_diff` — Diff summary or patch text
- `git_status` — Working tree status

Hybrid mode tools (opt-in, when `harnessMode: "hybrid"`):
- `hybrid_prepare_worker_run` — Prepare a bounded worker run (does not call LLM)
- `hybrid_start_worker_run` — Start a worker run (calls OpenAI-compatible provider)
- `hybrid_get_worker_run` — Get worker run status and proposed patches
- `hybrid_cancel_worker_run` — Cancel a prepared or running worker
- `hybrid_list_worker_runs` — List worker runs for a parent run

## Internal daemon methods

Deterministic mode (always available):
- `run.prepare` — Initialize run state
- `run.refresh` — Return updated run-state snapshot
- `run.replan` — Deterministic replanning
- `approval.resolve` — Resolve pending approvals
- `workspace.summary` — Workspace overview
- `file.read` — Read file contents
- `code.search` — Text/symbol search
- `patch.apply` — Apply patches
- `tests.run` — Run tests
- `git.status` — Working tree status
- `git.diff` — Diff summary/patch

Hybrid mode (opt-in):
- `hybrid.worker.prepare` — Prepare a worker run
- `hybrid.worker.start` — Start a worker run (calls provider)
- `hybrid.worker.get` — Get worker run
- `hybrid.worker.cancel` — Cancel a worker
- `hybrid.worker.list` — List worker runs for a parent

## Dual-mode safety model

### Deterministic mode (default)
- No model provider SDKs or API calls
- No hidden agent loops
- All file writes through `apply_patch`
- All test execution through `run_tests`

### Hybrid mode (opt-in)
- Requires `CHATCODEX_HYBRID_ENABLED=true` in server config
- Requires per-run `harnessMode: "hybrid"` at prepare time
- Workers are bounded implementation tools — not orchestrators
- Workers return `proposed_edits[]` only; they never call `patch.apply`
- ChatGPT reviews proposed edits and explicitly invokes `apply_patch`
- Concurrency limits enforced: max 3 global, max 3 per parent run
- Cancellation supported but does not mutate workspace files

## Explicit non-goals for v1

- Anthropic provider integration
- non-OpenAI-compatible provider integrations
- widgets
- OAuth
- external sandbox providers
- worktree orchestration
- review workflows
- any LLM calls in deterministic mode
