# Multi-Agent ChatCodex — Architecture Draft

## Problem

Today ChatCodex is single-session: one workspace, one agent, one turn loop.
We want **N ChatGPT agents** working on the same project in parallel, each in
its own isolated session, using subagents heavily and scheduling themselves to
run again and again.

## Constraints (non-negotiable)

1. **No hidden LLM in the backend.** The Rust server stays deterministic.
   Subagents are just more ChatGPT instances calling the same deterministic
   tools — the backend never owns an agent loop.
2. **Sanctioned exception:** `memory_reflect` calls Hindsight's synthesis
   endpoint (server-side LLM, memory service, not a harness loop).
3. **All file writes go through `apply_patch`.** No backend-side codegen.
4. **All execution runs in a read-only sandbox.** No backend-side execution
   that isn't a tool call from ChatGPT.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    ChatGPT (N instances)                  │
│                                                          │
│  Agent A (parent)    Agent B (subagent)   Agent C (cron) │
│       │                    │                    │         │
│       ▼                    ▼                    ▼         │
│  ┌──────────────────────────────────────────────┐        │
│  │         ChatCodex MCP Server (Rust)          │        │
│  │                                              │        │
│  │  Session A ── workspace_a/                   │        │
│  │  Session B ── workspace_b/                   │        │
│  │  Session C ── workspace_c/                   │        │
│  │                                              │        │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐   │        │
│  │  │ Session  │  │ Session  │  │ Session  │   │        │
│  │  │ Manager  │  │ Scheduler│  │ Hindsight│   │        │
│  │  └──────────┘  └──────────┘  │  Client  │   │        │
│  │                              └──────────┘   │        │
│  └──────────────────────────────────────────────┘        │
│           │                    │                          │
│           ▼                    ▼                          │
│     ┌──────────┐        ┌──────────┐                     │
│     │  Shared  │        │ Hindsight│                     │
│     │  Git     │        │ Memory   │                     │
│     │  Remote  │        │ Server   │                     │
│     └──────────┘        └──────────┘                     │
└─────────────────────────────────────────────────────────┘
```

## Layer 1: Multi-Session Server

The Rust MCP server maintains a `HashMap<SessionId, SessionState>` in memory.

```rust
struct SessionState {
    id: SessionId,
    workspace: AbsolutePathBuf,
    sandbox: FileSystemSandboxContext,
    todo: Vec<TodoItem>,
    plan: String,
    created_at: DateTime<Utc>,
    last_active: DateTime<Utc>,
    parent_session: Option<SessionId>,  // for subagent tracking
}
```

### New tools

**Session lifecycle:**
- `create_session(repo_url, branch?) → session_id` — clones the repo into a
  fresh workspace dir, returns the session ID
- `list_sessions() → [{id, status, branch, last_active}]` — list all sessions
- `get_session_status(session_id) → {id, branch, todo, last_active}`
- `close_session(session_id)` — tear down the workspace, free resources

**Existing tools become session-scoped.** Every tool that touches the workspace
accepts an optional `session_id` parameter. When omitted, uses the "default"
session (backward compatible). When provided, operates on that session's
workspace.

```rust
// All existing tools gain an optional session_id
fn setup_workspace(session_id?: String, repo_url: String, branch?: String) -> ...
fn exec_command(session_id?: String, command: String) -> ...
fn read_file(session_id?: String, path: String) -> ...
fn apply_patch(session_id?: String, patch: String) -> ...
// ... etc
```

**Implementation detail:** The server already has a `NativeHarnessMcp` struct
that holds state. We add a `sessions: HashMap<String, SessionState>` field.
Each session gets its own workspace dir under a configurable root
(e.g. `/tmp/chatcodex-sessions/<id>/`).

## Layer 2: Subagent Spawning

### The mechanism

`spawn_subagent(goal, context, parent_session?) → {subagent_id, connection_url, token}`

The server:
1. Creates a new session (clones the repo, sets up sandbox)
2. Stores the parent-child relationship
3. Returns a **connection URL** that the parent ChatGPT instance can open as a
   new MCP connection

**How ChatGPT opens it:** ChatGPT natively supports multiple MCP connections.
The parent agent calls `spawn_subagent`, gets back a URL, and ChatGPT opens
that URL as a new MCP-connected session. The subagent is just another ChatGPT
instance talking to its own isolated workspace.

**No backend LLM involved.** The server never runs a subagent process. It just
creates the session and hands back the keys. ChatGPT does the rest.

### Subagent lifecycle

```
Parent ChatGPT                    ChatCodex Server              Subagent ChatGPT
       │                                │                              │
       │── spawn_subagent(goal) ────────▶│                              │
       │                                │── create session ──────────▶│
       │◀── {url, token} ───────────────│                              │
       │                                │                              │
       │── (opens MCP connection to url)│                              │
       │                                │◀── MCP connect ─────────────│
       │                                │                              │
       │                                │   [subagent works]          │
       │                                │   [commits, pushes]         │
       │                                │                              │
       │── get_subagent_status(id) ─────▶│                              │
       │◀── {done, branch, commits} ────│                              │
       │                                │                              │
       │── merge_subagent(id) ─────────▶│                              │
       │                                │── git merge sub_branch ─────▶│
       │◀── {merged} ──────────────────│                              │
