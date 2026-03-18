# Example Prompts for ChatGPT

This document provides concrete prompts you can use with ChatGPT when it's connected to ChatCodex via MCP. Each prompt demonstrates a workflow pattern.

## Quick Reference

| Task | Tool(s) |
|------|---------|
| Start new work | `codex_prepare_run` |
| Check progress | `refresh_run_state` |
| Read code | `read_file`, `search_code` |
| Make changes | `apply_patch` |
| Run tests | `run_tests` |
| See changes | `show_diff`, `git_status` |
| Check policy | `preview_patch_policy`, `preview_test_policy` |
| Approve actions | `approve_action` |
| Update plan | `replan_run` |
| Finish work | `finalize_run` |
| Resume work | `reopen_run` |
| Replace approach | `supersede_run` |
| Manage queue | `list_runs`, `get_run_queue_overview` |

---

## Starting Work

### Prompt: Create a new run for a coding task

> Create a run to add input validation to the login form. The validation should check email format and require passwords to be at least 8 characters.

**Expected ChatGPT behavior:**
1. Call `get_workspace_summary` to understand the project
2. Call `codex_prepare_run` with:
   - `userGoal`: Clear description
   - `plan`: Step-by-step approach
   - `focusPaths`: Relevant directories
   - `policy`: Optional thresholds

### Prompt: Start with a specific plan

> Create a run to fix the database connection timeout. Plan: 1) Find where connection pooling is configured, 2) Add timeout settings, 3) Add retry logic, 4) Test connection resilience.

**Expected ChatGPT behavior:**
1. Create run with explicit plan steps
2. `pendingSteps` will contain the plan
3. ChatGPT should work through steps sequentially

### Prompt: Limit scope to specific files

> Create a run to refactor the authentication module, but only focus on files in `/src/auth/` and `/src/middleware/`.

**Expected ChatGPT behavior:**
1. Create run with `focusPaths: ["/src/auth/", "/src/middleware/"]`
2. Patches outside these paths will require approval

---

## Inspecting State

### Prompt: Check current progress

> What's the current state of my run?

**Expected ChatGPT behavior:**
1. Call `refresh_run_state` with the active runId
2. Report:
   - Current status
   - Completed/pending steps
   - Last action and observation
   - Recommended next action

### Prompt: Get full run details

> Show me all the details about run `run_abc123`.

**Expected ChatGPT behavior:**
1. Call `get_run_state` for authoritative full state
2. Report: status, plan, history, metadata, policy, etc.

### Prompt: See what changed

> What files have been modified in this run?

**Expected ChatGPT behavior:**
1. Call `show_diff` to see changes
2. Call `git_status` to see working tree

### Prompt: Check the history

> What happened in this run?

**Expected ChatGPT behavior:**
1. Call `get_run_history` for audit trail
2. Report key events: patches, tests, approvals, etc.

---

## Reading Code

### Prompt: Read a specific file

> Read the file `/src/routes/user.rs` and explain its structure.

**Expected ChatGPT behavior:**
1. Call `read_file({ path: "/src/routes/user.rs" })`
2. Summarize the content

### Prompt: Read a specific section

> Read lines 100-150 of `/src/main.rs`.

**Expected ChatGPT behavior:**
1. Call `read_file({ path: "/src/main.rs", startLine: 100, endLine: 150 })`

### Prompt: Search for code

> Find all references to `validate_password` function.

**Expected ChatGPT behavior:**
1. Call `search_code({ query: "validate_password" })`
2. Report locations and context

### Prompt: Search with path filter

> Find all `TODO` comments in the `/src/` directory.

**Expected ChatGPT behavior:**
1. Call `search_code({ query: "TODO", path: "/src" })`

---

## Making Changes

### Prompt: Apply a simple patch

> Add a health check endpoint that returns `{ "status": "ok" }` at GET /health.

**Expected ChatGPT behavior:**
1. Understand the file structure
2. Call `apply_patch` with appropriate edits
3. Verify with `show_diff`

### Prompt: Preview before applying

> I want to make a large refactoring change. First, tell me if it will require approval.

**Expected ChatGPT behavior:**
1. Call `preview_patch_policy` with the proposed edits
2. Report `decision` and `riskReason`
3. If `requires_approval`, ask for confirmation before proceeding

### Prompt: Handle approval required

> The patch was blocked for approval. Approve it.

