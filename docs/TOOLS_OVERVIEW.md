# MCP Tools Overview

This document organizes the 45+ MCP tools into workflow groups for easier understanding.

## Tool Groups

### Lifecycle Tools

Control the run lifecycle from start to finish.

| Tool | Purpose | When to Use |
|------|---------|-------------|
| `codex_prepare_run` | Create a new run | Starting new work |
| `refresh_run_state` | Get current snapshot | After each action |
| `replan_run` | Update the plan | When approach changes |
| `finalize_run` | Close run with outcome | When work is done |
| `reopen_run` | Resume finalized run | Continuing previous work |
| `supersede_run` | Create successor run | Replacing failed approach |

**Typical flow:**
```
prepare → (work) → finalize
                    ↓
                reopen → (continue) → finalize
                    ↓
              supersede → (new approach) → finalize
```

### Inspection Tools

Understand the current context without making changes.

| Tool | Purpose | When to Use |
|------|---------|-------------|
| `get_run_state` | Full authoritative state | Need complete picture |
| `get_run_history` | Audit trail | Debugging, history |
| `list_runs` | Query multiple runs | Queue management |
| `get_run_queue_overview` | Aggregate counts | Quick status |
| `get_workspace_summary` | Project structure | Starting exploration |

### Code Exploration Tools

Read and search the codebase.

| Tool | Purpose | When to Use |
|------|---------|-------------|
| `read_file` | Read file contents | Understanding specific files |
| `search_code` | Find patterns | Locating code, references |
| `git_status` | Working tree status | Check changes |
| `show_diff` | See changes | Verify patches |

### Execution Tools

Make changes and run tests. These may require approval based on policy.

| Tool | Purpose | Policy Gates |
|------|---------|--------------|
| `apply_patch` | Apply file changes | Deletion, large edits, sensitive paths |
| `run_tests` | Execute tests | Non-standard make targets |

**Approval flow:**
```
apply_patch → (requires_approval?) → approve_action → apply_patch
run_tests → (requires_approval?) → approve_action → run_tests
```

### Policy Preview Tools

Check if actions will require approval before attempting.

| Tool | Purpose |
|------|---------|
| `preview_patch_policy` | Will patch need approval? |
| `preview_test_policy` | Will tests need approval? |

### Approval Tools

Resolve policy-blocked actions.

| Tool | Purpose |
|------|---------|
| `approve_action` | Approve or deny pending action |

### Queue Management Tools

Organize and prioritize multiple runs.

| Tool | Purpose |
|------|---------|
| `set_run_priority` | Set priority (low/normal/high/urgent) |
| `assign_run_owner` | Assign or clear ownership |
| `set_run_due_date` | Set or clear deadline |
| `pin_run` / `unpin_run` | Mark as important |
| `snooze_run` / `unsnooze_run` | Defer and restore |
| `archive_run` / `unarchive_run` | Organize completed work |
| `annotate_run` | Add labels and notes |

### Metadata Tools

Set run dependencies and effort estimates.

| Tool | Purpose |
|------|---------|
| `set_run_dependencies` | Set blocker dependencies |

### Saved View Tools

Save and reuse queue filter configurations.

| Tool | Purpose |
|------|---------|
| `create_queue_view` | Create a saved view |
| `update_queue_view` | Modify view configuration |
| `delete_queue_view` | Remove a view |
| `get_queue_view` | Retrieve view definition |
| `list_queue_views` | List all saved views |

---

## Workflow Patterns

### Pattern: New Feature

```
1. get_workspace_summary     # Understand project
2. codex_prepare_run         # Create run
3. read_file / search_code   # Explore
4. apply_patch               # Make changes
5. run_tests                 # Validate
6. show_diff                 # Review
7. finalize_run              # Close
```

### Pattern: Bug Fix

```
1. search_code               # Locate issue
2. read_file                 # Understand context
3. codex_prepare_run         # Create run
4. apply_patch               # Fix
5. run_tests                 # Verify
6. finalize_run              # Close
```

