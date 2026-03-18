# First Run Workflow Guide

This guide walks through a complete workflow for using ChatCodex. It assumes you have completed the [onboarding setup](./ONBOARDING.md) and have ChatGPT connected to your MCP server.

## Workflow Overview

```
┌──────────────┐
│ 1. Prepare   │ ─── Create run with goal + plan
└──────┬───────┘
       ▼
┌──────────────┐
│ 2. Inspect   │ ─── Check state, understand context
└──────┬───────┘
       ▼
┌──────────────┐
│ 3. Execute   │ ─── Read/search → Patch → Test
└──────┬───────┘
       ▼
┌──────────────┐
│ 4. Iterate   │ ─── Refresh, replan if needed
└──────┬───────┘
       ▼
┌──────────────┐
│ 5. Close     │ ─── Finalize or supersede
└──────────────┘
```

## Step 1: Prepare a Run

### Goal

Create a new run with a clear user goal and initial plan.

### Tools Used

- `get_workspace_summary` — Understand the project
- `codex_prepare_run` — Create the run

### Example Conversation

**You (to ChatGPT):**
> I want to add a health check endpoint to this API. The endpoint should return `{ "status": "ok" }` at GET /health.

**ChatGPT (uses MCP tools):**

```
First, I'll check the workspace structure:
→ get_workspace_summary()

Now I'll create a run:
→ codex_prepare_run({
    userGoal: "Add health check endpoint at GET /health returning {status: ok}",
    plan: [
      "Read the existing API route structure",
      "Find where routes are defined",
      "Add new health route",
      "Test the endpoint"
    ],
    focusPaths: ["/src/routes/", "/src/main.rs"],
    policy: {
      patchEditThreshold: 10
    }
  })
```

### What Happens

1. A new run is created with a unique ID
2. The run starts in `prepared` status
3. The plan is stored with `pendingSteps`
4. Focus paths constrain where patches can be applied
5. Policy settings control approval thresholds

### Output to Note

```
{
  "runId": "run_abc123",
  "status": "prepared",
  "recommendedNextAction": "Read the existing API route structure",
  "recommendedTool": "read_file"
}
```

Save the `runId` — you'll use it for all subsequent operations.

## Step 2: Inspect Run State

### Goal

Understand the current run context before making changes.

### Tools Used

- `refresh_run_state` — Get current snapshot
- `get_run_state` — Full authoritative state
- `read_file` — Read specific files
- `search_code` — Find relevant code

### Example Conversation

**You (to ChatGPT):**
> Show me the current run state and find where routes are defined.

**ChatGPT:**
```
→ refresh_run_state({ runId: "run_abc123" })
→ search_code({ query: "route", path: "/src" })
→ read_file({ path: "/src/main.rs" })
```

### What to Look For

- `status`: Should be `active` after first refresh
- `recommendedNextAction`: What ChatGPT should do next
- `pendingSteps`: Remaining plan items
- `lastObservation`: What happened in the last action
- `focusPaths`: Where patches can be applied

## Step 3: Execute Work

### Goal

Make progress through the plan by reading, patching, and testing.

### Tools Used

- `read_file` — Read files to understand structure
- `search_code` — Find patterns and references
- `apply_patch` — Apply changes (may require approval)
- `run_tests` — Execute tests (may require approval)
- `show_diff` — See what changed
- `git_status` — Check working tree

### Patch Application Flow

```
1. Read file → Understand current code
2. Prepare patch → apply_patch()
3. If policy blocks:
   ├─ Preview: preview_patch_policy()
   ├─ Review: approve_action({ decision: "approve" })
   └─ Retry: apply_patch() succeeds
4. Verify: show_diff()
```

### Example Patch Application

**ChatGPT:**
```
I'll add the health check route:

→ apply_patch({
    runId: "run_abc123",
    edits: [{
      path: "/src/routes/mod.rs",
      operation: "insert",
      lineNumber: 15,
      newText: "pub mod health;"
    }]
  })
```

### Policy Gates

Patches may be blocked for approval if:
- Deleting files
- More than 5 edits at once
- Editing sensitive paths (`.env`, `.git/`, etc.)
- Editing outside focus paths

### Handling Approval Required

If `apply_patch` returns `approvalRequired: true`:

```
{
  "approvalRequired": true,
  "approvalId": "apr_xyz789",
  "policyRationale": "Patch contains 8 edits, exceeds threshold of 5",
  "retryableAction": { ... }
}
```

**You (to ChatGPT):**
> Approve this patch.

**ChatGPT:**
```
→ approve_action({
    approvalId: "apr_xyz789",
    decision: "approve"
  })

The patch was approved. Now I'll apply it:
→ apply_patch({ runId: "run_abc123", ... })
```

### Preview Mode

Before applying, check if approval will be needed:

```
→ preview_patch_policy({
    runId: "run_abc123",
    edits: [ ... ]
  })

Response:
{
  "decision": "requires_approval",
  "riskReason": "8 edits exceeds threshold"
}
```

## Step 4: Iterate

### Goal

React to results, replan if needed, and continue.

### Tools Used

- `refresh_run_state` — Get latest state
- `replan_run` — Update the plan
- `run_tests` — Validate changes

### When to Replan

- Tests fail unexpectedly
- Discovery reveals different approach needed
- Plan was incomplete