**Expected ChatGPT behavior:**
1. Call `approve_action({ approvalId: "...", decision: "approve" })`
2. Retry the patch with `apply_patch`

### Prompt: Deny an approval

> That patch looks too risky. Deny it.

**Expected ChatGPT behavior:**
1. Call `approve_action({ approvalId: "...", decision: "deny" })`
2. Suggest alternative approach

---

## Testing

### Prompt: Run tests

> Run the tests to make sure nothing is broken.

**Expected ChatGPT behavior:**
1. Call `run_tests({ scope: "make", target: "test" })`
2. Report results

### Prompt: Check if tests need approval

> Will running `make integration-test` require approval?

**Expected ChatGPT behavior:**
1. Call `preview_test_policy({ scope: "make", target: "integration-test" })`
2. Report if approval is needed

### Prompt: Run specific test target

> Run only the auth tests.

**Expected ChatGPT behavior:**
1. Call `run_tests({ scope: "make", target: "test-auth" })`
2. Report results

---

## Updating Plans

### Prompt: Change approach mid-run

> The original plan isn't working. I need to take a different approach: 1) Rewrite the authentication module, 2) Add integration tests.

**Expected ChatGPT behavior:**
1. Call `replan_run` with:
   - `reason`: Why the change
   - `updatedSteps`: New plan
   - `failureContext`: Optional context about what failed

### Prompt: Update after test failure

> The tests failed because the database module wasn't imported. Update the plan to fix this first.

**Expected ChatGPT behavior:**
1. Call `replan_run` with `failureContext`
2. Add steps to fix the import issue
3. Stale retryable actions are invalidated

---

## Finishing Work

### Prompt: Mark as completed

> The feature is working. Mark this run as completed.

**Expected ChatGPT behavior:**
1. Call `finalize_run({ outcomeKind: "completed", summary: "..." })`
2. Confirm status is `finalized:completed`

### Prompt: Mark as failed

> I couldn't get this working. Mark the run as failed.

**Expected ChatGPT behavior:**
1. Call `finalize_run({ outcomeKind: "failed", summary: "...", reason: "..." })`

### Prompt: Abandon work

> I'm no longer interested in this task. Close it as abandoned.

**Expected ChatGPT behavior:**
1. Call `finalize_run({ outcomeKind: "abandoned", summary: "..." })`

---

## Resuming Work

### Prompt: Reopen a completed run

> I need to go back to the login validation run. Reopen it.

**Expected ChatGPT behavior:**
1. Find the run (may need `list_runs`)
2. Call `reopen_run({ runId: "...", reason: "..." })`
3. Status goes from `finalized:*` to `active`

### Prompt: Continue with a new approach

> The original approach was wrong. Start a new run to replace it with a different implementation.

**Expected ChatGPT behavior:**
1. Call `supersede_run({ runId: "...", newUserGoal: "...", reason: "..." })`
2. Creates new run in `prepared` status
3. Links runs via lineage fields

---

## Queue Management

### Prompt: See all my runs

> Show me all my active runs.

**Expected ChatGPT behavior:**
1. Call `list_runs({ status: "active" })`
2. Summarize each run

### Prompt: Check queue overview

> Give me a summary of my queue.

**Expected ChatGPT behavior:**
1. Call `get_run_queue_overview({ today: "YYYY-MM-DD" })`
2. Report: total, ready, blocked, overdue, etc.

### Prompt: Prioritize a run

> Make run `run_abc123` urgent.

**Expected ChatGPT behavior:**
1. Call `set_run_priority({ runId: "run_abc123", priority: "urgent" })`

### Prompt: Assign ownership

> Assign run `run_abc123` to Alice.

**Expected ChatGPT behavior:**
1. Call `assign_run_owner({ runId: "run_abc123", assignee: "alice", ownershipNote: "..." })`

### Prompt: Set deadline

> This run needs to be done by Friday.

**Expected ChatGPT behavior:**
1. Call `set_run_due_date({ runId: "...", dueDate: "2026-03-21" })`

### Prompt: Defer work

> I can't work on this run right now. Snooze it until next week.

**Expected ChatGPT behavior:**
1. Call `snooze_run({ runId: "...", untilDate: "...", reason: "..." })`

### Prompt: Pin important run

> Pin this run so I can find it easily.

**Expected ChatGPT behavior:**
1. Call `pin_run({ runId: "..." })`

### Prompt: Add notes

> Tag this run as "database" and add a note about the connection issue.

