# Validation Plan

This document defines the minimum workflow set that must pass for ChatCodex MVP confidence.

## Validation Philosophy

We validate that the **product experience matches the documentation**, not just that individual components work.

Each validation target includes:
- What the docs promise
- What must actually work
- How to verify

## Core Workflows

### V1: Happy-Path Task Lifecycle

**What docs promise:**
> Create a run, work through steps, finalize completed.

**What must work:**
1. `codex_prepare_run` creates a run in `prepared` or `active` status
2. `refresh_run_state` returns current state with recommendations
3. `read_file` returns file contents
4. `search_code` returns matches
5. `apply_patch` applies edits (or returns clear approval requirement)
6. `run_tests` executes tests (or returns clear approval requirement)
7. `finalize_run` transitions run to `finalized:completed`

**Verification:**
- [ ] Unit tests: `run.prepare`, `run.refresh`, `run.finalize`
- [ ] Integration test: prepare → refresh → finalize
- [ ] Manual walkthrough: Create run, make change, finalize

### V2: Approval-Gated Execution

**What docs promise:**
> Patches and tests may be blocked for approval. Preview policy, approve, retry.

**What must work:**
1. `preview_patch_policy` returns `requires_approval` or `proceed`
2. `apply_patch` with >5 edits returns `approvalRequired: true`
3. `approve_action({ decision: "approve" })` resolves approval
4. Same `apply_patch` succeeds after approval
5. `approve_action({ decision: "deny" })` invalidates retryable action

**Verification:**
- [ ] Unit tests: Policy evaluation
- [ ] Integration test: policy block → approve → retry
- [ ] Manual walkthrough: Large patch, approval flow

### V3: Replan Flow

**What docs promise:**
> Update the plan when approach changes. Stale retryable actions are invalidated.

**What must work:**
1. `replan_run` updates `plan`, `pendingSteps`, `completedSteps`
2. `replan_run` with `failureContext` invalidates retryable action
3. `replan_run` without `failureContext` preserves retryable action
4. `refresh_run_state` reflects updated plan

**Verification:**
- [ ] Unit tests: `run.replan`
- [ ] Integration test: prepare → replan → refresh
- [ ] Manual walkthrough: Create run, replan

### V4: Recovery Flows

**What docs promise:**
> Reopen finalized runs, supersede with new approach, unsnooze deferred work.

**What must work:**
1. `finalize_run` with `completed/failed/abandoned` creates finalized run
2. `reopen_run` transitions from `finalized:*` to `active`
3. `supersede_run` creates new run with lineage
4. `snooze_run` marks run as snoozed, excludes from default list
5. `unsnooze_run` restores visibility
6. `archive_run` marks run as archived
7. `unarchive_run` restores archived run

**Verification:**
- [ ] Unit tests: Each lifecycle transition
- [ ] Integration test: finalize → reopen → finalize
- [ ] Integration test: finalize → supersede → finalize
- [ ] Manual walkthrough: Full lifecycle

### V5: Queue Inspection

**What docs promise:**
> List runs with filters, get aggregate overview, manage queue.

**What must work:**
1. `list_runs` returns runs with correct filtering
2. `list_runs({ status: "active" })` returns only active runs
3. `list_runs({ archivedOnly: true })` returns only archived
4. `get_run_queue_overview` returns aggregate counts
5. `get_run_history` returns audit trail
6. Metadata (priority, assignee, due date) is visible in list/get

**Verification:**
- [ ] Unit tests: List filtering
- [ ] Integration test: Create multiple runs, list, filter
- [ ] Manual walkthrough: Create runs, inspect queue

### V6: Metadata Visibility

**What docs promise:**
> Set priority, assignee, due dates. See metadata in run state.

**What must work:**
1. `set_run_priority` updates priority field
2. `assign_run_owner` updates assignee field
3. `set_run_due_date` updates due date field
4. `get_run_state` includes all metadata
5. `list_runs` can filter by metadata

**Verification:**
- [ ] Unit tests: Metadata setters
- [ ] Integration test: Set metadata, verify in get/list
- [ ] Manual walkthrough: Set and verify