### Example Replan

**ChatGPT:**
```
The tests failed because the health module wasn't imported.

→ replan_run({
    runId: "run_abc123",
    reason: "Tests failed - need to add module import",
    failureContext: "cargo test failed: module health not found",
    updatedSteps: [
      "Add health module import to main.rs",
      "Retry tests"
    ]
  })
```

### Test Execution

```
→ run_tests({
    runId: "run_abc123",
    scope: "make",
    target: "test"
  })
```

Tests may be blocked if `target` is not a standard safe target (`test`, `check`, `lint`, `build`, etc.).

## Step 5: Close the Run

### Goal

Finalize with a clear outcome.

### Tools Used

- `finalize_run` — Close as completed/failed/abandoned
- `get_run_state` — Verify final state

### Completion

**You (to ChatGPT):**
> The health check is working. Finalize this run as completed.

**ChatGPT:**
```
→ finalize_run({
    runId: "run_abc123",
    outcomeKind: "completed",
    summary: "Added GET /health endpoint returning {status: ok}"
  })

Response:
{
  "status": "finalized:completed",
  "finalizedAt": "2026-03-18T12:34:56Z"
}
```

### Failure

If the task couldn't be completed:

```
→ finalize_run({
    runId: "run_abc123",
    outcomeKind: "failed",
    summary: "Could not add health check",
    reason: "Upstream API incompatibility"
  })
```

### Abandonment

If you decide to stop working on it:

```
→ finalize_run({
    runId: "run_abc123",
    outcomeKind: "abandoned",
    summary: "No longer needed"
  })
```

## Managing Multiple Runs

### Queue Overview

```
→ get_run_queue_overview({
    today: "2026-03-18"
  })

Response:
{
  "totalVisible": 5,
  "readyCount": 2,
  "blockedCount": 1,
  "overdueCount": 1
}
```

### List Runs

```
→ list_runs({
    status: "active",
    limit: 20
  })
```

### Prioritize

```
→ set_run_priority({
    runId: "run_xyz",
    priority: "urgent"
  })
```

### Set Due Date

```
→ set_run_due_date({
    runId: "run_xyz",
    dueDate: "2026-03-20"
  })
```

### Snooze (Defer)

```
→ snooze_run({
    runId: "run_xyz",
    untilDate: "2026-03-25",
    reason: "Waiting for upstream fix"
  })
```

## Recovery Workflows

### Reopen a Finalized Run

```
→ reopen_run({
    runId: "run_abc123",
    reason: "Bug found in production"
  })
```

Status goes from `finalized:completed` back to `active`.

### Supersede with New Run

If the original approach was wrong:

```
→ supersede_run({
    runId: "run_abc123",
    newUserGoal: "Add health check with database connectivity check",
    reason: "Original implementation incomplete"
  })
```

This:
1. Creates a new run in `prepared` status
2. Links the runs via lineage fields
3. Preserves the old run as history

### Unsnooze

```
→ unsnooze_run({ runId: "run_xyz" })
```

### Unarchive

```
→ unarchive_run({ runId: "run_xyz" })
```

## Saved Views

Save commonly-used queue filters:

```
→ create_queue_view({
    name: "urgent-ready",
    description: "Urgent runs ready to work on",
    filters: {
      priorityFilter: "urgent",
      blockedOnly: false,
      snoozedOnly: false
    },
    sort: { sortByPriority: true }
  })
```

Apply a saved view by filtering `list_runs` with the same criteria, or retrieve the view definition:

```
→ get_queue_view({ viewId: "qv_abc" })
```

## Common Patterns

### Pattern: Safe Exploration

Before making changes, use preview mode:

```
1. preview_patch_policy({ ... }) → "proceed" or "requires_approval"
2. If requires_approval, decide to:
   - approve_action() → proceed
   - Or revise the patch
```

### Pattern: Test-First

Run tests before patching:

```
1. run_tests({ scope: "make", target: "test" })
2. Note passing/failing tests
3. Apply patch
4. run_tests() again
5. Compare results
```

### Pattern: Incremental Changes

Small patches, frequent tests:

```
1. apply_patch() with 1-2 edits
2. run_tests()
3. Repeat
```

Less likely to trigger policy gates, easier to debug.

### Pattern: Blocked Queue Review

Check what's blocking work:

```
→ list_runs({ blockedOnly: true })
→ get_run_state({ runId: "blocked_run" })
// Check blockedByRunIds
→ get_run_state({ runId: "blocking_run" })
```

## Checklist: First Successful Run

- [ ] Created run with `codex_prepare_run`
- [ ] Inspected state with `refresh_run_state`
- [ ] Read files with `read_file` and `search_code`
- [ ] Applied patch with `apply_patch`
- [ ] Ran tests with `run_tests`
- [ ] Checked changes with `show_diff`
- [ ] Finalized run with `finalize_run`
- [ ] Verified final state with `get_run_state`

## Next Steps

- **Example prompts**: See [EXAMPLE_PROMPTS.md](./EXAMPLE_PROMPTS.md)
- **API reference**: See [MCP_TOOL_CONTRACTS.md](./MCP_TOOL_CONTRACTS.md)
- **Troubleshooting**: See [ONBOARDING.md](./ONBOARDING.md)