### Pattern: Policy Gate

```
1. preview_patch_policy      # Check before applying
2. (if requires_approval)
   ├─ approve_action         # Approve
   └─ apply_patch            # Apply
```

### Pattern: Queue Management

```
1. get_run_queue_overview    # See status
2. list_runs                 # Get details
3. set_run_priority          # Prioritize
4. assign_run_owner          // Assign
5. set_run_due_date          # Set deadline
```

### Pattern: Blocked Run

```
1. list_runs({ blockedOnly: true })   # Find blocked
2. get_run_state                       # Get details
3. (resolve blocker)
4. unsnooze_run / reopen_run           # Resume
```

---

## Policy Gates

### Patch Policy

Actions that may require approval:
- File deletion
- More than 5 edits at once
- Editing sensitive paths (`.env`, `.git/`, `id_rsa`, etc.)
- Editing outside `focusPaths`

### Test Policy

Actions that may require approval:
- Non-standard make targets
- Anything other than: `test`, `check`, `lint`, `build`, `clean`, `all`, `verify`, `fmt`, `format`

### Customizing Policy

At run creation, you can customize:
```javascript
{
  policy: {
    patchEditThreshold: 10,        // Allow more edits
    deleteRequiresApproval: false, // Allow deletion
    extraSafeMakeTargets: ["itest", "e2e"]
  }
}
```

---

## Tool Reference

### Lifecycle

| Tool | Parameters | Returns |
|------|------------|---------|
| `codex_prepare_run` | userGoal, plan, focusPaths?, policy? | runId, status, recommendedNextAction |
| `refresh_run_state` | runId | status, completedSteps, pendingSteps, recommendedNextAction |
| `replan_run` | runId, reason, updatedSteps?, failureContext? | planDelta, status |
| `finalize_run` | runId, outcomeKind, summary, reason? | status, finalizedAt |
| `reopen_run` | runId, reason | status, reopenedAt |
| `supersede_run` | runId, newUserGoal?, reason | successorRunId |

### Inspection

| Tool | Parameters | Returns |
|------|------------|---------|
| `get_run_state` | runId | full run state |
| `get_run_history` | runId, limit? | history entries |
| `list_runs` | status?, limit?, ... | run summaries |
| `get_run_queue_overview` | workspaceId?, today? | aggregate counts |
| `get_workspace_summary` | workspaceId? | detected tooling |

### Execution

| Tool | Parameters | Returns |
|------|------------|---------|
| `read_file` | path, startLine?, endLine? | file contents |
| `search_code` | query, path? | matches with context |
| `apply_patch` | runId, edits | result, approvalRequired? |
| `run_tests` | runId, scope, target | test results |
| `show_diff` | runId | diff summary |
| `git_status` | (none) | working tree status |

### Policy

| Tool | Parameters | Returns |
|------|------------|---------|
| `preview_patch_policy` | runId, edits | decision, riskReason? |
| `preview_test_policy` | runId, scope, target | decision, riskReason? |
| `approve_action` | approvalId, decision | result |

---

## Next Steps

- **Quick start**: [MVP_README.md](./MVP_README.md) for the fastest path to first use
- **Onboarding**: [ONBOARDING.md](./ONBOARDING.md)
- **Workflow guide**: [FIRST_RUN_WORKFLOW.md](./FIRST_RUN_WORKFLOW.md)
- **Example prompts**: [EXAMPLE_PROMPTS.md](./EXAMPLE_PROMPTS.md)
- **Operator guide**: [OPERATOR_GUIDE.md](./OPERATOR_GUIDE.md) for production operations
- **Intervention patterns**: [INTERVENTION_PATTERNS.md](./INTERVENTION_PATTERNS.md) for recovery playbooks
- **API contracts**: [MCP_TOOL_CONTRACTS.md](./MCP_TOOL_CONTRACTS.md)