### V7: Saved Views

**What docs promise:**
> Save and reuse queue filter configurations.

**What must work:**
1. `create_queue_view` creates view with filters
2. `list_queue_views` returns views
3. `get_queue_view` returns view definition
4. `update_queue_view` modifies view
5. `delete_queue_view` removes view
6. Name uniqueness is enforced

**Verification:**
- [ ] Unit tests: View CRUD
- [ ] Integration test: Create, list, get, delete
- [ ] Manual walkthrough: Create and use saved view

---

## Test Matrix

| Workflow | Unit Tests | Integration Tests | Manual Walkthrough |
|----------|------------|-------------------|-------------------|
| V1: Happy path | ✅ Existing | ✅ Needed | ✅ Needed |
| V2: Approval gates | ✅ Existing | ⚠️ Partial | ✅ Needed |
| V3: Replan | ✅ Existing | ⚠️ Partial | ✅ Needed |
| V4: Recovery | ✅ Existing | ⚠️ Partial | ✅ Needed |
| V5: Queue inspection | ✅ Existing | ⚠️ Partial | ✅ Needed |
| V6: Metadata | ✅ Existing | ⚠️ Partial | ✅ Needed |
| V7: Saved views | ✅ Existing | ⚠️ Partial | ✅ Needed |

Legend:
- ✅ Existing: Already covered by tests
- ⚠️ Partial: Some tests exist, need more
- ✅ Needed: Manual validation required

---

## Integration Test Locations

### Rust Daemon Tests

Location: `codex-rs/deterministic-daemon/src/tests.rs` (or adjacent test modules)

Priority tests:
1. **Lifecycle integration**: prepare → refresh → finalize
2. **Approval flow**: patch blocked → approve → retry
3. **Replan flow**: prepare → replan → verify stale actions
4. **Recovery**: finalize → reopen → finalize
5. **Supersede**: finalize → supersede → verify lineage

### TypeScript Gateway Tests

Location: `apps/chatgpt-mcp/src/__tests__/` (if exists)

Priority tests:
1. **Tool-to-daemon mapping**: Verify tools call correct daemon methods
2. **Schema validation**: Verify inputs/outputs match contracts
3. **Error handling**: Verify errors are returned clearly

---

## Manual Validation Requirements

Each manual validation should be performed by an operator following the documented workflow exactly.

### Minimum Manual Validations

1. **First Run Walkthrough**
   - Follow `docs/FIRST_RUN_WORKFLOW.md`
   - Create run, inspect, execute, finalize
   - Verify each step matches documentation

2. **Approval Flow Walkthrough**
   - Attempt a large patch (>5 edits)
   - Verify approval is required
   - Approve and retry
   - Verify patch succeeds

3. **Queue Management Walkthrough**
   - Create multiple runs
   - Set priority, assignee, due dates
   - List with filters
   - Get queue overview

4. **Recovery Walkthrough**
   - Finalize a run
   - Reopen it
   - Work and finalize again
   - Try supersede flow

---

## Success Criteria

A workflow is **validated** when:
- Unit tests pass for all components
- Integration tests pass for end-to-end flow
- Manual walkthrough completes without undocumented surprises
- Documentation matches actual behavior

### MVP Readiness Threshold

MVP is ready when:
- V1 (Happy path): ✅ Validated
- V2 (Approval gates): ✅ Validated
- V3 (Replan): ✅ Validated
- V4 (Recovery): ✅ Validated
- V5 (Queue inspection): ✅ Validated
- V6 (Metadata): ✅ Validated
- V7 (Saved views): ⚠️ Optional for MVP

---

## Next Steps

1. **Quick start**: See [MVP_README.md](./MVP_README.md) for the fastest path to first use
2. **Add missing integration tests** for V1-V6
3. **Create manual walkthrough checklist** (see `MANUAL_VALIDATION_WALKTHROUGH.md`)
4. **Run manual validations** for each workflow
5. **Document findings** and update readiness assessment
6. **Fix any blocking issues** discovered during validation
6. **Quick start**: See [MVP_README.md](./MVP_README.md) for the fastest path to first use