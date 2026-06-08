# Native Harness MCP Implementation Plan

> This file is the source of truth for repository status and takeover. Update it
> in the same commit as every material implementation change.

## Current Status

- Branch: `codex/native-harness-mcp`
- Phase: native harness facade design and implementation
- Last updated: 2026-06-08
- Previous approach: custom `deterministic-*` Rust crates plus a TypeScript MCP
  gateway. It remains in the branch temporarily for reference but is deprecated.
- Current build health:
  - TypeScript source builds and typechecks.
  - Existing TypeScript tests are unreliable because ignored stale files in
    `apps/chatgpt-mcp/dist` are included by the test glob.
  - Rust validation requires the repository toolchain in
    `codex-rs/rust-toolchain.toml` (`1.93.0`). The shell may resolve an older
    Homebrew Cargo unless commands use `rustup run 1.93.0`.

## Goal

Expose the model-facing tools of the official Codex harness to ChatGPT through a
remote MCP server. ChatGPT is the only LLM and owns every reasoning/tool loop.
The backend exposes deterministic Codex harness primitives only.

The MCP tool names, descriptions, input schemas, execution behavior, command
policy, approvals, process sessions, patch handling, and sandbox semantics must
come from the vendored Codex implementation rather than a parallel reimplementation.

## Required Architecture

```text
User
  -> ChatGPT-hosted model
  -> Streamable HTTP MCP
  -> native Rust harness MCP adapter
  -> Codex tool registry and handlers
  -> sandbox / filesystem / process / git
```

There is no provider SDK, model request, Codex turn, review generation, hidden
agent loop, or autonomous continuation in the backend.

## Product Scope

This is a personal, single-user application:

- one deployed ChatCodex instance;
- one configured workspace root, `/workspaces`;
- multiple projects may exist below that root;
- `/data` stores application state;
- bearer authentication is sufficient for the first remote release;
- Docker is the cross-platform deployment and host-isolation boundary;
- concurrent operations against one project do not need sophisticated tenancy
  or scheduling in the first release.

## Native Tool Surface

Tool definitions must be generated from Codex `ToolSpec` values at runtime.
Expected baseline tools include:

- `exec_command`
- `write_stdin`
- `apply_patch`
- `update_plan`
- other deterministic tools enabled by the selected Codex configuration

Git is invoked through `exec_command`, as in Codex. Codex command parsing,
known-safe command classification, approval policy, and sandbox enforcement
remain authoritative.

The adapter must exclude any tool that owns reasoning or agent execution,
including:

- `codex`
- `codex-reply`
- multi-agent spawn/send/wait/resume/close tools
- turn/review continuation APIs

## Approved Design Decisions

1. Implement the MCP server in Rust, not TypeScript.
2. Add the smallest public facade to `codex-core` needed to:
   - construct the native configured tool specs and registry;
   - construct a harness execution context without a model client;
   - dispatch one explicit tool call and return its native output.
3. Do not copy Codex schemas into the adapter.
4. Reuse Codex approval and sandbox machinery.
5. Bridge command and patch approvals to MCP elicitation.
6. Support stdio for local tests and Streamable HTTP for ChatGPT.
7. Use simple bearer authentication for remote HTTP.
8. Package the app as a non-root Ubuntu 24.04 multi-stage image.
9. Mount `/workspaces` read-write and `/data` read-write. Optional Git/SSH
   credentials are read-only.
10. Do not mount the Docker socket.

## Work Queue

### M0: Documentation and Branch Bootstrap

Status: in progress

- [x] Create `codex/native-harness-mcp`.
- [x] Record the approved architecture.
- [ ] Commit design and implementation plan.

Acceptance:

- A new agent can identify the goal, current state, next task, and verification
  commands from this file alone.

### M1: Public Native Harness Facade

Status: pending

Files expected:

- `codex-rs/core/src/harness_mcp.rs` (new)
- `codex-rs/core/src/lib.rs`
- narrow visibility changes under `codex-rs/core/src/tools/`

Actions:

- [ ] Add failing tests that request the native tool catalog.
- [ ] Expose configured `ToolSpec` values without model construction.
- [ ] Filter agent-owned tools by capability/category, not copied schemas.
- [ ] Add failing tests for direct native tool dispatch.
- [ ] Build a harness context using existing Codex session/config primitives.
- [ ] Dispatch explicit calls through the existing handlers.
- [ ] Verify `exec_command`, `write_stdin`, and `apply_patch`.

Acceptance:

- Tool schemas serialize directly from Codex `ToolSpec`.
- Calls execute through Codex handlers and policy code.
- No model client or turn-generation API is invoked.

### M2: Native MCP Server

Status: pending

Files expected:

- `codex-rs/native-harness-mcp/Cargo.toml`
- `codex-rs/native-harness-mcp/src/main.rs`
- focused modules for catalog mapping, call dispatch, auth, and transport
- `codex-rs/Cargo.toml`

Actions:

- [ ] Add the crate to the workspace.
- [ ] Map Codex `ToolSpec` to MCP `Tool` without schema rewriting.
- [ ] Implement `tools/list`.
- [ ] Implement `tools/call`.
- [ ] Preserve freeform `apply_patch` input.
- [ ] Add stdio transport.
- [ ] Add Streamable HTTP transport.
- [ ] Add constant-time bearer-token validation.
- [ ] Add `/healthz`.

Acceptance:

- MCP lists the same deterministic model-facing tools as the native catalog.
- MCP calls return native handler output.
- Agent-loop tools are absent.
- Unauthorized remote calls receive HTTP 401.

### M3: Native Approval Bridge

Status: pending

Actions:

- [ ] Add tests for an `exec_command` that requires approval.
- [ ] Translate native command approval events to MCP `elicitation/create`.
- [ ] Translate native patch approval events to MCP `elicitation/create`.
- [ ] Feed approve/deny results back to the suspended Codex operation.
- [ ] Deny conservatively when the client lacks elicitation support or disconnects.

Acceptance:

- Approval is consumed by the exact suspended operation.
- Approval does not create a retry loop.
- Denial prevents execution.

### M4: Workspace Confinement

Status: pending

Actions:

- [ ] Read `CHATCODEX_WORKSPACE_ROOT`, defaulting to `/workspaces`.
- [ ] Resolve configured project paths beneath the canonical workspace root.
- [ ] Reject absolute or traversing paths outside the root.
- [ ] Set Codex sandbox mode to `workspace-write`.
- [ ] Ensure escalation cannot grant access outside container-mounted roots.

Acceptance:

- All MCP operations are scoped beneath `/workspaces`.
- Symlink and traversal escapes are rejected.

### M5: Container and Coolify Deployment

Status: pending

Files expected:

- `Dockerfile`
- `docker-compose.yml`
- `.dockerignore`
- deployment documentation

Actions:

- [ ] Build the Rust binary in a builder stage.
- [ ] Create an Ubuntu 24.04 runtime with Git and common shell utilities.
- [ ] Run as a non-root `chatcodex` user.
- [ ] Make the image filesystem read-only except explicit mounts and tmpfs.
- [ ] Drop all capabilities and enable `no-new-privileges`.
- [ ] Add CPU, memory, and PID limits in Compose.
- [ ] Mount `/workspaces` and `/data`.
- [ ] Document optional read-only SSH credential mounting.
- [ ] Add Coolify deployment instructions.

Acceptance:

- The image starts on Docker/Coolify.
- Git works in mounted repositories.
- No host path other than configured mounts is visible.

### M6: Remove Deprecated Implementation

Status: pending

Actions:

- [ ] Delete `codex-rs/deterministic-protocol`.
- [ ] Delete `codex-rs/deterministic-core`.
- [ ] Delete `codex-rs/deterministic-daemon`.
- [ ] Delete `apps/chatgpt-mcp`.
- [ ] Remove obsolete workspace/manifests/docs references.
- [ ] Rewrite architecture and onboarding docs around the native harness.

Acceptance:

- There is one implementation of tool schemas, policy, patching, and execution:
  Codex.

### M7: End-to-End Validation

Status: pending

Actions:

- [ ] Compare MCP catalog names and schemas with the native Codex catalog.
- [ ] Exercise `exec_command` with `git status`.
- [ ] Exercise `apply_patch`.
- [ ] Exercise a yielded process with `write_stdin`.
- [ ] Exercise approval approve and deny paths.
- [ ] Verify workspace escape rejection.
- [ ] Verify HTTP authentication.
- [ ] Build and smoke-test the Docker image.
- [ ] Grep for provider/model calls and forbidden agent-loop surfaces.

Acceptance:

- ChatGPT can perform an explicit inspect/edit/test/diff loop using native Codex
  tools.
- The backend never starts a Codex/model turn.
- All focused tests, Clippy, formatting, and container checks pass.

## Verification Commands

Run from `codex-rs/` unless noted:

```bash
rustup run 1.93.0 cargo fmt --check
rustup run 1.93.0 cargo test -p codex-core -p native-harness-mcp
rustup run 1.93.0 cargo clippy -p codex-core -p native-harness-mcp --all-targets -- -D warnings
```

Repository invariant scan:

```bash
rg -n \
  'turn/start|turn/steer|review/start|codex-reply|continue_run|resume_thread|agent_step|fix_end_to_end' \
  codex-rs/native-harness-mcp codex-rs/core/src/harness_mcp.rs
```

Container validation:

```bash
docker compose build
docker compose config
docker compose up -d
curl -fsS http://127.0.0.1:3000/healthz
docker compose down
```

## Takeover Notes

Start with M1. Before editing:

1. Read `docs/NATIVE_HARNESS_MCP_DESIGN.md`.
2. Read `codex-rs/core/src/tools/spec.rs`,
   `codex-rs/core/src/tools/registry.rs`, and
   `codex-rs/core/src/tools/router.rs`.
3. Use tests first.
4. Keep changes to `codex-core` narrowly focused on exposing existing behavior.
5. Update this file whenever scope, status, files, validation evidence, or next
   actions change.

Do not restore or extend the deprecated deterministic control-plane design.
