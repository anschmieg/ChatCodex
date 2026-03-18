# ChatCodex MVP Checkpoint Review

**Generated:** 2026-03-18
**After Milestone:** M29 (Saved Queue Views)

---

## 1. Executive Verdict

**ChatCodex is NOT yet a functional MVP.** The system has comprehensive capability surface area, but critical gaps prevent real-world use:

1. **No documented onboarding path** — there is no guide for a new user or ChatGPT to start using the system
2. **No end-to-end proof** — we have not demonstrated a complete workflow from goal to working code
3. **No integration tests** — individual handlers work, but there are no scenario tests validating real workflows
4. **No documentation for the target audience** — the docs explain architecture, not usage

**The gap is not feature count.** The gap is **glue, validation, and documentation**.

---

## 2. What the Product Is

ChatCodex is a **deterministic coding harness control plane** that lets ChatGPT operate on a codebase without any backend LLM.

### Core Value Proposition

> ChatGPT creates, manages, and executes coding tasks with structured state, policy gates, and audit trails — but ChatGPT retains full control. The backend is purely deterministic.

### Architecture (Working)

```
ChatGPT-hosted model
  → MCP server (TypeScript, thin gateway)
    → JSON-RPC
      → Rust daemon (deterministic, stateful)
        → filesystem / git / patch / tests / approvals
```

### Key Constraint (Preserved)

> The only LLM in the stack is ChatGPT.

No backend model calls, no hidden agent loops, no autonomous continuation. This constraint has been preserved through 30 milestones.

---

## 3. What Is Already Working

### ✅ Run Lifecycle (Solid)

| Capability | Status | Notes |
|------------|--------|-------|
| Prepare run | ✅ Working | Goal, plan, focus paths, policy |
| Refresh state | ✅ Working | Read-only snapshot with recommendations |
| Replan | ✅ Working | Deterministic plan updates |
| Finalize | ✅ Working | Outcome: completed/failed/abandoned |
| Reopen | ✅ Working | Continue finalized runs |
| Supersede | ✅ Working | Create successor run with lineage |
| Archive/Unarchive | ✅ Working | Organizational controls |

### ✅ Execution Flow (Solid)

| Capability | Status | Notes |
|------------|--------|-------|
| Read files | ✅ Working | Line ranges supported |
| Search code | ✅ Working | Text/symbol search |
| Apply patch | ✅ Working | Policy-gated |
| Run tests | ✅ Working | Policy-gated |
| Show diff | ✅ Working | Git diff summary |
| Git status | ✅ Working | Working tree status |

### ✅ Policy System (Solid)

| Capability | Status | Notes |
|------------|--------|-------|
| Patch policy | ✅ Working | Deletion, large edits, sensitive paths |
| Test policy | ✅ Working | Safe make targets |
| Approval workflow | ✅ Working | Create, resolve, retryable actions |
| Preflight preview | ✅ Working | Read-only policy evaluation |
| Per-run policy | ✅ Working | Customizable thresholds |

### ✅ Queue Management (Comprehensive)

| Capability | Status | Notes |
|------------|--------|-------|
| List runs | ✅ Working | Extensive filtering |
| Queue overview | ✅ Working | Aggregate counts |
| Saved views | ✅ Working | CRUD for reusable filters |
| Priority | ✅ Working | low/normal/high/urgent |
| Ownership | ✅ Working | Assignee + note |
| Due dates | ✅ Working | Deadline metadata |
| Dependencies | ✅ Working | blocked_by_run_ids |
| Effort | ✅ Working | tiny/small/medium/large/xlarge |
| Staleness | ✅ Working | Age-based freshness |
| Triage | ✅ Working | ready/blocked/deferred |
| Pin/Snooze | ✅ Working | Visibility controls |
| Annotate | ✅ Working | Labels + notes |
| Archive | ✅ Working | Organizational controls |

### ✅ Inspection & Audit (Solid)

| Capability | Status | Notes |
|------------|--------|-------|
| Get run state | ✅ Working | Full authoritative state |
| Run history | ✅ Working | Audit trail |
| Workspace summary | ✅ Working | Detected tooling |

### ✅ Implementation Quality (Good)

