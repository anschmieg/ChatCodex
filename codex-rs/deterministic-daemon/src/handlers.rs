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
    let runs = store.list_runs(
        limit,
        p.workspace_id.as_deref(),
        p.status.as_deref(),
        include_archived,
        archived_only,
    )?;
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
        Some(&format!("{{\"reason\":\"{}\"}}", p.reason)),
    );

    Ok((serde_json::to_value(result)?, Some(state)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::Store;
    use deterministic_protocol::RunPolicy;

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
}
