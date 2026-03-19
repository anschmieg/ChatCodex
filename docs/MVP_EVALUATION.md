# MVP Evaluation Guide

This document explains how to evaluate ChatCodex as an MVP candidate.

---

## What Is MVP Success?

ChatCodex MVP is considered successful when:

1. **A new user can set up the system** following [MVP_README.md](./MVP_README.md) in under 30 minutes
2. **A complete workflow works end-to-end**: create run → make change → run tests → finalize
3. **Operator can manage the queue**: view, prioritize, reassign, recover runs
4. **Policy controls work**: approval gates block risky operations appropriately
5. **The architecture constraint holds**: only ChatGPT is in the loop

---

## What to Validate First

### Priority 1: Core Workflow (Must Pass)

Run through [FIRST_RUN_WORKFLOW.md](./FIRST_RUN_WORKFLOW.md):

1. Create a run
2. Read some code
3. Apply a small patch
4. Run tests
5. Finalize the run

If this fails, the MVP is not ready.

### Priority 2: Policy Controls

Test approval gates:

1. Create a patch with >5 edits
2. Verify `preview_patch_policy` shows approval required
3. Approve the action
4. Apply the patch
5. Verify patch succeeds

### Priority 3: Queue Management

Test operator tasks:

1. Create multiple runs
2. List runs with filters
3. Set priorities
4. Archive completed runs
5. Reopen and continue a run

### Priority 4: Recovery

Test intervention patterns:

1. Create a run
2. Finalize it
3. Reopen the run
4. Make more changes
5. Finalize again

---

## Known Limitations

These are expected limitations in the MVP:

| Limitation | Impact | Future |
|------------|--------|--------|
| Single workspace | One project per daemon | Multi-workspace support |
| No concurrent runs | Only one active run | Parallel execution |
| Manual approvals | Operator must approve | Auto-approve options |
| No web UI | CLI/MCP only | Dashboard |
| SQLite only | No external DB | PostgreSQL, etc. |

**The MVP is not a full product.** It's a proof of concept demonstrating the architecture.

---

## What's Not in Scope

The following are explicitly NOT part of MVP:

- ✗ Team/permission system
- ✗ Web dashboard
- ✗ GitHub/Jira integrations
- ✗ Automated scheduling
- ✗ Run templates
- ✗ Multi-daemon coordination

---

## Decision Criteria

### Ready for MVP Evaluation

- [ ] Setup completes in <30 minutes
- [ ] Core workflow (V1) works
- [ ] Policy controls (V2) work
- [ ] Recovery (V4) works
- [ ] Queue inspection (V5) works
- [ ] Metadata tracking (V6) works

### Not Ready — Blockers

- [ ] Setup fails or takes >60 minutes
- [ ] Core workflow incomplete
- [ ] Policy controls don't enforce
- [ ] No operator intervention capability
- [ ] Architecture constraint violated

---

## What Comes After M35

If the MVP is accepted, M36+ could focus on:

### Potential Future Work

1. **Web UI** — Dashboard for queue management
2. **Persistence improvements** — External database support
3. **Multi-run support** — Parallel execution
4. **Template system** — Reusable run patterns
5. **Integration** — GitHub, project management tools

### Out of Scope Forever

- **Backend LLM** — Architecture constraint is permanent
- **Autonomous execution** — Human always in the loop

---

## How to Report Findings

After evaluating, record:

1. **Setup time**: How long to first run?
2. **Workflow completion**: Which steps worked/failed?
3. **Issues found**: Bugs, confusion points, missing docs
4. **Overall verdict**: Ready / Needs work / Not viable

See [MVP_CHECKPOINT_REVIEW.md](./MVP_CHECKPOINT_REVIEW.md) for previous assessment.

---

## Quick Evaluation Checklist

For a fast evaluation, complete these steps:

- [ ] Clone and build (see [MVP_README.md](./MVP_README.md))
- [ ] Start daemon + gateway
- [ ] Connect ChatGPT via MCP
- [ ] Create a run
- [ ] Apply a patch
- [ ] Run tests
- [ ] Finalize the run
- [ ] List the queue

If all 8 steps pass, the MVP is functional.

---

**Ready to start?** Go to [MVP_README.md](./MVP_README.md) for the fastest path to first use.