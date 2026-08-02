# Multi-Agent ChatCodex — Implementation Plan

## Goal

N ChatGPT agents working on the same project in parallel, each in its own
isolated worktree session, using subagents heavily and scheduling themselves
to run again and again.

## Architecture (short)

- **Worktrees** — each session is a `git worktree add` on its own branch,
  sharing a bare `.git`. Cheap, branch-isolated, local merges.
- **Static MCP** — all agents connect to the same server. `spawn_subagent`
  creates a worktree + one-time token; `claim_session` attaches a subagent.
- **Polling scheduler** — `schedule_resume` + `poll_due_tasks`. No webhooks.
- **Hindsight** — cross-session context via `memory_search`/`retain`/`reflect`.
- **Aggressive nudging** — the system prompt tells ChatGPT to subagent
  everything and schedule itself or lose the work.

See `docs/multi-agent-architecture.md` for the full architecture.

---

## Phase 1: Worktree-based session manager

**Goal:** Multiple ChatGPT instances can connect to the same server, each
getting an isolated worktree on its own branch. Existing single-session
behavior is preserved (backward compatible).

### Tasks

#### 1.1 Add session config to the server

- Add CLI args or env vars:
  - `--sessions-root` (default: `/tmp/chatcodex-sessions/`)
  - `--bare-repo` (optional; if not set, session manager is disabled)
- Store in `NativeHarnessMcp` as `sessions_root: PathBuf` and
  `bare_repo: Option<PathBuf>`

#### 1.2 Create `SessionManager` struct

Fields:
- `sessions: HashMap<String, SessionState>`
- `sessions_root: PathBuf`
- `bare_repo: PathBuf`

Methods:
- `create(repo_url, branch?) -> Result<SessionId>` — bare clone if needed,
  then `git worktree add` on a new branch (or existing). Returns session ID.
- `get(id) -> Option<&SessionState>`
- `list() -> Vec<SessionSummary>`
- `close(id) -> Result<()>` — `git worktree remove`, delete dir
- `gc() -> Result<()>` — `git worktree prune`, remove stale dirs

#### 1.3 Add `session_id` to all workspace tools

Every tool that touches the workspace gains an optional `session_id` param:

- `setup_workspace(session_id?, repo_url, branch?)`
- `exec_command(session_id?, command, ...)`
- `read_file(session_id?, path, ...)`
- `search_code(session_id?, pattern, ...)`
- `list_directory(session_id?, path)`
- `apply_patch(session_id?, patch)`
- `view_image(session_id?, path)`
- `git(session_id?, args...)`
- `git_status(session_id?)`
- `git_diff(session_id?, ...)`
- `git_commit(session_id?, ...)`
- `git_branch(session_id?, ...)`
- `git_checkout(session_id?, ...)`
- `git_push(session_id?, ...)`
- `todo(session_id?, ...)`
- `update_plan(session_id?, plan)`

When `session_id` is omitted, use the "default" session (backward compatible).
When provided, look up the session's worktree path and use that as the
workspace root.

#### 1.4 Implement session lifecycle tools

- `create_session(repo_url, branch?) -> {session_id, worktree_path, branch}`
- `list_sessions() -> [{session_id, branch, status, last_active}]`
- `get_session_status(session_id) -> {session_id, branch, todo, last_active}`
- `close_session(session_id) -> {closed: true}`

#### 1.5 Add worktree GC

- Background thread or periodic check: remove worktrees for sessions that
  have been idle > N hours (configurable).
- Run `git worktree prune` after each removal.

#### 1.6 Tests

- Unit tests for `SessionManager` with a temp bare repo
- Integration test: create two sessions, verify they're on different branches
- Test backward compatibility (no session_id → default session)

### Dependencies

None — this is self-contained in the Rust server.

---

## Phase 2: Subagent spawning

**Goal:** A parent ChatGPT instance can spawn subagents. Each subagent is a
separate ChatGPT instance that claims a session and works independently.

### Tasks

#### 2.1 Implement `spawn_subagent`

- `spawn_subagent(goal, context, parent_session?) -> {subagent_id, branch, token}`
- Creates a new worktree on a branch named `subagent/<id>/<slug>`
- Generates a one-time-use token (UUID + HMAC or stored in DB)
- Stores parent-child relationship in `SessionState.parent_session`
- Returns the subagent ID, branch name, and claim token

#### 2.2 Implement `claim_session`

- `claim_session(session_id, token) -> {claimed: true, branch}`
- Validates the one-time token
- Marks the session as claimed (no other agent can claim it)
- Returns the branch name so the subagent knows where it is

#### 2.3 Implement subagent status tools

- `get_subagent_status(subagent_id) -> {status, branch, last_commit, todo}`
  - Status: "pending" (not yet claimed), "active" (claimed, working),
    "done" (merged or closed)
- `list_subagents(parent_session?) -> [{id, status, goal, branch}]`

#### 2.4 Implement `merge_subagent`

- `merge_subagent(subagent_id, into_branch?) -> {merged: true, conflicts?}`
- Runs `git merge <subagent-branch>` in the parent's worktree
- If conflicts, returns the conflict list and does NOT auto-resolve
- If clean, deletes the subagent branch (optional, configurable)

#### 2.5 Token management

- Store tokens in memory (HashMap) or SQLite
- Tokens expire after N minutes (configurable, default 30)
- One-time use: consumed on first `claim_session`
- Clean up expired tokens periodically

#### 2.6 Tests

- Test `spawn_subagent` creates a worktree on the right branch
- Test `claim_session` with valid/invalid/expired tokens
- Test `merge_subagent` with clean merge and with conflicts
- Test parent-child relationship tracking

