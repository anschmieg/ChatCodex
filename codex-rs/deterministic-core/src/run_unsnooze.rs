//! Handler logic for `run.unsnooze`.
//!
//! Provides a deterministic, explicit unsnooze surface for ChatGPT to restore
//! a snoozed run back into the normal visible working set.
//!
//! Rules:
//! - Only snoozed runs (with `snooze_metadata` set) may be unsnoozed.
//! - Non-snoozed runs are rejected with a deterministic error.
//! - Unsnoozing clears `snooze_metadata` only.
//! - Unsnoozing does not execute work.
//! - Unsnoozing does not change status, plan, retryable action, lineage,
//!   archive state, pin state, or any other lifecycle field.
//! - An audit entry is appended by the daemon layer.
//! - The unsnooze reason must be non-empty and at most `SNOOZE_REASON_MAX_LEN` characters.

use anyhow::{bail, Result};
use deterministic_protocol::{RunState, RunUnsnoozeParams, RunUnsnoozeResult, SNOOZE_REASON_MAX_LEN};

/// Unsnooze a run, clearing its snooze metadata.
///
/// Deterministic rules:
/// - Only snoozed runs (with `snooze_metadata` set) may be unsnoozed.
/// - Clears `snooze_metadata` and updates `updated_at`.
///
/// Returns the updated run state (via mutation) and a result DTO.
pub fn unsnooze(params: &RunUnsnoozeParams, state: &mut RunState) -> Result<RunUnsnoozeResult> {
    if state.snooze_metadata.is_none() {
        bail!("run '{}' is not snoozed", params.run_id);
    }

    let reason = params.reason.trim();
    if reason.is_empty() {
        bail!("unsnooze reason must not be empty");
    }
    if reason.len() > SNOOZE_REASON_MAX_LEN {
        bail!(
            "unsnooze reason exceeds maximum length of {SNOOZE_REASON_MAX_LEN} characters"
        );
    }

    state.snooze_metadata = None;
    state.updated_at = chrono::Utc::now().to_rfc3339();

    Ok(RunUnsnoozeResult {
        run_id: params.run_id.clone(),
        status: state.status.clone(),
        message: format!("Run '{}' unsnoozed.", params.run_id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use deterministic_protocol::{RunOutcome, RunPolicy, SnoozeMetadata};

    fn make_state(run_id: &str, status: &str) -> RunState {
        RunState {
            run_id: run_id.into(),
            workspace_id: "/tmp/ws".into(),
            user_goal: "fix bug".into(),
            status: status.into(),
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

    fn make_snoozed_state(run_id: &str, status: &str) -> RunState {
        let mut state = make_state(run_id, status);
        state.snooze_metadata = Some(SnoozeMetadata {
            reason: "blocked on external dependency".into(),
            snoozed_at: "2024-01-01T00:00:00Z".into(),
        });
        state
    }

    #[test]
    fn unsnooze_snoozed_run_succeeds() {
        let mut state = make_snoozed_state("run-1", "active");
        let params = RunUnsnoozeParams {
            run_id: "run-1".into(),
            reason: "dependency resolved".into(),
        };
        let result = unsnooze(&params, &mut state).unwrap();
        assert_eq!(result.run_id, "run-1");
        assert!(state.snooze_metadata.is_none());
    }

    #[test]
    fn unsnooze_finalized_snoozed_run_succeeds() {
        let mut state = make_snoozed_state("run-2", "finalized:completed");
        state.finalized_outcome = Some(RunOutcome {
            outcome_kind: "completed".into(),
            summary: "Done".into(),
            reason: None,
            finalized_at: "2024-01-01T01:00:00Z".into(),
        });
        let params = RunUnsnoozeParams {
            run_id: "run-2".into(),
            reason: "restore for inspection".into(),
        };
        let result = unsnooze(&params, &mut state).unwrap();
        assert_eq!(result.run_id, "run-2");
        // Status must not change.
        assert_eq!(state.status, "finalized:completed");
        // Finalized outcome must be preserved.
        assert!(state.finalized_outcome.is_some());
        assert!(state.snooze_metadata.is_none());
    }

    #[test]
    fn unsnooze_non_snoozed_run_rejected() {
        let mut state = make_state("run-3", "active");
        let params = RunUnsnoozeParams {
            run_id: "run-3".into(),
            reason: "restore".into(),
        };
        let err = unsnooze(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("not snoozed"));
    }

    #[test]
    fn unsnooze_empty_reason_rejected() {
        let mut state = make_snoozed_state("run-4", "active");
        let params = RunUnsnoozeParams {
            run_id: "run-4".into(),
            reason: "".into(),
        };
        let err = unsnooze(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn unsnooze_whitespace_only_reason_rejected() {
        let mut state = make_snoozed_state("run-5", "active");
        let params = RunUnsnoozeParams {
            run_id: "run-5".into(),
            reason: "   ".into(),
        };
        let err = unsnooze(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn unsnooze_reason_too_long_rejected() {
        let mut state = make_snoozed_state("run-6", "active");
        let params = RunUnsnoozeParams {
            run_id: "run-6".into(),
            reason: "x".repeat(SNOOZE_REASON_MAX_LEN + 1),
        };
        let err = unsnooze(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("exceeds maximum length"));
    }

    #[test]
    fn unsnooze_does_not_change_status() {
        let mut state = make_snoozed_state("run-7", "awaiting-approval");
        let params = RunUnsnoozeParams {
            run_id: "run-7".into(),
            reason: "ready".into(),
        };
        unsnooze(&params, &mut state).unwrap();
        assert_eq!(state.status, "awaiting-approval");
    }

    #[test]
    fn unsnooze_updates_updated_at() {
        let mut state = make_snoozed_state("run-8", "active");
        let params = RunUnsnoozeParams {
            run_id: "run-8".into(),
            reason: "ready to proceed".into(),
        };
        unsnooze(&params, &mut state).unwrap();
        assert_ne!(state.updated_at, "2024-01-01T00:00:00Z");
    }

    #[test]
    fn unsnooze_result_message_contains_run_id() {
        let mut state = make_snoozed_state("run-9", "active");
        let params = RunUnsnoozeParams {
            run_id: "run-9".into(),
            reason: "done".into(),
        };
        let result = unsnooze(&params, &mut state).unwrap();
        assert!(result.message.contains("run-9"));
    }

    #[test]
    fn unsnooze_clears_snooze_metadata_only() {
        let mut state = make_snoozed_state("run-10", "active");
        state.last_action = Some("some action".into());
        let params = RunUnsnoozeParams {
            run_id: "run-10".into(),
            reason: "done".into(),
        };
        unsnooze(&params, &mut state).unwrap();
        assert!(state.snooze_metadata.is_none());
        // Other fields preserved.
        assert_eq!(state.last_action.as_deref(), Some("some action"));
    }

    #[test]
    fn unsnooze_does_not_change_pin_metadata() {
        use deterministic_protocol::PinMetadata;
        let mut state = make_snoozed_state("run-11", "active");
        state.pin_metadata = Some(PinMetadata {
            reason: "important".into(),
            pinned_at: "2024-01-01T00:00:00Z".into(),
        });
        let params = RunUnsnoozeParams {
            run_id: "run-11".into(),
            reason: "restore".into(),
        };
        unsnooze(&params, &mut state).unwrap();
        assert!(state.snooze_metadata.is_none());
        // Pin metadata must be preserved.
        assert!(state.pin_metadata.is_some());
    }
}
