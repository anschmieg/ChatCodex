# Multi-Agent ChatCodex — Architecture Draft (v2)

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
5. **ChatGPT MCP connections are static.** The MCP servers are configured
   once in ChatGPT's config. ChatGPT cannot dynamically open new MCP
   connections from tool results. All agents connect to the same server;
   the server differentiates them by session.

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                    ChatGPT (N instances)                       │
│                                                               │
│  Agent A (parent)    Agent B (subagent)   Agent C (cron)      │
│       │                    │                    │              │
│       ▼                    ▼                    ▼              │
│  ┌──────────────────────────────────────────────────────┐     │
│  │         ChatCodex MCP Server (Rust)                  │     │
│  │                                                       │     │
│  │  ┌──────────────┐  ┌──────────────┐  ┌────────────┐ │     │
│  │  │ Session      │  │ Worktree     │  │ Scheduler  │ │     │
│  │  │ Manager      │  │ Manager      │  │ (SQLite)   │ │     │
│  │  └──────┬───────┘  └──────┬───────┘  └──────┬─────┘ │     │
│  │         │                 │                  │        │     │
│  │         ▼                 ▼                  ▼        │     │
│  │  ┌──────────────────────────────────────────────┐     │     │
│  │  │  Shared Git Repo (bare clone + worktrees)    │     │     │
│  │  │                                              │     │     │
│  │  │  .git/  (bare)                               │     │     │
│  │  │  worktrees/                                   │     │     │
│  │  │    session_a/  ←─ worktree on branch/feature │     │     │
│  │  │    session_b/  ←─ worktree on branch/bugfix  │     │     │
│  │  │    session_c/  ←─ worktree on branch/refactor│     │     │
│  │  └──────────────────────────────────────────────┘     │     │
│  │                                                       │     │
│  │  ┌──────────────────────────────────────────────┐     │     │
│  │  │  Hindsight Client (memory_search/retain/reflect)│     │     │
│  │  └──────────────────────────────────────────────┘     │     │
│  └──────────────────────────────────────────────────────┘     │
│           │                    │                              │
│           ▼                    ▼                              │
│     ┌──────────┐        ┌──────────┐                         │
│     │  Shared  │        │ Hindsight│                         │
│     │  Git     │        │ Memory   │                         │
│     │  Remote  │        │ Server   │                         │
│     └──────────┘        └──────────┘                         │
└──────────────────────────────────────────────────────────────┘
```

## Layer 1: Multi-Session Server via Git Worktrees

Instead of cloning the repo N times (expensive, slow), we use **git worktrees**.

### How worktrees work

```bash
# Initial setup: bare clone
git clone --bare https://github.com/org/repo.git /srv/chatcodex/repos/repo.git

# Each session = one worktree
git --git-dir=/srv/chatcodex/repos/repo.git worktree add \
  /srv/chatcodex/sessions/session_a \
  origin/main

