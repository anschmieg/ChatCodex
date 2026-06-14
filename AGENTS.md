# AGENTS.md

## Mission

This repository implements a **deterministic coding harness control plane for ChatGPT**.

The required architecture is:

ChatGPT-hosted model
→ MCP server we own (native Rust)
→ filesystem / git / patch / test / approvals / sandbox

## Absolute rules

1. **The only LLM in the stack is ChatGPT.**
   - Do not add any provider SDKs or model calls.
   - Do not call OpenAI, Anthropic, Google, xAI, Ollama, or any other model provider.
   - Do not create a hidden agent loop anywhere in the backend.

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
   - The Rust server is deterministic.
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

## Architecture

ChatCodex lives in its own workspace at `chatcodex/` and depends on upstream Codex crates in `codex-rs/` via path dependencies.

```text
chatcodex/
  Cargo.toml
  Cargo.lock
  crates/
    mcp-server/       # Native Rust MCP server
    oauth/            # OAuth 2.1 authorization layer

codex-rs/             # upstream Codex checkout (unchanged)
```

## Scope for coding-agent tasks

Implement features in the native Rust MCP server:

### Rust
Create or extend:
- `chatcodex/crates/mcp-server`
- `chatcodex/crates/oauth`

Implement:
- MCP tool catalog and dispatch
- Streamable HTTP transport
- OAuth 2.1 authorization server
- Cloudflare Access JWT verification
- Bearer-token middleware
- Prometheus metrics, structured logging, graceful shutdown
- Tool handlers for:
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
- Keep deterministic logic in Rust.
- Add tests for invariants where practical.
- If something from upstream Codex would introduce agent-owned inference, do not wire it in.