```

### New tools

- `spawn_subagent(goal, context, parent_session?) → {subagent_id, connection_url, token}`
- `get_subagent_status(subagent_id) → {status, branch, last_commit, todo}`
- `list_subagents(parent_session?) → [{id, status, goal}]`
- `merge_subagent(subagent_id, into_branch?) → {merged, conflicts?}`

### Nudging ChatGPT to use subagents

The system prompt (or a prompt template) includes instructions like:

> **Subagent workflow:** For any task that can be parallelized (e.g. implement
> two features, fix multiple bugs, review different modules), spawn subagents
> using `spawn_subagent`. Each subagent gets its own workspace and works
> independently. Use `get_subagent_status` to check progress and
> `merge_subagent` to integrate results. Subagents push to their own branches;
> you merge them into yours.

## Layer 3: Self-Scheduling

### The mechanism

`schedule_resume(cron_expr, context, session_id?) → schedule_id`

The server stores the schedule in a lightweight SQLite database. A companion
**scheduler process** (separate binary, or a thread in the server) checks for
due tasks and triggers them.

**Trigger mechanism:** Since ChatGPT doesn't natively accept webhooks to start
new sessions, we use a **polling bridge**:

1. The scheduler marks a task as "due" in the DB
2. A lightweight bridge service (Node.js or Rust) exposes a webhook endpoint
3. The bridge calls a configurable URL that starts a new ChatGPT session
   (e.g. via OpenAI's Assistants API, or a custom webhook receiver)

**Alternative (simpler, works today):**
- `poll_due_tasks() → [{schedule_id, context}]` — ChatGPT calls this at the
  start of each turn. If there's a due task, it picks it up.
- This is polling, not push, but requires zero infrastructure changes.

### New tools

- `schedule_resume(cron_expr, context) → schedule_id`
- `list_schedules() → [{id, cron, context, last_run, next_run}]`
- `cancel_schedule(schedule_id)`
- `poll_due_tasks() → [{schedule_id, context}]` — called by ChatGPT each turn

### Nudging ChatGPT to self-schedule

> **Self-scheduling:** When you have work that should continue later (e.g. a
> long refactor, a build that takes hours, a task that depends on external
> input), schedule a resume using `schedule_resume`. At the start of each turn,
> call `poll_due_tasks` to pick up any scheduled work. Use Hindsight
> (`memory_retain`/`memory_search`) to pass context between scheduled runs.

## Layer 4: Coordination via Git + Hindsight

### Git branch strategy

```
main ────── A1 ── A2 ── M ────────────────
                \         /
agent-a/feature  └── B1 ─┘
                         \
agent-b/bugfix            └── C1 ── C2
```

- Each agent works on its own branch
- `git_push` pushes to the shared remote
- `merge_subagent` merges a subagent's branch into the parent's
- `git_merge` (new tool) merges any branch into the current session's branch
- Conflicts are surfaced to the agent that calls the merge

### Hindsight for cross-session context

- `memory_retain` — agents store what they did, decisions made, next steps
- `memory_search` — agents pick up context from other agents' sessions
- `memory_reflect` — synthesize across sessions ("what's the status of X?")

This is the only sanctioned LLM call in the stack. It runs on Hindsight's
server-side model, not on ChatGPT's budget.

## Layer 5: Prompt Engineering (the nudging)

The ChatGPT system prompt gets a new section:

```
## Multi-agent workflow

You can spawn subagents for parallel work. Each subagent is a separate ChatGPT
instance with its own workspace. Use this for:
- Implementing multiple features simultaneously
- Running tests while writing code
- Reviewing code while writing more code
- Any task that can be parallelized

Workflow:
1. Call spawn_subagent(goal, context) for each parallel task
2. Each subagent works independently on its own branch
3. Check progress with get_subagent_status
4. Merge completed subagents with merge_subagent
5. Resolve any merge conflicts

For long-running work, use schedule_resume to continue later:
1. Call schedule_resume(cron, context) when you need to pause
2. At the start of each turn, call poll_due_tasks
3. Pick up where you left off using Hindsight context

Use Hindsight memory to share context between agents and sessions:
- memory_retain: store decisions, status, next steps
- memory_search: find what other agents decided
- memory_reflect: synthesize across sessions
```

## Implementation Phases

### Phase 1: Multi-session server (this sprint)
- Add `SessionManager` to the Rust server
- Add `session_id` parameter to all workspace tools
- Implement `create_session`, `list_sessions`, `close_session`
- Backward compatible: `session_id` defaults to "default"

### Phase 2: Subagent spawning
- Implement `spawn_subagent` — creates a session, returns connection URL
- Implement `get_subagent_status`, `list_subagents`, `merge_subagent`
- Add prompt nudges to AGENTS.md and the system prompt template

### Phase 3: Self-scheduling
- Add SQLite-based scheduler to the server
- Implement `schedule_resume`, `poll_due_tasks`, `cancel_schedule`
- Build the polling bridge (or webhook receiver)

### Phase 4: Polish
- Session cleanup (GC idle sessions)
- Resource limits (max sessions per user, disk quotas)
- Dashboard for monitoring active sessions
- Conflict resolution UX

## Open Questions

1. **MCP connection URL format** — How does ChatGPT open a new MCP connection
   from a tool call? Does it need a specific URL scheme? Can the server return
   a URL that ChatGPT interprets as "open this as a new MCP connection"?

2. **Subagent session persistence** — If the parent agent's session ends, do
   subagents keep running? (Yes — they're independent sessions.)

3. **Resource limits** — How many concurrent sessions can one server handle?
   Each session needs its own workspace clone (~100MB for a medium repo).

4. **Auth for subagent connections** — The `spawn_subagent` token should be
   one-time-use and scoped to that subagent session.

5. **Scheduler trigger** — The polling approach (`poll_due_tasks`) works today
   but is wasteful. A webhook receiver would be better but needs ChatGPT-side
   support or a bridge service.