# Session A works in /srv/chatcodex/sessions/session_a/
# Session B works in /srv/chatcodex/sessions/session_b/
# They share the same .git (bare repo), different branches enforced
```

**Key properties:**
- Worktrees **enforce branch isolation** — you cannot have two worktrees on the same branch. Each worktree is pinned to its own branch.
- Shared `.git` means `git merge` between worktrees is a local operation — no push/pull needed.
- Worktrees are cheap — no full clone per session, just a working tree + index.
- `git worktree remove` cleans up cleanly.

### Session state

```rust
struct SessionState {
    id: SessionId,
    worktree_path: AbsolutePathBuf,  // e.g. /srv/chatcodex/sessions/<id>/
    branch: String,                   // the branch this worktree is on
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
- `create_session(repo_url, branch?) → session_id` — creates a worktree on a
  new branch (or an existing one), returns the session ID
- `list_sessions() → [{id, branch, status, last_active}]` — list all sessions
- `get_session_status(session_id) → {id, branch, todo, last_active}`
- `close_session(session_id)` — remove the worktree, free resources

**Existing tools become session-scoped.** Every tool that touches the workspace
accepts an optional `session_id` parameter. When omitted, uses the "default"
session (backward compatible). When provided, operates on that session's
worktree.

```rust
fn setup_workspace(session_id?: String, repo_url: String, branch?: String) -> ...
fn exec_command(session_id?: String, command: String) -> ...
fn read_file(session_id?: String, path: String) -> ...
fn apply_patch(session_id?: String, patch: String) -> ...
// ... etc
```

**Implementation detail:** The server already has a `NativeHarnessMcp` struct
that holds state. We add:
- `sessions: HashMap<String, SessionState>`
- `bare_repo_path: AbsolutePathBuf` — the shared bare clone
- A configurable root for worktree dirs (e.g. `/srv/chatcodex/sessions/`)

## Layer 2: Subagent Spawning (No Dynamic MCP Connections)

**Key constraint:** ChatGPT cannot dynamically open new MCP connections.
All agents connect to the same server via the same MCP config.

### The mechanism

`spawn_subagent(goal, context, parent_session?) → {subagent_id, branch}`

The server:
1. Creates a new worktree on a fresh branch (e.g. `subagent/<id>/feature`)
2. Stores the parent-child relationship
3. Returns the subagent ID and branch name

**How the subagent connects:** The subagent is a separate ChatGPT instance
with the same MCP server configured. When it connects, it calls
`claim_session(session_id, token)` to attach to the worktree. The token is
returned by `spawn_subagent` and is one-time-use.

### Subagent lifecycle

```
Parent ChatGPT                    ChatCodex Server              Subagent ChatGPT
       │                                │                              │
       │── spawn_subagent(goal) ────────▶│                              │
       │                                │── git worktree add ────────▶│
       │◀── {id, branch, token} ────────│                              │
       │                                │                              │
       │  [parent continues working]    │                              │
       │                                │                              │
       │                                │◀── claim_session(id, token) ─│
       │                                │  [subagent now owns session] │
       │                                │                              │
       │                                │   [subagent works]           │
       │                                │   [commits, pushes]          │
       │                                │                              │
       │── get_subagent_status(id) ─────▶│                              │
       │◀── {done, branch, commits} ────│                              │
       │                                │                              │
       │── merge_subagent(id) ─────────▶│                              │
       │                                │── git merge sub_branch ─────▶│
       │◀── {merged, conflicts?} ──────│                              │
```

### New tools

- `spawn_subagent(goal, context, parent_session?) → {subagent_id, branch, token}`
- `claim_session(session_id, token) → {claimed, branch}` — called by the
  subagent ChatGPT instance to attach to a session
- `get_subagent_status(subagent_id) → {status, branch, last_commit, todo}`
- `list_subagents(parent_session?) → [{id, status, goal}]`
- `merge_subagent(subagent_id, into_branch?) → {merged, conflicts?}`

### Nudging ChatGPT to use subagents

The system prompt includes aggressive instructions:

> ## Multi-agent workflow (MANDATORY)
>
> You MUST use subagents for any task that can be parallelized. This is not
> optional. If you have two or more independent work items, you MUST spawn
> subagents.
>
> **When to subagent:**
> - Implementing multiple features → one subagent per feature
> - Running tests while writing code → subagent for tests
> - Reviewing code while writing more code → subagent for review
> - Any task that takes >5 minutes → subagent for parts of it
>
> **Workflow:**
> 1. Call `spawn_subagent(goal, context)` for each parallel task
> 2. Each subagent works independently on its own branch (worktree)
> 3. Check progress with `get_subagent_status`
> 4. Merge completed subagents with `merge_subagent`
> 5. Resolve any merge conflicts immediately
>
> **DO NOT** do work sequentially that could be done in parallel. This is the
> single most important optimization. Subagents are free. Use them.

## Layer 3: Self-Scheduling

### The mechanism

`schedule_resume(cron_expr, context, session_id?) → schedule_id`

The server stores the schedule in a lightweight SQLite database. A companion
**scheduler process** (separate binary, or a thread in the server) checks for
due tasks.

**Trigger mechanism:** Since ChatGPT doesn't natively accept webhooks to start
new sessions, we use a **polling bridge**:

- `poll_due_tasks() → [{schedule_id, context}]` — ChatGPT calls this at the
  start of each turn. If there's a due task, it picks it up.
- This is polling, not push, but requires zero infrastructure changes.

### New tools

- `schedule_resume(cron_expr, context) → schedule_id`
- `list_schedules() → [{id, cron, context, last_run, next_run}]`
- `cancel_schedule(schedule_id)`
- `poll_due_tasks() → [{schedule_id, context}]` — called by ChatGPT each turn

### Nudging ChatGPT to self-schedule

> ## Self-scheduling (MANDATORY)
>
> You MUST schedule yourself to continue later when:
> - A task will take more than one session to complete
> - You're waiting for external input (build, test, review)
> - You've made progress but the work isn't done
>
> **Workflow:**
> 1. Call `memory_retain` to save your current state, decisions, and next steps
> 2. Call `schedule_resume(cron, context)` to schedule your next run
> 3. At the start of EVERY turn, call `poll_due_tasks` to pick up scheduled work
> 4. Use `memory_search` to restore context from your previous run
>
> **DO NOT** leave work unfinished without scheduling a resume. If you stop
> without scheduling, the work is lost.

## Layer 4: Coordination via Git Worktrees + Hindsight

### Git worktree strategy

```
bare repo .git/
├── refs/heads/
│   ├── main
│   ├── agent-a/feature-x
│   ├── agent-b/bugfix-y
│   └── subagent-c/refactor-z
│
worktrees/
├── session_a/  ←─ worktree on agent-a/feature-x
├── session_b/  ←─ worktree on agent-b/bugfix-y
└── session_c/  ←─ worktree on subagent-c/refactor-z
```

- Each agent works on its own branch via a worktree
- `git_push` pushes to the shared remote
- `merge_subagent` merges a subagent's branch into the parent's worktree
  (local `git merge` — no push needed since they share the bare repo)
- Worktrees **enforce** that no two sessions can be on the same branch

### Hindsight for cross-session context

- `memory_retain` — agents store what they did, decisions made, next steps
- `memory_search` — agents pick up context from other agents' sessions
- `memory_reflect` — synthesize across sessions ("what's the status of X?")

This is the only sanctioned LLM call in the stack. It runs on Hindsight's
server-side model, not on ChatGPT's budget.

## Implementation Phases

### Phase 1: Worktree-based session manager (this sprint)
- Add `SessionManager` with bare repo + worktree support
- Add `session_id` parameter to all workspace tools
- Implement `create_session`, `list_sessions`, `close_session`
- Backward compatible: `session_id` defaults to "default"

### Phase 2: Subagent spawning
- Implement `spawn_subagent` — creates a worktree on a new branch
- Implement `claim_session` — one-time token to attach to a session
- Implement `get_subagent_status`, `list_subagents`, `merge_subagent`
- Add aggressive prompt nudges to AGENTS.md

### Phase 3: Self-scheduling
- Add SQLite-based scheduler to the server
- Implement `schedule_resume`, `poll_due_tasks`, `cancel_schedule`
- Add scheduling nudges to the prompt

### Phase 4: Polish
- Session cleanup (GC idle worktrees)
- Resource limits (max sessions per user, disk quotas)
- Dashboard for monitoring active sessions
- Conflict resolution UX

## Open Questions

1. **Worktree + sandbox compatibility** — The read-only sandbox wraps a
   directory. Can we point each session's sandbox at its worktree? Yes —
   the sandbox is per-session, and each worktree is a separate directory.

2. **Worktree GC** — `git worktree prune` cleans up stale worktree metadata.
   We should run this periodically and also remove the actual directories.

3. **Token security** — `spawn_subagent` tokens should be one-time-use,
   scoped to that subagent session, with an expiry.

4. **Scheduler polling overhead** — `poll_due_tasks` is called every turn.
   For a SQLite DB with a handful of rows, this is negligible.

5. **Worktree limits** — Git has no hard limit on worktrees, but each one
   holds a working tree + index in memory. Practical limit is probably
   ~50-100 worktrees per bare repo on a VPS.