**Expected ChatGPT behavior:**
1. Call `annotate_run({ runId: "...", labels: ["database"], note: "..." })`

---

## Saved Views

### Prompt: Save a queue view

> Save a view called "urgent-blocked" that shows urgent runs that are currently blocked.

**Expected ChatGPT behavior:**
1. Call `create_queue_view({ name: "urgent-blocked", filters: { priorityFilter: "urgent", blockedOnly: true } })`

### Prompt: List saved views

> What saved queue views do I have?

**Expected ChatGPT behavior:**
1. Call `list_queue_views()`
2. Report view names and descriptions

### Prompt: Use a saved view

> Show me runs matching my "urgent-blocked" view.

**Expected ChatGPT behavior:**
1. Call `get_queue_view({ viewId: "..." })`
2. Call `list_runs` with the view's filters

---

## Recovery Patterns

### Prompt: Handle blocked run

> Run `run_abc123` is blocked. What's blocking it?

**Expected ChatGPT behavior:**
1. Call `get_run_state({ runId: "run_abc123" })`
2. Check `blockedByRunIds`
3. Inspect blocking runs
4. Suggest resolution

### Prompt: Find stale runs

> Which runs haven't been updated in a while?

**Expected ChatGPT behavior:**
1. Call `list_runs({ staleOnly: true })`
2. Report runs with staleness

### Prompt: Clean up archive

> Archive all completed runs older than 30 days.

**Expected ChatGPT behavior:**
1. Call `list_runs({ status: "finalized:completed" })`
2. For each, check `finalizedAt`
3. Call `archive_run` for qualifying runs

---

## Policy and Safety

### Prompt: Check patch policy

> Before I apply this large refactoring, will it require approval?

**Expected ChatGPT behavior:**
1. Call `preview_patch_policy` with the proposed edits
2. Report decision and rationale

### Prompt: Check test policy

> Can I run `make deploy` without approval?

**Expected ChatGPT behavior:**
1. Call `preview_test_policy({ scope: "make", target: "deploy" })`
2. If `requires_approval`, explain that `deploy` is not a safe target

### Prompt: Why was this blocked?

> Why did my patch require approval?

**Expected ChatGPT behavior:**
1. The `apply_patch` response includes `policyRationale`
2. Explain which policy rule was triggered

---

## Complete Workflows

### Workflow: Feature Implementation

```
1. "Create a run to add password strength meter to signup form."
2. "Check the current run state."
3. "Find where the signup form is defined."
4. "Read the password input component."
5. "Apply a patch to add the strength meter."
6. "Run tests."
7. "If tests fail, update plan and fix."
8. "Finalize as completed."
```

### Workflow: Bug Fix

```
1. "Create a run to fix the null pointer exception in user lookup."
2. "Search for user lookup code."
3. "Read the relevant function."
4. "Identify the null case."
5. "Apply a patch to add null check."
6. "Run tests."
7. "Finalize as completed."
```

### Workflow: Blocked Resolution

```
1. "Show me blocked runs."
2. "What's blocking run X?"
3. "Resolve the blocking run or wait for it to complete."
4. "Unblock run X."
```

### Workflow: Queue Review

```
1. "Show me the queue overview."
2. "Which runs are ready to work on?"
3. "Prioritize the urgent ones."
4. "Pick up the highest priority ready run."
```

---

## Tips for ChatGPT Integration

### Be Explicit About Run IDs

When you have multiple runs, explicitly mention the run ID:
> "In run `run_abc123`, read the file `/src/main.rs`."

### Use Focus Paths

When creating runs, specify focus paths to reduce approval friction:
> "Focus only on `/src/auth/` and `/src/middleware/`."

### Preview Large Changes

Before large patches, preview policy:
> "Preview if this patch will need approval."

### Keep Patches Small

Large patches trigger approvals. Prefer incremental changes:
> "Apply this as two separate smaller patches instead."

### Use Annotations

Tag and note runs for discoverability:
> "Label this run as 'backend' and 'auth' with a note about the OAuth scope issue."

---

## Next Steps

- **Setup**: See [ONBOARDING.md](./ONBOARDING.md)
- **Workflow details**: See [FIRST_RUN_WORKFLOW.md](./FIRST_RUN_WORKFLOW.md)
- **API reference**: See [MCP_TOOL_CONTRACTS.md](./MCP_TOOL_CONTRACTS.md)