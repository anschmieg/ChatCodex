---
applyTo: "chatcodex/**"
---

# Rust-specific instructions

## Goal

Implement the ChatCodex MCP server in Rust.

## Required crates

Create these crates in `chatcodex/crates/`:
- `mcp-server`
- `oauth`

## Required boundaries

- `mcp-server`: MCP tool catalog, request dispatch, and HTTP transport
- `oauth`: OAuth 2.1 authorization layer, Cloudflare Access, JWT verification

## Forbidden dependencies and behavior

- No model provider SDKs
- No hidden agent loop
- No runtime use of `turn/start`, `turn/steer`, `review/start`
- No autonomous "continue work" functionality

## Required features

- Native MCP tool catalog over streamable HTTP
- OAuth 2.1 authorization server with PKCE
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

## Design notes

- Favor small explicit structs over loose maps.
- Keep deterministic logic in Rust; TypeScript layer is validation + mapping only.
- If reusing upstream code, isolate it behind thin wrappers.
- Add a hard failure path for accidental model-call code.
