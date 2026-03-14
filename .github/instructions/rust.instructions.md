---
applyTo: "codex-rs/**"
---

# Rust-specific instructions

## Goal

Implement the deterministic backend in Rust.

## Required crates

Create these crates in `codex-rs/`:
- `deterministic-protocol`
- `deterministic-core`
- `deterministic-daemon`

## Required boundaries

- `deterministic-protocol`: shared request/response and model types only
- `deterministic-core`: all deterministic business logic
- `deterministic-daemon`: transport, persistence, and handler wiring

## Forbidden dependencies and behavior

- No model provider SDKs
- No hidden agent loop
- No runtime use of `turn/start`, `turn/steer`, `review/start`
- No autonomous “continue work” functionality

## Required features in the first slice

- run state model
- SQLite persistence
- JSON-RPC over HTTP
- handlers for:
  - `run.prepare`
  - `workspace.summary`
  - `file.read`
  - `git.status`
  - `code.search`
  - `patch.apply`
  - `tests.run`
  - `git.diff`

## Design notes

- Favor small explicit structs over loose maps.
- Add compile-time separation between deterministic and agent runtime concerns.
- If reusing upstream code, isolate it behind deterministic wrappers.
- Add a hard failure path for accidental model-call code.
