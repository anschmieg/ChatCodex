//! Handler logic for `run.supersede`.
//!
//! Provides a deterministic, explicit supersession surface for ChatGPT to
//! replace a finalized run with a new successor run.  Only finalized runs may
//! be superseded.  Supersession is audited, does not execute work, and
//! preserves the full history of the original run.

use anyhow::{bail, Result};
use deterministic_protocol::{RunState, RunSupersedeParams, RunSupersedeResult};

/// Supersede a finalized run by creating a new successor run.
///
/// Deterministic lifecycle rules:
/// - Only finalized runs (`status` starts with `"finalized:"`) may be superseded.
/// - Active, prepared, or awaiting-approval runs are rejected.
/// - Supersession does not execute work or trigger autonomous follow-up.
/// - The original run is marked with `superseded_by_run_id`; the successor
///   run is created with `supersedes_run_id` pointing to the original.
/// - Prior audit history on the original run is preserved.
/// - The successor run starts in `"prepared"` status.
///
/// Returns the updated original state and the newly created successor state.
pub fn supersede(
    params: &RunSupersedeParams,
    original_state: &mut RunState,
    new_run_id: &str,
) -> Result<(RunSupersedeResult, RunState)> {
    // Enforce: only finalized runs can be superseded.
    if !original_state.status.starts_with("finalized:") {
        bail!(
            "run '{}' cannot be superseded: status is '{}' (only finalized runs may be superseded)",
            params.run_id,
            original_state.status
        );
    }

    let now = chrono::Utc::now().to_rfc3339();

    // Determine the goal for the successor run.
    let successor_goal = params
        .new_user_goal
        .as_deref()
        .filter(|g| !g.is_empty())
        .unwrap_or(&original_state.user_goal)
        .to_string();

    // Mark the original run as superseded (it remains finalized, history intact).
    original_state.superseded_by_run_id = Some(new_run_id.to_string());
    original_state.supersession_reason = Some(params.reason.clone());
    original_state.superseded_at = Some(now.clone());
    original_state.updated_at = now.clone();

    // Build the successor run in `prepared` status.
    let successor = RunState {
        run_id: new_run_id.to_string(),
        workspace_id: original_state.workspace_id.clone(),
        user_goal: successor_goal,
        status: "prepared".to_string(),
        plan: vec![],
        current_step: 0,
        completed_steps: vec![],
        pending_steps: vec![],
        last_action: None,
        last_observation: None,
        recommended_next_action: Some(
            "Successor run prepared. Call refresh_run_state to inspect its initial state, then replan_run to define the new plan.".to_string(),
        ),
        recommended_tool: Some("refresh_run_state".to_string()),
        latest_diff_summary: None,
        latest_test_result: None,
        focus_paths: original_state.focus_paths.clone(),
        warnings: vec![],
        retryable_action: None,
        policy_profile: original_state.policy_profile.clone(),
        finalized_outcome: None,
        reopen_metadata: None,
        supersedes_run_id: Some(params.run_id.clone()),
        superseded_by_run_id: None,
        supersession_reason: Some(params.reason.clone()),
        superseded_at: Some(now.clone()),
        archive_metadata: None,
        unarchive_metadata: None,
        annotation: None,
        pin_metadata: None,
        snooze_metadata: None,
        priority: original_state.priority,
        assignee: None,
        ownership_note: None,
        due_date: None,
        blocked_by_run_ids: vec![],
        created_at: now.clone(),
        updated_at: now.clone(),
    };

    let result = RunSupersedeResult {
        original_run_id: params.run_id.clone(),
        successor_run_id: new_run_id.to_string(),
        superseded_at: now,
        successor_status: successor.status.clone(),
        recommended_next_action:
            "Supersession complete. Call refresh_run_state on the successor run to inspect its initial state, then replan_run to define the new plan."
                .to_string(),
        recommended_tool: "refresh_run_state".to_string(),
    };

    Ok((result, successor))
}

