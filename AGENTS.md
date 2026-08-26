# AGENTS.md

## Mission

This repository implements a **deterministic coding harness control plane for ChatGPT**.

The required architecture is:

ChatGPT-hosted model
→ MCP server we own (native Rust)
→ filesystem / git / patch / sandbox

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
   - Sanctioned exception: `memory_reflect` calls Hindsight's synthesis endpoint,
     which runs on Hindsight's own server-side LLM (a memory service, not a
     harness control loop). Approved by Adrian. `memory_search`/`memory_retain`
     are pure retrieval/storage and add no LLM.

3. **Required architecture**
   - ChatGPT reasons.
   - MCP tools are deterministic.
   - The Rust server is deterministic.
   - All policy enforcement is server-side.
   - Workspace source file writes happen through `apply_patch`.
   - Server-owned project/run metadata is persisted by deterministic atomic JSON beneath the workspace base.
   - `exec_command` runs inside a read-only filesystem sandbox.

4. **Public MCP tool surface**

   Project lifecycle:
   - `project_create` — create or register a persistent project
   - `project_select` — select a persistent project for later tools
   - `project_list` — list persistent projects in the client namespace
   - `project_get` — get a project by id or the selected project

   Run lifecycle:
   - `run_start` — start and select a persistent coding run
   - `run_list` — list persistent runs
   - `run_get` — get a run by id or the selected run
   - `run_update` — update phase/status/plan/checklist/checkpoints/limits counters
   - `run_resume` — select an existing non-terminal run after ChatGPT is asked to continue
   - `run_cancel` — cancel a non-completed run
   - `run_followup_lease` — acquire a duplicate-safe app continuation lease

   Legacy-compatible lifecycle:
   - `setup_workspace` — clone a git repo or create a scratch sandbox
   - `todo` — manage a persistent task checklist
   - `update_plan` — replace the task plan

   When a run is selected, `setup_workspace`, `todo`, `update_plan`, and all workspace tools operate on that run's project state. Without a selected run, they retain legacy selected-project fallback behavior where practical.

   Sandboxed filesystem tools:
   - `exec_command` — run a bash command in the read-only sandbox
   - `write_stdin` — write to or poll a running command session
   - `read_file` — read a file with optional line range
   - `search_code` — grep workspace source files
   - `list_directory` — list entries in a workspace directory
   - `apply_patch` — the *only* workspace source write path
   - `view_image` — display an image from the workspace

   Git tools (unsandboxed for writes, sandboxed for reads):
   - `git` — run arbitrary local git commands (network blocked)
   - `git_status` — `git status --porcelain`
   - `git_diff` — `git diff` with optional paths/staged flag
   - `git_commit` — create a local commit
   - `git_branch` — create or move a branch
   - `git_checkout` — switch branches
   - `git_push` — push a branch to the origin remote, optionally opening a PR (the only sanctioned outbound git op; authenticated via server-side `CHATCODEX_GITHUB_TOKEN`)

   Private repositories: `setup_workspace` authenticates clones of
   `https://github.com/...` URLs using the server-side
   `CHATCODEX_GITHUB_TOKEN` environment variable (credential-free URLs only —
   URLs with embedded credentials are rejected). The token is never written
   to `.git/config` or exposed to the model: it is injected into git via an
   ephemeral credential helper that reads the environment variable, and
   `git_push` passes it through the same mechanism. Without the token
   configured, public-repo behavior is unchanged.

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
- Tool handlers for all tools in the public surface above

## Build pipeline

ChatCodex uses a tuned Cargo build pipeline for <2 min incremental compilation:

### Configuration
- **Profile** (`chatcodex/Cargo.toml`): `opt-level = 1`, `debug = 1` (line tables), `codegen-units = 256`
- **Linker**: Default (BFD ld on ARM64; mold preferred on x86_64)
- **Caching**: `sccache` via `chatcodex/.cargo/config.toml:rustc-wrapper`
- **Target dir**: Redirected to `~/.cache/codex-rs/chatcodex-target` by `chatcodex/.envrc`
- **sccache**: 2G limit, stored in `~/.cache/codex-rs/sccache`

### Commands
```bash
# Build (in chatcodex/)
cd chatcodex && cargo build -p chatcodex-mcp-server

# Full clean + rebuild
scripts/clean-chatcodex-build.sh
cd chatcodex && cargo build -p chatcodex-mcp-server

# Disk cleanup
cargo cache -a                          # Remove stale registry/git checkouts
scripts/clean-chatcodex-build.sh        # Wipe all build artifacts + sccache
```

### Performance
| Scenario | Time |
|----------|------|
| Full build (cold) | ~13 min |
| Incremental (1 crate changed) | ~15 s |
| Incremental (2 crates changed) | ~51 s |

## Quality bar

- Prefer compiling code over placeholder docs.
- Prefer thin, real implementations over mocks.
- Do not silently skip the no-hidden-agent invariants.
- Keep deterministic logic in Rust.
- Add tests for invariants where practical.
- If something from upstream Codex would introduce agent-owned inference, do not wire it in.
