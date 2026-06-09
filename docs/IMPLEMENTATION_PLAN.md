# Native Harness MCP Implementation Plan

> This file is the source of truth for repository status and takeover. Update it
> in the same commit as every material implementation change.

## Current Status

- Branch: `codex/native-harness-mcp`
- Phase: native dispatch complete; OAuth and approval bridge next
- Last updated: 2026-06-09
- Previous approach: custom `deterministic-*` Rust crates plus a TypeScript MCP
  gateway. It remains in the branch temporarily for reference but is deprecated.
- Current build health:
  - TypeScript source builds and typechecks.
  - Existing TypeScript tests are unreliable because ignored stale files in
    `apps/chatgpt-mcp/dist` are included by the test glob.
  - Rust `1.93.0` is installed and the native catalog contract test passes.
  - Validation must put the rustup `1.93.0` `bin` directory first in `PATH`
    and set `RUSTC`, `RUSTDOC`, and `RUSTFMT` explicitly because this machine
    has a broken Homebrew Rust earlier in `PATH`.
  - The Ubuntu image builds locally with the full `codex-core` dependency and
    passes health, bearer-authentication, MCP initialization, non-root, Git,
    ripgrep, and mount-point smoke checks.
  - The Rust builder needs more than `2 GiB`; `6 GiB` is the verified local
    configuration. The runtime image is not expected to need comparable memory.
  - The Coolify application `okgs4ck888w0ws48wow48co8` tracks branch
    `codex/native-harness-mcp`, builds
    `/deploy/chatcodex/Dockerfile`, exposes port `3000`, and checks `/healthz`.
  - Coolify deployment `e2qfqjefe8zg3uso0rmsomq5` successfully deployed commit
    `d9feb912bda5863cbc8fc0f68ceaf4de747dd5ed` on 2026-06-09. The application
    is `running:healthy` with zero restarts.
  - Local native dispatch tests pass for `exec_command`, persistent
    `write_stdin` sessions, and the executable's native `apply_patch`
    self-invocation mode.
  - A stdio MCP smoke test successfully called `update_plan` through
    `tools/call` and received the native `Plan updated` result.
  - Focused core tests, native MCP crate tests, touched-file formatting, and
    Clippy with warnings denied pass for the native-dispatch implementation.
  - Rebuilding the Docker image for this commit is currently blocked because
    the local Docker socket requires escalation and the Codex approval service
    is rejecting escalations after the account reached its usage limit.
  - The deployed endpoint is not yet connectable from ChatGPT. Cloudflare
    Access currently returns a browser-oriented `302`/WARP challenge instead
    of MCP OAuth discovery, and the origin accepts only one static bearer
    secret. The new native-dispatch build has not yet been deployed.

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

There is no model request, Codex turn, review generation, hidden agent loop, or
autonomous continuation in the backend. Upstream `SessionServices` currently
requires an inert `ModelClient` value; ChatCodex constructs it without
authentication and never schedules prewarm or invokes it.

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
   - construct a harness execution context without starting model work;
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

### MCP Compatibility Note

MCP tool calls carry JSON-object arguments, while Codex's preferred
`apply_patch` tool is freeform text. The adapter must use Codex's own
function-form `apply_patch` variant for the MCP catalog and dispatch it through
the same native handler. Do not invent a ChatCodex-specific patch schema.

## Work Queue

### M0: Documentation and Branch Bootstrap

Status: completed

- [x] Create `codex/native-harness-mcp`.
- [x] Record the approved architecture.
- [x] Commit design and implementation plan.

Acceptance:

- A new agent can identify the goal, current state, next task, and verification
  commands from this file alone.

### M1: Public Native Harness Facade

Status: completed

Files expected:

- `codex-rs/core/src/harness_mcp.rs` (new)
- `codex-rs/core/src/lib.rs`
- narrow visibility changes under `codex-rs/core/src/tools/`

Actions:

- [x] Add failing tests that request the native tool catalog.
- [x] Expose configured `ToolSpec` values without model construction.
- [x] Exclude agent-owned tools by disabling their native capabilities.
- [x] Add failing tests for direct native tool dispatch.
- [x] Build a harness context using existing Codex session/config primitives.
- [x] Dispatch explicit calls through the existing handlers.
- [x] Verify `exec_command`, `write_stdin`, and `apply_patch`.

Acceptance:

- Tool schemas serialize directly from Codex `ToolSpec`.
- Calls execute through Codex handlers and policy code.
- No model client or turn-generation API is invoked.

Implemented:

- `codex_core::harness_mcp::native_tool_catalog()` builds the catalog through
  the existing Codex registry.
