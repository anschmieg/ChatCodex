# Manual Validation Walkthrough

This document provides step-by-step instructions for manually validating ChatCodex. Follow these in order to verify the product works as documented.

## Prerequisites

Complete the setup from [ONBOARDING.md](./ONBOARDING.md):
- [ ] Rust toolchain installed
- [ ] Node.js installed
- [ ] Repository cloned
- [ ] Daemon builds
- [ ] Gateway builds
- [ ] MCP client configured (ChatGPT with MCP support)

## Validation Environments

Each validation can be run against:
- **Local daemon** (recommended for initial validation)
- **Test workspace** (a fresh git repo for testing)

### Setup Test Workspace

```bash
mkdir -p ~/test-workspace
cd ~/test-workspace
git init
echo 'fn main() { println!("hello"); }' > main.rs
git add .
git commit -m "initial"
```

---

## V1: Happy-Path Task Lifecycle

**Goal:** Create a run, work through steps, finalize.

### Step 1: Verify Daemon

```bash
cd codex-rs
./target/release/deterministic-daemon --port 3100 --data-dir ./runs &
curl http://localhost:3100/healthz
```

Expected: `{"status":"ok"}`

### Step 2: Verify MCP Tools Visible

In ChatGPT:
> "List the available MCP tools."

Expected: Should list 45+ tools including `codex_prepare_run`, `refresh_run_state`, etc.

### Step 3: Create a Run

In ChatGPT:
> "Create a run to add a goodbye function to main.rs. The plan is: 1) Read main.rs, 2) Add goodbye function, 3) Verify."

Expected:
- Response includes `runId`
- Status is `prepared` or `active`
- `recommendedNextAction` is provided

### Step 4: Inspect Run State

In ChatGPT:
> "Show me the current state of that run."

Expected:
- Status is `active` (after refresh)
- `pendingSteps` contains remaining steps
- `completedSteps` may contain completed steps

### Step 5: Read Files

In ChatGPT:
> "Read main.rs."

Expected:
- File contents returned
- No errors

### Step 6: Apply a Patch

In ChatGPT:
> "Add a function `fn goodbye() { println!("goodbye"); }` to main.rs and call it from main."

Expected:
- Patch is applied
- `show_diff` shows the change
- OR `approvalRequired: true` with clear reason

If approval required:
> "Approve the patch."

Expected:
- Approval resolved
- Patch applied on retry

### Step 7: Run Tests (Optional)

In ChatGPT:
> "Run tests."

Expected:
- Tests execute (or approval required message)

### Step 8: Finalize the Run

In ChatGPT:
> "Finalize the run as completed."

Expected:
- Status is `finalized:completed`
- `finalizedAt` timestamp provided

### Step 9: Verify Final State

In ChatGPT:
> "Show me the final state of the run."

Expected:
- Status is `finalized:completed`
- `finalizedOutcome` contains summary

**V1 Checklist:**
- [ ] Daemon health check passed
- [ ] MCP tools visible in ChatGPT
- [ ] Run created successfully
- [ ] Run state retrieved
- [ ] File read successful
- [ ] Patch applied (or approval flow worked)
- [ ] Run finalized

---

## V2: Approval-Gated Execution

**Goal:** Verify policy gates and approval flow.

### Step 1: Preview Patch Policy

In ChatGPT:
> "Preview if applying a patch with 10 edits would require approval."

Expected:
- `decision: "requires_approval"` or `"proceed"`
- Clear `riskReason` if blocked

### Step 2: Apply Large Patch

In ChatGPT:
> "Create a run with a patch that modifies more than 5 files."

Expected:
- `approvalRequired: true`
- `approvalId` provided
- `policyRationale` explains why

### Step 3: Approve

In ChatGPT:
> "Approve that patch."

Expected:
- `approve_action` returns success
- Approval is resolved

### Step 4: Retry Patch

In ChatGPT:
> "Apply the patch again."

Expected:
- Patch succeeds (approval already resolved)

**V2 Checklist:**
- [ ] Preview policy works
- [ ] Large patch blocked correctly
- [ ] Approval resolves
- [ ] Retry succeeds

---

## V3: Replan Flow

**Goal:** Update plan when approach changes.

### Step 1: Create Run

In ChatGPT:
> "Create a run to add a feature. Plan: 1) Add function, 2) Add tests."

### Step 2: Replan

In ChatGPT:
> "The tests failed because we need to fix imports first. Update the plan to: 1) Fix imports, 2) Add function, 3) Add tests."

Expected:
- `plan` is updated
- `pendingSteps` reflects new plan
- `replanDelta` or similar confirmation

### Step 3: Verify State

In ChatGPT:
> "Show me the run state."

Expected:
- Plan shows updated steps

**V3 Checklist:**
- [ ] Replan updates plan
- [ ] State reflects new plan

---

## V4: Recovery Flows

**Goal:** Reopen and supersede workflows.

### Test A: Reopen

### Step 1: Create and Finalize

