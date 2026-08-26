# MCP Tools Overview

ChatCodex exposes a compact deterministic tool catalog. ChatGPT owns reasoning
and chains these tools while work remains.

## Tool Groups

### Project Lifecycle

| Tool | Purpose |
|------|---------|
| `project_create` | Create or register a persistent repo, workspace, or scratch project |
| `project_select` | Select an existing project for subsequent tools |
| `project_list` | List projects in the current `CHATCODEX_CLIENT_ID` namespace |
| `project_get` | Get a project by id or return the selected project |

Project ids are stable for existing repos, registered workspaces, and scratch
projects. Project metadata is persisted under the client namespace and never
stores credentials.

### Run Lifecycle

| Tool | Purpose |
|------|---------|
| `run_start` | Create and select a persistent coding run |
| `run_list` | List runs, optionally filtered by project or status |
| `run_get` | Get a run by id or return the selected run |
| `run_update` | Update phase, status, plan, checklist, checkpoints, and counters |
| `run_resume` | Select a non-terminal run after ChatGPT is asked to continue |
| `run_cancel` | Cancel a non-completed run |
| `run_followup_lease` | Acquire a duplicate-safe app continuation lease |

Run phases are `inspect`, `plan`, `execute`, and `verify`. Run statuses are
`active`, `paused`, `blocked`, `awaiting_approval`, `completed`, and
`cancelled`.

### Legacy-Compatible Lifecycle

| Tool | Purpose |
|------|---------|
| `setup_workspace` | Clone a repo or create a scratch sandbox and register it as a project |
| `update_plan` | Replace the selected run's plan, or legacy plan state without a run |
| `todo` | Replace or update the selected run's checklist, or legacy checklist state without a run |

### Workspace Inspection

| Tool | Purpose |
|------|---------|
| `read_file` | Read file contents with optional line ranges |
| `search_code` | Search source files for a text pattern |
| `list_directory` | List a workspace directory |
| `view_image` | Display a workspace image |
| `git_status` | Show `git status --porcelain` |
| `git_diff` | Show `git diff` |

### Workspace Mutation And Execution

| Tool | Purpose | Policy |
|------|---------|--------|
| `exec_command` | Run a command in the read-only sandbox | Rejected if dangerous or disallowed by run autonomy |
| `write_stdin` | Interact with a running command session | Bound to the existing session |
| `apply_patch` | Apply workspace source edits | Only workspace source write path |
| `git` | Run local-only git commands | Outbound network operations blocked |
| `git_commit` | Create a local commit | Requires run autonomy to allow commits |
| `git_branch` | Create or move a local branch | Requires run autonomy to allow local git writes |
| `git_checkout` | Switch branches | Requires run autonomy to allow local git writes |

## Typical Flow

```text
project_create or project_select
run_start
read_file/search_code/list_directory/git_status
update_plan and todo
apply_patch/exec_command/git tools as needed
run_update through inspect -> plan -> execute -> verify
verify acceptance criteria
run_update(status: "completed", work_remaining: false)
```

If work remains after a tool response, ChatGPT should continue with the next
fine-grained tool call instead of stopping. If external effects are needed and
no existing authorization permits them, the run should move to
`awaiting_approval`.

## Active Context

When a run is selected, all coding tools operate on that run's project. When no
run is selected, tools use the selected project for legacy client
compatibility.

Every coding tool result includes `run_metadata` when a run is active:

- `run_id`
- `phase`
- `status`
- `work_remaining`
- `next_action`
- `limits`
- `lease`

## Continuation Component

Run lifecycle tools advertise the `ui://chatcodex/run-status.html` MCP app
resource. The component can ask ChatGPT to continue only after it acquires a
server-issued `run_followup_lease`. The follow-up message contains only the run
id.

No follow-up is sent for terminal, paused, blocked, or awaiting-approval runs.
If the component or OpenAI app bridge is missing, the persisted run can still be
listed, inspected, and resumed explicitly.

## Tool Reference

| Group | Tools |
|-------|-------|
| Project | `project_create`, `project_select`, `project_list`, `project_get` |
| Run | `run_start`, `run_list`, `run_get`, `run_update`, `run_resume`, `run_cancel`, `run_followup_lease` |
| Legacy lifecycle | `setup_workspace`, `update_plan`, `todo` |
| Files | `read_file`, `search_code`, `list_directory`, `apply_patch`, `view_image` |
| Commands | `exec_command`, `write_stdin` |
| Git | `git`, `git_status`, `git_diff`, `git_commit`, `git_branch`, `git_checkout` |

See [MCP_TOOL_CONTRACTS.md](./MCP_TOOL_CONTRACTS.md) for field-level contracts.
