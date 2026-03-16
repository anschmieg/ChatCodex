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
///   pending approvals remain.  If a retryable action is recorded,
///   marks it as recommended so ChatGPT knows to retry.
/// - `"deny"` → transitions the run to `blocked`.  Invalidates any
///   retryable action so ChatGPT knows not to retry unchanged.
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
            // Mark retryable action as recommended if present and valid.
            if let Some(ref mut ra) = state.retryable_action
                && ra.is_valid
            {
                ra.is_recommended = true;
            }

            if remaining_pending_count == 0 {
                // This was the last pending approval — unblock the run.
                state.status = "active".to_string();

                // Use the retryable action's tool if available.
                let (action_text, tool) = if let Some(ref ra) = state.retryable_action {
                    if ra.is_valid {
                        (
                            format!("Retry the approved action: {}", ra.summary),
                            ra.recommended_tool.clone(),
                        )
                    } else {
                        (
                            format!(
                                "Previously gated action is no longer valid{}. Consider replanning.",
                                ra.invalidation_reason
                                    .as_deref()
                                    .map(|r| format!(": {r}"))
                                    .unwrap_or_default()
                            ),
                            "replan_run".to_string(),
                        )
                    }
                } else {
                    (
                        "Retry the previously gated action.".to_string(),
                        "refresh_run_state".to_string(),
                    )
                };

                state.recommended_next_action = Some(action_text.clone());
                state.recommended_tool = Some(tool.clone());
                (
                    "active".to_string(),
                    "Approval granted; run unblocked.".to_string(),
                    Some(action_text),
                    Some(tool),
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
            // Invalidate retryable action on denial.
            if let Some(ref mut ra) = state.retryable_action {
                ra.is_valid = false;
                ra.is_recommended = false;
                ra.invalidation_reason = Some(format!(
                    "Action denied{}",
                    params
                        .reason
                        .as_deref()
                        .map(|r| format!(": {r}"))
                        .unwrap_or_default()
                ));
            }

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
        retryable_action: state.retryable_action.clone(),
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
            retryable_action: None,
            policy_profile: deterministic_protocol::RunPolicy::default(),
            finalized_outcome: None,
            reopen_metadata: None,
            supersedes_run_id: None,
            superseded_by_run_id: None,
            supersession_reason: None,
            superseded_at: None,
            archive_metadata: None,
            unarchive_metadata: None,
            annotation: None,
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

    // ---- Milestone 6: retryable action guidance tests ----

    fn make_retryable_action() -> deterministic_protocol::RetryableAction {
        deterministic_protocol::RetryableAction {
            kind: "patch.apply".into(),
            summary: "Edit src/main.rs".into(),
            payload: Some(r#"{"run_id":"r1","edits":[]}"#.into()),
            retryable_reason: "Blocked by approval policy".into(),
            is_valid: true,
            is_recommended: false,
            invalidation_reason: None,
            recommended_tool: "apply_patch".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn approve_with_retryable_action_recommends_retry() {
        let mut state = make_state();
        state.status = "awaiting_approval".into();
        state.retryable_action = Some(make_retryable_action());

        let params = ApprovalResolveParams {
            run_id: "r1".into(),
            approval_id: "a1".into(),
            decision: "approve".into(),
            reason: None,
        };
        let result = resolve(&params, &mut state, 0).unwrap();
        assert_eq!(result.status, "active");
        // Retryable action should be recommended.
        let ra = result.retryable_action.as_ref().unwrap();
        assert!(ra.is_valid);
        assert!(ra.is_recommended);
        assert_eq!(ra.recommended_tool, "apply_patch");
        // State recommended tool should point at the retryable action's tool.
        assert_eq!(state.recommended_tool.as_deref(), Some("apply_patch"));
        assert!(state.recommended_next_action.as_deref().unwrap().contains("Retry"));
    }

    #[test]
    fn approve_with_invalid_retryable_action_recommends_replan() {
        let mut state = make_state();
        state.status = "awaiting_approval".into();
        let mut ra = make_retryable_action();
        ra.is_valid = false;
        ra.invalidation_reason = Some("superseded by replan".into());
        state.retryable_action = Some(ra);

        let params = ApprovalResolveParams {
            run_id: "r1".into(),
            approval_id: "a1".into(),
            decision: "approve".into(),
            reason: None,
        };
        let result = resolve(&params, &mut state, 0).unwrap();
        assert_eq!(result.status, "active");
        assert_eq!(state.recommended_tool.as_deref(), Some("replan_run"));
        assert!(state.recommended_next_action.as_deref().unwrap().contains("no longer valid"));
    }

    #[test]
    fn deny_invalidates_retryable_action() {
        let mut state = make_state();
        state.status = "awaiting_approval".into();
        state.retryable_action = Some(make_retryable_action());

        let params = ApprovalResolveParams {
            run_id: "r1".into(),
            approval_id: "a1".into(),
            decision: "deny".into(),
            reason: Some("too risky".into()),
        };
        let result = resolve(&params, &mut state, 0).unwrap();
        assert_eq!(result.status, "blocked");
        let ra = result.retryable_action.as_ref().unwrap();
        assert!(!ra.is_valid);
        assert!(!ra.is_recommended);
        assert!(ra.invalidation_reason.as_deref().unwrap().contains("denied"));
        assert_eq!(state.recommended_tool.as_deref(), Some("replan_run"));
    }

    #[test]
    fn approve_without_retryable_action_uses_defaults() {
        let mut state = make_state();
        state.status = "awaiting_approval".into();
        // No retryable_action set.

        let params = ApprovalResolveParams {
            run_id: "r1".into(),
            approval_id: "a1".into(),
            decision: "approve".into(),
            reason: None,
        };
        let result = resolve(&params, &mut state, 0).unwrap();
        assert_eq!(result.status, "active");
        assert!(result.retryable_action.is_none());
        assert_eq!(state.recommended_tool.as_deref(), Some("refresh_run_state"));
    }

    #[test]
    fn approve_with_remaining_still_awaiting() {
        let mut state = make_state();
        state.status = "awaiting_approval".into();
        state.retryable_action = Some(make_retryable_action());

        let params = ApprovalResolveParams {
            run_id: "r1".into(),
            approval_id: "a1".into(),
            decision: "approve".into(),
            reason: None,
        };
        let result = resolve(&params, &mut state, 1).unwrap();
        assert_eq!(result.status, "awaiting_approval");
        // Retryable action is marked recommended but can't be retried yet.
        let ra = result.retryable_action.as_ref().unwrap();
        assert!(ra.is_valid);
        assert!(ra.is_recommended);
        assert_eq!(state.recommended_tool.as_deref(), Some("approve_action"));
    }
}
