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
    let runs = store.list_runs(
        limit,
        p.workspace_id.as_deref(),
        p.status.as_deref(),
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
}