### Dependencies

Phase 1 (SessionManager must exist)

---

## Phase 3: Self-scheduling

**Goal:** ChatGPT can schedule itself to resume later. At the start of each
turn, it polls for due tasks and picks up where it left off.

### Tasks

#### 3.1 Add SQLite dependency

- Add `rusqlite` or `sqlx` to `Cargo.toml`
- Create schema: `schedules(id, cron_expr, context_json, session_id?,
  last_run, next_run, created_at)`

#### 3.2 Implement scheduler tools

- `schedule_resume(cron_expr, context, session_id?) -> {schedule_id}`
  - Parses the cron expression
  - Stores in SQLite
  - Returns schedule ID
- `list_schedules() -> [{id, cron, context, last_run, next_run}]`
- `cancel_schedule(schedule_id) -> {cancelled: true}`
- `poll_due_tasks() -> [{schedule_id, context, session_id}]`
  - Queries SQLite for schedules where `next_run <= now()`
  - Returns them (does NOT mark as run — the agent does that on pickup)

#### 3.3 Scheduler tick

- On `poll_due_tasks`, also update `next_run` for recurring schedules
- For one-shot schedules, mark as completed after first pickup

#### 3.4 Tests

- Test schedule creation and cron parsing
- Test `poll_due_tasks` returns due items
- Test recurring schedule updates `next_run`
- Test cancellation

### Dependencies

Phase 1 (session IDs needed for `session_id` field)

---

## Phase 4: Prompt engineering

**Goal:** ChatGPT actually uses subagents and self-scheduling. The prompt
must be aggressive enough to overcome ChatGPT's default sequential behavior.

### Tasks

#### 4.1 Write the system prompt section

Add to the MCP server's prompt template (or AGENTS.md):

```
## Multi-agent workflow (MANDATORY)

You MUST use subagents for any task that can be parallelized. This is not
optional. If you have two or more independent work items, you MUST spawn
subagents.

When to subagent:
- Implementing multiple features → one subagent per feature
- Running tests while writing code → subagent for tests
- Reviewing code while writing more code → subagent for review
- Any task that takes >5 minutes → subagent for parts of it

Workflow:
1. Call spawn_subagent(goal, context) for each parallel task
2. Each subagent works independently on its own branch (worktree)
3. Check progress with get_subagent_status
4. Merge completed subagents with merge_subagent
5. Resolve any merge conflicts immediately

DO NOT do work sequentially that could be done in parallel. This is the
single most important optimization. Subagents are free. Use them.

## Self-scheduling (MANDATORY)

You MUST schedule yourself to continue later when:
- A task will take more than one session to complete
- You're waiting for external input (build, test, review)
- You've made progress but the work isn't done

Workflow:
1. Call memory_retain to save your current state, decisions, and next steps
2. Call schedule_resume(cron, context) to schedule your next run
3. At the start of EVERY turn, call poll_due_tasks to pick up scheduled work
4. Use memory_search to restore context from your previous run

DO NOT leave work unfinished without scheduling a resume. If you stop
without scheduling, the work is lost.
```

#### 4.2 Serve the prompt via MCP

- Add a `get_prompt("system")` handler that returns the full system prompt
- Or embed it in the server's `instructions` field (MCP protocol supports
  server-level instructions)

#### 4.3 Test with real ChatGPT

- Deploy the server
- Connect a ChatGPT instance
- Verify it actually calls `spawn_subagent` and `schedule_resume`
- Iterate on prompt wording based on observed behavior

### Dependencies

Phases 1-3 (tools must exist before the prompt can reference them)

---

## Phase 5: Polish

### Tasks

#### 5.1 Session GC

- Background thread: close sessions idle > N hours
- Configurable timeout (env var or CLI arg)
- Log when sessions are auto-closed

#### 5.2 Resource limits

- Max sessions per user (configurable)
- Max worktrees per bare repo (git has no hard limit, but disk space)
- Return clear errors when limits are hit

#### 5.3 Monitoring

- Prometheus metrics: active sessions, subagents spawned, merges, schedules
- Structured logging for all session lifecycle events
- Health check endpoint

#### 5.4 Conflict resolution UX

- When `merge_subagent` hits conflicts, return the conflict files
- Optionally add a `resolve_conflict(file, resolution)` tool
- Or leave conflict resolution to the agent (it can read the conflicted
  files and apply a patch)

### Dependencies

Phases 1-4

---

## Timeline estimate

| Phase | Effort | Dependencies |
|-------|--------|-------------|
| Phase 1: Worktree session manager | 3-5 days | None |
| Phase 2: Subagent spawning | 2-3 days | Phase 1 |
| Phase 3: Self-scheduling | 1-2 days | Phase 1 |
| Phase 4: Prompt engineering | 1 day | Phases 1-3 |
| Phase 5: Polish | 2-3 days | Phases 1-4 |

Total: ~9-14 days of focused work.

## Key risks

1. **Worktree + sandbox compatibility** — The read-only sandbox wraps a
   directory. Each session's sandbox must point at its worktree. This should
   work since the sandbox is per-session, but needs verification.

2. **Token security** — One-time tokens are stored in memory. If the server
   restarts, all pending tokens are lost. For v1 this is acceptable (tokens
   expire in 30 min anyway). For v2, store in SQLite.

3. **ChatGPT compliance** — The prompt nudging is aggressive, but ChatGPT
   may still ignore it. We may need to iterate on the prompt multiple times.

4. **Worktree limits** — Git has no hard limit, but each worktree holds a
   working tree + index. On a VPS with 8GB RAM, ~50-100 worktrees is the
   practical limit.
