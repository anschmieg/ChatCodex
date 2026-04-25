# AGENTS.md

## Mission

This repository implements a **deterministic coding harness control plane for ChatGPT** with an optional **dual-mode architecture**:

- **Deterministic mode** (default): ChatGPT is the only LLM. The Rust daemon and MCP gateway stay deterministic and code-only. No model calls, no agent loops, no autonomous continuation.
- **Hybrid mode** (opt-in): ChatGPT remains the orchestrator but may start bounded implementation-worker runs using a configured OpenAI-compatible external/local LLM. Workers return proposed patches only — ChatGPT explicitly applies changes through `apply_patch`.

The required architecture is:

ChatGPT-hosted model
→ MCP server we own
→ deterministic Rust harness daemon
→ filesystem / git / patch / test / approvals / sandbox

Hybrid workers (when enabled) call an OpenAI-compatible endpoint but never write files directly.

## Absolute rules

1. **The only LLM in deterministic mode is ChatGPT.**
   - Do not add any provider SDKs or model calls in deterministic mode.
   - Do not call OpenAI, Anthropic, Google, xAI, Ollama, or any other model provider in deterministic mode.
   - Do not create a hidden agent loop anywhere in the backend.

2. **Hybrid mode is opt-in and worker-constrained.**
   - Hybrid mode requires explicit server config (`CHATCODEX_HYBRID_ENABLED=true`) and per-run `harnessMode: "hybrid"`.
   - Hybrid workers are bounded: they receive a task goal and return proposed patches.
   - Hybrid workers never apply patches, run tests, commit, or mutate files directly.
   - ChatGPT must explicitly invoke `apply_patch` and `run_tests` to make workspace changes.
   - Workers return proposed edits only — all actual file writes go through `apply_patch`.

   - v1 supports OpenAI-compatible HTTP only (not Anthropic). Ollama works through an OpenAI-compatible local endpoint when available.

2. **Forbidden architecture**
   - ChatGPT must never call a coarse tool that causes Codex or another harness to continue its own agent loop.
   - Do not expose or use runtime flows such as:
     - `turn/start`
     - `turn/steer`
     - `review/start`
     - `codex()`
     - `codex-reply()`
     - `continue_run`
     - `resume_thread`
     - `agent_step`
     - `fix_end_to_end`
   - The backend must never own planning/execution through an LLM.

3. **Required architecture**
   - ChatGPT reasons.
   - MCP tools are deterministic.
   - The Rust daemon is deterministic.
   - All policy enforcement is server-side.
   - All file writes happen through `apply_patch`.
   - All test execution happens through `run_tests`.
   - `run_command` is restricted and whitelisted.

4. **Public MCP tool surface**
   - `codex_prepare_run`
   - `refresh_run_state`
   - `replan_run`
   - `approve_action`
   - `get_workspace_summary`
   - `search_code`
   - `read_file`
   - `apply_patch`
   - `run_tests`
   - `show_diff`
   - `git_status`
   - **Hybrid mode tools (opt-in):**
     - `hybrid_prepare_worker_run`
     - `hybrid_start_worker_run`
     - `hybrid_get_worker_run`
     - `hybrid_cancel_worker_run`
     - `hybrid_list_worker_runs`

5. **Internal daemon JSON-RPC surface**
   - `run.prepare`
   - `run.refresh`
   - `run.replan`
   - `workspace.summary`
   - `approval.resolve`
   - `code.search`
   - `file.read`
   - `patch.apply`
   - `tests.run`
   - `git.diff`
   - `git.status`
   - **Hybrid mode methods (opt-in):**
     - `hybrid.worker.prepare`
     - `hybrid.worker.start`
     - `hybrid.worker.get`
     - `hybrid.worker.cancel`
     - `hybrid.worker.list`

## Scope for the first coding-agent task

Implement only the first substantial slice:

- Milestone 0: design docs and repo bootstrap
- Milestone 1: deterministic Rust daemon skeleton
- Milestone 2: TypeScript MCP gateway skeleton
- Milestone 3: minimal end-to-end loop

That means:

### Rust
Create:
- `codex-rs/deterministic-protocol`
- `codex-rs/deterministic-core`
- `codex-rs/deterministic-daemon`

Implement:
- protocol types
- run state model
- SQLite persistence
- `/healthz`
- `/rpc`
- handlers for:
  - `run.prepare`
  - `workspace.summary`
  - `file.read`
  - `git.status`
  - `code.search`
  - `patch.apply`
  - `tests.run`
  - `git.diff`

### TypeScript
Create:
- `apps/chatgpt-mcp`

Implement:
- MCP server bootstrap
- tool registration
- internal daemon client
- MCP tools for:
  - `codex_prepare_run`
  - `get_workspace_summary`
  - `read_file`
  - `git_status`
  - `search_code`
  - `apply_patch`
  - `run_tests`
  - `show_diff`

## Quality bar

- Prefer compiling code over placeholder docs.
- Prefer thin, real implementations over mocks.
- Do not silently skip the no-hidden-agent invariants.
- Keep TypeScript thin: validation + mapping + daemon calls only.
- Keep deterministic logic in Rust.
- Add tests for invariants where practical.
- If something from upstream Codex would introduce agent-owned inference, do not wire it in.
