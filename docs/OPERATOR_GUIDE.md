# Operator Guide

This guide is for human operators running ChatCodex in production. It explains what to monitor, how to recognize problems, and when to intervene.

## Operator Role

ChatCodex is designed for **human-in-the-loop operation**. The operator's role is to:

1. **Monitor** — Watch the queue for signals
2. **Diagnose** — Identify runs needing attention
3. **Intervene** — Apply deterministic recovery actions
4. **Verify** — Confirm interventions worked

The architecture rule is absolute:

> The only LLM in the stack is ChatGPT. The backend is fully deterministic.

This means:
- No autonomous background processes
- No hidden agent loops
- Every action is explicitly triggered by ChatGPT or the operator
- The operator has full visibility and control

---

## Monitoring Surfaces

### Queue Overview

The primary monitoring surface is the queue overview:

```
> Show me the queue overview.

→ get_run_queue_overview({ today: "2026-03-18" })

Response:
{
  "totalVisible": 12,
  "readyCount": 5,
  "blockedCount": 2,
  "overdueCount": 1,
  "snoozedCount": 3,
  "archivedCount": 8
}
```

**What to watch:**

| Signal | Meaning | Action |
|--------|---------|--------|
| `blockedCount` > 0 | Runs waiting on dependencies | Resolve blockers or wait |
| `overdueCount` > 0 | Runs past due date | Prioritize or reassign |
| High `snoozedCount` | Many deferred runs | Review if still needed |
| High `readyCount` | Many runs competing | Prioritize, assign owners |

### Saved Views

Create saved views for daily monitoring:

```
> Create a view called "needs-attention" for blocked or overdue runs.

→ create_queue_view({
    name: "needs-attention",
    description: "Runs requiring operator intervention",
    filters: { blockedOnly: true, staleOnly: true },
    sort: { sortByPriority: true }
  })
```

Check this view daily:

```
> Show me the "needs-attention" view.

→ get_queue_view({ viewId: "qv_xxx" })
→ list_runs({ blockedOnly: true, staleOnly: true })
```

### Run State Signals

When inspecting individual runs, watch for:

| Field | Healthy | Needs Attention |
|-------|---------|-----------------|
| `status` | `active`, `prepared` | `finalized:failed` |
| `isBlocked` | `false` | `true` — check `blockedByRunIds` |
| `snoozedUntil` | `null` | Date in past — unsnooze |
| `dueDate` | Future or `null` | Past — overdue |
| `staleHours` | Low | High — hasn't been touched in a while |
| `retryableAction` | `null` | Present — action pending approval/denial |
| `assignee` | Set | `null` — consider assigning |

---

## State Diagram

Understanding run states helps you know when to intervene:

```
                    ┌─────────────┐
                    │  prepared   │
                    └──────┬──────┘
                           │ refresh
                           ▼
                    ┌─────────────┐
              ┌─────│   active    │◄────┐
              │     └──────┬──────┘     │
              │            │            │
              │            │ finalize   │ reopen
         snooze│            ▼            │
              │     ┌─────────────┐     │
              │     │  finalized  │─────┘
              │     │ :completed  │
              │     │ :failed     │
              │     │ :abandoned  │
              │     └──────┬──────┘
              │            │
              │            │ archive
              ▼            ▼
       ┌─────────────┐ ┌─────────────┐
       │   snoozed   │ │  archived   │
       └─────────────┘ └─────────────┘
              │
              │ unsnooze
              └──────────────────────► active

              ┌─────────────┐
              │   pinned    │◄── pinned items surface at top of list
              └─────────────┘
```

---

## Daily Operations

### Morning Queue Review

**Goal:** Identify runs needing attention before starting work.

```
1. "Show me the queue overview."
2. "Show me blocked runs."
3. "Show me overdue runs."
4. "Show me stale runs."
```

For each problematic run:
1. `get_run_state` for full details
2. Check history for context
3. Apply intervention (see [INTERVENTION_PATTERNS.md](./INTERVENTION_PATTERNS.md))
4. Update metadata (priority, owner, due date) as needed

### During Active Work

When ChatGPT is working on runs:

1. **Watch for policy blocks** — Approval requests need your decision
2. **Monitor progress** — Check `refresh_run_state` periodically
3. **Handle blockers** — If run is blocked, resolve dependency

### End of Day

1. **Finalize completed runs** — Don't leave runs in `active` state overnight
2. **Snooze deferred work** — Set `snoozedUntil` for runs waiting on external factors
3. **Archive completed runs** — Keep queue clean

---

## Intervention Decision Tree

