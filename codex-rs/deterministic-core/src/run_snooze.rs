//! Handler logic for `run.snooze`.
//!
//! Provides a deterministic, explicit snooze surface for ChatGPT to defer a
//! run out of the default visible working set without archiving it.
//!
//! Rules:
//! - Any run (regardless of status) may be snoozed.
//! - Snoozing a run that is already snoozed replaces the snooze metadata (idempotent).
//! - Snoozing does not execute work.
//! - Snoozing does not change status, plan, retryable action, lineage, archive
//!   state, pin state, or any other lifecycle field.
//! - An audit entry is appended by the daemon layer.
//! - The snooze reason must be non-empty and at most `SNOOZE_REASON_MAX_LEN` characters.

use anyhow::{bail, Result};
use deterministic_protocol::{
    RunSnoozeParams, RunSnoozeResult, RunState, SnoozeMetadata, SNOOZE_REASON_MAX_LEN,
};

/// Snooze a run, recording compact snooze metadata.
///
/// Deterministic rules:
/// - Any run may be snoozed regardless of current status.
/// - If already snoozed, the metadata is replaced (idempotent re-snooze).
/// - Only `snooze_metadata` and `updated_at` are mutated on `state`.
///
/// Returns the updated run state (via mutation) and a result DTO.
pub fn snooze(params: &RunSnoozeParams, state: &mut RunState) -> Result<RunSnoozeResult> {
    let reason = params.reason.trim();
    if reason.is_empty() {
        bail!("snooze reason must not be empty");
    }
    if reason.len() > SNOOZE_REASON_MAX_LEN {
        bail!(
            "snooze reason exceeds maximum length of {SNOOZE_REASON_MAX_LEN} characters"
        );
    }

    let snoozed_at = chrono::Utc::now().to_rfc3339();

    state.snooze_metadata = Some(SnoozeMetadata {
        reason: reason.to_string(),
        snoozed_at: snoozed_at.clone(),
    });
    state.updated_at = snoozed_at.clone();

    Ok(RunSnoozeResult {
        run_id: params.run_id.clone(),
        status: state.status.clone(),
        snoozed_at,
        reason: reason.to_string(),
        message: format!("Run '{}' snoozed.", params.run_id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use deterministic_protocol::{RunOutcome, RunPolicy};

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
            effort: None,
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn snooze_active_run_succeeds() {
        let mut state = make_state("run-1", "active");
        let params = RunSnoozeParams {
            run_id: "run-1".into(),
            reason: "blocked on external dependency".into(),
        };
        let result = snooze(&params, &mut state).unwrap();
        assert_eq!(result.run_id, "run-1");
        assert_eq!(result.reason, "blocked on external dependency");
        assert!(!result.snoozed_at.is_empty());
        assert!(state.snooze_metadata.is_some());
        assert_eq!(
            state.snooze_metadata.as_ref().unwrap().reason,
            "blocked on external dependency"
        );
    }

    #[test]
    fn snooze_finalized_run_succeeds() {
        let mut state = make_state("run-2", "finalized:completed");
        state.finalized_outcome = Some(RunOutcome {
            outcome_kind: "completed".into(),
            summary: "Done".into(),
            reason: None,
            finalized_at: "2024-01-01T01:00:00Z".into(),
        });
        let params = RunSnoozeParams {
            run_id: "run-2".into(),
            reason: "review later".into(),
        };
        let result = snooze(&params, &mut state).unwrap();
        assert_eq!(result.run_id, "run-2");
        // Status must not change.
        assert_eq!(state.status, "finalized:completed");
        // Finalized outcome must be preserved.
        assert!(state.finalized_outcome.is_some());
        assert!(state.snooze_metadata.is_some());
    }

    #[test]
    fn snooze_prepared_run_succeeds() {
        let mut state = make_state("run-3", "prepared");
        let params = RunSnoozeParams {
            run_id: "run-3".into(),
            reason: "not yet needed".into(),
        };
        snooze(&params, &mut state).unwrap();
        assert!(state.snooze_metadata.is_some());
        assert_eq!(state.status, "prepared");
    }

    #[test]
    fn snooze_replaces_existing_snooze_metadata() {
        let mut state = make_state("run-4", "active");
        state.snooze_metadata = Some(SnoozeMetadata {
            reason: "old reason".into(),
            snoozed_at: "2024-01-01T00:00:00Z".into(),
        });
        let params = RunSnoozeParams {
            run_id: "run-4".into(),
            reason: "new reason".into(),
        };
        let result = snooze(&params, &mut state).unwrap();
        assert_eq!(result.reason, "new reason");
        assert_eq!(state.snooze_metadata.as_ref().unwrap().reason, "new reason");
    }

    #[test]
    fn snooze_empty_reason_rejected() {
        let mut state = make_state("run-5", "active");
        let params = RunSnoozeParams {
            run_id: "run-5".into(),
            reason: "".into(),
        };
        let err = snooze(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn snooze_whitespace_only_reason_rejected() {
        let mut state = make_state("run-6", "active");
        let params = RunSnoozeParams {
            run_id: "run-6".into(),
            reason: "   ".into(),
        };
        let err = snooze(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn snooze_reason_too_long_rejected() {
        let mut state = make_state("run-7", "active");
        let params = RunSnoozeParams {
            run_id: "run-7".into(),
            reason: "x".repeat(SNOOZE_REASON_MAX_LEN + 1),
        };
        let err = snooze(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("exceeds maximum length"));
    }

    #[test]
    fn snooze_does_not_change_status() {
        let mut state = make_state("run-8", "awaiting-approval");
        let params = RunSnoozeParams {
            run_id: "run-8".into(),
            reason: "blocked".into(),
        };
        snooze(&params, &mut state).unwrap();
        assert_eq!(state.status, "awaiting-approval");
    }

    #[test]
    fn snooze_updates_updated_at() {
        let mut state = make_state("run-9", "active");
        let params = RunSnoozeParams {
            run_id: "run-9".into(),
            reason: "deferred".into(),
        };
        snooze(&params, &mut state).unwrap();
        assert_ne!(state.updated_at, "2024-01-01T00:00:00Z");
    }

    #[test]
    fn snooze_reason_trimmed() {
        let mut state = make_state("run-10", "active");
        let params = RunSnoozeParams {
            run_id: "run-10".into(),
            reason: "  trimmed reason  ".into(),
        };
        let result = snooze(&params, &mut state).unwrap();
        assert_eq!(result.reason, "trimmed reason");
        assert_eq!(
            state.snooze_metadata.as_ref().unwrap().reason,
            "trimmed reason"
        );
    }

    #[test]
    fn snooze_result_message_contains_run_id() {
        let mut state = make_state("run-11", "active");
        let params = RunSnoozeParams {
            run_id: "run-11".into(),
            reason: "test".into(),
        };
        let result = snooze(&params, &mut state).unwrap();
        assert!(result.message.contains("run-11"));
    }

    #[test]
    fn snooze_does_not_change_pin_metadata() {
        use deterministic_protocol::PinMetadata;
        let mut state = make_state("run-12", "active");
        state.pin_metadata = Some(PinMetadata {
            reason: "important".into(),
            pinned_at: "2024-01-01T00:00:00Z".into(),
        });
        let params = RunSnoozeParams {
            run_id: "run-12".into(),
            reason: "defer".into(),
        };
        snooze(&params, &mut state).unwrap();
        // Pin metadata must be preserved.
        assert!(state.pin_metadata.is_some());
        assert_eq!(state.pin_metadata.as_ref().unwrap().reason, "important");
    }
}