In ChatGPT:
> "Create a run to add a comment. Finalize it as completed."

### Step 2: Reopen

In ChatGPT:
> "Reopen that run to add more changes."

Expected:
- Status changes from `finalized:completed` to `active`
- `reopenMetadata` present

### Step 3: Continue Work

In ChatGPT:
> "Add another comment. Finalize again."

**V4A Checklist:**
- [ ] Finalize works
- [ ] Reopen works
- [ ] Re-finalize works

### Test B: Supersede

### Step 1: Create and Finalize

In ChatGPT:
> "Create a run. Finalize it."

### Step 2: Supersede

In ChatGPT:
> "That approach was wrong. Supersede it with a new run for a different implementation."

Expected:
- New run created
- Original run has `supersededByRunId`
- New run has `supersedesRunId`

**V4B Checklist:**
- [ ] Supersede creates new run
- [ ] Lineage is tracked

---

## V5: Queue Inspection

**Goal:** List, filter, and overview.

### Step 1: Create Multiple Runs

In ChatGPT:
> "Create three runs: one for feature A, one for feature B, one for bug fix C."

### Step 2: List All Runs

In ChatGPT:
> "Show me all active runs."

Expected:
- List of runs returned
- Each has `runId`, `userGoal`, `status`

### Step 3: Filter by Status

In ChatGPT:
> "Show me only finalized runs."

Expected:
- Only finalized runs returned

### Step 4: Get Queue Overview

In ChatGPT:
> "Give me a queue overview."

Expected:
- Aggregate counts returned
- `totalVisible`, `readyCount`, `blockedCount`, etc.

**V5 Checklist:**
- [ ] List runs works
- [ ] Status filtering works
- [ ] Queue overview returns counts

---

## V6: Metadata Visibility

**Goal:** Set and retrieve metadata.

### Step 1: Set Priority

In ChatGPT:
> "Set the priority of run X to urgent."

Expected:
- Success confirmation

### Step 2: Assign Owner

In ChatGPT:
> "Assign run X to Alice."

Expected:
- Success confirmation

### Step 3: Set Due Date

In ChatGPT:
> "Set run X's due date to 2026-04-01."

Expected:
- Success confirmation

### Step 4: Verify in State

In ChatGPT:
> "Show me the full state of run X."

Expected:
- `priority`, `assignee`, `dueDate` visible

**V6 Checklist:**
- [ ] Set priority works
- [ ] Assign owner works
- [ ] Set due date works
- [ ] Metadata visible in state

---

## V7: Saved Views (Optional for MVP)

**Goal:** Create and use saved views.

### Step 1: Create View

In ChatGPT:
> "Create a saved view called 'urgent-active' for urgent runs that are active."

Expected:
- View created with `viewId`

### Step 2: List Views

In ChatGPT:
> "Show me my saved views."

Expected:
- View appears in list

### Step 3: Get View

In ChatGPT:
> "Show me the definition of the 'urgent-active' view."

Expected:
- Filter configuration returned

**V7 Checklist:**
- [ ] Create view works
- [ ] List views works
- [ ] Get view works

---

## Smoke Test Summary

### Critical Path (Must Pass)

| Validation | Status |
|------------|--------|
| V1: Happy path | [ ] |
| V2: Approval gates | [ ] |
| V3: Replan | [ ] |
| V4: Recovery | [ ] |
| V5: Queue inspection | [ ] |
| V6: Metadata | [ ] |

### Optional (Nice to Have)

| Validation | Status |
|------------|--------|
| V7: Saved views | [ ] |

---

## Recording Results

After completing validation, record findings in `docs/MVP_CHECKPOINT_REVIEW.md`:

### What Worked

- List workflows that passed without issues
- Note any particularly smooth interactions

### What Needs Fixing

- List workflows that had problems
- Document specific bugs or gaps
- Note any documentation inaccuracies

### Open Questions

- Any workflows that feel fragile?
- Any missing error messages?
- Any confusing behavior?

---

## Troubleshooting

### "MCP tools not visible"

1. Verify daemon is running: `curl http://localhost:3100/healthz`
2. Verify gateway started: Check logs for errors
3. Verify MCP configuration: Check path is absolute
4. Restart ChatGPT session

### "Run creation fails"

1. Check daemon logs for errors
2. Verify workspace path is correct
3. Try with minimal parameters

### "Patch fails silently"

1. Check if approval is required
2. Preview policy first
3. Check daemon logs

### "State seems inconsistent"

1. Try `refresh_run_state`
2. Check `get_run_state` for authoritative state
3. Check daemon logs for errors

---

## Next Steps

After completing manual validation:

1. Update `docs/MVP_CHECKPOINT_REVIEW.md` with findings
2. File issues for any bugs discovered
3. Mark completed validations in `docs/VALIDATION_PLAN.md`
4. Decide MVP readiness

**Quick start**: See [MVP_README.md](./MVP_README.md) for the fastest path to first use
5. See [MVP_README.md](./MVP_README.md) for the fastest path to first use