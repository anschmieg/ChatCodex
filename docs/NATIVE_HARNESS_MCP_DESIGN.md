# Native Harness MCP Design

## Purpose

ChatCodex exposes deterministic coding tools to ChatGPT. ChatGPT is the only
model and owns all reasoning, planning, and decisions about what to do next.
The backend never starts a Codex thread, turn, review, model call, or hidden
agent loop.

## Dependency Boundary

ChatCodex code lives in two independent crates:

- `chatcodex/crates/mcp-server`
- `chatcodex/crates/oauth`

They depend only on public upstream crate APIs. No source file in `codex-core`
or another upstream crate is patched.

The harness uses:

- `codex-exec-server` for process and filesystem backends;
- `codex-sandboxing` for read-only command transformation;
- `codex-shell-command` for deterministic dangerous-command classification;
- `codex-apply-patch` for workspace source writes;
- `codex-protocol` for permission profiles.

It does not import or instantiate a Codex session, model client, turn manager,
tool registry, or tool router.

## Persistent Lifecycle Model

Projects and runs are persisted per `CHATCODEX_CLIENT_ID` namespace beneath the
workspace base:

```text
<workspace-base>/clients/<client-id>/.chatcodex/state.json
```

The client id is sanitized before it is used in a path. State is written with a
lock file and atomic JSON replacement. The store never persists credentials:
git project sources are redacted before they are stored.

Projects have stable ids for:

- cloned or reused git repositories;
- registered workspace directories beneath the workspace base;
- persistent scratch projects.

Runs belong to projects and carry objective, acceptance criteria, phase,
status, plan, checklist, checkpoints, autonomy limits, continuation lease state,
counters, and timestamps. A selected run replaces the old process-global active
workspace pointer for coding tools. If no run is selected, tools fall back to
the selected project for legacy clients.

## Public Tool Surface

The MCP catalog is a strict allowlist.

Project lifecycle:

- `project_create`
- `project_select`
- `project_list`
- `project_get`

Run lifecycle:

- `run_start`
- `run_list`
- `run_get`
- `run_update`
- `run_resume`
- `run_cancel`
- `run_followup_lease`

Legacy-compatible lifecycle:

- `setup_workspace`
- `update_plan`
- `todo`

Workspace and file tools:

- `exec_command`
- `write_stdin`
- `read_file`
- `search_code`
- `list_directory`
- `apply_patch`
- `view_image`

Git tools:

- `git`
- `git_status`
- `git_diff`
- `git_commit`
- `git_branch`
- `git_checkout`

Unknown tool names are rejected. Schemas are owned by the ChatCodex adapter so
upstream catalog changes cannot silently expand the public surface.

## Run State

Run phases are:

- `inspect`
- `plan`
- `execute`
- `verify`

Run statuses are:

- `active`
- `paused`
- `blocked`
- `awaiting_approval`
- `completed`
- `cancelled`

Server-side transition validation prevents terminal runs from being resumed or
mutated and prevents cancelled runs from becoming active again. Paused,
blocked, and awaiting-approval runs never receive automatic follow-up leases.

Autonomy limits are deterministic counters:

- maximum continuation turns;
- maximum runtime seconds;
- maximum tool steps;
- booleans for allowed local commands, file edits, and local git commits.

External effects require `awaiting_approval` unless an existing authorized
mechanism explicitly permits them.

## Result Metadata

When a run is active, every coding tool result includes authoritative
`run_metadata`:

- `run_id`
- `project_id`
- `phase`
- `status`
- `work_remaining`
- `next_action`
- `limits`
- `lease`

This metadata is generated from the persisted run state by the server. It is
not inferred by the client.

## ChatGPT App Resource

The server exposes a standards-compliant MCP app resource at:

```text
ui://chatcodex/run-status.html
```

The run-status component is self-contained HTML/JS. It only asks for a
continuation when all of these conditions hold:

- a run id is present;
- the run status is `active`;
- `work_remaining` is true;
- the OpenAI app bridge is present;
- the component successfully acquires `run_followup_lease`.

The lease response includes nonce, expiry, delay, max turns, max runtime, max
steps, and current run metadata. Duplicate nonces and active unexpired leases
are not granted twice. The follow-up message contains only the run id. Missing
app bridge or resource rendering leaves the run safely resumable through
`run_resume`.

## Execution Policy

`exec_command` and `write_stdin` use the public exec-server process API.
Commands are rejected when upstream's public command classifier marks them
dangerous. Accepted commands are transformed through the public sandbox manager
with a read-only filesystem permission profile.

This means commands may inspect repositories and run operations that do not
write to the filesystem. They cannot modify source files, Git metadata, build
artifacts, package environments, or system state.

`apply_patch` is the only workspace source write path. It calls the public
`codex_apply_patch::apply_patch` API with a workspace-write filesystem sandbox
whose writable root is the selected project workspace.

`view_image`, `read_file`, `search_code`, and `list_directory` resolve paths
beneath the selected run's project, or beneath the selected project when no run
is selected.

`update_plan` and `todo` persist into the selected run when one exists. Without
a selected run, they write legacy per-client state for compatibility.

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

The canonical workspace base defaults to `/workspaces`. Each client receives a
sub-directory under `/workspaces/clients/<client_id>/`. Project workspaces live
under that namespace unless explicitly registered from an existing directory
beneath the workspace base.

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
`codex()`, `codex-reply()`, hidden continuation, sub-agent, or model-provider
operations. It contains no provider SDK and makes no model request.

`run_resume` is a deterministic state-selection tool. It does not execute work
or run a loop; ChatGPT must continue by invoking fine-grained tools.

## Verification

The focused test suite verifies the exact catalog, schema parity, persistence
across store and harness restart, project/client isolation, invalid transition
rejection, duplicate-safe lease expiry and limit handling, result metadata,
app resource metadata, legacy lifecycle fallback, path confinement,
dangerous-command classification, and patch application.
