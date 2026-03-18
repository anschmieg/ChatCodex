//! JSON-RPC handler dispatch.

use anyhow::Result;
use deterministic_protocol::methods::Method;
use deterministic_protocol::*;

use crate::persistence::Store;

/// Dispatch a JSON-RPC request to the appropriate handler.
///
/// Returns `(result_value, optional_run_state)` so the router can wrap
/// both in the response envelope.
pub fn dispatch(
    method: Method,
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    match method {
        Method::RunPrepare => handle_run_prepare(params, store),
        Method::RunRefresh => handle_run_refresh(params, store),
        Method::RunReplan => handle_run_replan(params, store),
        Method::WorkspaceSummary => handle_workspace_summary(params),
        Method::FileRead => handle_file_read(params, store),
        Method::GitStatus => handle_git_status(params, store),
        Method::CodeSearch => handle_code_search(params, store),
        Method::PatchApply => handle_patch_apply(params, store),
        Method::TestsRun => handle_tests_run(params, store),
        Method::GitDiff => handle_git_diff(params, store),
        Method::ApprovalResolve => handle_approval_resolve(params, store),
        // Milestone 7: read-only history and state inspection
        Method::RunsList => handle_runs_list(params, store),
        Method::RunGet => handle_run_get(params, store),
        Method::RunHistory => handle_run_history(params, store),
        // Milestone 9: read-only preflight evaluation
        Method::PatchPreflight => handle_patch_preflight(params, store),
        Method::TestsPreflight => handle_tests_preflight(params, store),
        // Milestone 10: deterministic run finalization
        Method::RunFinalize => handle_run_finalize(params, store),
        // Milestone 11: deterministic run reopening
        Method::RunReopen => handle_run_reopen(params, store),
        // Milestone 12: deterministic run supersession
        Method::RunSupersede => handle_run_supersede(params, store),
        // Milestone 13: deterministic run archiving
        Method::RunArchive => handle_run_archive(params, store),
        // Milestone 14: deterministic run unarchiving
        Method::RunUnarchive => handle_run_unarchive(params, store),
        // Milestone 15: deterministic run labeling / annotation
        Method::RunAnnotate => handle_run_annotate(params, store),
        // Milestone 16: deterministic run pinning
        Method::RunPin => handle_run_pin(params, store),
        Method::RunUnpin => handle_run_unpin(params, store),
        // Milestone 17: deterministic run snoozing
        Method::RunSnooze => handle_run_snooze(params, store),
        Method::RunUnsnooze => handle_run_unsnooze(params, store),
        // Milestone 18: deterministic run priority
        Method::RunSetPriority => handle_run_set_priority(params, store),
        // Milestone 19: deterministic run ownership/assignee
        Method::RunAssignOwner => handle_run_assign_owner(params, store),
        // Milestone 20: deterministic run due dates
        Method::RunSetDueDate => handle_run_set_due_date(params, store),
        // Milestone 21: deterministic run dependency links
        Method::RunSetDependencies => handle_run_set_dependencies(params, store),
        // Milestone 24: deterministic queue overview
        Method::RunsQueueOverview => handle_runs_queue_overview(params, store),
    }
}