| Aspect | Status | Notes |
|--------|--------|-------|
| SQLite persistence | ✅ Working | Migration-safe |
| TypeScript thinness | ✅ Maintained | Validation + mapping only |
| No-hidden-agent invariants | ✅ Preserved | CI-enforced |
| Rust build/test/clippy | ✅ Passing | Milestone-scoped |

---

## 4. What Is Not Yet Proven

### ❌ End-to-End Workflow

**We have not validated that ChatGPT can actually use this system to complete real work.**

Missing:
- No integration tests that simulate ChatGPT workflows
- No "happy path" scenario demonstrating: prepare → read → patch → test → finalize
- No proof that the MCP tools compose correctly in ChatGPT's hands

### ❌ Onboarding Documentation

**There is no guide for a new user or ChatGPT to start.**

Missing:
- No "Getting Started" for ChatGPT MCP usage
- No example prompts for common workflows
- No explanation of when to use which tools
- No description of the expected ChatGPT behavior

### ❌ Operator Guidance

**A human operator doesn't know what to expect from ChatGPT.**

Missing:
- What should ChatGPT do after `prepare_run`?
- How should ChatGPT decide to call `replan_run`?
- When should ChatGPT use `finalize_run` vs `supersede_run`?
- What is the expected interaction pattern?

### ❌ Scenario Tests

**Unit tests pass, but workflows are untested.**

Missing:
- No tests that verify: policy gates correctly block → approval resolves → execution resumes
- No tests that verify: create run → snooze → unsnooze → complete → archive
- No tests that verify: create run → get blocked → supersede → complete successor

---

## 5. MVP Gap List

### Critical (Must Fix for MVP)

1. **Onboarding Guide for ChatGPT**
   - How to configure the MCP server
   - What to say to ChatGPT to start a run
   - Example prompts for common tasks
   - Expected tool call sequence

2. **End-to-End Validation**
   - At least one integration test that simulates a complete workflow
   - Manual test: start ChatGPT with MCP, complete a real coding task

3. **Error Recovery Documentation**
   - What happens when tests fail?
   - What happens when a patch is rejected?
   - What should ChatGPT do when stuck?

### Important (Significant for MVP)

4. **Queue Workflow Guide**
   - How to manage multiple concurrent runs
   - When to prioritize/snooze/archive
   - How to handle blocked runs

5. **Policy Configuration Guide**
   - How to tune thresholds
   - How to add custom safe targets
   - How to set focus paths

6. **State Inspection Patterns**
   - How to interpret `recommendedNextAction`
   - How to read the audit trail
   - How to understand policy rationale

### Deferrable (Nice to Have)

7. **Performance Metrics**
   - How long does a typical run cycle take?
   - What is the daemon memory footprint?

8. **Advanced Patterns**
   - Worktree isolation
   - Multi-workspace scenarios
   - Long-running run management

---

## 6. Feature Triage

### Core / Must-Have for MVP

| Milestone | Feature | MVP Status |
|------------|---------|------------|
| M0 | Bootstrap | ✅ Essential |
| M1-M3 | Daemon + MCP + Loop | ✅ Essential |
| M4 | Statefulness | ✅ Essential |
| M5 | Approval Policy | ✅ Essential |
| M6 | Retryable Actions | ✅ Essential |
| M7 | History + Audit | ✅ Essential |
| M8 | Per-Run Policy | ✅ Essential |
| M9 | Preflight | ✅ Essential |
| M10 | Finalize | ✅ Essential |
| M11 | Reopen | ✅ Essential |
| M12 | Supersede | ✅ Essential |

### Queue Organization (Useful, Could Defer)

| Milestone | Feature | MVP Status |
|------------|---------|------------|
| M13 | Archive | 🟡 Useful |
| M14 | Unarchive | 🟡 Useful |
| M15 | Annotate | 🟡 Useful |
| M16 | Pin | 🟡 Useful |
| M17 | Snooze | 🟡 Useful |
| M18 | Priority | 🟡 Useful |
| M19 | Ownership | 🟡 Useful |
| M20 | Due Dates | 🟡 Useful |
| M21 | Dependencies | 🟡 Useful |
| M23 | Blocker Filters | 🟡 Useful |
| M24 | Queue Overview | 🟡 Useful |
| M25 | Effort | 🟡 Useful |
| M26 | Staleness | 🟡 Useful |
| M27 | Triage | 🟡 Useful |
| M28 | Overview Tool | 🟡 Useful |
| M29 | Saved Views | 🟡 Useful |

