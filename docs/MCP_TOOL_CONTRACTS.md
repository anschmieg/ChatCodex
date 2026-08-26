# MCP Tool Contracts

These are the deterministic public MCP tools exposed to ChatGPT by the native
Rust server. ChatGPT remains the only LLM and owns all reasoning. Tools mutate
only explicit server state or workspace data described below.

## Shared Lifecycle State

Lifecycle state is namespaced by `CHATCODEX_CLIENT_ID` and persisted under the
workspace base:

```text
<workspace-base>/clients/<client-id>/.chatcodex/state.json
```

Writes use a lock file plus atomic JSON replacement. Git credentials are never
persisted; repository sources are redacted before storage.

## Project Shape

Project fields:

- `id: string`
- `name: string`
- `kind: "repo" | "workspace" | "scratch"`
- `workspace_root: string`
- `source: object`
- `created_at_ms: integer`
- `updated_at_ms: integer`

`source.type` is one of:

- `scratch`
- `git`
- `workspace`

Git sources include redacted `url`, optional `host`, and `path`.
Workspace sources include `registered_path`.

## Run Shape

Run fields:

- `id: string`
- `project_id: string`
- `objective: string`
- `acceptance_criteria: string[]`
- `phase: "inspect" | "plan" | "execute" | "verify"`
- `status: "active" | "paused" | "blocked" | "awaiting_approval" | "completed" | "cancelled"`
- `plan: PlanItem[]`
- `checklist: ChecklistItem[]`
- `checkpoints: Checkpoint[]`
- `autonomy: AutonomyEnvelope`
- `counters: RunCounters`
- `continuation: ContinuationState`
- `work_remaining: boolean`
- `next_action: string`
- `created_at_ms: integer`
- `updated_at_ms: integer`
- `started_at_ms: integer`
- `completed_at_ms?: integer`
- `cancelled_at_ms?: integer`

Plan item statuses are `pending`, `in_progress`, or `completed`. Checklist
statuses are `pending`, `checked`, or `dismissed`.

Autonomy fields:

- `max_turns: integer`
- `max_runtime_seconds: integer`
- `max_steps: integer` — hard count of coding-tool calls; the server increments it automatically
- `allow_local_commands: boolean`
- `allow_file_edits: boolean`
- `allow_git_commits: boolean`

## Run Metadata

When a run is selected and active, coding tool results include:

- `run_id`
- `project_id`
- `phase`
- `status`
- `work_remaining`
- `next_action`
- `limits`
- `lease`

The server attaches this from authoritative persisted run state. Clients should
not infer it from local state.

## Project Tools

### project_create

Create or register a persistent project and optionally select it.

Input:

- `kind: "repo" | "workspace" | "scratch"`
- `name?: string`
- `source?: string`
- `path?: string`
- `select?: boolean` default `true`
- `timeout_ms?: integer`

Behavior:

- `repo` clones or reuses a repository and stores a stable id based on the
  redacted source.
- `scratch` creates or reuses a persistent git-initialized sandbox.
- `workspace` registers an existing directory beneath the workspace base.

Returns:

- `project`
- `action`
- `selected`

### project_select

Select a project as the default workspace context.

Input:

- `project_id: string`

Returns:

- `project`
- `selected: true`

Selecting a project clears a selected run from another project.

### project_list

List projects for the current client namespace.

Input: empty object.

Returns:

- `projects`
- `active_project_id`

### project_get

Get a project by id, or the selected project when `project_id` is omitted.

Input:

- `project_id?: string`

Returns:

- `project`
- `selected`

## Run Tools

### run_start

Start and optionally select a persistent coding run.

Input:

- `project_id?: string`; defaults to selected project
- `objective: string`
- `acceptance_criteria?: string[]`
- `autonomy?: AutonomyEnvelope`
- `select?: boolean` default `true`

Returns:

- `run`
- `run_metadata`

### run_list

List persistent runs.

Input:

- `project_id?: string`
- `status?: "active" | "paused" | "blocked" | "awaiting_approval" | "completed" | "cancelled"`

Returns:

- `runs`
- `active_run_id`

### run_get

Get a run by id, or the selected run when `run_id` is omitted.

Input:

- `run_id?: string`

Returns:

- `run`
- `run_metadata`

### run_update

Deterministically update lifecycle state.

Input:

- `run_id?: string`; defaults to selected run
- `phase?: "inspect" | "plan" | "execute" | "verify"`
- `status?: "active" | "paused" | "blocked" | "awaiting_approval" | "completed" | "cancelled"`
- `acceptance_criteria?: string[]`
- `plan?: PlanItem[]`
- `checklist?: ChecklistItem[]`
- `checkpoint?: { message: string }`
- `work_remaining?: boolean`
- `next_action?: string`
- `step_delta?: integer` — optional manual accounting in addition to automatic coding-tool step counting

Phase changes must follow `inspect -> plan -> execute -> verify`; `verify -> execute` is allowed for corrective work. `completed` is accepted only from `verify`. Runtime or step exhaustion pauses the run and blocks further coding tools.

Returns:

- `run`
- `run_metadata`

Behavior:

- Invalid transitions are rejected.
- More than one `in_progress` plan item is rejected.
- Runtime, turn, and step limits are enforced server-side.
- Terminal runs cannot be mutated.

### run_resume

Select a non-terminal run after ChatGPT receives a user request or a
component follow-up containing only the run id.

Input:

- `run_id?: string`; defaults to selected run

Returns:

- `run`
- `run_metadata`

Behavior:

- Completed and cancelled runs cannot be resumed.
- Limit-exhausted runs cannot be resumed.
- The tool does not perform work or start a loop.

### run_cancel

Cancel a non-completed run and clear continuation lease state.

Input:

- `run_id?: string`; defaults to selected run

Returns:

- `run`
- `run_metadata`

### run_followup_lease

Acquire a duplicate-safe continuation lease for the ChatGPT app component.

Input:

- `run_id: string`
- `requested_nonce?: string`
- `ttl_ms?: integer`
- `delay_ms?: integer`

Returns:

- `run_id`
- `granted`
- `duplicate`
- `nonce`
- `acquired_at_ms`
- `expires_at_ms`
- `delay_ms`
- `max_turns`
- `max_runtime_seconds`
- `max_steps`
- `reason`
- `run_metadata`

Behavior:

- Grants only for `active` runs with `work_remaining: true`.
- Never grants for paused, blocked, awaiting-approval, completed, or cancelled runs.
- Never grants after turn/runtime/step limits are exhausted.
- Reusing a nonce is reported as duplicate and does not issue a new lease.
- A live unexpired lease prevents another active lease from being issued.

## Legacy-Compatible Lifecycle Tools

### setup_workspace

Clone a git repository or create a scratch sandbox, register it as a persistent
project, and select it.

Input:

- `source: string`; git URL or literal `sandbox`
- `name?: string`
- `timeout_ms?: integer`

Returns:

- `workspace_root`
- `source`
- `action`
- `project_id`

### update_plan

Replace the deterministic task plan.

Input:

- `explanation?: string`
- `plan: PlanItem[]`

Returns the same plan payload.

When a run is selected, the plan is written to that run. Otherwise it is stored
as legacy client state. At most one item may be `in_progress`.

### todo

Manage a persistent checklist.

Input:

- `action?: "replace" | "update"` default `replace`
- `items: ChecklistItem[]`

Returns:

- `items`
- `summary`
- `all_done`

When a run is selected, the checklist is written to that run. Otherwise it is
stored as legacy client state.

## Workspace Tools

All workspace tools operate on the selected run's project when a run is
selected. Without a selected run, they operate on the selected project for
legacy compatibility.

### exec_command

Run a command in the read-only command sandbox.

Input:

- `cmd: string`
- `yield_time_ms?: integer`
- `max_output_tokens?: integer`
- `timeout_ms?: integer`

Returns:

- `output`
- `exit_code`
- `session_id`
- `run_metadata?`

If a selected run's autonomy envelope disallows local commands, the call is
rejected before execution.

### write_stdin

Write to or poll a running command session.

Input:

- `session_id: string`
- `chars?: string`
- `yield_time_ms?: integer`

Returns:

- `output`
- `exited`
- `exit_code`
- `run_metadata?`

### read_file

Read a file under the selected workspace.

Input:

- `path: string`
- `start_line?: integer`
- `end_line?: integer`

Returns:

- `path`
- `total_lines`
- `start_line`
- `end_line`
- `content`
- `run_metadata?`

### search_code

Search workspace files for a text pattern.

Input:

- `query: string`
- `path_glob?: string`
- `max_results?: integer`

Returns:

- `matches`
- `run_metadata?`

### list_directory

List entries in a workspace directory.

Input:

- `path?: string`

Returns:

- `path`
- `entries`
- `run_metadata?`

### apply_patch

Apply a patch inside the selected workspace.

Input:

- `input: string`

Returns:

- `result`
- `run_metadata?`

`apply_patch` is the only workspace source write path. If a selected run's
autonomy envelope disallows file edits, the call is rejected before mutation.

### view_image

Read an image located inside the workspace.

Input:

- `path: string`

Returns:

- `path`
- `run_metadata?`

## Git Tools

Outbound network operations are rejected by deterministic policy. Read-only git
inspection runs through the read-only sandbox. Local git metadata writes are
limited to explicit git tools and are still network-blocked.

### git

Run a local-only git command.

Input:

- `command: string`
- `timeout_ms?: integer`

Returns:

- `stdout`
- `stderr`
- `exit_code`
- `run_metadata?`

Commands such as `commit`, `merge`, `rebase`, `cherry-pick`, `reset`, `branch`,
and `checkout` require `allow_git_commits: true` in the selected run autonomy
envelope.

### git_status

Run `git status --porcelain`.

Input: empty object.

Returns:

- `entries`
- `stderr`
- `run_metadata?`

### git_diff

Run `git diff`.

Input:

- `paths?: string[]`
- `staged?: boolean`

Returns:

- `diff`
- `stderr`
- `run_metadata?`

### git_commit

Create a local git commit.

Input:

- `message: string`
- `allow_empty?: boolean`
- `timeout_ms?: integer`

Returns:

- `stdout`
- `stderr`
- `exit_code`
- `run_metadata?`

Requires `allow_git_commits: true` in the selected run autonomy envelope.

### git_branch

Create or move a local git branch.

Input:

- `name: string`
- `start_point?: string`
- `force?: boolean`
- `timeout_ms?: integer`

Returns:

- `stdout`
- `stderr`
- `exit_code`
- `run_metadata?`

Requires `allow_git_commits: true` in the selected run autonomy envelope when a
run is active.

### git_checkout

Switch branches.

Input:

- `target: string`
- `create_branch?: boolean`
- `timeout_ms?: integer`

Returns:

- `stdout`
- `stderr`
- `exit_code`
- `run_metadata?`

Requires `allow_git_commits: true` in the selected run autonomy envelope when a
run is active.

## ChatGPT App Resource

The server exposes:

```text
ui://chatcodex/run-status.html
```

Tools that return run status advertise this resource through MCP app metadata.
The component is self-contained HTML/JS and uses the OpenAI app bridge only
after receiving run metadata. It acquires `run_followup_lease`, waits the
server-provided delay, and sends a follow-up message containing only the run id.
If the bridge or resource is unavailable, the run remains persisted and
resumable through `run_resume`.