/// Build a retryable action record when an operation is gated by approval.
fn build_retryable_action(
    kind: &str,
    summary: &str,
    payload_json: Option<String>,
    retryable_reason: &str,
    recommended_tool: &str,
) -> RetryableAction {
    RetryableAction {
        kind: kind.to_string(),
        summary: summary.to_string(),
        payload: payload_json,
        retryable_reason: retryable_reason.to_string(),
        is_valid: true,
        is_recommended: false,
        invalidation_reason: None,
        recommended_tool: recommended_tool.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn handle_run_prepare(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunPrepareParams = serde_json::from_value(params)?;
    let (result, state) = deterministic_core::run_prepare::prepare(&p)?;
    store.save_run(&state)?;
    // Audit trail: run prepared.
    let _ = store.append_audit_entry(
        &state.run_id,
        "run_prepared",
        &format!("Run prepared: {}", state.user_goal),
        None,
    );
    Ok((serde_json::to_value(result)?, Some(state)))
}

fn handle_run_refresh(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunRefreshParams = serde_json::from_value(params)?;
    let state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    let pending_approvals = store.get_pending_approvals(&p.run_id)?;

    // Try to get a live diff summary from the workspace.
    let live_diff = {
        let ws = &state.workspace_id;
        let diff_params = GitDiffParams {
            run_id: p.run_id.clone(),
            paths: vec![],
            format: Some("summary".into()),
        };
        deterministic_core::git_diff::diff(&diff_params, ws)
            .ok()
            .map(|r| r.diff_summary)
    };

    let result = deterministic_core::run_refresh::refresh(
        &p,
        &state,
        &pending_approvals,
        live_diff.as_deref(),
    )?;
    // Audit trail: refresh performed.
    let _ = store.append_audit_entry(
        &p.run_id,
        "refresh_performed",
        &format!("Refresh performed; status={}", state.status),
        None,
    );
    Ok((serde_json::to_value(result)?, Some(state)))
}

fn handle_run_replan(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunReplanParams = serde_json::from_value(params)?;
    let mut state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    let result = deterministic_core::run_replan::replan(&p, &mut state)?;
    store.save_run(&state)?;
    // Audit trail: replan performed.
    let _ = store.append_audit_entry(
        &p.run_id,
        "replan_performed",
        &format!("Replan performed: {}", p.reason),
        None,
    );
    Ok((serde_json::to_value(result)?, Some(state)))
}

fn handle_workspace_summary(
    params: serde_json::Value,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: WorkspaceSummaryParams = serde_json::from_value(params)?;
    let result = deterministic_core::workspace_summary::summary(&p)?;
    Ok((serde_json::to_value(result)?, None))
}

fn handle_file_read(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: FileReadParams = serde_json::from_value(params)?;
    let ws = store
        .workspace_for_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;
    let result = deterministic_core::file_read::read(&p, &ws)?;
    let run_state = store.get_run(&p.run_id)?;
    Ok((serde_json::to_value(result)?, run_state))
}

fn handle_git_status(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: GitStatusParams = serde_json::from_value(params)?;
    let ws = store
        .workspace_for_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;
    let result = deterministic_core::git_status::status(&p, &ws)?;
    let run_state = store.get_run(&p.run_id)?;
    Ok((serde_json::to_value(result)?, run_state))
}

fn handle_code_search(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: CodeSearchParams = serde_json::from_value(params)?;
    let ws = store
        .workspace_for_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;
    let result = deterministic_core::code_search::search(&p, &ws)?;
    let run_state = store.get_run(&p.run_id)?;
    Ok((serde_json::to_value(result)?, run_state))
}

fn handle_patch_apply(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: PatchApplyParams = serde_json::from_value(params)?;
    let ws = store
        .workspace_for_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    // Load run state for policy evaluation.
    let mut state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    // Evaluate approval policy before applying the patch.
    let decision =
        deterministic_core::approval_policy::evaluate_patch(&p, &state.policy_profile);

    match decision {
        deterministic_core::approval_policy::PolicyDecision::RequiresApproval {
            action_summary,
            risk_reason,
            policy_rationale,
        } => {
            let approval = deterministic_core::approval::create_approval(
                &mut state,
                &action_summary,
                &risk_reason,
                &policy_rationale,
            );

            // Milestone 6: record retryable action.
            let payload_json = serde_json::to_string(&p).ok();
            state.retryable_action = Some(build_retryable_action(
                "patch.apply",
                &action_summary,
                payload_json,
                &format!("Blocked by approval policy: {policy_rationale}"),
                "apply_patch",
            ));

            store.save_approval(&approval)?;
            store.save_run(&state)?;
            // Audit trail: approval created for patch.
            let _ = store.append_audit_entry(
                &p.run_id,
                "approval_created",
                &format!("Approval required for patch: {action_summary}"),
                None,
            );
            let result = PatchApplyResult {
                changed_files: vec![],
                diff_stats: String::new(),
                approval_required: Some(approval),
            };
            Ok((serde_json::to_value(result)?, Some(state)))
        }
        deterministic_core::approval_policy::PolicyDecision::Proceed => {
            // Clear retryable action on successful execution.
            state.retryable_action = None;
            store.save_run(&state)?;
            let result = deterministic_core::patch_apply::apply(&p, &ws)?;
            // Audit trail: patch applied.
            let _ = store.append_audit_entry(
                &p.run_id,
                "patch_applied",
                &format!(
                    "Patch applied: {} file(s) changed",
                    result.changed_files.len()
                ),
                None,
            );
            let run_state = store.get_run(&p.run_id)?;
            Ok((serde_json::to_value(result)?, run_state))
        }
    }
}

fn handle_tests_run(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: TestsRunParams = serde_json::from_value(params)?;
    let ws = store
        .workspace_for_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    // Load run state for policy evaluation.
    let mut state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    // Evaluate approval policy before running tests.
    let decision = deterministic_core::approval_policy::evaluate_test_run(&p, &state.policy_profile);

    match decision {
        deterministic_core::approval_policy::PolicyDecision::RequiresApproval {
            action_summary,
            risk_reason,
            policy_rationale,
        } => {
            let approval = deterministic_core::approval::create_approval(
                &mut state,
                &action_summary,
                &risk_reason,
                &policy_rationale,
            );

            // Milestone 6: record retryable action.
            let payload_json = serde_json::to_string(&p).ok();
            state.retryable_action = Some(build_retryable_action(
                "tests.run",
                &action_summary,
                payload_json,
                &format!("Blocked by approval policy: {policy_rationale}"),
                "run_tests",
            ));

            store.save_approval(&approval)?;
            store.save_run(&state)?;
            // Audit trail: approval created for tests.
            let _ = store.append_audit_entry(
                &p.run_id,
                "approval_created",
                &format!("Approval required for tests: {action_summary}"),
                None,
            );
            let result = TestsRunResult {
                resolved_command: String::new(),
                exit_code: -1,
                stdout: String::new(),
                stderr: String::new(),
                summary: format!("Approval required: {action_summary}"),
                approval_required: Some(approval),
            };
            Ok((serde_json::to_value(result)?, Some(state)))
        }
        deterministic_core::approval_policy::PolicyDecision::Proceed => {
            // Clear retryable action on successful execution.
            state.retryable_action = None;
            store.save_run(&state)?;
            let result = deterministic_core::tests_run::run(&p, &ws)?;
            // Audit trail: tests run.
            let _ = store.append_audit_entry(
                &p.run_id,
                "tests_run",
                &format!(
                    "Tests run: scope={} exit_code={}",
                    p.scope, result.exit_code
                ),
                None,
            );
            let run_state = store.get_run(&p.run_id)?;
            Ok((serde_json::to_value(result)?, run_state))
        }
    }
}

fn handle_git_diff(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: GitDiffParams = serde_json::from_value(params)?;
    let ws = store
        .workspace_for_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;
    let result = deterministic_core::git_diff::diff(&p, &ws)?;
    let run_state = store.get_run(&p.run_id)?;
    Ok((serde_json::to_value(result)?, run_state))
}

fn handle_approval_resolve(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: ApprovalResolveParams = serde_json::from_value(params)?;

    // Verify the approval exists and belongs to the specified run.
    let approval = store
        .get_approval(&p.approval_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown approval: {}", p.approval_id))?;
    if approval.run_id != p.run_id {
        return Err(anyhow::anyhow!(
            "approval {} does not belong to run {}",
            p.approval_id,
            p.run_id
        ));
    }

    let mut state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    // Resolve in SQLite first.
    store.resolve_approval(&p.approval_id, &p.decision, p.reason.as_deref())?;

    // Count remaining pending approvals (after this resolution).
    let remaining = store.get_pending_approvals(&p.run_id)?;

    let result = deterministic_core::approval::resolve(&p, &mut state, remaining.len())?;
    store.save_run(&state)?;
    // Audit trail: approval resolved.
    let _ = store.append_audit_entry(
        &p.run_id,
        "approval_resolved",
        &format!(
            "Approval {} resolved: decision={}",
            p.approval_id, p.decision
        ),
        None,
    );
    Ok((serde_json::to_value(result)?, Some(state)))
}

// ---- Milestone 7: read-only history and state inspection ----

fn handle_runs_list(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunsListParams = serde_json::from_value(params)?;
    let limit = p.limit.unwrap_or(20).min(100);
    let include_archived = p.include_archived.unwrap_or(false);
    let archived_only = p.archived_only.unwrap_or(false);
    // Milestone 15: normalize the label filter before passing to the store.
    let label_filter_owned = p
        .label
        .as_deref()
        .map(|l| l.trim().to_lowercase());
    // Milestone 16: pinned_only filter.
    let pinned_only = p.pinned_only.unwrap_or(false);
    // Milestone 17: snooze filtering.
    let include_snoozed = p.include_snoozed.unwrap_or(false);
    let snoozed_only = p.snoozed_only.unwrap_or(false);
    // Milestone 18: priority filter and sort.
    let priority_filter = p.priority_filter;
    let sort_by_priority = p.sort_by_priority.unwrap_or(false);
    let mut runs = store.list_runs(
        limit,
        p.workspace_id.as_deref(),
        p.status.as_deref(),
        include_archived,
        archived_only,
        label_filter_owned.as_deref(),
        pinned_only,
        include_snoozed,
        snoozed_only,
    )?;
    // Milestone 18: post-filter by exact priority if requested.
    if let Some(pf) = priority_filter {
        runs.retain(|r| r.priority == pf);
    }
    // Milestone 18: stable priority-descending sort (urgent → high → normal → low)
    // applied on top of the pinned-first / updated_at ordering from SQL.
    if sort_by_priority {
        runs.sort_by(|a, b| {
            b.priority.sort_key().cmp(&a.priority.sort_key())
        });
    }
    // Milestone 19: assignee filter
    if let Some(ref assignee_filter) = p.assignee {
        runs.retain(|r| r.assignee.as_deref() == Some(assignee_filter.as_str()));
    }
    // Milestone 20: due_on_or_before filter.
    if let Some(ref threshold) = p.due_on_or_before {
        runs.retain(|r| {
            r.due_date
                .as_deref()
                .map(|d| d <= threshold.as_str())
                .unwrap_or(false)
        });
    }
    // Milestone 20: sort_by_due_date — soonest first as primary key,
    // runs with no due date sort last.
    if p.sort_by_due_date.unwrap_or(false) {
        runs.sort_by(|a, b| match (&a.due_date, &b.due_date) {
            (Some(da), Some(db)) => da.cmp(db),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
    }
    // Milestone 21: blocked_only filter — keep only runs with at least one blocker.
    if p.blocked_only.unwrap_or(false) {
        runs.retain(|r| r.is_blocked.unwrap_or(false));
    }
    // Milestone 21: blocked_by_run_id filter — keep only runs blocked by a specific run ID.
    // We need the full blocked_by_run_ids list for this; fetch via get_run per candidate.
    if let Some(ref blocker_id) = p.blocked_by_run_id {
        runs.retain(|r| {
            // Only runs already known to be blocked are candidates.
            if !r.is_blocked.unwrap_or(false) {
                return false;
            }
            store
                .get_run(&r.run_id)
                .ok()
                .flatten()
                .map(|state| state.blocked_by_run_ids.contains(blocker_id))
                .unwrap_or(false)
        });
    }
    // Milestone 23: compute blocker-impact map and populate RunSummary fields.
    let blocker_map = store.get_blocker_impact_map()?;
    for run in &mut runs {
        let count = blocker_map.get(&run.run_id).copied().unwrap_or(0);
        run.is_blocking = Some(count > 0);
        run.blocking_run_count = Some(count);
        run.blocking_reason = if count > 0 {
            Some(format!("blocking {count} run(s)"))
        } else {
            None
        };
    }
    // Milestone 23: blocking_only filter — keep only runs blocking at least one other run.
    if p.blocking_only.unwrap_or(false) {
        runs.retain(|r| r.is_blocking.unwrap_or(false));
    }
    // Milestone 23: blocking_run_count_at_least filter.
    if let Some(min_count) = p.blocking_run_count_at_least {
        runs.retain(|r| r.blocking_run_count.unwrap_or(0) >= min_count);
    }
    let count = runs.len();
    let result = RunsListResult { runs, count };
    Ok((serde_json::to_value(result)?, None))
}

fn handle_run_get(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunGetParams = serde_json::from_value(params)?;
    let state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;
    let pending_approvals = store.get_pending_approvals(&p.run_id)?;

    let retryable_action = state.retryable_action.clone();
    let latest_diff_summary = state.latest_diff_summary.clone();
    let latest_test_result = state.latest_test_result.clone();
    let recommended_next_action = state.recommended_next_action.clone();
    let recommended_tool = state.recommended_tool.clone();
    let warnings = state.warnings.clone();
    let effective_policy = state.policy_profile.clone();
    let finalized_outcome = state.finalized_outcome.clone();
    let reopen_metadata = state.reopen_metadata.clone();
    let supersedes_run_id = state.supersedes_run_id.clone();
    let superseded_by_run_id = state.superseded_by_run_id.clone();
    let supersession_reason = state.supersession_reason.clone();
    let superseded_at = state.superseded_at.clone();
    let archive_metadata = state.archive_metadata.clone();
    let unarchive_metadata = state.unarchive_metadata.clone();
    let annotation = state.annotation.clone();
    let pin_metadata = state.pin_metadata.clone();
    let priority = state.priority;
    let due_date = state.due_date.clone();
    let blocked_by_run_ids = state.blocked_by_run_ids.clone();

    // Milestone 23: compute blocker-impact for this run.
    let blocker_map = store.get_blocker_impact_map()?;
    let blocking_count = blocker_map.get(&p.run_id).copied().unwrap_or(0);
    let is_blocking = Some(blocking_count > 0);
    let blocking_run_count = Some(blocking_count);
    let blocking_reason = if blocking_count > 0 {
        Some(format!("blocking {blocking_count} run(s)"))
    } else {
        None
    };

    let result = RunGetResult {
        run_state: state.clone(),
        pending_approvals,
        retryable_action,
        latest_diff_summary,
        latest_test_result,
        recommended_next_action,
        recommended_tool,
        warnings,
        effective_policy,
        finalized_outcome,
        reopen_metadata,
        supersedes_run_id,
        superseded_by_run_id,
        supersession_reason,
        superseded_at,
        archive_metadata,
        unarchive_metadata,
        annotation,
        pin_metadata,
        priority,
        due_date,
        blocked_by_run_ids,
        is_blocking,
        blocking_run_count,
        blocking_reason,
    };
    Ok((serde_json::to_value(result)?, Some(state)))
}

fn handle_run_history(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunHistoryParams = serde_json::from_value(params)?;
    // Verify the run exists.
    let _ = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;
    let limit = p.limit.unwrap_or(50).min(200);
    let entries = store.get_audit_entries(&p.run_id, limit)?;
    let count = entries.len();
    let result = RunHistoryResult {
        run_id: p.run_id,
        entries,
        count,
    };
    Ok((serde_json::to_value(result)?, None))
}

// ---- Milestone 9: deterministic preflight / preview (read-only) ----

/// Evaluate patch policy without applying any changes.
///
/// This handler is strictly read-only: it loads the run's effective policy,
/// calls the same `evaluate_patch` logic used by `patch.apply`, and returns
/// the decision.  It never modifies files, run state, approvals, or the
/// audit trail.
fn handle_patch_preflight(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: PatchPreflightParams = serde_json::from_value(params)?;

    let state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    // Reuse the same PatchApplyParams layout for evaluation.
    let apply_params = PatchApplyParams {
        run_id: p.run_id,
        edits: p.edits,
    };

    let decision =
        deterministic_core::approval_policy::evaluate_patch(&apply_params, &state.policy_profile);

    let result = map_policy_decision(decision, state.policy_profile);
    Ok((serde_json::to_value(result)?, None))
}

/// Evaluate test-run policy without executing any tests.
///
/// This handler is strictly read-only: it loads the run's effective policy,
/// calls the same `evaluate_test_run` logic used by `tests.run`, and returns
/// the decision.  It never executes commands, mutates run state, or writes
/// to the audit trail.
fn handle_tests_preflight(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: TestsPreflightParams = serde_json::from_value(params)?;

    let state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    // Reuse the same TestsRunParams layout for evaluation.
    let run_params = TestsRunParams {
        run_id: p.run_id,
        scope: p.scope,
        target: p.target,
        reason: p.reason.unwrap_or_default(),
    };

    let decision =
        deterministic_core::approval_policy::evaluate_test_run(&run_params, &state.policy_profile);

    let result = map_policy_decision(decision, state.policy_profile);
    Ok((serde_json::to_value(result)?, None))
}

/// Map a `PolicyDecision` from approval_policy into a `PreflightResult`.
fn map_policy_decision(
    decision: deterministic_core::approval_policy::PolicyDecision,
    effective_policy: deterministic_protocol::RunPolicy,
) -> PreflightResult {
    match decision {
        deterministic_core::approval_policy::PolicyDecision::Proceed => PreflightResult {
            decision: deterministic_protocol::PreflightDecision::Proceed,
            action_summary: None,
            risk_reason: None,
            policy_rationale: None,
            effective_policy,
        },
        deterministic_core::approval_policy::PolicyDecision::RequiresApproval {
            action_summary,
            risk_reason,
            policy_rationale,
        } => PreflightResult {
            decision: deterministic_protocol::PreflightDecision::RequiresApproval,
            action_summary: Some(action_summary),
            risk_reason: Some(risk_reason),
            policy_rationale: Some(policy_rationale),
            effective_policy,
        },
    }
}

// ---- Milestone 10: deterministic run finalization ----

/// Finalize a run with a structured outcome record.
///
/// Deterministic lifecycle rules (enforced in `deterministic_core::run_finalize`):
/// - `outcome_kind` must be `"completed"`, `"failed"`, or `"abandoned"`.
/// - A run that is already finalized cannot be finalized again.
/// - Finalization never triggers autonomous follow-up work.
fn handle_run_finalize(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunFinalizeParams = serde_json::from_value(params)?;
    let mut state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    let result = deterministic_core::run_finalize::finalize(&p, &mut state)?;
    store.save_run(&state)?;

    // Audit trail: run finalized.
    let _ = store.append_audit_entry(
        &p.run_id,
        "run_finalized",
        &format!(
            "Run finalized: outcome_kind={}, summary={}",
            p.outcome_kind, p.summary
        ),
        p.reason.as_deref(),
    );

    Ok((serde_json::to_value(result)?, Some(state)))
}

// ---- Milestone 11: deterministic run reopening ----

/// Reopen a previously finalized run.
///
/// Deterministic lifecycle rules (enforced in `deterministic_core::run_reopen`):
/// - Only finalized runs (`status` starts with `"finalized:"`) may be reopened.
/// - Active, prepared, or awaiting-approval runs are rejected.
/// - Reopening does not execute work; it transitions status to `"active"`.
/// - Prior audit history and plan are preserved.
/// - Reopen metadata is persisted; reopen_count increments on each reopen.
fn handle_run_reopen(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunReopenParams = serde_json::from_value(params)?;
    let mut state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    let result = deterministic_core::run_reopen::reopen(&p, &mut state)?;
    store.save_run(&state)?;

    // Audit trail: run reopened.
    let _ = store.append_audit_entry(
        &p.run_id,
        "run_reopened",
        &format!(
            "Run reopened from '{}': reason={}",
            result.reopened_from_outcome_kind, p.reason
        ),
        Some(&format!(
            "{{\"reopened_from\":\"{}\",\"reopen_count\":{}}}",
            result.reopened_from_outcome_kind, result.reopen_count
        )),
    );

    Ok((serde_json::to_value(result)?, Some(state)))
}

// ---- Milestone 12: deterministic run supersession ----

/// Supersede a finalized run by creating a new successor run.
///
/// Deterministic lifecycle rules (enforced in `deterministic_core::run_supersede`):
/// - Only finalized runs (`status` starts with `"finalized:"`) may be superseded.
/// - Active, prepared, or awaiting-approval runs are rejected.
/// - Supersession creates a new run; it does not mutate the original run back to active.
/// - Prior audit history and plan on the original run are preserved.
/// - Both the original and successor runs record lineage metadata.
fn handle_run_supersede(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunSupersedeParams = serde_json::from_value(params)?;
    let mut original_state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    let successor_run_id = deterministic_core::run_supersede::make_successor_run_id(&p.run_id);

    let (result, successor_state) =
        deterministic_core::run_supersede::supersede(&p, &mut original_state, &successor_run_id)?;

    // Persist both the updated original and the new successor.
    store.save_run(&original_state)?;
    store.save_run(&successor_state)?;

    // Audit trail: original run superseded.
    let _ = store.append_audit_entry(
        &p.run_id,
        "run_superseded",
        &format!(
            "Run superseded by '{}': reason={}",
            successor_run_id, p.reason
        ),
        Some(&format!(
            "{{\"superseded_by\":\"{}\",\"reason\":\"{}\"}}",
            successor_run_id, p.reason
        )),
    );

    // Audit trail: successor run created from supersession.
    let _ = store.append_audit_entry(
        &successor_run_id,
        "run_created_from_supersession",
        &format!(
            "Successor run created, supersedes '{}': reason={}",
            p.run_id, p.reason
        ),
        Some(&format!(
            "{{\"supersedes\":\"{}\",\"reason\":\"{}\"}}",
            p.run_id, p.reason
        )),
    );

    Ok((serde_json::to_value(result)?, Some(successor_state)))
}

// ---- Milestone 13: deterministic run archiving ----

/// Archive an explicitly finalized run.
///
/// Deterministic lifecycle rules (enforced in `deterministic_core::run_archive`):
/// - Only finalized runs (`status` starts with `"finalized:"`) may be archived.
/// - Active, prepared, or awaiting-approval runs are rejected.
/// - Already-archived runs are rejected.
/// - Archiving does not execute work or trigger autonomous follow-up.
/// - The run's plan, completed steps, and audit history are preserved.
/// - Archive metadata is appended to the run state and persisted.
fn handle_run_archive(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunArchiveParams = serde_json::from_value(params)?;
    let mut state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    let result = deterministic_core::run_archive::archive(&p, &mut state)?;

    // Persist the updated run state with archive metadata.
    store.save_run(&state)?;

    // Audit trail: run archived.
    let _ = store.append_audit_entry(
        &p.run_id,
        "run_archived",
        &format!("Run archived: reason={}", p.reason),
        Some(
            &serde_json::json!({ "reason": p.reason })
                .to_string(),
        ),
    );

    Ok((serde_json::to_value(result)?, Some(state)))
}

// ---- Milestone 14: deterministic run unarchiving ----

/// Unarchive (restore) an explicitly archived run.
///
/// Deterministic lifecycle rules (enforced in `deterministic_core::run_unarchive`):
/// - Only archived runs (with `archive_metadata` set) may be unarchived.
/// - Non-archived runs are rejected.
/// - Already-unarchived runs are rejected.
/// - Unarchiving does not execute work or trigger autonomous follow-up.
/// - The run's plan, completed steps, finalized outcome, and audit history are preserved.
/// - Original archive metadata remains intact for historical inspection.
/// - Unarchive metadata is appended to the run state and persisted.
fn handle_run_unarchive(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunUnarchiveParams = serde_json::from_value(params)?;
    let mut state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    let result = deterministic_core::run_unarchive::unarchive(&p, &mut state)?;

    // Persist the updated run state with unarchive metadata.
    store.save_run(&state)?;

    // Audit trail: run unarchived.
    let _ = store.append_audit_entry(
        &p.run_id,
        "run_unarchived",
        &format!("Run unarchived: reason={}", p.reason),
        Some(
            &serde_json::json!({ "reason": p.reason })
                .to_string(),
        ),
    );

    Ok((serde_json::to_value(result)?, Some(state)))
}

// ---- Milestone 15: deterministic run labeling / annotation ----

/// Annotate a run with organization metadata (labels and/or operator note).
///
/// Deterministic rules:
/// - Labels are normalized to lowercase, deduplicated, and sorted.
/// - Labels must consist of lowercase ASCII letters, digits, hyphens, or
///   underscores; each bounded to `LABEL_MAX_LEN` chars; at most
///   `LABEL_MAX_COUNT` labels per run.
/// - Operator note is bounded to `OPERATOR_NOTE_MAX_LEN` characters.
/// - At least one of `labels` or `operatorNote` must be provided.
/// - This operation does not execute work, replan, reopen, finalize,
///   archive, unarchive, or supersede the run.
/// - An audit entry is appended.
fn handle_run_annotate(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunAnnotateParams = serde_json::from_value(params)?;
    let mut state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    let result = deterministic_core::run_annotate::annotate(&p, &mut state)?;
    store.save_run(&state)?;

    // Audit trail: run annotated.
    let labels_json = serde_json::to_string(&result.annotation.labels).unwrap_or_default();
    let note_updated = result.annotation.operator_note.is_some() || p.operator_note.as_deref() == Some("");
    let _ = store.append_audit_entry(
        &p.run_id,
        "run_annotated",
        &format!("Run annotated: labels={labels_json}"),
        Some(
            &serde_json::json!({
                "labels": result.annotation.labels,
                "note_updated": note_updated,
            })
            .to_string(),
        ),
    );

    Ok((serde_json::to_value(result)?, Some(state)))
}

// ---- Milestone 16: deterministic run pinning ----

/// Pin a run to keep it prominent in the working set.
///
/// Deterministic rules:
/// - Any run may be pinned regardless of current status.
/// - If already pinned, the metadata is replaced (idempotent re-pin).
/// - This operation updates pin metadata only.
/// - It does not execute work, replan, reopen, finalize, archive, unarchive,
///   or supersede the run.
/// - An audit entry is appended.
fn handle_run_pin(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunPinParams = serde_json::from_value(params)?;
    let mut state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    let result = deterministic_core::run_pin::pin(&p, &mut state)?;
    store.save_run(&state)?;

    let _ = store.append_audit_entry(
        &p.run_id,
        "run_pinned",
        &format!("Run pinned: {}", result.reason),
        Some(
            &serde_json::json!({
                "reason": result.reason,
                "pinned_at": result.pinned_at,
            })
            .to_string(),
        ),
    );

    Ok((serde_json::to_value(result)?, Some(state)))
}

/// Unpin a run, removing it from the prominent working-set position.
///
/// Deterministic rules:
/// - Only pinned runs (with `pin_metadata` set) may be unpinned.
/// - This operation clears pin metadata only.
/// - It does not execute work, replan, reopen, finalize, archive, unarchive,
///   or supersede the run.
/// - An audit entry is appended.
fn handle_run_unpin(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunUnpinParams = serde_json::from_value(params)?;
    let mut state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    let result = deterministic_core::run_unpin::unpin(&p, &mut state)?;
    store.save_run(&state)?;

    let _ = store.append_audit_entry(
        &p.run_id,
        "run_unpinned",
        &format!("Run unpinned: {}", p.reason),
        Some(
            &serde_json::json!({
                "reason": p.reason,
            })
            .to_string(),
        ),
    );

    Ok((serde_json::to_value(result)?, Some(state)))
}

/// Snooze a run, deferring it out of the default visible working set.
///
/// Deterministic rules:
/// - Any run may be snoozed regardless of current status.
/// - Snoozing a run that is already snoozed replaces the snooze metadata (idempotent).
/// - This operation updates snooze metadata only.
/// - It does not execute work, replan, reopen, finalize, archive, unarchive,
///   pin, unpin, or supersede the run.
/// - An audit entry is appended.
fn handle_run_snooze(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunSnoozeParams = serde_json::from_value(params)?;
    let mut state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    let result = deterministic_core::run_snooze::snooze(&p, &mut state)?;
    store.save_run(&state)?;

    let _ = store.append_audit_entry(
        &p.run_id,
        "run_snoozed",
        &format!("Run snoozed: {}", result.reason),
        Some(
            &serde_json::json!({
                "reason": result.reason,
                "snoozed_at": result.snoozed_at,
            })
            .to_string(),
        ),
    );

    Ok((serde_json::to_value(result)?, Some(state)))
}

/// Unsnooze a run, restoring it to the normal visible working set.
///
/// Deterministic rules:
/// - Only snoozed runs (with `snooze_metadata` set) may be unsnoozed.
/// - This operation clears snooze metadata only.
/// - It does not execute work, replan, reopen, finalize, archive, unarchive,
///   pin, unpin, or supersede the run.
/// - An audit entry is appended.
fn handle_run_unsnooze(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunUnsnoozeParams = serde_json::from_value(params)?;
    let mut state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    let result = deterministic_core::run_unsnooze::unsnooze(&p, &mut state)?;
    store.save_run(&state)?;

    let _ = store.append_audit_entry(
        &p.run_id,
        "run_unsnoozed",
        &format!("Run unsnoozed: {}", p.reason),
        Some(
            &serde_json::json!({
                "reason": p.reason,
            })
            .to_string(),
        ),
    );

    Ok((serde_json::to_value(result)?, Some(state)))
}

// ---- Milestone 18: deterministic run priority ----

/// Set the explicit priority of a run.
///
/// Deterministic rules:
/// - Any run may have its priority updated regardless of current status.
/// - This operation updates priority only.
/// - It does not execute work, replan, reopen, finalize, archive, unarchive,
///   pin, unpin, snooze, or unsnooze the run.
/// - An audit entry is appended.
fn handle_run_set_priority(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunSetPriorityParams = serde_json::from_value(params)?;
    let mut state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    let result = deterministic_core::run_set_priority::set_priority(&p, &mut state)?;
    store.save_run(&state)?;

    let _ = store.append_audit_entry(
        &p.run_id,
        "run_priority_set",
        &format!(
            "Run priority set: {} → {} ({})",
            result.previous_priority.as_str(),
            result.priority.as_str(),
            result.reason
        ),
        Some(
            &serde_json::json!({
                "previous_priority": result.previous_priority.as_str(),
                "priority": result.priority.as_str(),
                "reason": result.reason,
                "set_at": result.set_at,
            })
            .to_string(),
        ),
    );

    Ok((serde_json::to_value(result)?, Some(state)))
}

fn handle_run_assign_owner(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunAssignOwnerParams = serde_json::from_value(params)?;
    let mut state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    let result = deterministic_core::run_assign_owner::assign_owner(&p, &mut state)?;
    store.save_run(&state)?;

    let _ = store.append_audit_entry(
        &p.run_id,
        "run_owner_assigned",
        &result.message,
        Some(
            &serde_json::json!({
                "previous_assignee": result.previous_assignee,
                "assignee": result.assignee,
                "ownership_note_set": result.ownership_note.is_some(),
            })
            .to_string(),
        ),
    );

    Ok((serde_json::to_value(result)?, Some(state)))
}

/// Set or clear the due date on a run.
///
/// - An audit entry is appended.
fn handle_run_set_due_date(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunSetDueDateParams = serde_json::from_value(params)?;
    let mut state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    let result = deterministic_core::run_set_due_date::set_due_date(&p, &mut state)?;
    store.save_run(&state)?;

    let _ = store.append_audit_entry(
        &p.run_id,
        "run_due_date_set",
        &result.message,
        Some(
            &serde_json::json!({
                "previous_due_date": result.previous_due_date,
                "due_date": result.due_date,
                "updated_at": result.updated_at,
            })
            .to_string(),
        ),
    );

    Ok((serde_json::to_value(result)?, Some(state)))
}

// ---- Milestone 21: deterministic run dependency links ----

fn handle_run_set_dependencies(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunSetDependenciesParams = serde_json::from_value(params)?;
    let mut state = store
        .get_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;

    // Collect all known run IDs for existence validation.
    let all_runs = store.list_runs(
        usize::MAX,
        None,
        None,
        true, // include_archived
        false,
        None,
        false,
        true, // include_snoozed
        false,
    )?;
    let known_ids: Vec<String> = all_runs.into_iter().map(|r| r.run_id).collect();

    let result = deterministic_core::run_set_dependencies::set_dependencies(&p, &mut state, &known_ids)?;
    store.save_run(&state)?;

    let _ = store.append_audit_entry(
        &p.run_id,
        "run_dependencies_set",
        &result.message,
        Some(
            &serde_json::json!({
                "previous_blocked_by_run_ids": result.previous_blocked_by_run_ids,
                "blocked_by_run_ids": result.blocked_by_run_ids,
                "updated_at": result.updated_at,
            })
            .to_string(),
        ),
    );

    Ok((serde_json::to_value(result)?, Some(state)))
}

// ---------------------------------------------------------------------------
// runs.overview (Milestone 24)

#[allow(clippy::collapsible_if)]
/// Handle the runs.overview method - deterministic queue overview.
fn handle_runs_queue_overview(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunsQueueOverviewParams = serde_json::from_value(params)?;

    let include_archived = p.include_archived.unwrap_or(false);
    let include_snoozed = p.include_snoozed.unwrap_or(false);
    let today = p.today.as_deref();

    // Use list_runs with high limit to get all runs for overview
    // This uses the existing filter semantics from RunsListParams
    let archived_only = false; // We handle filtering ourselves
    let all_runs = store.list_runs(
        10000, // High limit to get all runs
        p.workspace_id.as_deref(),
        None, // status_filter - none for overview
        include_archived,
        archived_only,
        None, // label_filter
        false, // pinned_only
        include_snoozed,
        false, // snoozed_only
    )?;

    // Use run_readiness module to derive readiness
    use deterministic_core::run_readiness::derive_readiness;

    let mut total_count = 0;
    let mut ready_count = 0;
    let mut blocked_count = 0;
    let mut attention_count = 0;
    let mut pinned_count = 0;
    let mut snoozed_count = 0;
    let mut overdue_count = 0;
    let archived_count = 0;

    let mut priority_counts = std::collections::HashMap::new();
    let mut assignee_counts = std::collections::HashMap::new();
    let mut status_counts = std::collections::HashMap::new();

    for run in &all_runs {
        let is_archived = run.is_archived.unwrap_or(false);
        let is_snoozed = run.is_snoozed.unwrap_or(false);
        let is_pinned = run.is_pinned.unwrap_or(false);
        let blocked_by_count = run.blocked_by_count.unwrap_or(0);

        // Skip based on filters (list_runs handles SQL-level, we handle in-memory)
        if is_snoozed && !include_snoozed {
            continue;
        }

        // Count snoozed
        if is_snoozed {
            snoozed_count += 1;
        }

        // Count pinned
        if is_pinned {
            pinned_count += 1;
        }

        // Count blocked
        if blocked_by_count > 0 {
            blocked_count += 1;
        }

        // Derive readiness using the deterministic function
        let readiness = derive_readiness(
            &run.status,
            is_archived,
            is_snoozed,
            blocked_by_count,
            run.priority,
            run.due_date.as_deref(),
            is_pinned,
            today,
        );

        // Count ready
        if readiness.is_ready {
            ready_count += 1;
            total_count += 1;
        } else {
            total_count += 1;
        }

        // Count needs attention
        if readiness.needs_attention {
            attention_count += 1;
        }

        // Count overdue
        if today.is_some() {
            if let Some(due) = &run.due_date {
                if let Some(t) = today {
                    if due.as_str() < t {
                        overdue_count += 1;
                    }
                }
            }
        }

        // Priority counts
        let priority_str = match run.priority {
            deterministic_protocol::RunPriority::Low => "low",
            deterministic_protocol::RunPriority::Normal => "normal",
            deterministic_protocol::RunPriority::High => "high",
            deterministic_protocol::RunPriority::Urgent => "urgent",
        };
        *priority_counts.entry(priority_str.to_string()).or_insert(0) += 1;

        // Assignee counts
        let assignee = run.assignee.clone().unwrap_or_else(|| "unassigned".to_string());
        *assignee_counts.entry(assignee).or_insert(0) += 1;

        // Status counts - use prefix
        let status_prefix = if run.status.starts_with("finalized:") {
            "finalized"
        } else if run.status.starts_with("awaiting_approval:") {
            "awaiting_approval"
        } else if run.status.starts_with("pending:") {
            "pending"
        } else {
            &run.status
        };
        *status_counts.entry(status_prefix.to_string()).or_insert(0) += 1;
    }

    let result = RunQueueOverview {
        total_visible: total_count,
        ready_count,
        blocked_count,
        needs_attention_count: attention_count,
        pinned_count,
        snoozed_count,
        overdue_count: if today.is_some() { Some(overdue_count) } else { None },
        archived_count: if include_archived { Some(archived_count) } else { None },
        by_priority: PriorityCounts {
            low: *priority_counts.get("low").unwrap_or(&0),
            normal: *priority_counts.get("normal").unwrap_or(&0),
            high: *priority_counts.get("high").unwrap_or(&0),
            urgent: *priority_counts.get("urgent").unwrap_or(&0),
        },
        by_assignee: assignee_counts,
        by_status: status_counts,
    };

    Ok((serde_json::to_value(result)?, None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::Store;
    use deterministic_protocol::{RunPolicy, RunPriority, RunSetDependenciesResult, RunsListResult};

    fn make_run_state(run_id: &str) -> RunState {
        RunState {
            run_id: run_id.into(),
            workspace_id: "/tmp/ws".into(),
            user_goal: "fix".into(),
            status: "active".into(),
            plan: vec!["step 1".into()],
            current_step: 0,
            completed_steps: vec![],
            pending_steps: vec!["step 1".into()],
            last_action: None,
            last_observation: None,
            recommended_next_action: None,
            recommended_tool: None,
            latest_diff_summary: None,
            latest_test_result: None,
            focus_paths: vec![],
            warnings: vec![],
            retryable_action: None,
            policy_profile: RunPolicy::default(),
            finalized_outcome: None,
            reopen_metadata: None,
            supersedes_run_id: None,
            superseded_by_run_id: None,
            supersession_reason: None,
            superseded_at: None,
            archive_metadata: None,
            unarchive_metadata: None,
            annotation: None,
            pin_metadata: None,
            snooze_metadata: None,
            priority: deterministic_protocol::RunPriority::Normal,
            assignee: None,
            ownership_note: None,
            due_date: None,
            blocked_by_run_ids: vec![],
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        }
    }

    // -- patch.preflight tests -----------------------------------------------

    #[test]
    fn patch_preflight_proceed() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_pf_1");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_pf_1",
            "edits": [{ "path": "src/main.rs", "operation": "replace", "newText": "fn main(){}" }]
        });
        let (val, run_state) = dispatch(Method::PatchPreflight, params, &store).unwrap();
        let result: PreflightResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.decision, PreflightDecision::Proceed);
        assert!(result.action_summary.is_none());
        assert!(result.risk_reason.is_none());
        assert!(result.policy_rationale.is_none());
        // Preflight must not attach run_state (read-only, no side effect)
        assert!(run_state.is_none());
    }

    #[test]
    fn patch_preflight_requires_approval_for_delete() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_pf_2");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_pf_2",
            "edits": [{ "path": "src/lib.rs", "operation": "delete", "newText": "" }]
        });
        let (val, run_state) = dispatch(Method::PatchPreflight, params, &store).unwrap();
        let result: PreflightResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.decision, PreflightDecision::RequiresApproval);
        assert!(result.action_summary.is_some());
        assert!(result.risk_reason.is_some());
        assert!(result.policy_rationale.is_some());
        // No state mutation
        assert!(run_state.is_none());
        // Verify the run state was NOT modified (no retryable_action set)
        let loaded = store.get_run("r_pf_2").unwrap().unwrap();
        assert!(loaded.retryable_action.is_none());
    }

    #[test]
    fn patch_preflight_requires_approval_large_patch() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_pf_3");
        store.save_run(&state).unwrap();

        // Default threshold is 5; send 6 edits.
        let edits: Vec<serde_json::Value> = (0..6)
            .map(|i| {
                serde_json::json!({
                    "path": format!("src/file{i}.rs"),
                    "operation": "replace",
                    "newText": "x"
                })
            })
            .collect();
        let params = serde_json::json!({ "runId": "r_pf_3", "edits": edits });
        let (val, _) = dispatch(Method::PatchPreflight, params, &store).unwrap();
        let result: PreflightResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.decision, PreflightDecision::RequiresApproval);
        // No state mutation
        let loaded = store.get_run("r_pf_3").unwrap().unwrap();
        assert!(loaded.retryable_action.is_none());
    }

    #[test]
    fn patch_preflight_no_state_mutation() {
        // Confirm the store still reflects original state after preflight.
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_pf_nm");
        state.status = "active".into();
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_pf_nm",
            "edits": [{ "path": "x.rs", "operation": "delete", "newText": "" }]
        });
        let _ = dispatch(Method::PatchPreflight, params, &store).unwrap();

        let loaded = store.get_run("r_pf_nm").unwrap().unwrap();
        // status unchanged
        assert_eq!(loaded.status, "active");
        // no retryable_action set
        assert!(loaded.retryable_action.is_none());
        // no approvals created
        let approvals = store.get_pending_approvals("r_pf_nm").unwrap();
        assert!(approvals.is_empty());
    }

    // -- tests.preflight tests -----------------------------------------------

    #[test]
    fn tests_preflight_proceed_cargo() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_tf_1");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_tf_1",
            "scope": "cargo",
            "reason": "check correctness"
        });
        let (val, run_state) = dispatch(Method::TestsPreflight, params, &store).unwrap();
        let result: PreflightResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.decision, PreflightDecision::Proceed);
        assert!(result.action_summary.is_none());
        assert!(run_state.is_none());
    }

    #[test]
    fn tests_preflight_requires_approval_nonstandard_make_target() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_tf_2");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_tf_2",
            "scope": "make",
            "target": "deploy-prod",
            "reason": "deploy"
        });
        let (val, run_state) = dispatch(Method::TestsPreflight, params, &store).unwrap();
        let result: PreflightResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.decision, PreflightDecision::RequiresApproval);
        assert!(result.policy_rationale.is_some());
        assert!(run_state.is_none());
        // No state mutation
        let loaded = store.get_run("r_tf_2").unwrap().unwrap();
        assert!(loaded.retryable_action.is_none());
    }

    #[test]
    fn tests_preflight_proceed_safe_make_target() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_tf_3");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_tf_3",
            "scope": "make",
            "target": "test",
            "reason": "run tests"
        });
        let (val, _) = dispatch(Method::TestsPreflight, params, &store).unwrap();
        let result: PreflightResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.decision, PreflightDecision::Proceed);
    }

    #[test]
    fn tests_preflight_no_state_mutation() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_tf_nm");
        state.status = "active".into();
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_tf_nm",
            "scope": "make",
            "target": "deploy-prod",
            "reason": "deploy"
        });
        let _ = dispatch(Method::TestsPreflight, params, &store).unwrap();

        let loaded = store.get_run("r_tf_nm").unwrap().unwrap();
        assert_eq!(loaded.status, "active");
        assert!(loaded.retryable_action.is_none());
        let approvals = store.get_pending_approvals("r_tf_nm").unwrap();
        assert!(approvals.is_empty());
    }

    // -- method registry test ------------------------------------------------

    #[test]
    fn method_registry_includes_preflight_methods() {
        use deterministic_protocol::Method;
        let all = Method::all();
        assert!(all.contains(&Method::PatchPreflight));
        assert!(all.contains(&Method::TestsPreflight));
    }

    #[test]
    fn method_registry_includes_run_finalize() {
        use deterministic_protocol::Method;
        let all = Method::all();
        assert!(all.contains(&Method::RunFinalize));
        assert_eq!(Method::RunFinalize.as_str(), "run.finalize");
        assert_eq!(Method::parse_method("run.finalize"), Some(Method::RunFinalize));
    }

    #[test]
    fn forbidden_methods_not_registered() {
        use deterministic_protocol::Method;
        let forbidden_names = [
            "turn.start",
            "turn.steer",
            "review.start",
            "codex",
            "codex.reply",
            "resume_thread",
            "continue_run",
            "agent_step",
        ];
        for name in &forbidden_names {
            assert!(
                Method::parse_method(name).is_none(),
                "Forbidden method '{name}' must not be registered"
            );
        }
    }

    // -- run.finalize handler tests ------------------------------------------

    #[test]
    fn run_finalize_completed() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_fin_c");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_fin_c",
            "outcomeKind": "completed",
            "summary": "All steps finished"
        });
        let (val, run_state_opt) = dispatch(Method::RunFinalize, params, &store).unwrap();
        let result: RunFinalizeResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.outcome_kind, "completed");
        assert_eq!(result.run_id, "r_fin_c");
        assert!(!result.finalized_at.is_empty());
        assert_eq!(result.status, "finalized:completed");
        assert!(result.recommended_next_action.contains("complete"));

        // State must be updated in the store.
        let loaded = store.get_run("r_fin_c").unwrap().unwrap();
        let outcome = loaded.finalized_outcome.as_ref().unwrap();
        assert_eq!(outcome.outcome_kind, "completed");
        assert_eq!(outcome.summary, "All steps finished");
        assert!(outcome.reason.is_none());

        // run_state must be returned.
        assert!(run_state_opt.is_some());

        // Audit trail must have a finalization entry.
        let entries = store.get_audit_entries("r_fin_c", 10).unwrap();
        assert!(entries.iter().any(|e| e.event_kind == "run_finalized"));
    }

    #[test]
    fn run_finalize_failed_with_reason() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_fin_f");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_fin_f",
            "outcomeKind": "failed",
            "summary": "Tests broke",
            "reason": "compiler error"
        });
        let (val, _) = dispatch(Method::RunFinalize, params, &store).unwrap();
        let result: RunFinalizeResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.outcome_kind, "failed");
        assert_eq!(result.status, "finalized:failed");
        assert!(result.recommended_next_action.contains("failed"));

        let loaded = store.get_run("r_fin_f").unwrap().unwrap();
        let outcome = loaded.finalized_outcome.as_ref().unwrap();
        assert_eq!(outcome.reason.as_deref(), Some("compiler error"));
    }

    #[test]
    fn run_finalize_abandoned() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_fin_a");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_fin_a",
            "outcomeKind": "abandoned",
            "summary": "No longer needed",
            "reason": "scope changed"
        });
        let (val, _) = dispatch(Method::RunFinalize, params, &store).unwrap();
        let result: RunFinalizeResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.outcome_kind, "abandoned");
        assert_eq!(result.status, "finalized:abandoned");
        assert!(result.recommended_next_action.contains("abandoned"));
    }

    #[test]
    fn run_finalize_invalid_kind_rejected() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_fin_inv");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_fin_inv",
            "outcomeKind": "unknown_kind",
            "summary": "done"
        });
        let err = dispatch(Method::RunFinalize, params, &store).unwrap_err();
        assert!(err.to_string().contains("invalid outcome_kind"));

        // State must not be mutated.
        let loaded = store.get_run("r_fin_inv").unwrap().unwrap();
        assert!(loaded.finalized_outcome.is_none());
        assert_eq!(loaded.status, "active");
    }

    #[test]
    fn run_finalize_duplicate_rejected() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_fin_dup");
        store.save_run(&state).unwrap();

        // First finalization must succeed.
        let params1 = serde_json::json!({
            "runId": "r_fin_dup",
            "outcomeKind": "completed",
            "summary": "Done"
        });
        dispatch(Method::RunFinalize, params1, &store).unwrap();

        // Second finalization must be rejected.
        let params2 = serde_json::json!({
            "runId": "r_fin_dup",
            "outcomeKind": "abandoned",
            "summary": "Trying again"
        });
        let err = dispatch(Method::RunFinalize, params2, &store).unwrap_err();
        assert!(err.to_string().contains("already finalized"));

        // Original outcome must be preserved.
        let loaded = store.get_run("r_fin_dup").unwrap().unwrap();
        let outcome = loaded.finalized_outcome.as_ref().unwrap();
        assert_eq!(outcome.outcome_kind, "completed");
    }

    #[test]
    fn run_finalize_unknown_run_rejected() {
        let store = Store::open_in_memory().unwrap();
        let params = serde_json::json!({
            "runId": "nonexistent",
            "outcomeKind": "completed",
            "summary": "done"
        });
        let err = dispatch(Method::RunFinalize, params, &store).unwrap_err();
        assert!(err.to_string().contains("unknown run"));
    }

    #[test]
    fn run_finalize_audit_trail_entry_created() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_fin_aud");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_fin_aud",
            "outcomeKind": "completed",
            "summary": "audit test"
        });
        dispatch(Method::RunFinalize, params, &store).unwrap();

        let entries = store.get_audit_entries("r_fin_aud", 10).unwrap();
        let finalized_entry = entries
            .iter()
            .find(|e| e.event_kind == "run_finalized")
            .expect("run_finalized audit entry must be present");
        assert!(finalized_entry.summary.contains("completed"));
        assert_eq!(finalized_entry.run_id, "r_fin_aud");
    }

    // -----------------------------------------------------------------------
    // Milestone 11: run.reopen handler tests
    // -----------------------------------------------------------------------

    fn finalize_run_in_store(store: &Store, run_id: &str, outcome_kind: &str) {
        let params = serde_json::json!({
            "runId": run_id,
            "outcomeKind": outcome_kind,
            "summary": format!("Finalized as {outcome_kind}")
        });
        dispatch(Method::RunFinalize, params, store).unwrap();
    }

    #[test]
    fn run_reopen_completed_succeeds() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_ro_h1");
        store.save_run(&state).unwrap();
        finalize_run_in_store(&store, "r_ro_h1", "completed");

        let params = serde_json::json!({
            "runId": "r_ro_h1",
            "reason": "Found another bug"
        });
        let (val, run_state) = dispatch(Method::RunReopen, params, &store).unwrap();
        let result: RunReopenResult = serde_json::from_value(val).unwrap();

        assert_eq!(result.run_id, "r_ro_h1");
        assert_eq!(result.status, "active");
        assert_eq!(result.reopened_from_outcome_kind, "completed");
        assert_eq!(result.reopen_count, 1);
        assert!(!result.reopened_at.is_empty());
        assert_eq!(result.recommended_tool, "refresh_run_state");

        // Updated state should be active.
        let s = run_state.unwrap();
        assert_eq!(s.status, "active");
        assert!(s.reopen_metadata.is_some());
    }

    #[test]
    fn run_reopen_failed_succeeds() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_ro_h2");
        store.save_run(&state).unwrap();
        finalize_run_in_store(&store, "r_ro_h2", "failed");

        let params = serde_json::json!({ "runId": "r_ro_h2", "reason": "New clue" });
        let (val, _) = dispatch(Method::RunReopen, params, &store).unwrap();
        let result: RunReopenResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.reopened_from_outcome_kind, "failed");
        assert_eq!(result.status, "active");
    }

    #[test]
    fn run_reopen_abandoned_succeeds() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_ro_h3");
        store.save_run(&state).unwrap();
        finalize_run_in_store(&store, "r_ro_h3", "abandoned");

        let params = serde_json::json!({ "runId": "r_ro_h3", "reason": "Goal updated" });
        let (val, _) = dispatch(Method::RunReopen, params, &store).unwrap();
        let result: RunReopenResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.reopened_from_outcome_kind, "abandoned");
        assert_eq!(result.status, "active");
    }

    #[test]
    fn run_reopen_active_run_rejected() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_ro_h4");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({ "runId": "r_ro_h4", "reason": "Should fail" });
        let err = dispatch(Method::RunReopen, params, &store).unwrap_err();
        assert!(err.to_string().contains("cannot be reopened"));

        // State must not be mutated.
        let loaded = store.get_run("r_ro_h4").unwrap().unwrap();
        assert_eq!(loaded.status, "active");
        assert!(loaded.reopen_metadata.is_none());
    }

    #[test]
    fn run_reopen_unknown_run_rejected() {
        let store = Store::open_in_memory().unwrap();
        let params = serde_json::json!({ "runId": "nonexistent", "reason": "test" });
        let err = dispatch(Method::RunReopen, params, &store).unwrap_err();
        assert!(err.to_string().contains("unknown run"));
    }

    #[test]
    fn run_reopen_audit_trail_entry_created() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_ro_aud");
        store.save_run(&state).unwrap();
        finalize_run_in_store(&store, "r_ro_aud", "failed");

        let params = serde_json::json!({ "runId": "r_ro_aud", "reason": "new evidence" });
        dispatch(Method::RunReopen, params, &store).unwrap();

        let entries = store.get_audit_entries("r_ro_aud", 10).unwrap();
        let reopen_entry = entries
            .iter()
            .find(|e| e.event_kind == "run_reopened")
            .expect("run_reopened audit entry must be present");
        assert!(reopen_entry.summary.contains("failed"));
        assert_eq!(reopen_entry.run_id, "r_ro_aud");
    }

    #[test]
    fn run_reopen_persistence_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_ro_rt");
        store.save_run(&state).unwrap();
        finalize_run_in_store(&store, "r_ro_rt", "completed");

        let params = serde_json::json!({ "runId": "r_ro_rt", "reason": "roundtrip test" });
        dispatch(Method::RunReopen, params, &store).unwrap();

        let loaded = store.get_run("r_ro_rt").unwrap().unwrap();
        let meta = loaded.reopen_metadata.as_ref().unwrap();
        assert_eq!(meta.reason, "roundtrip test");
        assert_eq!(meta.reopened_from_outcome_kind, "completed");
        assert_eq!(meta.reopen_count, 1);
        assert!(loaded.finalized_outcome.is_none());
        assert_eq!(loaded.status, "active");
    }

    #[test]
    fn run_reopen_exposes_metadata_in_run_get() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_ro_get");
        store.save_run(&state).unwrap();
        finalize_run_in_store(&store, "r_ro_get", "completed");

        let reopen_params = serde_json::json!({ "runId": "r_ro_get", "reason": "check metadata in get" });
        dispatch(Method::RunReopen, reopen_params, &store).unwrap();

        let get_params = serde_json::json!({ "runId": "r_ro_get" });
        let (val, _) = dispatch(Method::RunGet, get_params, &store).unwrap();
        let get_result: RunGetResult = serde_json::from_value(val).unwrap();
        let meta = get_result.reopen_metadata.as_ref().unwrap();
        assert_eq!(meta.reopen_count, 1);
        assert_eq!(meta.reopened_from_outcome_kind, "completed");
    }

    // ---- Milestone 12: run supersession handler tests ----

    #[test]
    fn run_supersede_completed_run_creates_successor() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_sup_1");
        store.save_run(&state).unwrap();
        finalize_run_in_store(&store, "r_sup_1", "completed");

        let params = serde_json::json!({
            "runId": "r_sup_1",
            "reason": "scope changed after completion"
        });
        let (val, successor_state) = dispatch(Method::RunSupersede, params, &store).unwrap();
        let result: RunSupersedeResult = serde_json::from_value(val).unwrap();

        assert_eq!(result.original_run_id, "r_sup_1");
        assert!(!result.successor_run_id.is_empty());
        assert_eq!(result.successor_status, "prepared");
        assert!(!result.superseded_at.is_empty());
        assert_eq!(result.recommended_tool, "refresh_run_state");

        // Original run should be marked superseded.
        let orig = store.get_run("r_sup_1").unwrap().unwrap();
        assert!(orig.status.starts_with("finalized:"));
        assert_eq!(
            orig.superseded_by_run_id.as_deref(),
            Some(result.successor_run_id.as_str())
        );
        assert_eq!(
            orig.supersession_reason.as_deref(),
            Some("scope changed after completion")
        );

        // Successor run should be in "prepared" status.
        let successor = store.get_run(&result.successor_run_id).unwrap().unwrap();
        assert_eq!(successor.status, "prepared");
        assert_eq!(successor.supersedes_run_id.as_deref(), Some("r_sup_1"));

        // dispatch returns successor state
        assert!(successor_state.is_some());
        assert_eq!(successor_state.unwrap().run_id, result.successor_run_id);
    }

    #[test]
    fn run_supersede_failed_run_creates_successor() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_sup_failed");
        store.save_run(&state).unwrap();
        finalize_run_in_store(&store, "r_sup_failed", "failed");

        let params = serde_json::json!({
            "runId": "r_sup_failed",
            "newUserGoal": "fix with better approach",
            "reason": "previous approach failed"
        });
        let (val, _) = dispatch(Method::RunSupersede, params, &store).unwrap();
        let result: RunSupersedeResult = serde_json::from_value(val).unwrap();

        let successor = store.get_run(&result.successor_run_id).unwrap().unwrap();
        assert_eq!(successor.user_goal, "fix with better approach");
        assert_eq!(successor.supersedes_run_id.as_deref(), Some("r_sup_failed"));
    }

    #[test]
    fn run_supersede_active_run_rejected() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_sup_active");
        store.save_run(&state).unwrap();
        // DO NOT finalize — status stays "active"

        let params = serde_json::json!({
            "runId": "r_sup_active",
            "reason": "trying to supersede active run"
        });
        let err = dispatch(Method::RunSupersede, params, &store).unwrap_err();
        assert!(err.to_string().contains("cannot be superseded"));
        // Original run must be unchanged.
        let orig = store.get_run("r_sup_active").unwrap().unwrap();
        assert_eq!(orig.status, "active");
        assert!(orig.superseded_by_run_id.is_none());
    }

    #[test]
    fn run_supersede_unknown_run_rejected() {
        let store = Store::open_in_memory().unwrap();
        let params = serde_json::json!({
            "runId": "nonexistent",
            "reason": "should fail"
        });
        let err = dispatch(Method::RunSupersede, params, &store).unwrap_err();
        assert!(err.to_string().contains("unknown run"));
    }

    #[test]
    fn run_supersede_audit_trail_appended() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_sup_audit");
        store.save_run(&state).unwrap();
        finalize_run_in_store(&store, "r_sup_audit", "completed");

        let params = serde_json::json!({
            "runId": "r_sup_audit",
            "reason": "audit test"
        });
        let (val, _) = dispatch(Method::RunSupersede, params, &store).unwrap();
        let result: RunSupersedeResult = serde_json::from_value(val).unwrap();

        // Audit entries should have been appended to both runs.
        let orig_audit = store.get_audit_entries("r_sup_audit", 50).unwrap();
        let orig_superseded_event = orig_audit.iter().find(|e| e.event_kind == "run_superseded");
        assert!(orig_superseded_event.is_some(), "run_superseded audit entry missing for original");

        let succ_audit = store.get_audit_entries(&result.successor_run_id, 50).unwrap();
        let succ_created_event = succ_audit
            .iter()
            .find(|e| e.event_kind == "run_created_from_supersession");
        assert!(succ_created_event.is_some(), "run_created_from_supersession audit entry missing for successor");
    }

    #[test]
    fn run_supersede_lineage_visible_in_run_get() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_sup_get_orig");
        store.save_run(&state).unwrap();
        finalize_run_in_store(&store, "r_sup_get_orig", "completed");

        let params = serde_json::json!({
            "runId": "r_sup_get_orig",
            "reason": "checking run.get lineage"
        });
        let (val, _) = dispatch(Method::RunSupersede, params, &store).unwrap();
        let result: RunSupersedeResult = serde_json::from_value(val).unwrap();

        // run.get on original should expose superseded_by_run_id
        let get_orig = serde_json::json!({ "runId": "r_sup_get_orig" });
        let (orig_val, _) = dispatch(Method::RunGet, get_orig, &store).unwrap();
        let orig_get: RunGetResult = serde_json::from_value(orig_val).unwrap();
        assert_eq!(
            orig_get.superseded_by_run_id.as_deref(),
            Some(result.successor_run_id.as_str())
        );

        // run.get on successor should expose supersedes_run_id
        let get_succ = serde_json::json!({ "runId": result.successor_run_id });
        let (succ_val, _) = dispatch(Method::RunGet, get_succ, &store).unwrap();
        let succ_get: RunGetResult = serde_json::from_value(succ_val).unwrap();
        assert_eq!(succ_get.supersedes_run_id.as_deref(), Some("r_sup_get_orig"));
    }

    // ---- Milestone 13: run.archive handler tests ----

    #[test]
    fn run_archive_completed_run_succeeds() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_arch_h");
        store.save_run(&state).unwrap();
        finalize_run_in_store(&store, "r_arch_h", "completed");

        let params = serde_json::json!({
            "runId": "r_arch_h",
            "reason": "archiving completed run"
        });
        let (val, run_state) = dispatch(Method::RunArchive, params, &store).unwrap();
        let result: RunArchiveResult = serde_json::from_value(val).unwrap();

        assert_eq!(result.run_id, "r_arch_h");
        assert!(result.status.starts_with("finalized:"));
        assert!(!result.archived_at.is_empty());
        assert_eq!(result.reason, "archiving completed run");
        assert!(!result.message.is_empty());

        // run_state returned should carry archive_metadata.
        let rs = run_state.unwrap();
        assert!(rs.archive_metadata.is_some());

        // Persisted state should also carry archive_metadata.
        let loaded = store.get_run("r_arch_h").unwrap().unwrap();
        let meta = loaded.archive_metadata.expect("archive_metadata must be persisted");
        assert_eq!(meta.reason, "archiving completed run");
    }

    #[test]
    fn run_archive_failed_run_succeeds() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_arch_fail");
        store.save_run(&state).unwrap();
        finalize_run_in_store(&store, "r_arch_fail", "failed");

        let params = serde_json::json!({ "runId": "r_arch_fail", "reason": "failed build" });
        let (val, _) = dispatch(Method::RunArchive, params, &store).unwrap();
        let result: RunArchiveResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.run_id, "r_arch_fail");
    }

    #[test]
    fn run_archive_rejected_for_active_run() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_arch_act");
        store.save_run(&state).unwrap();
        // Status is "active" — not eligible.
        let params = serde_json::json!({ "runId": "r_arch_act", "reason": "should fail" });
        let err = dispatch(Method::RunArchive, params, &store).unwrap_err();
        assert!(err.to_string().contains("cannot be archived"));
    }

    #[test]
    fn run_archive_rejected_for_prepared_run() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_arch_prep");
        state.status = "prepared".into();
        store.save_run(&state).unwrap();
        let params = serde_json::json!({ "runId": "r_arch_prep", "reason": "should fail" });
        let err = dispatch(Method::RunArchive, params, &store).unwrap_err();
        assert!(err.to_string().contains("cannot be archived"));
    }

    #[test]
    fn run_archive_unknown_run_returns_error() {
        let store = Store::open_in_memory().unwrap();
        let params = serde_json::json!({ "runId": "no-such-run", "reason": "reason" });
        let err = dispatch(Method::RunArchive, params, &store).unwrap_err();
        assert!(err.to_string().contains("unknown run"));
    }

    #[test]
    fn run_archive_audit_trail_appended() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_arch_audit");
        store.save_run(&state).unwrap();
        finalize_run_in_store(&store, "r_arch_audit", "completed");

        let params = serde_json::json!({ "runId": "r_arch_audit", "reason": "audit trail test" });
        dispatch(Method::RunArchive, params, &store).unwrap();

        let audit = store.get_audit_entries("r_arch_audit", 50).unwrap();
        let archived_event = audit.iter().find(|e| e.event_kind == "run_archived");
        assert!(archived_event.is_some(), "run_archived audit entry must be present");
    }

    #[test]
    fn run_archive_visible_in_run_get() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_arch_get");
        store.save_run(&state).unwrap();
        finalize_run_in_store(&store, "r_arch_get", "completed");

        let params = serde_json::json!({ "runId": "r_arch_get", "reason": "get test" });
        dispatch(Method::RunArchive, params, &store).unwrap();

        // run.get should expose archive_metadata.
        let get_params = serde_json::json!({ "runId": "r_arch_get" });
        let (val, _) = dispatch(Method::RunGet, get_params, &store).unwrap();
        let get_result: RunGetResult = serde_json::from_value(val).unwrap();
        assert!(get_result.archive_metadata.is_some(), "archive_metadata must be visible in run.get");
        let meta = get_result.archive_metadata.unwrap();
        assert_eq!(meta.reason, "get test");
    }

    #[test]
    fn run_archive_excluded_from_default_list() {
        let store = Store::open_in_memory().unwrap();

        let active_state = make_run_state("r_list_active");
        store.save_run(&active_state).unwrap();

        let archived_state = make_run_state("r_list_arch");
        store.save_run(&archived_state).unwrap();
        finalize_run_in_store(&store, "r_list_arch", "completed");
        let arch_params = serde_json::json!({ "runId": "r_list_arch", "reason": "list test" });
        dispatch(Method::RunArchive, arch_params, &store).unwrap();

        // Default list_runs excludes archived.
        let list_params = serde_json::json!({});
        let (val, _) = dispatch(Method::RunsList, list_params, &store).unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();
        assert!(
            result.runs.iter().any(|r| r.run_id == "r_list_active"),
            "active run must be in default list"
        );
        assert!(
            !result.runs.iter().any(|r| r.run_id == "r_list_arch"),
            "archived run must NOT be in default list"
        );
    }

    #[test]
    fn run_archive_visible_with_include_archived_flag() {
        let store = Store::open_in_memory().unwrap();

        let archived_state = make_run_state("r_incl_arch");
        store.save_run(&archived_state).unwrap();
        finalize_run_in_store(&store, "r_incl_arch", "completed");
        let arch_params = serde_json::json!({ "runId": "r_incl_arch", "reason": "include test" });
        dispatch(Method::RunArchive, arch_params, &store).unwrap();

        // include_archived=true should show archived run.
        let list_params = serde_json::json!({ "includeArchived": true });
        let (val, _) = dispatch(Method::RunsList, list_params, &store).unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();
        assert!(
            result.runs.iter().any(|r| r.run_id == "r_incl_arch"),
            "archived run must appear when includeArchived=true"
        );
    }

    #[test]
    fn run_archive_archived_only_filter() {
        let store = Store::open_in_memory().unwrap();

        let active_state = make_run_state("r_ao_active");
        store.save_run(&active_state).unwrap();

        let archived_state = make_run_state("r_ao_arch");
        store.save_run(&archived_state).unwrap();
        finalize_run_in_store(&store, "r_ao_arch", "completed");
        let arch_params = serde_json::json!({ "runId": "r_ao_arch", "reason": "archived_only test" });
        dispatch(Method::RunArchive, arch_params, &store).unwrap();

        // archived_only=true should return only archived run.
        let list_params = serde_json::json!({ "archivedOnly": true });
        let (val, _) = dispatch(Method::RunsList, list_params, &store).unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();
        assert!(
            !result.runs.iter().any(|r| r.run_id == "r_ao_active"),
            "active run must NOT appear when archivedOnly=true"
        );
        assert!(
            result.runs.iter().any(|r| r.run_id == "r_ao_arch"),
            "archived run must appear when archivedOnly=true"
        );
    }

    // ---- Milestone 14: run.unarchive handler tests ----

    #[test]
    fn run_unarchive_completed_run_succeeds() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_unarch_c");
        state.status = "finalized:completed".into();
        store.save_run(&state).unwrap();
        // Archive it first.
        let arch_params = serde_json::json!({ "runId": "r_unarch_c", "reason": "archive first" });
        dispatch(Method::RunArchive, arch_params, &store).unwrap();

        // Now unarchive.
        let params = serde_json::json!({ "runId": "r_unarch_c", "reason": "restoring for inspection" });
        let (val, run_state) = dispatch(Method::RunUnarchive, params, &store).unwrap();
        let result: RunUnarchiveResult = serde_json::from_value(val).unwrap();

        assert_eq!(result.run_id, "r_unarch_c");
        assert!(result.status.starts_with("finalized:"));
        assert!(!result.unarchived_at.is_empty());
        assert_eq!(result.reason, "restoring for inspection");
        assert!(!result.message.is_empty());

        // run_state returned should carry unarchive_metadata.
        let rs = run_state.expect("run state must be returned");
        assert!(rs.unarchive_metadata.is_some());
        // Archive metadata must remain.
        assert!(rs.archive_metadata.is_some());

        // Persisted state should also carry unarchive_metadata.
        let loaded = store.get_run("r_unarch_c").unwrap().unwrap();
        let meta = loaded.unarchive_metadata.expect("unarchive_metadata must be persisted");
        assert_eq!(meta.reason, "restoring for inspection");
    }

    #[test]
    fn run_unarchive_failed_run_succeeds() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_unarch_f");
        state.status = "finalized:failed".into();
        store.save_run(&state).unwrap();
        let arch_params = serde_json::json!({ "runId": "r_unarch_f", "reason": "archive" });
        dispatch(Method::RunArchive, arch_params, &store).unwrap();

        let params = serde_json::json!({ "runId": "r_unarch_f", "reason": "reviewing failed build" });
        let (val, _) = dispatch(Method::RunUnarchive, params, &store).unwrap();
        let result: RunUnarchiveResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.run_id, "r_unarch_f");
    }

    #[test]
    fn run_unarchive_rejected_for_non_archived_run() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_unarch_na");
        state.status = "finalized:completed".into();
        store.save_run(&state).unwrap();

        let params = serde_json::json!({ "runId": "r_unarch_na", "reason": "trying" });
        let err = dispatch(Method::RunUnarchive, params, &store).unwrap_err();
        assert!(err.to_string().contains("cannot be unarchived") || err.to_string().contains("not archived"));
    }

    #[test]
    fn run_unarchive_unknown_run_returns_error() {
        let store = Store::open_in_memory().unwrap();
        let params = serde_json::json!({ "runId": "unknown-99", "reason": "test" });
        let err = dispatch(Method::RunUnarchive, params, &store).unwrap_err();
        assert!(err.to_string().contains("unknown run"));
    }

    #[test]
    fn run_unarchive_audit_trail_appended() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_unarch_audit");
        state.status = "finalized:completed".into();
        store.save_run(&state).unwrap();
        let arch_params = serde_json::json!({ "runId": "r_unarch_audit", "reason": "archive" });
        dispatch(Method::RunArchive, arch_params, &store).unwrap();

        let params = serde_json::json!({ "runId": "r_unarch_audit", "reason": "audit test" });
        dispatch(Method::RunUnarchive, params, &store).unwrap();

        let entries = store.get_audit_entries("r_unarch_audit", 50).unwrap();
        let has_unarchive_entry = entries.iter().any(|e| e.event_kind == "run_unarchived");
        assert!(has_unarchive_entry, "run_unarchived audit entry must be appended");
    }

    #[test]
    fn run_unarchive_restores_to_default_list() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_restore");
        state.status = "finalized:completed".into();
        store.save_run(&state).unwrap();

        // Archive the run.
        let arch_params = serde_json::json!({ "runId": "r_restore", "reason": "archive" });
        dispatch(Method::RunArchive, arch_params, &store).unwrap();

        // Verify it's excluded from default list.
        let (val, _) = dispatch(Method::RunsList, serde_json::json!({}), &store).unwrap();
        let list: RunsListResult = serde_json::from_value(val).unwrap();
        assert!(!list.runs.iter().any(|r| r.run_id == "r_restore"), "archived run must not appear in default list");

        // Unarchive the run.
        let unarch_params = serde_json::json!({ "runId": "r_restore", "reason": "restoring" });
        dispatch(Method::RunUnarchive, unarch_params, &store).unwrap();

        // Verify it appears in default list after restoration.
        let (val, _) = dispatch(Method::RunsList, serde_json::json!({}), &store).unwrap();
        let list: RunsListResult = serde_json::from_value(val).unwrap();
        assert!(list.runs.iter().any(|r| r.run_id == "r_restore"), "unarchived run must appear in default list");
    }

    #[test]
    fn run_unarchive_excluded_from_archived_only_filter() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_ao_unarch");
        state.status = "finalized:completed".into();
        store.save_run(&state).unwrap();

        // Archive then unarchive.
        let arch_params = serde_json::json!({ "runId": "r_ao_unarch", "reason": "archive" });
        dispatch(Method::RunArchive, arch_params, &store).unwrap();
        let unarch_params = serde_json::json!({ "runId": "r_ao_unarch", "reason": "unarchive" });
        dispatch(Method::RunUnarchive, unarch_params, &store).unwrap();

        // archivedOnly=true must NOT include the restored run.
        let (val, _) = dispatch(Method::RunsList, serde_json::json!({ "archivedOnly": true }), &store).unwrap();
        let list: RunsListResult = serde_json::from_value(val).unwrap();
        assert!(!list.runs.iter().any(|r| r.run_id == "r_ao_unarch"), "unarchived run must NOT appear with archivedOnly=true");
    }

    #[test]
    fn run_unarchive_visible_in_run_get() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_unarch_get");
        state.status = "finalized:completed".into();
        store.save_run(&state).unwrap();
        let arch_params = serde_json::json!({ "runId": "r_unarch_get", "reason": "archive" });
        dispatch(Method::RunArchive, arch_params, &store).unwrap();
        let unarch_params = serde_json::json!({ "runId": "r_unarch_get", "reason": "inspect" });
        dispatch(Method::RunUnarchive, unarch_params, &store).unwrap();

        let get_params = serde_json::json!({ "runId": "r_unarch_get" });
        let (val, _) = dispatch(Method::RunGet, get_params, &store).unwrap();
        let result: RunGetResult = serde_json::from_value(val).unwrap();
        assert!(result.unarchive_metadata.is_some(), "unarchive_metadata must be visible in run.get");
        assert!(result.archive_metadata.is_some(), "archive_metadata must still be present in run.get");
    }

    #[test]
    fn run_unarchive_persistence_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_unarch_persist");
        state.status = "finalized:completed".into();
        store.save_run(&state).unwrap();
        let arch_params = serde_json::json!({ "runId": "r_unarch_persist", "reason": "archive" });
        dispatch(Method::RunArchive, arch_params, &store).unwrap();

        let unarch_params = serde_json::json!({ "runId": "r_unarch_persist", "reason": "persistence test" });
        dispatch(Method::RunUnarchive, unarch_params, &store).unwrap();

        let loaded = store.get_run("r_unarch_persist").unwrap().unwrap();
        let meta = loaded.unarchive_metadata.expect("unarchive_metadata must persist");
        assert_eq!(meta.reason, "persistence test");
        assert!(!meta.unarchived_at.is_empty());
        // Archive metadata must remain intact.
        assert!(loaded.archive_metadata.is_some());
        // Status must remain finalized.
        assert!(loaded.status.starts_with("finalized:"));
    }

    // ---- Milestone 15: deterministic run labeling / annotation ----

    #[test]
    fn run_annotate_sets_labels() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_ann_labels");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_ann_labels",
            "labels": ["auth", "infra"]
        });
        let (val, _) = dispatch(Method::RunAnnotate, params, &store).unwrap();
        let result: RunAnnotateResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.run_id, "r_ann_labels");
        assert_eq!(result.annotation.labels, vec!["auth", "infra"]);
    }

    #[test]
    fn run_annotate_sets_operator_note() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_ann_note");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_ann_note",
            "operatorNote": "tracking the auth regression"
        });
        let (val, _) = dispatch(Method::RunAnnotate, params, &store).unwrap();
        let result: RunAnnotateResult = serde_json::from_value(val).unwrap();
        assert_eq!(
            result.annotation.operator_note.as_deref(),
            Some("tracking the auth regression")
        );
    }

    #[test]
    fn run_annotate_normalizes_labels() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_ann_norm");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_ann_norm",
            "labels": ["INFRA", "Auth", "auth"]
        });
        let (val, _) = dispatch(Method::RunAnnotate, params, &store).unwrap();
        let result: RunAnnotateResult = serde_json::from_value(val).unwrap();
        // Normalized, deduped, sorted.
        assert_eq!(result.annotation.labels, vec!["auth", "infra"]);
    }

    #[test]
    fn run_annotate_persists_to_store() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_ann_persist");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_ann_persist",
            "labels": ["ci"],
            "operatorNote": "CI regression"
        });
        dispatch(Method::RunAnnotate, params, &store).unwrap();

        let loaded = store.get_run("r_ann_persist").unwrap().unwrap();
        let annotation = loaded.annotation.expect("annotation must be persisted");
        assert_eq!(annotation.labels, vec!["ci"]);
        assert_eq!(annotation.operator_note.as_deref(), Some("CI regression"));
    }

    #[test]
    fn run_annotate_appends_audit_entry() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_ann_audit");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_ann_audit",
            "labels": ["blocked"]
        });
        dispatch(Method::RunAnnotate, params, &store).unwrap();

        let entries = store.get_audit_entries("r_ann_audit", 50).unwrap();
        let has_entry = entries.iter().any(|e| e.event_kind == "run_annotated");
        assert!(has_entry, "run_annotated audit entry must be appended");
    }

    #[test]
    fn run_annotate_visible_in_run_get() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_ann_get");
        store.save_run(&state).unwrap();

        let ann_params = serde_json::json!({
            "runId": "r_ann_get",
            "labels": ["feature"],
            "operatorNote": "feature work"
        });
        dispatch(Method::RunAnnotate, ann_params, &store).unwrap();

        let get_params = serde_json::json!({ "runId": "r_ann_get" });
        let (val, _) = dispatch(Method::RunGet, get_params, &store).unwrap();
        let result: RunGetResult = serde_json::from_value(val).unwrap();
        let annotation = result.annotation.expect("annotation must appear in run.get");
        assert_eq!(annotation.labels, vec!["feature"]);
        assert_eq!(annotation.operator_note.as_deref(), Some("feature work"));
    }

    #[test]
    fn run_annotate_visible_in_runs_list() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_ann_list");
        store.save_run(&state).unwrap();

        let ann_params = serde_json::json!({
            "runId": "r_ann_list",
            "labels": ["review"]
        });
        dispatch(Method::RunAnnotate, ann_params, &store).unwrap();

        let (val, _) = dispatch(Method::RunsList, serde_json::json!({}), &store).unwrap();
        let list: RunsListResult = serde_json::from_value(val).unwrap();
        let summary = list.runs.iter().find(|r| r.run_id == "r_ann_list").unwrap();
        assert_eq!(summary.labels, vec!["review"]);
    }

    #[test]
    fn run_annotate_list_filter_by_label() {
        let store = Store::open_in_memory().unwrap();

        let mut auth_state = make_run_state("r_filter_auth");
        auth_state.workspace_id = "/tmp/ws".into();
        store.save_run(&auth_state).unwrap();

        let mut infra_state = make_run_state("r_filter_infra");
        infra_state.workspace_id = "/tmp/ws".into();
        store.save_run(&infra_state).unwrap();

        // Annotate only auth run.
        let ann_params = serde_json::json!({
            "runId": "r_filter_auth",
            "labels": ["auth"]
        });
        dispatch(Method::RunAnnotate, ann_params, &store).unwrap();

        // Filter by label=auth.
        let (val, _) = dispatch(
            Method::RunsList,
            serde_json::json!({ "label": "auth" }),
            &store,
        )
        .unwrap();
        let list: RunsListResult = serde_json::from_value(val).unwrap();
        assert!(
            list.runs.iter().any(|r| r.run_id == "r_filter_auth"),
            "auth-labeled run must appear in label=auth filter"
        );
        assert!(
            !list.runs.iter().any(|r| r.run_id == "r_filter_infra"),
            "unlabeled run must NOT appear in label=auth filter"
        );
    }

    #[test]
    fn run_annotate_does_not_change_status() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_ann_status");
        state.status = "active".into();
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_ann_status",
            "labels": ["ci"]
        });
        dispatch(Method::RunAnnotate, params, &store).unwrap();

        let loaded = store.get_run("r_ann_status").unwrap().unwrap();
        assert_eq!(loaded.status, "active", "annotate must not change status");
    }

    #[test]
    fn run_annotate_rejects_empty_params() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_ann_empty");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({ "runId": "r_ann_empty" });
        let err = dispatch(Method::RunAnnotate, params, &store).unwrap_err();
        assert!(
            err.to_string().contains("at least one"),
            "must require at least one of labels/operatorNote"
        );
    }

    #[test]
    fn run_annotate_rejects_invalid_label() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_ann_invalid");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_ann_invalid",
            "labels": ["bad label with spaces"]
        });
        let err = dispatch(Method::RunAnnotate, params, &store).unwrap_err();
        assert!(
            err.to_string().contains("invalid character"),
            "must reject labels with invalid characters"
        );
    }

    // -----------------------------------------------------------------------
    // Milestone 17: run.snooze / run.unsnooze tests
    // -----------------------------------------------------------------------

    #[test]
    fn run_snooze_sets_snooze_metadata() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_snz_1");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_snz_1",
            "reason": "blocked on external dep"
        });
        let (val, run_state) = dispatch(Method::RunSnooze, params, &store).unwrap();
        let result: RunSnoozeResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.run_id, "r_snz_1");
        assert_eq!(result.reason, "blocked on external dep");
        assert!(!result.snoozed_at.is_empty());
        assert!(run_state.is_some());
        assert!(run_state.unwrap().snooze_metadata.is_some());
    }

    #[test]
    fn run_snooze_persists_to_store() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_snz_persist");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_snz_persist",
            "reason": "deferred"
        });
        dispatch(Method::RunSnooze, params, &store).unwrap();

        let loaded = store.get_run("r_snz_persist").unwrap().unwrap();
        assert!(loaded.snooze_metadata.is_some());
        assert_eq!(loaded.snooze_metadata.unwrap().reason, "deferred");
    }

    #[test]
    fn run_snooze_appends_audit_entry() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_snz_audit");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_snz_audit",
            "reason": "audit test"
        });
        dispatch(Method::RunSnooze, params, &store).unwrap();

        let history = store.get_run_history("r_snz_audit", 10).unwrap();
        let entry = history.iter().find(|e| e.event_kind == "run_snoozed");
        assert!(entry.is_some(), "run_snoozed audit entry must be appended");
    }

    #[test]
    fn run_snooze_does_not_change_status() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_snz_status");
        state.status = "active".into();
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_snz_status",
            "reason": "snooze"
        });
        dispatch(Method::RunSnooze, params, &store).unwrap();

        let loaded = store.get_run("r_snz_status").unwrap().unwrap();
        assert_eq!(loaded.status, "active", "snooze must not change status");
    }

    #[test]
    fn run_snooze_excluded_from_default_list() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_snz_list");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_snz_list",
            "reason": "defer"
        });
        dispatch(Method::RunSnooze, params, &store).unwrap();

        let list_params = serde_json::json!({ "limit": 50 });
        let (val, _) = dispatch(Method::RunsList, list_params, &store).unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();
        let found = result.runs.iter().any(|r| r.run_id == "r_snz_list");
        assert!(!found, "snoozed run must be excluded from default list");
    }

    #[test]
    fn run_snooze_included_with_include_snoozed() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_snz_incl");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_snz_incl",
            "reason": "defer"
        });
        dispatch(Method::RunSnooze, params, &store).unwrap();

        let list_params = serde_json::json!({ "limit": 50, "includeSnoozed": true });
        let (val, _) = dispatch(Method::RunsList, list_params, &store).unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();
        let found = result.runs.iter().any(|r| r.run_id == "r_snz_incl");
        assert!(found, "snoozed run must appear when includeSnoozed=true");
    }

    #[test]
    fn run_snooze_snoozed_only_filter() {
        let store = Store::open_in_memory().unwrap();
        let state_a = make_run_state("r_snz_only_a");
        let state_b = make_run_state("r_snz_only_b");
        store.save_run(&state_a).unwrap();
        store.save_run(&state_b).unwrap();

        let params = serde_json::json!({
            "runId": "r_snz_only_a",
            "reason": "defer"
        });
        dispatch(Method::RunSnooze, params, &store).unwrap();

        let list_params = serde_json::json!({ "limit": 50, "snoozedOnly": true });
        let (val, _) = dispatch(Method::RunsList, list_params, &store).unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();
        assert!(result.runs.iter().any(|r| r.run_id == "r_snz_only_a"), "snoozed run must appear");
        assert!(!result.runs.iter().any(|r| r.run_id == "r_snz_only_b"), "non-snoozed run must not appear");
    }

    #[test]
    fn run_snooze_rejects_empty_reason() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_snz_empty");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_snz_empty",
            "reason": ""
        });
        let err = dispatch(Method::RunSnooze, params, &store).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn run_unsnooze_clears_snooze_metadata() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_unsnz_1");
        store.save_run(&state).unwrap();

        // First snooze it.
        let snooze_params = serde_json::json!({
            "runId": "r_unsnz_1",
            "reason": "blocked"
        });
        dispatch(Method::RunSnooze, snooze_params, &store).unwrap();

        // Then unsnooze it.
        let unsnooze_params = serde_json::json!({
            "runId": "r_unsnz_1",
            "reason": "resolved"
        });
        let (val, run_state) = dispatch(Method::RunUnsnooze, unsnooze_params, &store).unwrap();
        let result: RunUnsnoozeResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.run_id, "r_unsnz_1");
        assert!(run_state.is_some());
        assert!(run_state.unwrap().snooze_metadata.is_none());
    }

    #[test]
    fn run_unsnooze_persists_to_store() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_unsnz_persist");
        store.save_run(&state).unwrap();

        dispatch(
            Method::RunSnooze,
            serde_json::json!({"runId": "r_unsnz_persist", "reason": "defer"}),
            &store,
        ).unwrap();
        dispatch(
            Method::RunUnsnooze,
            serde_json::json!({"runId": "r_unsnz_persist", "reason": "ready"}),
            &store,
        ).unwrap();

        let loaded = store.get_run("r_unsnz_persist").unwrap().unwrap();
        assert!(loaded.snooze_metadata.is_none(), "snooze_metadata must be cleared after unsnooze");
    }

    #[test]
    fn run_unsnooze_appends_audit_entry() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_unsnz_audit");
        store.save_run(&state).unwrap();

        dispatch(
            Method::RunSnooze,
            serde_json::json!({"runId": "r_unsnz_audit", "reason": "defer"}),
            &store,
        ).unwrap();
        dispatch(
            Method::RunUnsnooze,
            serde_json::json!({"runId": "r_unsnz_audit", "reason": "ready"}),
            &store,
        ).unwrap();

        let history = store.get_run_history("r_unsnz_audit", 10).unwrap();
        let entry = history.iter().find(|e| e.event_kind == "run_unsnoozed");
        assert!(entry.is_some(), "run_unsnoozed audit entry must be appended");
    }

    #[test]
    fn run_unsnooze_rejects_non_snoozed() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_unsnz_not");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_unsnz_not",
            "reason": "restore"
        });
        let err = dispatch(Method::RunUnsnooze, params, &store).unwrap_err();
        assert!(err.to_string().contains("not snoozed"));
    }

    #[test]
    fn run_unsnooze_restores_to_default_list() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_unsnz_list");
        store.save_run(&state).unwrap();

        dispatch(
            Method::RunSnooze,
            serde_json::json!({"runId": "r_unsnz_list", "reason": "defer"}),
            &store,
        ).unwrap();

        // Confirm excluded from default list.
        let (val, _) = dispatch(
            Method::RunsList,
            serde_json::json!({"limit": 50}),
            &store,
        ).unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();
        assert!(!result.runs.iter().any(|r| r.run_id == "r_unsnz_list"));

        // Unsnooze and confirm restored.
        dispatch(
            Method::RunUnsnooze,
            serde_json::json!({"runId": "r_unsnz_list", "reason": "ready"}),
            &store,
        ).unwrap();

        let (val, _) = dispatch(
            Method::RunsList,
            serde_json::json!({"limit": 50}),
            &store,
        ).unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();
        assert!(result.runs.iter().any(|r| r.run_id == "r_unsnz_list"), "unsnoozed run must appear in default list");
    }

    // ---- Milestone 18: run priority tests ----

    #[test]
    fn run_set_priority_persists() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_prio_persist");
        store.save_run(&state).unwrap();

        let (val, run_state) = dispatch(
            Method::RunSetPriority,
            serde_json::json!({"runId": "r_prio_persist", "priority": "urgent", "reason": "blocks release"}),
            &store,
        ).unwrap();

        let result: RunSetPriorityResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.priority, RunPriority::Urgent);
        assert_eq!(result.previous_priority, RunPriority::Normal);
        assert!(run_state.is_some());
        assert_eq!(run_state.unwrap().priority, RunPriority::Urgent);

        // Reload from store and verify persistence.
        let loaded = store.get_run("r_prio_persist").unwrap().unwrap();
        assert_eq!(loaded.priority, RunPriority::Urgent);
    }

    #[test]
    fn run_set_priority_appends_audit_entry() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_prio_audit");
        store.save_run(&state).unwrap();

        dispatch(
            Method::RunSetPriority,
            serde_json::json!({"runId": "r_prio_audit", "priority": "high", "reason": "elevated"}),
            &store,
        ).unwrap();

        let history = store.get_run_history("r_prio_audit", 10).unwrap();
        let entry = history.iter().find(|e| e.event_kind == "run_priority_set");
        assert!(entry.is_some(), "run_priority_set audit entry must be appended");
    }

    #[test]
    fn run_set_priority_rejects_unknown_run() {
        let store = Store::open_in_memory().unwrap();
        let params = serde_json::json!({
            "runId": "r_prio_unknown",
            "priority": "urgent",
            "reason": "test"
        });
        let err = dispatch(Method::RunSetPriority, params, &store).unwrap_err();
        assert!(err.to_string().contains("unknown run"));
    }

    #[test]
    fn run_set_priority_rejects_empty_reason() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_prio_empty_reason");
        store.save_run(&state).unwrap();

        let params = serde_json::json!({
            "runId": "r_prio_empty_reason",
            "priority": "urgent",
            "reason": ""
        });
        let err = dispatch(Method::RunSetPriority, params, &store).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn run_set_priority_list_filter_by_priority() {
        let store = Store::open_in_memory().unwrap();

        let low = make_run_state("r_prio_low");
        let normal = make_run_state("r_prio_normal");
        let mut high = make_run_state("r_prio_high");
        high.priority = RunPriority::High;
        let mut urgent = make_run_state("r_prio_urgent");
        urgent.priority = RunPriority::Urgent;

        store.save_run(&low).unwrap();
        store.save_run(&normal).unwrap();
        store.save_run(&high).unwrap();
        store.save_run(&urgent).unwrap();

        // Filter for urgent only.
        let (val, _) = dispatch(
            Method::RunsList,
            serde_json::json!({"limit": 50, "priorityFilter": "urgent"}),
            &store,
        ).unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.runs.len(), 1);
        assert_eq!(result.runs[0].run_id, "r_prio_urgent");

        // Filter for high only.
        let (val, _) = dispatch(
            Method::RunsList,
            serde_json::json!({"limit": 50, "priorityFilter": "high"}),
            &store,
        ).unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.runs.len(), 1);
        assert_eq!(result.runs[0].run_id, "r_prio_high");
    }

    #[test]
    fn run_set_priority_list_sort_by_priority() {
        let store = Store::open_in_memory().unwrap();

        let mut low = make_run_state("r_sort_low");
        low.priority = RunPriority::Low;
        let normal = make_run_state("r_sort_normal");
        let mut high = make_run_state("r_sort_high");
        high.priority = RunPriority::High;
        let mut urgent = make_run_state("r_sort_urgent");
        urgent.priority = RunPriority::Urgent;

        store.save_run(&low).unwrap();
        store.save_run(&normal).unwrap();
        store.save_run(&high).unwrap();
        store.save_run(&urgent).unwrap();

        let (val, _) = dispatch(
            Method::RunsList,
            serde_json::json!({"limit": 50, "sortByPriority": true}),
            &store,
        ).unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();
        // Urgent must come first, low must come last.
        let ids: Vec<&str> = result.runs.iter().map(|r| r.run_id.as_str()).collect();
        let urgent_pos = ids.iter().position(|&id| id == "r_sort_urgent").unwrap();
        let high_pos = ids.iter().position(|&id| id == "r_sort_high").unwrap();
        let normal_pos = ids.iter().position(|&id| id == "r_sort_normal").unwrap();
        let low_pos = ids.iter().position(|&id| id == "r_sort_low").unwrap();
        assert!(urgent_pos < high_pos, "urgent must precede high");
        assert!(high_pos < normal_pos, "high must precede normal");
        assert!(normal_pos < low_pos, "normal must precede low");
    }

    #[test]
    fn run_set_priority_summary_carries_priority() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_prio_summary");
        state.priority = RunPriority::Urgent;
        store.save_run(&state).unwrap();

        let (val, _) = dispatch(
            Method::RunsList,
            serde_json::json!({"limit": 50}),
            &store,
        ).unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();
        let summary = result.runs.iter().find(|r| r.run_id == "r_prio_summary").unwrap();
        assert_eq!(summary.priority, RunPriority::Urgent);
    }

    // -- run.assign_owner tests (Milestone 19) --------------------------------

    #[test]
    fn run_assign_owner_sets_assignee() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_ao_set")).unwrap();
        let (val, run_state) = dispatch(
            Method::RunAssignOwner,
            serde_json::json!({"runId": "r_ao_set", "assignee": "alice"}),
            &store,
        ).unwrap();
        let result: RunAssignOwnerResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.assignee.as_deref(), Some("alice"));
        assert_eq!(result.previous_assignee, None);
        assert!(run_state.is_some());
        assert_eq!(run_state.unwrap().assignee.as_deref(), Some("alice"));
    }

    #[test]
    fn run_assign_owner_clears_assignee() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_ao_clear");
        state.assignee = Some("bob".into());
        store.save_run(&state).unwrap();
        let (val, _) = dispatch(
            Method::RunAssignOwner,
            serde_json::json!({"runId": "r_ao_clear", "assignee": null}),
            &store,
        ).unwrap();
        let result: RunAssignOwnerResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.assignee, None);
        assert_eq!(result.previous_assignee.as_deref(), Some("bob"));
    }

    #[test]
    fn run_assign_owner_update_note() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_ao_note")).unwrap();
        let (val, _) = dispatch(
            Method::RunAssignOwner,
            serde_json::json!({"runId": "r_ao_note", "ownershipNote": "hand off to team B"}),
            &store,
        ).unwrap();
        let result: RunAssignOwnerResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.ownership_note.as_deref(), Some("hand off to team B"));
    }

    #[test]
    fn run_assign_owner_persists() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_ao_persist")).unwrap();
        dispatch(
            Method::RunAssignOwner,
            serde_json::json!({"runId": "r_ao_persist", "assignee": "carol"}),
            &store,
        ).unwrap();
        let loaded = store.get_run("r_ao_persist").unwrap().unwrap();
        assert_eq!(loaded.assignee.as_deref(), Some("carol"));
    }

    #[test]
    fn run_assign_owner_audit_entry() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_ao_audit")).unwrap();
        dispatch(
            Method::RunAssignOwner,
            serde_json::json!({"runId": "r_ao_audit", "assignee": "dave"}),
            &store,
        ).unwrap();
        let history = store.get_run_history("r_ao_audit", 10).unwrap();
        assert!(history.iter().any(|e| e.event_kind == "run_owner_assigned"));
    }

    #[test]
    fn run_assign_owner_does_not_change_status() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_ao_status");
        state.status = "finalized:completed".into();
        store.save_run(&state).unwrap();
        let (val, _) = dispatch(
            Method::RunAssignOwner,
            serde_json::json!({"runId": "r_ao_status", "assignee": "eve"}),
            &store,
        ).unwrap();
        let result: RunAssignOwnerResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.status, "finalized:completed");
    }

    #[test]
    fn run_assign_owner_list_filter_by_assignee() {
        let store = Store::open_in_memory().unwrap();
        let mut s1 = make_run_state("r_ao_list1");
        s1.assignee = Some("alice".into());
        let mut s2 = make_run_state("r_ao_list2");
        s2.assignee = Some("bob".into());
        let s3 = make_run_state("r_ao_list3");
        store.save_run(&s1).unwrap();
        store.save_run(&s2).unwrap();
        store.save_run(&s3).unwrap();
        let (val, _) = dispatch(
            Method::RunsList,
            serde_json::json!({"limit": 50, "assignee": "alice"}),
            &store,
        ).unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.runs.len(), 1);
        assert_eq!(result.runs[0].run_id, "r_ao_list1");
    }

    // ----- Milestone 20: run.set_due_date -----

    #[test]
    fn run_set_due_date_sets_date() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_dd_h1")).unwrap();
        let (val, _) = dispatch(
            Method::RunSetDueDate,
            serde_json::json!({"runId": "r_dd_h1", "dueDate": "2026-03-31"}),
            &store,
        ).unwrap();
        let result: RunSetDueDateResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.due_date.as_deref(), Some("2026-03-31"));
    }

    #[test]
    fn run_set_due_date_persists() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_dd_h2")).unwrap();
        dispatch(
            Method::RunSetDueDate,
            serde_json::json!({"runId": "r_dd_h2", "dueDate": "2026-06-30"}),
            &store,
        ).unwrap();
        let loaded = store.get_run("r_dd_h2").unwrap().unwrap();
        assert_eq!(loaded.due_date.as_deref(), Some("2026-06-30"));
    }

    #[test]
    fn run_set_due_date_clear() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_dd_h3");
        state.due_date = Some("2026-01-01".into());
        store.save_run(&state).unwrap();
        let (val, _) = dispatch(
            Method::RunSetDueDate,
            serde_json::json!({"runId": "r_dd_h3", "dueDate": null}),
            &store,
        ).unwrap();
        let result: RunSetDueDateResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.due_date, None);
        assert_eq!(result.previous_due_date.as_deref(), Some("2026-01-01"));
        let loaded = store.get_run("r_dd_h3").unwrap().unwrap();
        assert_eq!(loaded.due_date, None);
    }

    #[test]
    fn run_set_due_date_invalid_format_rejected() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_dd_h4")).unwrap();
        let err = dispatch(
            Method::RunSetDueDate,
            serde_json::json!({"runId": "r_dd_h4", "dueDate": "not-a-date"}),
            &store,
        ).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("10 characters") || msg.contains("non-digit") || msg.contains("separators"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn run_set_due_date_unknown_run_rejected() {
        let store = Store::open_in_memory().unwrap();
        let err = dispatch(
            Method::RunSetDueDate,
            serde_json::json!({"runId": "r_dd_missing", "dueDate": "2026-01-01"}),
            &store,
        ).unwrap_err();
        assert!(err.to_string().contains("unknown run"), "{err}");
    }

    #[test]
    fn run_set_due_date_audit_entry() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_dd_audit")).unwrap();
        dispatch(
            Method::RunSetDueDate,
            serde_json::json!({"runId": "r_dd_audit", "dueDate": "2026-09-01"}),
            &store,
        ).unwrap();
        let history = store.get_run_history("r_dd_audit", 10).unwrap();
        assert!(history.iter().any(|e| e.event_kind == "run_due_date_set"));
    }

    #[test]
    fn runs_list_filter_by_due_on_or_before() {
        let store = Store::open_in_memory().unwrap();
        let mut s1 = make_run_state("r_dd_flt1");
        s1.due_date = Some("2026-01-15".into());
        let mut s2 = make_run_state("r_dd_flt2");
        s2.due_date = Some("2026-06-30".into());
        let s3 = make_run_state("r_dd_flt3"); // no due date
        store.save_run(&s1).unwrap();
        store.save_run(&s2).unwrap();
        store.save_run(&s3).unwrap();
        let (val, _) = dispatch(
            Method::RunsList,
            serde_json::json!({"limit": 50, "dueOnOrBefore": "2026-03-31"}),
            &store,
        ).unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();
        let ids: Vec<&str> = result.runs.iter().map(|r| r.run_id.as_str()).collect();
        assert!(ids.contains(&"r_dd_flt1"), "should include r_dd_flt1 (2026-01-15 ≤ threshold)");
        assert!(!ids.contains(&"r_dd_flt2"), "r_dd_flt2 (2026-06-30) exceeds threshold");
        assert!(!ids.contains(&"r_dd_flt3"), "r_dd_flt3 has no due date");
    }

    #[test]
    fn runs_list_sort_by_due_date() {
        let store = Store::open_in_memory().unwrap();
        let mut s1 = make_run_state("r_dd_sort1");
        s1.due_date = Some("2026-12-31".into());
        let mut s2 = make_run_state("r_dd_sort2");
        s2.due_date = Some("2026-01-01".into());
        let mut s3 = make_run_state("r_dd_sort3");
        s3.due_date = Some("2026-06-15".into());
        let s4 = make_run_state("r_dd_sort4"); // no due date — sorts last
        store.save_run(&s1).unwrap();
        store.save_run(&s2).unwrap();
        store.save_run(&s3).unwrap();
        store.save_run(&s4).unwrap();
        let (val, _) = dispatch(
            Method::RunsList,
            serde_json::json!({"limit": 50, "sortByDueDate": true}),
            &store,
        ).unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();
        let ids: Vec<&str> = result.runs.iter().map(|r| r.run_id.as_str()).collect();
        let pos1 = ids.iter().position(|&x| x == "r_dd_sort1").unwrap();
        let pos2 = ids.iter().position(|&x| x == "r_dd_sort2").unwrap();
        let pos3 = ids.iter().position(|&x| x == "r_dd_sort3").unwrap();
        let pos4 = ids.iter().position(|&x| x == "r_dd_sort4").unwrap();
        assert!(pos2 < pos3, "2026-01-01 should sort before 2026-06-15");
        assert!(pos3 < pos1, "2026-06-15 should sort before 2026-12-31");
        assert!(pos1 < pos4, "run with a date should sort before run with no date");
    }

    #[test]
    fn run_set_due_date_does_not_change_status() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_dd_status");
        state.status = "finalized:completed".into();
        store.save_run(&state).unwrap();
        let (val, _) = dispatch(
            Method::RunSetDueDate,
            serde_json::json!({"runId": "r_dd_status", "dueDate": "2026-07-04"}),
            &store,
        ).unwrap();
        let result: RunSetDueDateResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.status, "finalized:completed");
    }

    #[test]
    fn run_get_includes_due_date() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_dd_get");
        state.due_date = Some("2026-11-01".into());
        store.save_run(&state).unwrap();
        let (val, _) = dispatch(
            Method::RunGet,
            serde_json::json!({"runId": "r_dd_get"}),
            &store,
        ).unwrap();
        let result: RunGetResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.due_date.as_deref(), Some("2026-11-01"));
    }

    // ----- Milestone 21: run.set_dependencies -----

    #[test]
    fn run_set_dependencies_sets_blockers() {
        let store = Store::open_in_memory().unwrap();
        let run_a = make_run_state("r_dep_a");
        let run_b = make_run_state("r_dep_b");
        store.save_run(&run_a).unwrap();
        store.save_run(&run_b).unwrap();

        let (val, _) = dispatch(
            Method::RunSetDependencies,
            serde_json::json!({"runId": "r_dep_a", "blockedByRunIds": ["r_dep_b"]}),
            &store,
        )
        .unwrap();
        let result: RunSetDependenciesResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.blocked_by_run_ids, vec!["r_dep_b"]);
        assert!(result.previous_blocked_by_run_ids.is_empty());
    }

    #[test]
    fn run_set_dependencies_persists() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_dp_a")).unwrap();
        store.save_run(&make_run_state("r_dp_b")).unwrap();

        dispatch(
            Method::RunSetDependencies,
            serde_json::json!({"runId": "r_dp_a", "blockedByRunIds": ["r_dp_b"]}),
            &store,
        )
        .unwrap();

        let loaded = store.get_run("r_dp_a").unwrap().unwrap();
        assert_eq!(loaded.blocked_by_run_ids, vec!["r_dp_b"]);
    }

    #[test]
    fn run_set_dependencies_clears() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_dc_a");
        state.blocked_by_run_ids = vec!["r_dc_b".to_string()];
        store.save_run(&state).unwrap();
        store.save_run(&make_run_state("r_dc_b")).unwrap();

        let (val, _) = dispatch(
            Method::RunSetDependencies,
            serde_json::json!({"runId": "r_dc_a", "blockedByRunIds": []}),
            &store,
        )
        .unwrap();
        let result: RunSetDependenciesResult = serde_json::from_value(val).unwrap();
        assert!(result.blocked_by_run_ids.is_empty());
        assert_eq!(result.previous_blocked_by_run_ids, vec!["r_dc_b"]);

        let loaded = store.get_run("r_dc_a").unwrap().unwrap();
        assert!(loaded.blocked_by_run_ids.is_empty());
    }

    #[test]
    fn run_set_dependencies_rejects_self_dep() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_ds_a")).unwrap();

        let err = dispatch(
            Method::RunSetDependencies,
            serde_json::json!({"runId": "r_ds_a", "blockedByRunIds": ["r_ds_a"]}),
            &store,
        )
        .unwrap_err();
        assert!(err.to_string().contains("cannot depend on itself"), "{err}");
    }

    #[test]
    fn run_set_dependencies_rejects_unknown_id() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_du_a")).unwrap();

        let err = dispatch(
            Method::RunSetDependencies,
            serde_json::json!({"runId": "r_du_a", "blockedByRunIds": ["ghost-run"]}),
            &store,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown run ID"), "{err}");
    }

    #[test]
    fn run_set_dependencies_unknown_target_run_rejected() {
        let store = Store::open_in_memory().unwrap();

        let err = dispatch(
            Method::RunSetDependencies,
            serde_json::json!({"runId": "no-such-run", "blockedByRunIds": []}),
            &store,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown run"), "{err}");
    }

    #[test]
    fn run_set_dependencies_audit_entry() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_daud_a")).unwrap();
        store.save_run(&make_run_state("r_daud_b")).unwrap();

        dispatch(
            Method::RunSetDependencies,
            serde_json::json!({"runId": "r_daud_a", "blockedByRunIds": ["r_daud_b"]}),
            &store,
        )
        .unwrap();

        let entries = store.get_audit_entries("r_daud_a", 10).unwrap();
        assert!(!entries.is_empty());
        let last = entries.last().unwrap();
        assert_eq!(last.event_kind, "run_dependencies_set");
    }

    #[test]
    fn run_set_dependencies_deduplicates() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_ddd_a")).unwrap();
        store.save_run(&make_run_state("r_ddd_b")).unwrap();

        let (val, _) = dispatch(
            Method::RunSetDependencies,
            serde_json::json!({"runId": "r_ddd_a", "blockedByRunIds": ["r_ddd_b", "r_ddd_b"]}),
            &store,
        )
        .unwrap();
        let result: RunSetDependenciesResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.blocked_by_run_ids, vec!["r_ddd_b"]);
    }

    #[test]
    fn run_get_includes_blocked_by_run_ids() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_dbg_a");
        state.blocked_by_run_ids = vec!["r_dbg_b".to_string()];
        store.save_run(&state).unwrap();
        store.save_run(&make_run_state("r_dbg_b")).unwrap();

        let (val, _) = dispatch(
            Method::RunGet,
            serde_json::json!({"runId": "r_dbg_a"}),
            &store,
        )
        .unwrap();
        let result: RunGetResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.blocked_by_run_ids, vec!["r_dbg_b"]);
    }

    #[test]
    fn runs_list_blocked_only_filter() {
        let store = Store::open_in_memory().unwrap();
        let state_a = make_run_state("r_lbo_a");
        let mut state_b = make_run_state("r_lbo_b");
        state_b.blocked_by_run_ids = vec!["r_lbo_a".to_string()];
        store.save_run(&state_a).unwrap();
        store.save_run(&state_b).unwrap();

        let (val, _) = dispatch(
            Method::RunsList,
            serde_json::json!({"blockedOnly": true}),
            &store,
        )
        .unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.count, 1);
        assert_eq!(result.runs[0].run_id, "r_lbo_b");
    }

    #[test]
    fn runs_list_blocked_by_run_id_filter() {
        let store = Store::open_in_memory().unwrap();
        let state_a = make_run_state("r_lbbid_a");
        let mut state_b = make_run_state("r_lbbid_b");
        state_b.blocked_by_run_ids = vec!["r_lbbid_a".to_string()];
        let mut state_c = make_run_state("r_lbbid_c");
        state_c.blocked_by_run_ids = vec!["r_lbbid_a".to_string()];
        let state_d = make_run_state("r_lbbid_d"); // unblocked
        store.save_run(&state_a).unwrap();
        store.save_run(&state_b).unwrap();
        store.save_run(&state_c).unwrap();
        store.save_run(&state_d).unwrap();

        let (val, _) = dispatch(
            Method::RunsList,
            serde_json::json!({"blockedByRunId": "r_lbbid_a"}),
            &store,
        )
        .unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.count, 2);
        let ids: Vec<&str> = result.runs.iter().map(|r| r.run_id.as_str()).collect();
        assert!(ids.contains(&"r_lbbid_b"), "{ids:?}");
        assert!(ids.contains(&"r_lbbid_c"), "{ids:?}");
    }

    #[test]
    fn runs_list_summary_shows_is_blocked() {
        let store = Store::open_in_memory().unwrap();
        let state_a = make_run_state("r_lsib_a");
        let mut state_b = make_run_state("r_lsib_b");
        state_b.blocked_by_run_ids = vec!["r_lsib_a".to_string()];
        store.save_run(&state_a).unwrap();
        store.save_run(&state_b).unwrap();

        let (val, _) = dispatch(Method::RunsList, serde_json::json!({}), &store).unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();

        let summary_b = result.runs.iter().find(|r| r.run_id == "r_lsib_b").unwrap();
        assert_eq!(summary_b.is_blocked, Some(true));
        assert_eq!(summary_b.blocked_by_count, Some(1));

        let summary_a = result.runs.iter().find(|r| r.run_id == "r_lsib_a").unwrap();
        assert_eq!(summary_a.is_blocked, Some(false));
        assert_eq!(summary_a.blocked_by_count, Some(0));
    }

    // -----------------------------------------------------------------------
    // Milestone 23: blocker-impact fields
    // -----------------------------------------------------------------------

    #[test]
    fn runs_list_shows_is_blocking_and_blocking_run_count() {
        // r_ib_a blocks r_ib_b and r_ib_c.  r_ib_a should appear as "blocking 2 run(s)".
        let store = Store::open_in_memory().unwrap();
        let state_a = make_run_state("r_ib_a");
        let mut state_b = make_run_state("r_ib_b");
        let mut state_c = make_run_state("r_ib_c");
        state_b.blocked_by_run_ids = vec!["r_ib_a".to_string()];
        state_c.blocked_by_run_ids = vec!["r_ib_a".to_string()];
        store.save_run(&state_a).unwrap();
        store.save_run(&state_b).unwrap();
        store.save_run(&state_c).unwrap();

        let (val, _) = dispatch(Method::RunsList, serde_json::json!({}), &store).unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();

        let summary_a = result.runs.iter().find(|r| r.run_id == "r_ib_a").unwrap();
        assert_eq!(summary_a.is_blocking, Some(true));
        assert_eq!(summary_a.blocking_run_count, Some(2));
        assert_eq!(summary_a.blocking_reason, Some("blocking 2 run(s)".to_string()));

        // Runs that are not blocking any other run should have is_blocking=false.
        let summary_b = result.runs.iter().find(|r| r.run_id == "r_ib_b").unwrap();
        assert_eq!(summary_b.is_blocking, Some(false));
        assert_eq!(summary_b.blocking_run_count, Some(0));
        assert!(summary_b.blocking_reason.is_none());
    }

    #[test]
    fn runs_list_blocking_only_filter() {
        // Only runs that are blocking at least one other run should be returned when blocking_only=true.
        let store = Store::open_in_memory().unwrap();
        let state_a = make_run_state("r_lbof_a");
        let mut state_b = make_run_state("r_lbof_b");
        let state_c = make_run_state("r_lbof_c");
        state_b.blocked_by_run_ids = vec!["r_lbof_a".to_string()];
        store.save_run(&state_a).unwrap();
        store.save_run(&state_b).unwrap();
        store.save_run(&state_c).unwrap();

        let (val, _) = dispatch(
            Method::RunsList,
            serde_json::json!({"blockingOnly": true}),
            &store,
        )
        .unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.count, 1, "only the blocker run should appear");
        assert_eq!(result.runs[0].run_id, "r_lbof_a");
        assert_eq!(result.runs[0].is_blocking, Some(true));
    }

    #[test]
    fn runs_list_blocking_run_count_at_least_filter() {
        // r_lbca_a blocks both _b and _c (count=2); _d blocks only _e (count=1).
        let store = Store::open_in_memory().unwrap();
        let state_a = make_run_state("r_lbca_a");
        let mut state_b = make_run_state("r_lbca_b");
        let mut state_c = make_run_state("r_lbca_c");
        let state_d = make_run_state("r_lbca_d");
        let mut state_e = make_run_state("r_lbca_e");
        state_b.blocked_by_run_ids = vec!["r_lbca_a".to_string()];
        state_c.blocked_by_run_ids = vec!["r_lbca_a".to_string()];
        state_e.blocked_by_run_ids = vec!["r_lbca_d".to_string()];
        store.save_run(&state_a).unwrap();
        store.save_run(&state_b).unwrap();
        store.save_run(&state_c).unwrap();
        store.save_run(&state_d).unwrap();
        store.save_run(&state_e).unwrap();

        // blocking_run_count_at_least=2 should return only _a.
        let (val, _) = dispatch(
            Method::RunsList,
            serde_json::json!({"blockingRunCountAtLeast": 2}),
            &store,
        )
        .unwrap();
        let result: RunsListResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.count, 1);
        assert_eq!(result.runs[0].run_id, "r_lbca_a");
        assert_eq!(result.runs[0].blocking_run_count, Some(2));
    }

    #[test]
    fn run_get_includes_blocker_impact_fields() {
        // r_gib_a blocks r_gib_b; run.get on r_gib_a should reflect that.
        let store = Store::open_in_memory().unwrap();
        let state_a = make_run_state("r_gib_a");
        let mut state_b = make_run_state("r_gib_b");
        state_b.blocked_by_run_ids = vec!["r_gib_a".to_string()];
        store.save_run(&state_a).unwrap();
        store.save_run(&state_b).unwrap();

        let (val, _) = dispatch(
            Method::RunGet,
            serde_json::json!({"runId": "r_gib_a"}),
            &store,
        )
        .unwrap();
        let result: RunGetResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.is_blocking, Some(true));
        assert_eq!(result.blocking_run_count, Some(1));
        assert_eq!(result.blocking_reason, Some("blocking 1 run(s)".to_string()));
    }

    #[test]
    fn run_get_not_blocking_shows_false() {
        // A standalone run should not be marked as blocking.
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_gnb_a");
        store.save_run(&state).unwrap();

        let (val, _) = dispatch(
            Method::RunGet,
            serde_json::json!({"runId": "r_gnb_a"}),
            &store,
        )
        .unwrap();
        let result: RunGetResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.is_blocking, Some(false));
        assert_eq!(result.blocking_run_count, Some(0));
        assert!(result.blocking_reason.is_none());
    }

    #[test]
    fn blocker_impact_deterministic_count_derivation() {
        // A run blocking N others must report exactly N, deterministically.
        let store = Store::open_in_memory().unwrap();
        let blocker = make_run_state("r_bid_blocker");
        let mut dep1 = make_run_state("r_bid_dep1");
        let mut dep2 = make_run_state("r_bid_dep2");
        let mut dep3 = make_run_state("r_bid_dep3");
        dep1.blocked_by_run_ids = vec!["r_bid_blocker".to_string()];
        dep2.blocked_by_run_ids = vec!["r_bid_blocker".to_string()];
        dep3.blocked_by_run_ids = vec!["r_bid_blocker".to_string()];
        store.save_run(&blocker).unwrap();
        store.save_run(&dep1).unwrap();
        store.save_run(&dep2).unwrap();
        store.save_run(&dep3).unwrap();

        let impact_map = store.get_blocker_impact_map().unwrap();
        assert_eq!(impact_map.get("r_bid_blocker").copied().unwrap_or(0), 3);
        // Dependency runs themselves should not appear as blockers.
        assert_eq!(impact_map.get("r_bid_dep1").copied().unwrap_or(0), 0);
    }

    #[test]
    fn run_set_dependencies_does_not_mutate_status() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_dns_a");
        state.status = "finalized:completed".into();
        store.save_run(&state).unwrap();
        store.save_run(&make_run_state("r_dns_b")).unwrap();

        let (val, _) = dispatch(
            Method::RunSetDependencies,
            serde_json::json!({"runId": "r_dns_a", "blockedByRunIds": ["r_dns_b"]}),
            &store,
        )
        .unwrap();
        let result: RunSetDependenciesResult = serde_json::from_value(val).unwrap();
        assert_eq!(result.status, "finalized:completed");

        let loaded = store.get_run("r_dns_a").unwrap().unwrap();
        assert_eq!(loaded.status, "finalized:completed");
    }
}
