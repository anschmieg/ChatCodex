# Native Harness MCP Design

## Purpose

ChatCodex exposes deterministic coding tools to ChatGPT. ChatGPT is the only
model and owns all reasoning and planning. The backend never starts a Codex
thread, turn, review, or other agent loop.

## Dependency Boundary

ChatCodex code lives in two independent crates:

- `chatcodex/crates/mcp-server`
- `chatcodex/crates/mcp-server-auth`

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
- `setup_workspace`
- `git`
- `git_status`
- `git_diff`
 - `git_commit`
 - `git_branch`
 - `git_checkout`
 - `read_file`
 - `search_code`
 - `list_directory`
 - `todo`

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

## Workspace Setup

The workspace root is a per-client mutable pointer. Before any filesystem,
command, or git tool runs, the client must call `setup_workspace` with either
a git URL or the literal string `"sandbox"`.

Sandbox workspaces are persistent scratch directories under
`/workspaces/clients/<client_id>/sandboxes/<name>` and are automatically
initialized as empty git repositories. Cloned repositories live under
`/workspaces/clients/<client_id>/repos/<name>`. When a target repo directory
already exists, `setup_workspace` verifies that its `origin` remote matches the
requested URL and returns it unchanged; a mismatch is a hard error.

Git clone errors are retried with exponential backoff for transient failures
(HTTP 429, 5xx, timeouts). Authentication, 404, DNS, and scheme errors are
returned to the client immediately.

## Git Tool Policy

A single generic `git` tool accepts local-only git subcommands. Outbound
## Git Tool Policy

The generic `git` tool accepts local-only git subcommands. Outbound network
operations (`push`, `fetch`, `pull`, `clone`, `ls-remote`, `remote add`,
`remote set-url`, `submodule update --init`) are rejected by the server.

Read-only git tools (`git_status`, `git_diff`, and read-only uses of `git`)
run through the exec-server read-only filesystem sandbox with no network
access.

Writable git operations cannot be expressed in the workspace-write sandbox
because the upstream sandbox protects `.git/` metadata under a writable
workspace root. The writable git surface (`git` for commands like `add`,
`rm`, `mv`; plus `git_commit`, `git_branch`, `git_checkout`) therefore runs
unsandboxed with declared network access restricted and outbound subcommands
still blocked by the parser. `apply_patch` remains the only path for
modifying working-tree file contents.

## Workspace and Container Boundary

The canonical workspace base defaults to `/workspaces`. The deployment mounts
only the selected host directory there. Each client receives a sub-directory
under `/workspaces/clients/<client_id>/`. `/toolchains` is a persistent
Docker-managed volume for service state and installed toolchains. No host
`/data` path or Docker socket is mounted.

The runtime image uses a non-root user, a read-only root filesystem, dropped
capabilities, and `no-new-privileges`. These container controls are independent
of the command sandbox and remain required defense in depth.

At startup the server validates that Bubblewrap is available (system `bwrap` or
a bundled `codex-resources/bwrap` binary) and that the workspace base directory
exists and is writable. If either check fails, the server exits immediately
with a descriptive error.

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