```
┌─────────────────────────────────────────────────────────────┐
│                    RUN NEEDS ATTENTION                       │
└───────────────────────────┬─────────────────────────────────┘
                            │
                            ▼
                    ┌───────────────┐
                    │ What's wrong? │
                    └───────┬───────┘
                            │
        ┌───────────────────┼───────────────────┐
        │                   │                   │
        ▼                   ▼                   ▼
   ┌─────────┐        ┌─────────┐        ┌─────────┐
   │ Blocked │        │  Stale  │        │ Overdue │
   └────┬────┘        └────┬────┘        └────┬────┘
        │                  │                   │
        ▼                  ▼                   ▼
   See: Dependency    See: Staleness     See: Prioritization
   Pattern            Pattern             Pattern

        ┌───────────────────┼───────────────────┐
        │                   │                   │
        ▼                   ▼                   ▼
   ┌──────────┐       ┌──────────┐       ┌──────────┐
   │ Wrong    │       │ Wrong    │       │ Approval │
   │ Approach │       │ Owner    │       │ Pending  │
   └────┬─────┘       └────┬─────┘       └────┬─────┘
        │                  │                   │
        ▼                  ▼                   ▼
   See: Supersede     See: Assignment    See: Approval
   Pattern            Pattern            Pattern
```

---

## Queue Shaping Tools

### Prioritization

```
> Make run X urgent.

→ set_run_priority({ runId: "...", priority: "urgent" })
```

Priorities: `low`, `normal`, `high`, `urgent`

### Assignment

```
> Assign run X to Alice.

→ assign_run_owner({ runId: "...", assignee: "alice" })
```

```
> Clear the owner on run X.

→ assign_run_owner({ runId: "...", assignee: null })
```

### Due Dates

```
> Set run X due by Friday.

→ set_run_due_date({ runId: "...", dueDate: "2026-03-21" })
```

```
> Clear the due date.

→ set_run_due_date({ runId: "...", dueDate: null })
```

### Dependencies

```
> Run X depends on run Y completing first.

→ set_run_dependencies({ runId: "X", blockedByRunIds: ["Y"] })
```

```
> Remove dependency.

→ set_run_dependencies({ runId: "...", blockedByRunIds: [] })
```

### Labels and Notes

```
> Tag run X as "backend" and "auth" with a note.

→ annotate_run({
    runId: "...",
    labels: ["backend", "auth"],
    note: "OAuth scope issue - waiting on upstream fix"
  })
```

---

## Lifecycle Actions

### Snooze (Defer)

Use when a run is waiting on external factors:

```
> Snooze run X until the API fix lands.

→ snooze_run({
    runId: "...",
    untilDate: "2026-03-25",
    reason: "Waiting for upstream API fix"
  })
```

Snoozed runs are excluded from default queue views.

### Unsnooze (Resume)

```
> Resume work on run X.

→ unsnooze_run({ runId: "..." })
```

### Archive (Organize)

Use for completed runs to clean the queue:

```
> Archive all completed runs older than 7 days.

→ list_runs({ status: "finalized:completed" })
// For each old run:
→ archive_run({ runId: "..." })
```

### Unarchive (Restore)

```
> Restore the archived run X.

→ unarchive_run({ runId: "..." })
```

### Reopen (Continue)

Use to resume finalized runs:

```
> Reopen run X — we found a bug.

→ reopen_run({
    runId: "...",
    reason: "Production bug in the feature"
  })
```

### Supersede (Replace)

Use when the original approach was wrong:

```
> The approach in run X is wrong. Start fresh.

→ supersede_run({
    runId: "...",
    newUserGoal: "New approach for feature X",
    reason: "Original architecture was flawed"
  })
```

### Finalize (Close)

Always close runs when done:

```
→ finalize_run({
    runId: "...",
    outcomeKind: "completed",  // or "failed" or "abandoned"
    summary: "Added health check endpoint"
  })
```

---

## Approval Handling

### When Approval is Required

Patches and tests may require approval:

| Trigger | Policy |
|---------|--------|
| File deletion | `deleteRequiresApproval: true` |
| Many edits | `patchEditThreshold: 5` |
| Sensitive paths | `.env`, `.git/`, credentials |
| Outside focus paths | Patch outside `focusPaths` |
| Non-standard make target | Not `test`, `check`, `lint`, `build`, etc. |

### Handling Approval Requests

When `apply_patch` or `run_tests` returns `approvalRequired: true`:

```
Response:
{
  "approvalRequired": true,
  "approvalId": "apr_xyz789",
  "policyRationale": "Patch contains 8 edits, exceeds threshold of 5"
}
```

**Operator decision:**

```
Option A: Approve
→ approve_action({ approvalId: "apr_xyz789", decision: "approve" })
→ apply_patch({ ... }) // Retry

Option B: Deny
→ approve_action({ approvalId: "apr_xyz789", decision: "deny" })
// ChatGPT must revise approach
```

### Preview Mode

Before applying, check if approval will be needed:

```
→ preview_patch_policy({ runId: "...", edits: [...] })
// Returns: { decision: "proceed" | "requires_approval", riskReason: "..." }
```

---

## Common Monitoring Patterns

### Pattern: Daily Standup Prep

```
> Show me the queue overview.
> Which runs are overdue?
> Which runs are blocked?
> Which runs are stale?
```

### Pattern: End of Sprint

```
> Show me all active runs.
> For each: check if still relevant.
> Finalize completed work.
> Archive old runs.
> Update priorities for next sprint.
```

### Pattern: On-call Handoff

