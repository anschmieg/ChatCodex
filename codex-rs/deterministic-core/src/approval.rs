//! Handler logic for `approval.resolve`.
//!
//! Deterministic approval resolution.  Pending approvals can be
//! approved or denied, causing the run to transition accordingly.
//! No LLM calls, no autonomous continuation.

use anyhow::Result;
use deterministic_protocol::{ApprovalResolveParams, ApprovalResolveResult, PendingApproval, RunState};

/// Create a pending approval and transition the run to `awaiting_approval`.
///
/// Called internally by daemon handlers when a risky operation is
/// detected (e.g. destructive file operations, patches outside
/// allowed expectations).
pub fn create_approval(
    state: &mut RunState,
    action_description: &str,
    risk_reason: &str,
    policy_rationale: &str,
) -> PendingApproval {
    let now = chrono::Utc::now().to_rfc3339();
    let approval_id = format!("appr_{}", uuid::Uuid::new_v4());

    state.status = "awaiting_approval".to_string();
    state.recommended_next_action = Some(format!(
        "Approval required: {action_description}. Use approve_action to approve or deny.",
    ));
    state.recommended_tool = Some("approve_action".to_string());
    state.updated_at = now.clone();

    PendingApproval {
        approval_id,
        run_id: state.run_id.clone(),
        action_description: action_description.to_string(),
        risk_reason: risk_reason.to_string(),
        policy_rationale: policy_rationale.to_string(),
        status: "pending".to_string(),
        created_at: now,
    }
}

/// Resolve a pending approval.
///
/// - `"approve"` → transitions the run back to `active` if no more
///   pending approvals remain.
/// - `"deny"` → transitions the run to `blocked`.
///
/// Returns a summary of what happened, including recommended next
/// action after the decision.
pub fn resolve(
    params: &ApprovalResolveParams,
    state: &mut RunState,
    remaining_pending_count: usize,
) -> Result<ApprovalResolveResult> {
    let now = chrono::Utc::now().to_rfc3339();

    let (new_status, summary, rec_action, rec_tool) = match params.decision.as_str() {
        "approve" => {
            if remaining_pending_count == 0 {
                // This was the last pending approval — unblock the run.
                state.status = "active".to_string();
                state.recommended_next_action =
                    Some("Retry the previously gated action.".to_string());
                state.recommended_tool = Some("refresh_run_state".to_string());
                (
                    "active".to_string(),
                    "Approval granted; run unblocked.".to_string(),
                    Some("Retry the previously gated action.".to_string()),
                    Some("refresh_run_state".to_string()),
                )
            } else {
                state.recommended_next_action = Some(format!(
                    "Resolve remaining {remaining_pending_count} pending approval(s)."
                ));
                state.recommended_tool = Some("approve_action".to_string());
                (
                    "awaiting_approval".to_string(),
                    format!(
                        "Approval granted; {remaining_pending_count} approval(s) still pending."
                    ),
                    Some(format!(
                        "Resolve remaining {remaining_pending_count} pending approval(s)."
                    )),
                    Some("approve_action".to_string()),
                )
            }
        }
        "deny" => {
            state.status = "blocked".to_string();
            state.recommended_next_action =
                Some("Replan the run to work around the denied action.".to_string());
            state.recommended_tool = Some("replan_run".to_string());
            (
                "blocked".to_string(),
                format!(
                    "Approval denied{}; run blocked.",
                    params
                        .reason
                        .as_deref()
                        .map(|r| format!(": {r}"))
                        .unwrap_or_default()
                ),
                Some("Replan the run to work around the denied action.".to_string()),
                Some("replan_run".to_string()),
            )
        }
        other => {
            return Err(anyhow::anyhow!(
                "invalid decision '{other}': must be 'approve' or 'deny'"
            ));
        }
    };

    state.updated_at = now;

    Ok(ApprovalResolveResult {
        approval_id: params.approval_id.clone(),
        run_id: params.run_id.clone(),
        decision: params.decision.clone(),
        status: new_status,
        summary,
        recommended_next_action: rec_action,
        recommended_tool: rec_tool,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state() -> RunState {
        RunState {
            run_id: "r1".into(),
            workspace_id: "/tmp/ws".into(),
            user_goal: "fix bug".into(),
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
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn create_approval_transitions_to_awaiting() {
        let mut state = make_state();
        let approval = create_approval(
            &mut state,
            "delete file",
            "destructive op",
            "Policy: file deletion requires approval",
        );
        assert_eq!(state.status, "awaiting_approval");
        assert_eq!(approval.status, "pending");
        assert!(approval.approval_id.starts_with("appr_"));
        assert_eq!(approval.policy_rationale, "Policy: file deletion requires approval");
        assert!(state.recommended_tool.as_deref() == Some("approve_action"));
    }

    #[test]
    fn resolve_approve_unblocks_run() {
        let mut state = make_state();
        state.status = "awaiting_approval".into();
        let params = ApprovalResolveParams {
            run_id: "r1".into(),
            approval_id: "a1".into(),
            decision: "approve".into(),
            reason: None,
        };
        let result = resolve(&params, &mut state, 0).unwrap();
        assert_eq!(result.status, "active");
        assert_eq!(state.status, "active");
        assert!(result.summary.contains("unblocked"));
        assert!(result.recommended_next_action.is_some());
        assert!(result.recommended_tool.is_some());
    }

    #[test]
    fn resolve_approve_with_remaining() {
        let mut state = make_state();
        state.status = "awaiting_approval".into();
        let params = ApprovalResolveParams {
            run_id: "r1".into(),
            approval_id: "a1".into(),
            decision: "approve".into(),
            reason: None,
        };
        let result = resolve(&params, &mut state, 2).unwrap();
        assert_eq!(result.status, "awaiting_approval");
        assert!(result.summary.contains("still pending"));
        assert_eq!(result.recommended_tool.as_deref(), Some("approve_action"));
    }

    #[test]
    fn resolve_deny_blocks_run() {
        let mut state = make_state();
        state.status = "awaiting_approval".into();
        let params = ApprovalResolveParams {
            run_id: "r1".into(),
            approval_id: "a1".into(),
            decision: "deny".into(),
            reason: Some("too risky".into()),
        };
        let result = resolve(&params, &mut state, 0).unwrap();
        assert_eq!(result.status, "blocked");
        assert_eq!(state.status, "blocked");
        assert!(result.summary.contains("denied"));
        assert!(result.summary.contains("too risky"));
        assert_eq!(result.recommended_tool.as_deref(), Some("replan_run"));
        assert!(result.recommended_next_action.as_deref().unwrap().contains("Replan"));
    }

    #[test]
    fn resolve_invalid_decision_fails() {
        let mut state = make_state();
        let params = ApprovalResolveParams {
            run_id: "r1".into(),
            approval_id: "a1".into(),
            decision: "maybe".into(),
            reason: None,
        };
        let result = resolve(&params, &mut state, 0);
        assert!(result.is_err());
    }
}
