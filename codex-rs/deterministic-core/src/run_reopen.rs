//! Handler logic for `run.reopen`.
//!
//! Provides a deterministic, explicit continuation surface for ChatGPT to
//! reopen a previously finalized run.  Only finalized runs may be reopened.
//! Reopening is audited, does not execute work, and preserves prior history.

use anyhow::{bail, Result};
use deterministic_protocol::{ReopenMetadata, RunReopenParams, RunReopenResult, RunState};

/// Reopen a finalized run so ChatGPT can continue working on it.
///
/// Deterministic lifecycle rules:
/// - Only finalized runs (`status` starts with `"finalized:"`) may be reopened.
/// - Active, prepared, or awaiting-approval runs are rejected.
/// - Reopening does not execute work or trigger autonomous follow-up.
/// - Prior audit history is preserved (not cleared).
/// - The `finalized_outcome` record is cleared; the closure is captured in
///   `reopen_metadata.reopened_from_outcome_kind` and in the audit trail.
/// - `reopen_count` is incremented on each successive reopen.
pub fn reopen(params: &RunReopenParams, state: &mut RunState) -> Result<RunReopenResult> {
    // Enforce: only finalized runs can be reopened.
    if !state.status.starts_with("finalized:") {
        bail!(
            "run '{}' cannot be reopened: status is '{}' (only finalized runs may be reopened)",
            params.run_id,
            state.status
        );
    }

    // Extract the outcome kind from the current finalized status.
    let reopened_from_outcome_kind = state
        .status
        .strip_prefix("finalized:")
        .unwrap_or("unknown")
        .to_string();

    let now = chrono::Utc::now().to_rfc3339();

    // Compute the new reopen_count (increment from prior metadata or start at 1).
    let reopen_count = state
        .reopen_metadata
        .as_ref()
        .map(|m| m.reopen_count + 1)
        .unwrap_or(1);

    // Build the updated reopen metadata (always reflects the most recent reopen).
    let reopen_meta = ReopenMetadata {
        reason: params.reason.clone(),
        reopened_at: now.clone(),
        reopened_from_outcome_kind: reopened_from_outcome_kind.clone(),
        reopen_count,
    };

    // Transition run back to active; clear the finalized outcome.
    state.status = "active".to_string();
    state.finalized_outcome = None;
    state.reopen_metadata = Some(reopen_meta);
    state.updated_at = now.clone();

    // Provide deterministic guidance on what to do next (no inference, no model calls).
    let (recommended_next_action, recommended_tool) = match reopened_from_outcome_kind.as_str() {
        "completed" => (
            "Run reopened from a completed state. Call refresh_run_state to inspect current state, then replan or continue from where the run left off.",
            "refresh_run_state",
        ),
        "failed" => (
            "Run reopened from a failed state. Call refresh_run_state to inspect current state and use replan_run to update the plan with new evidence.",
            "refresh_run_state",
        ),
        "abandoned" => (
            "Run reopened from an abandoned state. Call refresh_run_state to inspect current state, then replan_run with the updated goal.",
            "refresh_run_state",
        ),
        _ => (
            "Run reopened. Call refresh_run_state to inspect current state.",
            "refresh_run_state",
        ),
    };

    Ok(RunReopenResult {
        run_id: params.run_id.clone(),
        status: state.status.clone(),
        reopened_from_outcome_kind,
        reopen_count,
        reopened_at: now,
        recommended_next_action: recommended_next_action.to_string(),
        recommended_tool: recommended_tool.to_string(),
    })
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
    // Happy-path: reopen completed run
    // -----------------------------------------------------------------------

    #[test]
    fn reopen_completed_run_succeeds() {
        let mut state = make_active_state("r1");
        finalize_state(&mut state, "completed");

        let params = RunReopenParams {
            run_id: "r1".into(),
            reason: "Found another issue to fix".into(),
        };
        let result = reopen(&params, &mut state).unwrap();

        assert_eq!(result.run_id, "r1");
        assert_eq!(result.status, "active");
        assert_eq!(result.reopened_from_outcome_kind, "completed");
        assert_eq!(result.reopen_count, 1);
        assert!(!result.reopened_at.is_empty());
        assert!(result.recommended_next_action.contains("completed"));
        assert_eq!(result.recommended_tool, "refresh_run_state");

        // State: status is active, finalized_outcome cleared, reopen_metadata set.
        assert_eq!(state.status, "active");
        assert!(state.finalized_outcome.is_none());
        let meta = state.reopen_metadata.as_ref().unwrap();
        assert_eq!(meta.reason, "Found another issue to fix");
        assert_eq!(meta.reopened_from_outcome_kind, "completed");
        assert_eq!(meta.reopen_count, 1);
    }

    // -----------------------------------------------------------------------
    // Happy-path: reopen failed run
    // -----------------------------------------------------------------------

    #[test]
    fn reopen_failed_run_succeeds() {
        let mut state = make_active_state("r2");
        finalize_state(&mut state, "failed");

        let params = RunReopenParams {
            run_id: "r2".into(),
            reason: "New clue found".into(),
        };
        let result = reopen(&params, &mut state).unwrap();

        assert_eq!(result.reopened_from_outcome_kind, "failed");
        assert_eq!(result.status, "active");
        assert!(result.recommended_next_action.contains("failed"));
    }

    // -----------------------------------------------------------------------
    // Happy-path: reopen abandoned run
    // -----------------------------------------------------------------------

    #[test]
    fn reopen_abandoned_run_succeeds() {
        let mut state = make_active_state("r3");
        finalize_state(&mut state, "abandoned");

        let params = RunReopenParams {
            run_id: "r3".into(),
            reason: "Goal changed, work is needed again".into(),
        };
        let result = reopen(&params, &mut state).unwrap();

        assert_eq!(result.reopened_from_outcome_kind, "abandoned");
        assert_eq!(result.status, "active");
        assert!(result.recommended_next_action.contains("abandoned"));
    }

    // -----------------------------------------------------------------------
    // Reject: active run cannot be reopened
    // -----------------------------------------------------------------------

    #[test]
    fn reopen_active_run_rejected() {
        let mut state = make_active_state("r4");
        // Status is "active" — not finalized.

        let params = RunReopenParams {
            run_id: "r4".into(),
            reason: "trying anyway".into(),
        };
        let err = reopen(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("cannot be reopened"));
        assert!(err.to_string().contains("active"));
        // State must not be mutated.
        assert_eq!(state.status, "active");
        assert!(state.reopen_metadata.is_none());
    }

    // -----------------------------------------------------------------------
    // Reject: prepared run cannot be reopened
    // -----------------------------------------------------------------------

    #[test]
    fn reopen_prepared_run_rejected() {
        let mut state = make_active_state("r5");
        state.status = "prepared".into();

        let params = RunReopenParams {
            run_id: "r5".into(),
            reason: "trying".into(),
        };
        let err = reopen(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("cannot be reopened"));
        assert_eq!(state.status, "prepared");
    }

    // -----------------------------------------------------------------------
    // Reject: awaiting-approval run cannot be reopened
    // -----------------------------------------------------------------------

    #[test]
    fn reopen_awaiting_approval_rejected() {
        let mut state = make_active_state("r6");
        state.status = "awaiting_approval".into();

        let params = RunReopenParams {
            run_id: "r6".into(),
            reason: "trying".into(),
        };
        let err = reopen(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("cannot be reopened"));
    }

    // -----------------------------------------------------------------------
    // Lineage: reopen_count increments across successive reopens
    // -----------------------------------------------------------------------

    #[test]
    fn successive_reopens_increment_reopen_count() {
        let mut state = make_active_state("r7");
        finalize_state(&mut state, "failed");

        // First reopen.
        let p1 = RunReopenParams {
            run_id: "r7".into(),
            reason: "first reopen".into(),
        };
        let r1 = reopen(&p1, &mut state).unwrap();
        assert_eq!(r1.reopen_count, 1);
        assert_eq!(state.reopen_metadata.as_ref().unwrap().reopen_count, 1);

        // Finalize again.
        finalize_state(&mut state, "completed");

        // Second reopen.
        let p2 = RunReopenParams {
            run_id: "r7".into(),
            reason: "second reopen".into(),
        };
        let r2 = reopen(&p2, &mut state).unwrap();
        assert_eq!(r2.reopen_count, 2);
        assert_eq!(state.reopen_metadata.as_ref().unwrap().reopen_count, 2);
        // Most recent metadata reflects the last reopen.
        assert_eq!(
            state.reopen_metadata.as_ref().unwrap().reopened_from_outcome_kind,
            "completed"
        );
    }

    // -----------------------------------------------------------------------
    // Audit: prior history preserved (run state not wiped)
    // -----------------------------------------------------------------------

    #[test]
    fn reopen_preserves_plan_and_steps() {
        let mut state = make_active_state("r8");
        state.plan = vec!["step A".into(), "step B".into()];
        state.completed_steps = vec!["step A".into()];
        finalize_state(&mut state, "completed");

        let params = RunReopenParams {
            run_id: "r8".into(),
            reason: "more work needed".into(),
        };
        reopen(&params, &mut state).unwrap();

        // Plan and completed steps should be preserved.
        assert_eq!(state.plan, vec!["step A", "step B"]);
        assert_eq!(state.completed_steps, vec!["step A"]);
    }

    // -----------------------------------------------------------------------
    // Structural: reopen_count starts at 0 before first reopen
    // -----------------------------------------------------------------------

    #[test]
    fn fresh_run_has_no_reopen_metadata() {
        let state = make_active_state("r9");
        assert!(state.reopen_metadata.is_none());
    }
}
