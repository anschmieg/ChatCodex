# Intervention Patterns

This document provides concrete playbooks for recovering runs that need operator attention. Each pattern includes symptoms, diagnosis, and step-by-step resolution.

## How to Use This Document

1. **Identify the symptom** — What's wrong with the run?
2. **Find the matching pattern** — Use the table of contents
3. **Follow the playbook** — Step-by-step with example prompts

---

## Pattern Index

| Symptom | Pattern |
|---------|---------|
| Run stuck waiting on approval | [Approval Blocked](#approval-blocked) |
| Run waiting on another run | [Dependency Blocked](#dependency-blocked) |
| Run hasn't been touched in days | [Stale Run](#stale-run) |
| Run past due date | [Overdue Run](#overdue-run) |
| Wrong approach, need fresh start | [Supersede](#supersede-wrong-approach) |
| Need to resume completed work | [Reopen](#reopen-resume-work) |
| Need to pause work temporarily | [Snooze](#snooze-defer-work) |
| Wrong owner assigned | [Reassign](#reassign-owner) |
| Wrong priority | [Reprioritize](#reprioritize) |
| Wrong due date | [Reschedule](#reschedule-due-date) |
| Cleanup old runs | [Archive](#archive-cleanup) |
| Approve/deny pending action | [Approval Decision](#approval-decision) |
| Metadata needs correction | [Correct Metadata](#correct-metadata) |

---

## Approval Blocked

**Symptom:** Run has `retryableAction` pending, patch or test requires approval before proceeding.

**Diagnosis:**

```
> Show me the current state of run X.

Response includes:
{
  "retryableAction": {
    "actionType": "apply_patch",
    "approvalId": "apr_xyz789",
    "policyRationale": "8 edits exceeds threshold of 5"
  }
}
```

**Playbook:**

### Option A: Approve and Proceed

```
Step 1: Review the rationale
> Why does this need approval?

Response shows: "8 edits exceeds threshold of 5"

Step 2: Review what will change
> Show me the diff for this run.

→ show_diff({ runId: "..." })

Step 3: Decide to approve
> Approve the action.

→ approve_action({ approvalId: "apr_xyz789", decision: "approve" })

Step 4: Retry the action
> Apply the patch again.

→ apply_patch({ runId: "...", edits: [...] })
```

### Option B: Deny and Redirect

```
Step 1: Review the rationale
> Why does this need approval?

Step 2: Decide to deny
> Deny this action — too risky.

→ approve_action({ approvalId: "apr_xyz789", decision: "deny" })

Step 3: Redirect ChatGPT
> The patch is denied. Let's break it into smaller changes.

→ replan_run({ runId: "...", reason: "Patch denied, breaking into smaller changes", ... })
```

### Prevention: Preview Before Applying

```
> Before applying, check if this will need approval.

→ preview_patch_policy({ runId: "...", edits: [...] })

Response:
{ "decision": "requires_approval", "riskReason": "8 edits exceeds threshold" }

Operator can then:
- Approve larger change
- Request smaller patches
- Adjust policy thresholds at run creation
```

---

## Dependency Blocked

**Symptom:** Run has `isBlocked: true` and `blockedByRunIds` pointing to other runs.

**Diagnosis:**

```
> Show me blocked runs.

→ list_runs({ blockedOnly: true })

Response:
{
  "runs": [{
    "runId": "run_abc123",
    "isBlocked": true,
    "blockedByRunIds": ["run_def456"],
    "userGoal": "Add user authentication"
  }]
}

> What's blocking run_abc123?

→ get_run_state({ runId: "run_def456" })
```

**Playbook:**

### Option A: Resolve the Blocker

```
Step 1: Check blocker status
> Show me run_def456's state.

Step 2: If blocker is active:
  - Work on the blocker first
  - Or assign someone else to it

> Assign run_def456 to Bob so he can complete it.

→ assign_run_owner({ runId: "run_def456", assignee: "bob" })

Step 3: Once blocker is finalized:
> The dependency is complete. Continue with run_abc123.

The run is now unblocked automatically.
```

### Option B: Remove Dependency

```
If the dependency was set in error:

> Remove the dependency on run_abc123.

→ set_run_dependencies({ runId: "run_abc123", blockedByRunIds: [] })

Now the run is unblocked.
```

### Option C: Snooze While Waiting

```
If the blocker will take a while:

> Snooze run_abc123 until run_def456 is complete.

→ snooze_run({
    runId: "run_abc123",
    untilDate: "2026-03-25",
    reason: "Waiting for run_def456 (auth) to complete"
  })
```

---

## Stale Run

**Symptom:** Run hasn't been updated in a significant time (check `staleHours` or `lastUpdatedAt`).

**Diagnosis:**

```
> Show me stale runs.

→ list_runs({ staleOnly: true, minStaleHours: 24 })

Response:
{
  "runs": [{
    "runId": "run_xyz",
    "staleHours": 72,
    "userGoal": "Refactor database layer",
    "status": "active"
  }]
}
```

**Playbook:**

### Option A: Continue Work

```
Step 1: Check run state
> Show me the state of run_xyz.

Step 2: If still relevant:
> This run is still needed. Let's continue.
→ refresh_run_state({ runId: "run_xyz" })

Step 3: Assign to someone
> Assign run_xyz to Alice.

→ assign_run_owner({ runId: "run_xyz", assignee: "alice" })
```

### Option B: Snooze for Later

```
If not a priority now:

> Snooze run_xyz until next sprint.

→ snooze_run({
    runId: "run_xyz",
    untilDate: "2026-04-01",
    reason: "Deferring to next sprint"
  })
```

### Option C: Abandon

```
If no longer relevant:

> This run is no longer needed. Abandon it.

→ finalize_run({
    runId: "run_xyz",
    outcomeKind: "abandoned",
    summary: "No longer needed, stale for 72 hours"
  })
```

---

## Overdue Run

**Symptom:** Run's `dueDate` is in the past.

**Diagnosis:**

```
> Show me overdue runs.

→ list_runs({ overdueOnly: true })

Response:
{
  "runs": [{
    "runId": "run_overdue",
    "dueDate": "2026-03-15",  // Past date
    "userGoal": "Fix production bug",
    "status": "active"
  }]
}
```

**Playbook:**

### Option A: Prioritize

```
Step 1: Make it urgent
> Make run_overdue urgent.

→ set_run_priority({ runId: "run_overdue", priority: "urgent" })

Step 2: Assign to someone
> Assign run_overdue to the on-call engineer.

→ assign_run_owner({ runId: "run_overdue", assignee: "oncall" })

Step 3: Pin it for visibility
> Pin run_overdue so it's visible.

→ pin_run({ runId: "run_overdue" })
```

### Option B: Extend Deadline

```
If the deadline was unrealistic:

> Push the due date to next week.

→ set_run_due_date({
    runId: "run_overdue",
    dueDate: "2026-03-25"
  })
```

### Option C: Accept Missed Deadline

```
If the deadline can't be met:

Step 1: Clear the due date
> Remove the due date.

→ set_run_due_date({ runId: "run_overdue", dueDate: null })

Step 2: Add a note
> Add a note about the missed deadline.

→ annotate_run({
    runId: "run_overdue",
    note: "Deadline missed due to upstream delay. No new deadline."
  })
```

---

## Supersede: Wrong Approach

**Symptom:** Run's approach is fundamentally wrong, need to start fresh while preserving history.

**Diagnosis:**

```
> Run X has the wrong architecture. We need a different approach.
```

**Playbook:**

```
Step 1: Supersede the run
> Supersede run_abc123 with a new approach.

→ supersede_run({
    runId: "run_abc123",
    newUserGoal: "Implement feature using event-driven architecture instead",
    reason: "Original synchronous approach won't scale"
  })

Response:
{
  "successorRunId": "run_new456",
  "status": "prepared"
}

Step 2: The original run is now:
- Status: finalized:superseded
- Has: supersededByRunId: "run_new456"

Step 3: The new run has:
- Status: prepared
- Has: supersedesRunId: "run_abc123"
- Preserves: focusPaths, policy from original

Step 4: Work on the new run
> Start working on run_new456.

Step 5: Add a note explaining lineage
> Annotate run_new456 with context.

→ annotate_run({
    runId: "run_new456",
    note: "Supersedes run_abc123 - switching to event-driven architecture"
  })
```

**When to use:**
- Original approach was fundamentally flawed
- Requirements changed significantly
- Want to preserve history of attempt

**When NOT to use:**
- Small course correction → use `replan_run`
- Pausing work → use `snooze_run` or `finalize_run`

---

## Reopen: Resume Work

**Symptom:** Need to continue work on a finalized run.

**Diagnosis:**

```
> Run X was completed but we found a bug in production.
```

**Playbook:**

```
Step 1: Reopen the run
> Reopen run_abc123.

→ reopen_run({
    runId: "run_abc123",
    reason: "Production bug in the feature"
  })

Response:
{
  "status": "active",
  "reopenedAt": "2026-03-18T14:30:00Z",
  "reopenCount": 1
}

Step 2: The run is now active
- Status: active
- Previous work is preserved
- History shows reopen event

Step 3: Continue work
> Show me the state of run_abc123.

→ get_run_state({ runId: "run_abc123" })

Step 4: Make fixes
> Apply a patch for the bug.

Step 5: Finalize again
> Finalize as completed with the fix.

→ finalize_run({
    runId: "run_abc123",
    outcomeKind: "completed",
    summary: "Fixed production bug"
  })
```

**When to use:**
- Bug found in completed work
- Need to add missing feature
- Continue previous work

---

## Snooze: Defer Work

**Symptom:** Run needs to wait for something external.

**Diagnosis:**

```
> Run X is waiting on an upstream fix that won't land until next week.
```

**Playbook:**

```
Step 1: Snooze the run
> Snooze run_abc123 until the API fix lands.

→ snooze_run({
    runId: "run_abc123",
    untilDate: "2026-03-25",
    reason: "Waiting for upstream API fix in release 2.1"
  })

Response:
{
  "status": "snoozed",
  "snoozedUntil": "2026-03-25"
}

Step 2: Snoozed runs are:
- Excluded from default list_runs
- Visible with includeSnoozed: true
- Visible in snoozedOnly filter

Step 3: When ready to resume
> Unsnooze run_abc123.

→ unsnooze_run({ runId: "run_abc123" })
```

**When to use:**
- Waiting for external dependency
- Blocked on calendar event
- Low priority, revisit later

---

## Reassign Owner

**Symptom:** Run has wrong or no owner assigned.

**Diagnosis:**

```
> Show me unassigned runs.

→ list_runs({ assignee: null })

> Show me runs assigned to Alice who is on vacation.

→ list_runs({ assignee: "alice" })
```

**Playbook:**

```
Step 1: Assign to new owner
> Assign run_abc123 to Bob.

→ assign_run_owner({
    runId: "run_abc123",
    assignee: "bob",
    ownershipNote: "Alice is on vacation this week"
  })

Step 2: Clear ownership
> Remove the owner assignment.

→ assign_run_owner({
    runId: "run_abc123",
    assignee: null
  })
```

---

## Reprioritize

**Symptom:** Run has wrong priority level.

**Diagnosis:**

```
> Show me all urgent runs.

→ list_runs({ priorityFilter: "urgent" })

> This run should be urgent, not normal priority.
```

**Playbook:**

```
Step 1: Set priority
> Make run_abc123 urgent.

→ set_run_priority({
    runId: "run_abc123",
    priority: "urgent"
  })

Priorities: low, normal, high, urgent

Step 2: Priority affects sorting
> Show me runs sorted by priority.

→ list_runs({ sort: { sortByPriority: true } })
```

---

## Reschedule Due Date

**Symptom:** Run has wrong due date or needs one.

**Diagnosis:**

```
> Show me runs due this week.

→ list_runs({ dueOnOrBefore: "2026-03-21" })

> Run X needs a deadline.
```

**Playbook:**

```
Step 1: Set due date
> Set run_abc123 due by Friday.

→ set_run_due_date({
    runId: "run_abc123",
    dueDate: "2026-03-21"
  })

Step 2: Clear due date
> Remove the deadline.

→ set_run_due_date({
    runId: "run_abc123",
    dueDate: null
  })
```

---

## Archive: Cleanup

**Symptom:** Queue is cluttered with old completed runs.

**Diagnosis:**

```
> Show me all completed runs.

→ list_runs({ status: "finalized:completed" })

Response shows many old runs.
```

**Playbook:**

```
Step 1: Archive completed runs
> Archive run_abc123.

→ archive_run({ runId: "run_abc123" })

Step 2: Archived runs:
- Excluded from default list_runs
- Visible with includeArchived: true
- Visible in archivedOnly filter

Step 3: Bulk archive (manual process)
> Show me completed runs older than 30 days.
(Operator identifies old runs)
> Archive run X, run Y, run Z.

Step 4: Restore if needed
> Unarchive run_abc123.

→ unarchive_run({ runId: "run_abc123" })
```

---

## Approval Decision

**Symptom:** Action needs operator approval before proceeding.

**Diagnosis:**

```
> Why is run X waiting?

Response shows:
{
  "retryableAction": {
    "actionType": "apply_patch",
    "approvalId": "apr_123",
    "policyRationale": "Editing sensitive file .env"
  }
}
```

**Playbook:**

### Approve

```
Step 1: Check the policy rationale
> Why does this need approval?

Step 2: Review what will happen
> Show me what changes this patch will make.

Step 3: Approve
> Approve the patch.

→ approve_action({
    approvalId: "apr_123",
    decision: "approve"
  })

Step 4: Proceed
> Apply the patch.

→ apply_patch({ runId: "...", edits: [...] })
```

### Deny

```
Step 1: Review rationale

Step 2: Deny
> Deny this action — it's too risky.

→ approve_action({
    approvalId: "apr_123",
    decision: "deny"
  })

Step 3: Redirect
> Let's take a different approach.

ChatGPT will need to replan.
```

---

## Correct Metadata

**Symptom:** Run has incorrect labels, notes, or other metadata.

**Diagnosis:**

```
> Run X should be tagged as backend, not frontend.
```

**Playbook:**

```
Step 1: Add labels
> Tag run_abc123 as backend and api.

→ annotate_run({
    runId: "run_abc123",
    labels: ["backend", "api"]
  })

Note: Labels are additive. To clear, set to empty.

Step 2: Add a note
> Add a note about the architecture decision.

→ annotate_run({
    runId: "run_abc123",
    note: "Decided to use REST API instead of GraphQL"
  })

Step 3: View annotations
> Show me run_abc123's full state.

→ get_run_state({ runId: "run_abc123" })
// Check labels and notes fields
```

---

## Combined Intervention Examples

### Scenario: Blocked and Overdue

```
Symptom: Run is both blocked and past due date.

Diagnosis:
> Show me blocked runs that are also overdue.

Playbook:
Step 1: Assess priority
> Make this run urgent.

→ set_run_priority({ runId: "...", priority: "urgent" })

Step 2: Resolve blocker
> What's blocking this run?
> Assign the blocker to Bob.

→ assign_run_owner({ runId: "blocker", assignee: "bob" })

Step 3: Extend deadline if needed
> Push the deadline to next week.

→ set_run_due_date({ runId: "...", dueDate: "2026-03-25" })
```

### Scenario: Stale and Unassigned

```
Symptom: Run hasn't been touched in days and has no owner.

Playbook:
Step 1: Check if still relevant
> Show me the state of run X.

Step 2: If relevant:
> Assign run X to Alice and make it high priority.

→ assign_run_owner({ runId: "...", assignee: "alice" })
→ set_run_priority({ runId: "...", priority: "high" })

Step 3: If not relevant:
> Finalize run X as abandoned.

→ finalize_run({
    runId: "...",
    outcomeKind: "abandoned",
    summary: "No longer needed, stale for 72 hours"
  })
```

---

## Intervention Decision Quick Reference

| Situation | Action |
|-----------|--------|
| Needs approval | `approve_action` |
| Waiting on dependency | `snooze_run` or resolve blocker |
| Not touched in days | `finalize_run` or `snooze_run` or assign |
| Past due date | Prioritize, extend deadline, or clear |
| Wrong approach | `supersede_run` |
| Need to continue | `reopen_run` |
| Wrong owner | `assign_run_owner` |
| Wrong priority | `set_run_priority` |
| Wrong due date | `set_run_due_date` |
| Wrong dependency | `set_run_dependencies` |
| Needs notes/labels | `annotate_run` |
| Cleanup old work | `archive_run` |

---

## What's Next

- **Operator guide**: See [OPERATOR_GUIDE.md](./OPERATOR_GUIDE.md) for daily operations
- **First run workflow**: See [FIRST_RUN_WORKFLOW.md](./FIRST_RUN_WORKFLOW.md) for basics
- **Tool reference**: See [TOOLS_OVERVIEW.md](./TOOLS_OVERVIEW.md) for all tools