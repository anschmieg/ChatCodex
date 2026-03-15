//! Handler logic for `run.refresh`.
//!
//! Returns an updated snapshot of the run state.  This is a read-only
//! operation: it does **not** trigger actions or perform LLM reasoning.

use anyhow::Result;
use deterministic_protocol::{PendingApproval, RunRefreshParams, RunRefreshResult, RunState};

/// Refresh run state.
///
/// Merges persisted state with live workspace facts (git status, diff,
/// test results) to produce a consistent snapshot.  Pure and
/// deterministic — no side effects, no model calls.
pub fn refresh(
    _params: &RunRefreshParams,
    state: &RunState,
    pending_approvals: &[PendingApproval],
    live_diff_summary: Option<&str>,
) -> Result<RunRefreshResult> {
    let mut warnings = state.warnings.clone();

    // If there are pending approvals, ensure the status reflects that.
    if !pending_approvals.is_empty() && state.status != "awaiting_approval" {
        warnings.push("Run has pending approvals but status is not awaiting_approval".into());
    }

    // If status is done or failed, note it.
    if state.status == "failed" {
        warnings.push("Run is in a failed state — consider replanning".into());
    }

    // Surface retryable action staleness warnings (Milestone 6).
    if let Some(ref ra) = state.retryable_action
        && !ra.is_valid
    {
        if let Some(ref reason) = ra.invalidation_reason {
            warnings.push(format!("Retryable action '{}' is stale: {}", ra.kind, reason));
        } else {
            warnings.push(format!("Retryable action '{}' is no longer valid", ra.kind));
        }
    }

    Ok(RunRefreshResult {
        run_id: state.run_id.clone(),
        status: state.status.clone(),
        current_step: state.current_step,
        completed_steps: state.completed_steps.clone(),
        pending_steps: state.pending_steps.clone(),
        last_action: state.last_action.clone(),
        last_observation: state.last_observation.clone(),
        recommended_next_action: state.recommended_next_action.clone(),
        recommended_tool: state.recommended_tool.clone(),
        pending_approvals: pending_approvals.to_vec(),
        latest_diff_summary: live_diff_summary
            .map(String::from)
            .or_else(|| state.latest_diff_summary.clone()),
        latest_test_result: state.latest_test_result.clone(),
        retryable_action: state.retryable_action.clone(),
        warnings,
        effective_policy: state.policy_profile.clone(),
        finalized_outcome: state.finalized_outcome.clone(),
        reopen_metadata: state.reopen_metadata.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state(status: &str) -> RunState {
        RunState {
            run_id: "r1".into(),
            workspace_id: "/tmp/ws".into(),
            user_goal: "fix bug".into(),
            status: status.into(),
            plan: vec!["step 1".into(), "step 2".into()],
            current_step: 0,
            completed_steps: vec![],
            pending_steps: vec!["step 1".into(), "step 2".into()],
            last_action: None,
            last_observation: None,
            recommended_next_action: Some("inspect".into()),
            recommended_tool: Some("get_workspace_summary".into()),
            latest_diff_summary: None,
            latest_test_result: None,
            focus_paths: vec![],
            warnings: vec![],
            retryable_action: None,
            policy_profile: deterministic_protocol::RunPolicy::default(),
            finalized_outcome: None,
            reopen_metadata: None,
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn refresh_returns_consistent_snapshot() {
        let state = make_state("active");
        let params = RunRefreshParams {
            run_id: "r1".into(),
        };
        let result = refresh(&params, &state, &[], None).unwrap();
        assert_eq!(result.run_id, "r1");
        assert_eq!(result.status, "active");
        assert_eq!(result.pending_steps.len(), 2);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn refresh_warns_on_pending_approvals_mismatch() {
        let state = make_state("active");
        let approval = PendingApproval {
            approval_id: "a1".into(),
            run_id: "r1".into(),
            action_description: "delete file".into(),
            risk_reason: "destructive".into(),
            policy_rationale: "Policy: file deletion requires approval".into(),
            status: "pending".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
        };
        let params = RunRefreshParams {
            run_id: "r1".into(),
        };
        let result = refresh(&params, &state, &[approval], None).unwrap();
        assert!(!result.warnings.is_empty());
        assert!(result.warnings[0].contains("pending approvals"));
    }

    #[test]
    fn refresh_includes_live_diff() {
        let state = make_state("active");
        let params = RunRefreshParams {
            run_id: "r1".into(),
        };
        let result = refresh(&params, &state, &[], Some("3 files changed")).unwrap();
        assert_eq!(result.latest_diff_summary.as_deref(), Some("3 files changed"));
    }

    #[test]
    fn refresh_warns_on_failed_state() {
        let state = make_state("failed");
        let params = RunRefreshParams {
            run_id: "r1".into(),
        };
        let result = refresh(&params, &state, &[], None).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("failed")));
    }

    // ---- Milestone 6: retryable action in refresh ----

    #[test]
    fn refresh_surfaces_valid_retryable_action() {
        let mut state = make_state("active");
        state.retryable_action = Some(deterministic_protocol::RetryableAction {
            kind: "patch.apply".into(),
            summary: "Edit main.rs".into(),
            payload: None,
            retryable_reason: "Blocked by policy".into(),
            is_valid: true,
            is_recommended: true,
            invalidation_reason: None,
            recommended_tool: "apply_patch".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
        });
        let params = RunRefreshParams { run_id: "r1".into() };
        let result = refresh(&params, &state, &[], None).unwrap();
        let ra = result.retryable_action.as_ref().unwrap();
        assert!(ra.is_valid);
        assert!(ra.is_recommended);
        assert_eq!(ra.kind, "patch.apply");
        // No staleness warning.
        assert!(result.warnings.iter().all(|w| !w.contains("stale")));
    }

    #[test]
    fn refresh_warns_on_stale_retryable_action() {
        let mut state = make_state("active");
        state.retryable_action = Some(deterministic_protocol::RetryableAction {
            kind: "tests.run".into(),
            summary: "Run unit tests".into(),
            payload: None,
            retryable_reason: "Blocked by policy".into(),
            is_valid: false,
            is_recommended: false,
            invalidation_reason: Some("Invalidated by replan".into()),
            recommended_tool: "run_tests".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
        });
        let params = RunRefreshParams { run_id: "r1".into() };
        let result = refresh(&params, &state, &[], None).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("stale")));
        assert!(result.warnings.iter().any(|w| w.contains("tests.run")));
    }

    #[test]
    fn refresh_warns_on_invalid_retryable_action_without_reason() {
        let mut state = make_state("active");
        state.retryable_action = Some(deterministic_protocol::RetryableAction {
            kind: "patch.apply".into(),
            summary: "Edit".into(),
            payload: None,
            retryable_reason: "Blocked".into(),
            is_valid: false,
            is_recommended: false,
            invalidation_reason: None,
            recommended_tool: "apply_patch".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
        });
        let params = RunRefreshParams { run_id: "r1".into() };
        let result = refresh(&params, &state, &[], None).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("no longer valid")));
    }

    // ---- Milestone 8: effective_policy in refresh ----

    #[test]
    fn refresh_surfaces_default_effective_policy() {
        let state = make_state("active");
        let params = RunRefreshParams { run_id: "r1".into() };
        let result = refresh(&params, &state, &[], None).unwrap();
        let defaults = deterministic_protocol::RunPolicy::default();
        assert_eq!(result.effective_policy.patch_edit_threshold, defaults.patch_edit_threshold);
        assert_eq!(result.effective_policy.delete_requires_approval, defaults.delete_requires_approval);
    }

    #[test]
    fn refresh_surfaces_custom_effective_policy() {
        let mut state = make_state("active");
        state.policy_profile = deterministic_protocol::RunPolicy {
            patch_edit_threshold: 15,
            delete_requires_approval: false,
            ..Default::default()
        };
        let params = RunRefreshParams { run_id: "r1".into() };
        let result = refresh(&params, &state, &[], None).unwrap();
        assert_eq!(result.effective_policy.patch_edit_threshold, 15);
        assert!(!result.effective_policy.delete_requires_approval);
    }
}