```
> Show me pinned runs.           // Important items
> Show me blocked runs.          // Need attention
> Show me snoozed runs.          // May need unsnooze
> Show me runs assigned to me.   // Your active work
```

---

## Key Metrics to Track

While ChatCodex doesn't have built-in metrics, operators should watch:

| Metric | How to Check | What It Means |
|--------|--------------|---------------|
| Queue size | `get_run_queue_overview` | Work volume |
| Blocked rate | `blockedCount / totalVisible` | Dependency friction |
| Stale rate | `staleOnly` filter count | Abandoned work |
| Completion rate | `finalized:completed` count | Productivity |
| Failure rate | `finalized:failed` count | Quality issues |

---

## Understanding Error Messages

ChatCodex errors are designed to be actionable. When an operation fails, the error message typically includes:

1. **What failed** — The operation and the affected run/view
2. **Why it failed** — The specific constraint violated
3. **What to do next** — Recovery hints when applicable

### Common Error Patterns

| Error Pattern | Meaning | Recovery |
|---------------|---------|----------|
| `unknown run: X` | Run ID doesn't exist | Use `list_runs` to see available runs |
| `cannot be reopened: status is 'active'` | Run is not finalized | Finalize the run first, or check current status |
| `cannot be archived: status is 'active'` | Run is not finalized | Use `finalize_run` to close it first |
| `is already finalized` | Run already closed | Use `reopen_run` to continue work, or `supersede_run` for new approach |
| `is already archived` | Run already archived | Use `unarchive_run` to restore it |
| `is not archived` | Run was never archived | Use `list_runs` with `includeArchived` to see archived runs |
| `is not snoozed` | Run was not snoozed | Use `list_runs` with `includeSnoozed` to see snoozed runs |
| `view not found: X` | Saved view doesn't exist | Use `list_queue_views` to see available views |
| `a view named 'X' already exists` | Duplicate view name | Choose a different name or update the existing view |

### Error Message Format

Errors from ChatCodex follow this format:

```
ChatCodex error in <method>: <error message> <recovery hint>
```

Examples:

```
ChatCodex error in run.reopen: run 'run_abc123' cannot be reopened: status is 'active'
(only finalized runs may be reopened). Use get_run_state to inspect the run,
or finalize_run if work is complete.

ChatCodex error in run.finalize: run 'run_xyz' is already finalized (status: finalized:completed).
Use reopen_run to continue work, or supersede_run to start a new approach.
```

### When Errors Don't Include Recovery Hints

Some errors are straightforward and don't need hints:

- Validation errors (invalid format, empty fields)
- Permission errors
- Resource not found

For these, check the [INTERVENTION_PATTERNS.md](./INTERVENTION_PATTERNS.md) for recovery guidance.

---

## Understanding Error Messages

ChatCodex provides clear, actionable error messages. When operations fail, the error message typically includes:

1. **What failed** — The specific operation that couldn't complete
2. **Why it failed** — The constraint or state that prevented it
3. **What to do next** — A suggested recovery action

### Common Error Patterns

| Error Message | Cause | Recovery |
|---------------|-------|----------|
| `run cannot be reopened: status is 'active'` | Run isn't finalized | Use `finalize_run` first |
| `run is already finalized` | Run already closed | Use `reopen_run` or `supersede_run` |
| `run cannot be archived: status is 'active'` | Run not finalized | Use `finalize_run` first |
| `run is already archived` | Already archived | Use `unarchive_run` to restore |
| `run is not snoozed` | Not in snoozed state | Check status with `get_run_state` |
| `view not found` | Invalid view ID | Use `list_queue_views` to see available views |
| `unknown run` | Invalid run ID | Use `list_runs` to see available runs |
| `a view named 'X' already exists` | Duplicate name | Use different name or `update_queue_view` |

### Error Message Format

```
ChatCodex error in <method>: <error message> <recovery hint>
```

Example:
```
ChatCodex error in finalize_run: run 'run-abc' is already finalized (status: finalized:completed).
Use reopen_run to continue work, or supersede_run to start a new approach.
```

### When Errors Occur

Most errors fall into these categories:

1. **Lifecycle errors** — Wrong state for operation (e.g., finalizing an already-finalized run)
2. **Validation errors** — Invalid parameters (e.g., empty name, invalid date format)
3. **Not found errors** — Run or view doesn't exist
4. **Policy errors** — Operation blocked by policy (e.g., approval required)

For detailed recovery steps for each error type, see [INTERVENTION_PATTERNS.md](./INTERVENTION_PATTERNS.md).

---

## What's Next

- **Quick start**: See [MVP_README.md](./MVP_README.md) for the fastest path to first use
- **Intervention patterns**: See [INTERVENTION_PATTERNS.md](./INTERVENTION_PATTERNS.md) for specific recovery playbooks
- **First run workflow**: See [FIRST_RUN_WORKFLOW.md](./FIRST_RUN_WORKFLOW.md) for the basic workflow
- **Tool reference**: See [TOOLS_OVERVIEW.md](./TOOLS_OVERVIEW.md) for all available tools