/// Generate a new unique run ID for a successor.
///
/// Kept here so the core crate controls the format; the daemon passes it in.
pub fn make_successor_run_id(original_run_id: &str) -> String {
    let ts = chrono::Utc::now().timestamp_millis();
    format!("{original_run_id}-successor-{ts}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use deterministic_protocol::{RunOutcome, RunPolicy};

    fn make_active_state(run_id: &str) -> RunState {
        RunState {
            run_id: run_id.into(),
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

    fn finalize_state(state: &mut RunState, outcome_kind: &str) {
        state.status = format!("finalized:{outcome_kind}");
        state.finalized_outcome = Some(RunOutcome {
            outcome_kind: outcome_kind.into(),
            summary: "Done".into(),
            reason: None,
            finalized_at: "2024-01-01T01:00:00Z".into(),
        });
    }

    // -----------------------------------------------------------------------
    // Happy-path: supersede a completed run with inherited goal
    // -----------------------------------------------------------------------

    #[test]
    fn supersede_completed_run_inherits_goal() {
        let mut original = make_active_state("run-orig");
        finalize_state(&mut original, "completed");

        let params = RunSupersedeParams {
            run_id: "run-orig".into(),
            new_user_goal: None,
            reason: "Scope changed after completion".into(),
        };
        let (result, successor) = supersede(&params, &mut original, "run-new").unwrap();

        // Result fields
        assert_eq!(result.original_run_id, "run-orig");
        assert_eq!(result.successor_run_id, "run-new");
        assert_eq!(result.successor_status, "prepared");
        assert!(!result.superseded_at.is_empty());
        assert_eq!(result.recommended_tool, "refresh_run_state");

        // Original run: still finalized, now carries superseded_by
        assert!(original.status.starts_with("finalized:"));
        assert_eq!(original.superseded_by_run_id.as_deref(), Some("run-new"));
        assert_eq!(
            original.supersession_reason.as_deref(),
            Some("Scope changed after completion")
        );
        assert!(original.superseded_at.is_some());
        // Original run's finalized outcome is preserved
        assert!(original.finalized_outcome.is_some());

        // Successor run
        assert_eq!(successor.run_id, "run-new");
        assert_eq!(successor.status, "prepared");
        assert_eq!(successor.user_goal, "fix bug"); // inherited
        assert_eq!(successor.supersedes_run_id.as_deref(), Some("run-orig"));
        assert!(successor.superseded_by_run_id.is_none());
        assert_eq!(
            successor.supersession_reason.as_deref(),
            Some("Scope changed after completion")
        );
        assert!(successor.superseded_at.is_some());
        assert!(successor.finalized_outcome.is_none());
        assert!(successor.reopen_metadata.is_none());
    }

    // -----------------------------------------------------------------------
    // Happy-path: supersede a failed run with a new goal
    // -----------------------------------------------------------------------

    #[test]
    fn supersede_failed_run_with_new_goal() {
        let mut original = make_active_state("run-fail");
        finalize_state(&mut original, "failed");

        let params = RunSupersedeParams {
            run_id: "run-fail".into(),
            new_user_goal: Some("fix bug with different approach".into()),
            reason: "Previous approach failed; trying fresh".into(),
        };
        let (result, successor) = supersede(&params, &mut original, "run-v2").unwrap();

        assert_eq!(result.original_run_id, "run-fail");
        assert_eq!(result.successor_run_id, "run-v2");
        assert_eq!(successor.user_goal, "fix bug with different approach");
        assert_eq!(successor.supersedes_run_id.as_deref(), Some("run-fail"));
    }

    // -----------------------------------------------------------------------
    // Happy-path: supersede an abandoned run
    // -----------------------------------------------------------------------

    #[test]
    fn supersede_abandoned_run_succeeds() {
        let mut original = make_active_state("run-abn");
        finalize_state(&mut original, "abandoned");

        let params = RunSupersedeParams {
            run_id: "run-abn".into(),
            new_user_goal: None,
            reason: "Picking back up with fresh context".into(),
        };
        let (result, successor) = supersede(&params, &mut original, "run-abn-v2").unwrap();

        assert_eq!(result.successor_run_id, "run-abn-v2");
        assert_eq!(successor.supersedes_run_id.as_deref(), Some("run-abn"));
        assert_eq!(
            original.superseded_by_run_id.as_deref(),
            Some("run-abn-v2")
        );
    }

    // -----------------------------------------------------------------------
    // Reject: active run cannot be superseded
    // -----------------------------------------------------------------------

    #[test]
    fn supersede_active_run_rejected() {
        let mut original = make_active_state("run-act");
        // Status is "active" — not finalized.

        let params = RunSupersedeParams {
            run_id: "run-act".into(),
            new_user_goal: None,
            reason: "trying anyway".into(),
        };
        let err = supersede(&params, &mut original, "new-id").unwrap_err();
        assert!(err.to_string().contains("cannot be superseded"));
        assert!(err.to_string().contains("active"));
        // State must not be mutated.
        assert_eq!(original.status, "active");
        assert!(original.superseded_by_run_id.is_none());
    }

    // -----------------------------------------------------------------------
    // Reject: prepared run cannot be superseded
    // -----------------------------------------------------------------------

    #[test]
    fn supersede_prepared_run_rejected() {
        let mut original = make_active_state("run-prep");
        original.status = "prepared".into();

        let params = RunSupersedeParams {
            run_id: "run-prep".into(),
            new_user_goal: None,
            reason: "trying".into(),
        };
        let err = supersede(&params, &mut original, "new-id").unwrap_err();
        assert!(err.to_string().contains("cannot be superseded"));
        assert_eq!(original.status, "prepared");
    }

    // -----------------------------------------------------------------------
    // Successor inherits workspace and policy from original
    // -----------------------------------------------------------------------

    #[test]
    fn successor_inherits_workspace_and_policy() {
        let mut original = make_active_state("run-ws");
        original.workspace_id = "/projects/my-repo".into();
        original.policy_profile = RunPolicy {
            patch_edit_threshold: 3,
            ..RunPolicy::default()
        };
        original.focus_paths = vec!["src/".into(), "tests/".into()];
        finalize_state(&mut original, "completed");

        let params = RunSupersedeParams {
            run_id: "run-ws".into(),
            new_user_goal: None,
            reason: "New iteration".into(),
        };
        let (_result, successor) = supersede(&params, &mut original, "run-ws-v2").unwrap();

        assert_eq!(successor.workspace_id, "/projects/my-repo");
        assert_eq!(successor.policy_profile.patch_edit_threshold, 3);
        assert_eq!(successor.focus_paths, vec!["src/", "tests/"]);
    }

    #[test]
    fn successor_inherits_priority() {
        let mut original = make_active_state("run-prio");
        original.priority = deterministic_protocol::RunPriority::Urgent;
        finalize_state(&mut original, "completed");

        let params = RunSupersedeParams {
            run_id: "run-prio".into(),
            new_user_goal: None,
            reason: "carry urgency forward".into(),
        };
        let (_result, successor) = supersede(&params, &mut original, "run-prio-v2").unwrap();
        assert_eq!(
            successor.priority,
            deterministic_protocol::RunPriority::Urgent
        );
    }

    // -----------------------------------------------------------------------
    // Original audit history is preserved (plan and completed steps intact)
    // -----------------------------------------------------------------------

    #[test]
    fn supersede_preserves_original_history() {
        let mut original = make_active_state("run-hist");
        original.plan = vec!["step A".into(), "step B".into()];
        original.completed_steps = vec!["step A".into()];
        finalize_state(&mut original, "completed");

        let params = RunSupersedeParams {
            run_id: "run-hist".into(),
            new_user_goal: None,
            reason: "Continue work".into(),
        };
        supersede(&params, &mut original, "run-hist-v2").unwrap();

        // Original plan and completed steps must not be cleared.
        assert_eq!(original.plan, vec!["step A", "step B"]);
        assert_eq!(original.completed_steps, vec!["step A"]);
        // Finalized outcome must still be present.
        assert!(original.finalized_outcome.is_some());
    }

    // -----------------------------------------------------------------------
    // Successor starts with an empty plan (fresh start)
    // -----------------------------------------------------------------------

    #[test]
    fn successor_starts_with_empty_plan() {
        let mut original = make_active_state("run-plan");
        original.plan = vec!["step 1".into(), "step 2".into()];
        original.completed_steps = vec!["step 1".into()];
        finalize_state(&mut original, "failed");

        let params = RunSupersedeParams {
            run_id: "run-plan".into(),
            new_user_goal: None,
            reason: "Fresh start".into(),
        };
        let (_result, successor) = supersede(&params, &mut original, "run-plan-v2").unwrap();

        assert!(successor.plan.is_empty());
        assert!(successor.completed_steps.is_empty());
        assert!(successor.pending_steps.is_empty());
        assert_eq!(successor.current_step, 0);
    }

    // -----------------------------------------------------------------------
    // Empty new_user_goal falls back to original goal
    // -----------------------------------------------------------------------

    #[test]
    fn empty_new_goal_inherits_original() {
        let mut original = make_active_state("run-goal");
        original.user_goal = "original goal".into();
        finalize_state(&mut original, "completed");

        let params = RunSupersedeParams {
            run_id: "run-goal".into(),
            new_user_goal: Some("".into()), // empty string → inherit
            reason: "reason".into(),
        };
        let (_result, successor) = supersede(&params, &mut original, "run-goal-v2").unwrap();
        assert_eq!(successor.user_goal, "original goal");
    }

    // -----------------------------------------------------------------------
    // make_successor_run_id produces a non-empty string
    // -----------------------------------------------------------------------

    #[test]
    fn make_successor_run_id_is_non_empty() {
        let id = make_successor_run_id("run-abc");
        assert!(id.starts_with("run-abc-successor-"));
        assert!(id.len() > "run-abc-successor-".len());
    }
}
