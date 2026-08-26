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
-> persistent project/run state / filesystem / git / patch / sandbox

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
- Persistent project and run lifecycle state
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
- Token/client storage

## Public MCP tools

Project lifecycle:
- `project_create` — Create or register a persistent repo, workspace, or scratch project
- `project_select` — Select an existing persistent project
- `project_list` — List projects in the client namespace
- `project_get` — Get a project by id or return the selected project

Run lifecycle:
- `run_start` — Start and select a persistent coding run
- `run_list` — List persistent runs
- `run_get` — Get a run by id or return the selected run
- `run_update` — Update phase, status, plan, checklist, checkpoints, and counters
- `run_resume` — Select a non-terminal run after ChatGPT is asked to continue
- `run_cancel` — Cancel a non-completed run
- `run_followup_lease` — Acquire a duplicate-safe app continuation lease

Legacy-compatible lifecycle:
- `setup_workspace` — Clone a repo or create a scratch sandbox and register it as a project
- `update_plan` — Replace the selected run's plan or legacy plan state
- `todo` — Manage the selected run's checklist or legacy checklist state

Workspace and file tools:
- `read_file` — Read file contents with optional line ranges
- `search_code` — Text search with snippets
- `list_directory` — List workspace directory entries
- `view_image` — Display a workspace image

Execution tools:
- `exec_command` — Run a command in the read-only sandbox
- `write_stdin` — Interact with a running command session
- `apply_patch` — Apply patches; the only workspace source write path

Git tools:
- `git` — Run local-only git commands with outbound operations blocked
- `git_status` — Working tree status
- `git_diff` — Diff summary or patch text
- `git_commit` — Create a local commit when run autonomy permits it
- `git_branch` — Create or move a local branch
- `git_checkout` — Switch branches

## Persistent lifecycle

Project and run state is namespaced by `CHATCODEX_CLIENT_ID` and persisted as
atomic JSON beneath the workspace base. The selected run/project in persisted
state is the authoritative coding context. Coding tools operate on the selected
run's project when present and fall back to the selected project for legacy
clients.

Runs carry objective, acceptance criteria, phase, status, plan, checklist,
checkpoints, autonomy limits, continuation lease/counter state, and timestamps.
The server validates transitions and limits deterministically; it never
executes work on its own.

## ChatGPT App resource

The server exposes a run-status MCP app resource at
`ui://chatcodex/run-status.html`. The component requests a continuation only
after a duplicate-safe lease is granted for an active run with remaining work.
The follow-up message contains only the run id.

## First implementation slice

Implement only:
- docs and scaffolding
- native Rust MCP server
- OAuth authorization layer
- minimal end-to-end loop:
  - create/select project
  - start/select run
  - read
  - search
  - patch
  - verify
  - diff

## Explicit non-goals for first slice

- external sandbox providers
- worktree orchestration
- review workflows
- provider integrations
- any LLM calls
