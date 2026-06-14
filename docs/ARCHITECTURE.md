# Architecture

## Objective

Build a deterministic coding harness control plane that lets ChatGPT behave like it is operating in a Codex-like environment, without any backend LLM.

## Absolute rule

The only LLM in the stack is ChatGPT.

## Forbidden architecture

ChatGPT -> MCP tool -> Codex/OpenCode/Goose/other harness continues its own internal agent loop

This is forbidden even if the transport is MCP, ACP, JSON-RPC, or HTTP.

## Required architecture

User in ChatGPT
-> ChatGPT-hosted model
-> MCP server we own (native Rust)
-> filesystem / git / patch / tests / approvals / sandbox

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

```text
chatcodex/
  Cargo.toml
  Cargo.lock
  crates/
    mcp-server/       # Native Rust MCP server
    oauth/            # OAuth 2.1 authorization layer

codex-rs/             # upstream Codex checkout (do not modify)
deploy/chatcodex/     # deployment artifacts
docs/                 # documentation
```

## Rust crates

### chatcodex/crates/mcp-server

Native Rust MCP server:
- MCP tool catalog and dispatch
- Streamable HTTP transport
- Prometheus metrics
- Structured logging
- Graceful shutdown
- CORS and rate limiting
- Health endpoint

Depends on upstream Codex crates for:
- patch application (`codex-apply-patch`)
- execution (`codex-exec-server`)
- sandboxing (`codex-sandboxing`)
- protocol types (`codex-protocol`)

### chatcodex/crates/oauth

OAuth 2.1 authorization layer:
- Authorization server with PKCE
- Cloudflare Access JWT verification
- Bearer-token middleware
- Client registration
- Token introspection and revocation
- SQLite-backed storage

## Public MCP tools (11 total)

Deterministic control tools:
- `codex_prepare_run` — Initialize a coding run with goal and plan
- `refresh_run_state` — Read-only run state snapshot
- `replan_run` — Deterministic rule-based replanning
- `approve_action` — Resolve pending approvals

Workspace and file tools:
- `get_workspace_summary` — Workspace overview and detected tooling
- `read_file` — Read file contents with optional line ranges
- `search_code` — Text/symbol search with snippets

Execution tools (policy-gated):
- `apply_patch` — Apply patches (gates: delete, >5 edits, sensitive paths, out-of-focus)
- `run_tests` — Execute whitelisted test commands (gates: non-standard make targets)

Git tools:
- `show_diff` — Diff summary or patch text
- `git_status` — Working tree status

## First implementation slice

Implement only:
- docs and scaffolding
- native Rust MCP server
- OAuth authorization layer
- minimal end-to-end loop:
  - prepare
  - read
  - search
  - patch
  - test
  - diff

## Explicit non-goals for first slice

- widgets
- external sandbox providers
- worktree orchestration
- review workflows
- provider integrations
- any LLM calls
