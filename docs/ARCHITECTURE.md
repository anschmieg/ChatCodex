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
-> MCP server we own
-> internal JSON-RPC
-> deterministic Rust harness daemon
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

### deterministic-daemon
- HTTP JSON-RPC transport
- SQLite persistence with automatic schema migration
- handler wiring
- health endpoint

#### SQLite persistence

The daemon stores run state in a local SQLite database (`runs.db`). The persistence layer automatically migrates older databases to the current schema using `ALTER TABLE ADD COLUMN` for backward compatibility. This allows the daemon to start and operate correctly even when an older database (e.g., from Milestone 3) is present. Missing columns are added with safe deterministic defaults (empty JSON arrays `[]` for list fields, `NULL` for optional fields).

## TypeScript MCP gateway

Thin gateway:
- tool registration
- Zod schemas
- daemon client
- response formatting

No repo logic belongs here.

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

## Internal daemon methods (11 total)

Run lifecycle:
- `run.prepare` — Initialize run state
- `run.refresh` — Return updated run-state snapshot
- `run.replan` — Deterministic replanning
- `approval.resolve` — Resolve pending approvals

Workspace and file:
- `workspace.summary` — Workspace overview
- `file.read` — Read file contents
- `code.search` — Text/symbol search

Execution (policy-gated):
- `patch.apply` — Apply patches
- `tests.run` — Run tests

Git:
- `git.status` — Working tree status
- `git.diff` — Diff summary/patch

## First implementation slice

Implement only:
- docs and scaffolding
- deterministic daemon
- MCP gateway
- minimal end-to-end loop:
  - prepare
  - read
  - search
  - patch
  - test
  - diff

## Explicit non-goals for first slice

- widgets
- OAuth
- external sandbox providers
- worktree orchestration
- review workflows
- provider integrations
- any LLM calls
