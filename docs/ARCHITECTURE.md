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
- SQLite persistence
- handler wiring
- health endpoint

## TypeScript MCP gateway

Thin gateway:
- tool registration
- Zod schemas
- daemon client
- response formatting

No repo logic belongs here.

## Public MCP tools

Deterministic control tools:
- `codex_prepare_run`
- `refresh_run_state`
- `replan_run`
- `approve_action`
- `get_workspace_summary`

Fine-grained execution tools:
- `search_code`
- `read_file`
- `apply_patch`
- `run_tests`
- `run_command`
- `show_diff`
- `git_status`

## Internal daemon methods

- `run.prepare`
- `run.refresh`
- `run.replan`
- `run.get`
- `workspace.summary`
- `workspace.register`
- `approval.resolve`
- `code.search`
- `file.read`
- `patch.apply`
- `tests.run`
- `command.exec`
- `git.diff`
- `git.status`

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
