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
    }
}

fn handle_run_prepare(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunPrepareParams = serde_json::from_value(params)?;
    let (result, state) = deterministic_core::run_prepare::prepare(&p)?;
    store.save_run(&state)?;
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
        deterministic_core::approval_policy::evaluate_patch(&p, &state.focus_paths);

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
            store.save_approval(&approval)?;
            store.save_run(&state)?;
            let result = PatchApplyResult {
                changed_files: vec![],
                diff_stats: String::new(),
                approval_required: Some(approval),
            };
            Ok((serde_json::to_value(result)?, Some(state)))
        }
        deterministic_core::approval_policy::PolicyDecision::Proceed => {
            let result = deterministic_core::patch_apply::apply(&p, &ws)?;
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
    let decision = deterministic_core::approval_policy::evaluate_test_run(&p);

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
            store.save_approval(&approval)?;
            store.save_run(&state)?;
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
            let result = deterministic_core::tests_run::run(&p, &ws)?;
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

    Ok((serde_json::to_value(result)?, Some(state)))
}