- The initial native profile exposes `exec_command`, `write_stdin`,
  `update_plan`, `apply_patch`, and `view_image`.
- Model, web, connector, prompt, code-mode, JavaScript, artifact, and
  collaboration capabilities are disabled at profile construction.
- `NativeHarness` owns one persistent native `Session`, `TurnContext`,
  `ToolRouter`, and diff tracker. Calls use `ToolCallSource::Direct`; no turn
  or agent loop is started.
- The harness uses `workspace-write` plus `AskForApproval::Never` until M3.
  Native policy and sandbox code remain authoritative, and operations that
  require escalation fail instead of waiting for an unavailable approval UI.
- Unit tests use `DangerFullAccess` only to avoid unsupported nested
  `sandbox-exec` inside the Codex desktop test sandbox.

### M2: Native MCP Server

Status: completed

Files expected:

- `codex-rs/native-harness-mcp/Cargo.toml`
- `codex-rs/native-harness-mcp/src/main.rs`
- focused modules for catalog mapping, call dispatch, auth, and transport
- `codex-rs/Cargo.toml`

Actions:

- [x] Add the crate to the workspace.
- [x] Map Codex function `ToolSpec` values to MCP `Tool` without input-schema
      rewriting.
- [x] Implement `tools/list`.
- [x] Implement `tools/call`.
- [x] Use Codex's native function-form `apply_patch` schema because MCP call
      arguments cannot carry a raw freeform string.
- [x] Add stdio transport.
- [x] Add Streamable HTTP transport.
- [x] Add constant-time bearer-token validation.
- [x] Add `/healthz`.

Acceptance:

- MCP lists the same deterministic model-facing tools as the native catalog.
- MCP calls return native handler output.
- Agent-loop tools are absent.
- Unauthorized remote calls receive HTTP 401.

Implemented:

- `codex-native-harness-mcp` is a Rust workspace package.
- Its stdio server advertises the native Codex catalog through `tools/list`.
- Its HTTP server exposes Streamable HTTP at `/mcp`, requires
  `CHATCODEX_BEARER_TOKEN` (with `MCP_AUTH_TOKEN` retained as a deployment
  compatibility alias), and leaves `/healthz` unauthenticated for container
  orchestration.
- `tools/call` serializes the MCP argument object directly into Codex's native
  function payload and dispatches one named handler. Native object output is
  returned as MCP `structuredContent`.
- One shared `NativeHarness` preserves unified-exec process IDs across
  `exec_command` and `write_stdin` calls.
- The executable uses Codex's standard `codex-arg0` dispatcher for native
  `apply_patch` self-invocation and Linux sandbox/helper paths.

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

Status: in progress

Actions:

- [x] Read `CHATCODEX_WORKSPACE_ROOT`, defaulting to `/workspaces`.
- [ ] Resolve configured project paths beneath the canonical workspace root.
- [ ] Reject absolute or traversing paths outside the root.
- [x] Set Codex sandbox mode to `workspace-write`.
- [ ] Ensure escalation cannot grant access outside container-mounted roots.

Acceptance:

- All MCP operations are scoped beneath `/workspaces`.
- Symlink and traversal escapes are rejected.

### M5: Container and Coolify Deployment

Status: in progress

Files expected:

- `deploy/chatcodex/Dockerfile`
- `.dockerignore`
- deployment documentation

Actions:

- [x] Build the Rust binary in a builder stage.
- [x] Create an Ubuntu 24.04 runtime with Git and common shell utilities.
- [x] Run as a non-root `chatcodex` user.
- [ ] Make the image filesystem read-only except explicit mounts and tmpfs.
- [ ] Drop all capabilities and enable `no-new-privileges`.
- [ ] Add CPU, memory, and PID limits in Compose.
- [ ] Mount `/workspaces` and `/data`; the image creates both mount points,
      but Coolify persistent storage still needs verification.
- [ ] Document optional read-only SSH credential mounting.
- [ ] Add Coolify deployment instructions.

Implemented:

- `deploy/chatcodex/Dockerfile` matches the existing Coolify application path.
- The runtime image contains Git, curl, ripgrep, and CA certificates and runs
  as UID `10001`.
- The image sets `CHATCODEX_DATA_DIR=/data` and `CODEX_HOME=/data/codex`, so
  native helper aliases and harness state live on the data mount.
- Container builds disable the upstream workspace's fat LTO, use two Cargo
  build jobs, and set `opt-level=0` so full `codex-core` compilation fits
  moderate Docker/Coolify builders. Builds in a `2 GiB` Colima VM exhausted
  memory in upstream protocol and core crates; `6 GiB` is verified. BuildKit
  cache mounts preserve Cargo downloads and compiled dependencies across
  retries.