**Assessment:** These queue features are well-implemented but represent organizational polish. A minimal MVP could ship without them. They do not block real work; they make managing multiple runs easier.

### Not Yet Implemented

| Milestone | Feature | MVP Status |
|------------|---------|------------|
| M22 | Readiness Views | ❓ Not found — may be merged into M23 |
| M30+ | Future | 📋 Deferred |

---

## 7. Recommended Next Steps

### Immediate (Next Sprint)

1. **Create ChatGPT Onboarding Guide** (`docs/CHATGPT_ONBOARDING.md`)
   - MCP server setup instructions
   - Example ChatGPT conversation showing a complete workflow
   - Expected tool call sequence
   - Common patterns and gotchas

2. **Write End-to-End Integration Test**
   - Test that simulates: prepare → read → patch → test → finalize
   - Test that simulates: prepare → policy block → approve → resume
   - Test that simulates: prepare → finalize → archive → list

3. **Manual Validation Walkthrough**
   - Start a real MCP server
   - Use a real ChatGPT instance to complete a coding task
   - Document any friction or confusion

### Short-Term (2-4 Weeks)

4. **Create Operator Guide** (`docs/OPERATOR_GUIDE.md`)
   - What to expect from ChatGPT
   - How to monitor runs
   - How to interpret policy decisions
   - How to intervene when stuck

5. **Add Scenario Tests**
   - Blocked run → approve → resume
   - Failed test → replan → retry
   - Stale run → snooze → revisit
   - Multiple concurrent runs

6. **Error Message Review**
   - Ensure all error messages are actionable
   - Ensure ChatGPT can understand what went wrong

### Medium-Term (After MVP)

7. **Performance Profiling**
   - Benchmark daemon response times
   - Memory usage under load
   - SQLite query optimization

8. **Advanced Documentation**
   - Policy tuning guide
   - Focus paths best practices
   - Multi-run orchestration patterns

---

## 8. Open Questions

1. **What is the primary use case?**
   - Single-run focused coding tasks?
   - Queue management for multiple tasks?
   - Long-running projects with checkpoints?

2. **Who is the target user?**
   - Developer using ChatGPT as coding assistant?
   - Team lead managing multiple tasks?
   - CI/CD integration for automated workflows?

3. **What level of ChatGPT autonomy is expected?**
   - Should ChatGPT drive the entire loop?
   - Should the human approve every step?
   - Is there a middle ground?

4. **What is "done" for MVP?**
   - One successful coding task completion?
   - All core workflows validated?
   - Documentation complete?

---

## 9. Suggested Roadmap Adjustment

### Current Roadmap (Post-M29)

```
M30: MVP Readiness Review ← YOU ARE HERE
M31+: Future Features
```

### Recommended Adjustment

```
M30: Onboarding Documentation
     - ChatGPT MCP setup guide
     - Example prompts and workflows
     - Expected tool sequences

M31: End-to-End Validation
     - Integration tests for core workflows
     - Manual validation walkthrough
     - Error scenario coverage

M32: Operator Guidance
     - Operator guide
     - Monitoring patterns
     - Intervention procedures

M33: MVP Release Candidate
     - Freeze features
     - Documentation complete
     - All workflows validated

M34+: Queue Polish (Previously M13-M29)
     - These features are useful but non-essential
     - Can be added incrementally post-MVP
```

---

## 10. Conclusion

**ChatCodex has built an impressive deterministic control plane with comprehensive capability coverage. The architecture is sound, the constraints are preserved, and the implementation quality is good.**

**However, capability count ≠ product readiness.**

The critical gaps are:

1. **No onboarding path** — a new user cannot start using the system
2. **No end-to-end proof** — we have not shown that real work can be completed
3. **No documentation for usage** — only architecture docs exist

**These are documentation and validation gaps, not feature gaps.**

The recommended next milestone should focus entirely on **making the existing system usable** rather than adding more capabilities.

---

**Verdict:** The system is **feature-complete for a minimal MVP**, but **not yet usable** due to missing onboarding, validation, and documentation.