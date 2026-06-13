# Native Harness MCP Design

## Purpose

ChatCodex exposes deterministic coding tools to ChatGPT. ChatGPT is the only
model and owns all reasoning and planning. The backend never starts a Codex
thread, turn, review, or other agent loop.

## Dependency Boundary

ChatCodex code lives in two independent crates:

- `codex-rs/native-harness-mcp`
- `codex-rs/native-harness-mcp-auth`

They depend only on public upstream crate APIs. No source file in `codex-core`
or another upstream crate is patched. The only upstream workspace change is
registering these two crates in `codex-rs/Cargo.toml`.

The harness uses:

- `codex-exec-server` for process and filesystem backends;
- `codex-sandboxing` for read-only command transformation;
- `codex-shell-command` for deterministic dangerous-command classification;
- `codex-apply-patch` for workspace writes;
- `codex-protocol` for permission profiles.

It does not import or instantiate a Codex session, model client, turn manager,
tool registry, or tool router.

## Public Tool Surface

The MCP catalog is a strict allowlist:

- `exec_command`
- `write_stdin`
- `update_plan`
- `apply_patch`
- `view_image`

Unknown tool names are rejected. Schemas are owned by the ChatCodex adapter so
upstream catalog changes cannot silently expand the public surface.

## Execution Policy

`exec_command` and `write_stdin` use the public exec-server process API.
Commands are rejected when upstream's public command classifier marks them
dangerous. Accepted commands are transformed through the public sandbox manager
with a read-only filesystem permission profile.

This means commands may inspect repositories and run operations that do not
write to the filesystem. They cannot modify source files, Git metadata, build
artifacts, package environments, or system state.

`apply_patch` is the only workspace write path. It calls the public
`codex_apply_patch::apply_patch` API with a workspace-write filesystem sandbox
whose writable root is the configured workspace. Temporary directories are not
added as writable roots.

`view_image` resolves paths beneath the configured workspace and reads through
the exec-server filesystem API. Absolute paths outside the workspace are
rejected.

`update_plan` is deterministic in-memory state. It validates that no more than
one plan item is `in_progress`.

## Workspace and Container Boundary

The canonical workspace root defaults to `/workspaces`. The deployment mounts
only the selected host project directory there. `/toolchains` is a persistent
Docker-managed volume for service state and installed toolchains. No host
`/data` path or Docker socket is mounted.

The runtime image uses a non-root user, a read-only root filesystem, dropped
capabilities, and `no-new-privileges`. These container controls are independent
of the command sandbox and remain required defense in depth.

## Authentication and Transport

The binary supports stdio and Streamable HTTP. HTTP exposes `/healthz`,
Prometheus metrics, OAuth discovery, dynamic client registration, authorization,
token, introspection, revocation, and JWKS endpoints. `/mcp` requires a bearer
token minted by the local OAuth layer after Cloudflare Access authentication.

## Non-Goals

The backend does not expose `turn/start`, `turn/steer`, `review/start`,
`codex()`, `codex-reply()`, continuation, resume, sub-agent, or model-provider
operations. It contains no provider SDK and makes no model request.

## Verification

The focused test suite verifies the exact catalog, path confinement,
dangerous-command classification, and patch application. Repository checks also
assert that no upstream implementation directory differs from the selected
upstream release.