- Local container validation passed on 2026-06-09: `/healthz` returned exactly
  `{"status":"ok"}`, unauthenticated `/mcp` returned `401`, authenticated MCP
  initialization succeeded, UID `10001` was active, and Git `2.43.0` plus
  ripgrep `14.1.0` were available.
- Application `okgs4ck888w0ws48wow48co8` now runs this branch at
  `https://codex.nothing.pink`. Coolify's in-container health check passed on
  its first attempt during the rolling update.
- Coolify deployment `i3ajr3ar5u5ygi35m7pu9cxl` failed on 2026-06-09 when its
  cold, single-job Rust build was terminated after approximately ten minutes;
  no Rust compiler error was emitted. A second single-job attempt followed the
  same timeline and was cancelled before the same ceiling. Builder concurrency
  is now two jobs to use the deployment server's available CPU; the succeeding
  build completed the rolling update in approximately ten minutes.
- Direct public HTTP verification from this development machine is intercepted
  by Cloudflare Access and requests WARP authentication. Bearer enforcement and
  MCP initialization are verified against the exact image locally; deployed
  `/healthz` is verified by Coolify from inside the production container.
- ChatGPT readiness requires Cloudflare Access Managed OAuth (or another
  MCP-compliant OAuth 2.1 authorization server), RFC 9728 protected-resource
  discovery, and removal or replacement of the origin's static-bearer gate.
  The protected MCP endpoint must return `401` with a `Bearer`
  `WWW-Authenticate` challenge rather than a browser `302`.
- Native `tools/call` dispatch is implemented. OAuth discovery and token
  validation are now the remaining ChatGPT connection blockers.

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
- [ ] Exercise `exec_command` with `git status` in the container.
- [x] Exercise native `apply_patch` self-invocation.
- [x] Exercise a yielded process with `write_stdin`.
- [ ] Exercise approval approve and deny paths.
- [ ] Verify workspace escape rejection.
- [x] Verify HTTP authentication.
- [x] Build and smoke-test the Docker image.
- [x] Grep for provider/model calls and forbidden agent-loop surfaces.

Acceptance:

- ChatGPT can perform an explicit inspect/edit/test/diff loop using native Codex
  tools.
- The backend never starts a Codex/model turn.
- All focused tests, Clippy, formatting, and container checks pass.

Latest focused evidence:

- `cargo test -p codex-native-harness-mcp` passes, including native
  apply-patch mode.
- `cargo test -p codex-core harness_mcp::tests --no-default-features` passes
  all four catalog/dispatch tests.
- `cargo clippy -p codex-native-harness-mcp --all-targets -- -D warnings`
  passes.
- A raw stdio MCP exchange completed `initialize` and
  `tools/call(update_plan)` with `isError: false`.
- Docker and Coolify verification remain pending for this exact commit.

## Verification Commands

Run from `codex-rs/` unless noted. First pin the working toolchain:

```bash
export RUST_TOOLCHAIN_BIN="$HOME/.rustup/toolchains/1.93.0-aarch64-apple-darwin/bin"
export PATH="$RUST_TOOLCHAIN_BIN:$PATH"
export RUSTC="$RUST_TOOLCHAIN_BIN/rustc"
export RUSTDOC="$RUST_TOOLCHAIN_BIN/rustdoc"
export RUSTFMT="$RUST_TOOLCHAIN_BIN/rustfmt"

cargo fmt -p codex-native-harness-mcp --check
cargo test -p codex-native-harness-mcp
cargo test -p codex-core \
  harness_mcp::tests
cargo clippy -p codex-native-harness-mcp --all-targets -- -D warnings
```

The whole-workspace format check currently fails in the deprecated
`deterministic-*` crates; do not reformat those unrelated files as part of
native harness work.

Repository invariant scan:

```bash
rg -n \
  'turn/start|turn/steer|review/start|codex-reply|continue_run|resume_thread|agent_step|fix_end_to_end' \
  codex-rs/native-harness-mcp codex-rs/core/src/harness_mcp.rs
```

Container validation (allocate at least `6 GiB` to the builder):

```bash
docker build -f deploy/chatcodex/Dockerfile -t chatcodex:native-dev .
docker run --rm -d --name chatcodex-native-smoke \
  -p 3301:3000 \
  -e MCP_AUTH_TOKEN=container-smoke \
  chatcodex:native-dev
curl -fsS http://127.0.0.1:3301/healthz
docker exec chatcodex-native-smoke git --version
docker stop chatcodex-native-smoke
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
