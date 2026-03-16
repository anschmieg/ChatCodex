//! Handler logic for `run.unpin`.
//!
//! Provides a deterministic, explicit unpin surface for ChatGPT to remove a
//! run from the prominent working-set position.
//!
//! Rules:
//! - Only pinned runs (with `pin_metadata` set) may be unpinned.
//! - Non-pinned runs are rejected with a deterministic error.
//! - Unpinning clears `pin_metadata` only.
//! - Unpinning does not execute work.
//! - Unpinning does not change status, plan, retryable action, lineage,
//!   archive state, or any other lifecycle field.
//! - An audit entry is appended by the daemon layer.
//! - The unpin reason must be non-empty and at most `PIN_REASON_MAX_LEN` characters.

use anyhow::{bail, Result};
use deterministic_protocol::{RunState, RunUnpinParams, RunUnpinResult, PIN_REASON_MAX_LEN};

/// Unpin a run, clearing its pin metadata.
///
/// Deterministic rules:
/// - Only pinned runs (with `pin_metadata` set) may be unpinned.
/// - Clears `pin_metadata` and updates `updated_at`.
///
/// Returns the updated run state (via mutation) and a result DTO.
pub fn unpin(params: &RunUnpinParams, state: &mut RunState) -> Result<RunUnpinResult> {
    if state.pin_metadata.is_none() {
        bail!("run '{}' is not pinned", params.run_id);
    }

    let reason = params.reason.trim();
    if reason.is_empty() {
        bail!("unpin reason must not be empty");
    }
    if reason.len() > PIN_REASON_MAX_LEN {
        bail!(
            "unpin reason exceeds maximum length of {PIN_REASON_MAX_LEN} characters"
        );
    }

    state.pin_metadata = None;
    state.updated_at = chrono::Utc::now().to_rfc3339();

    Ok(RunUnpinResult {
        run_id: params.run_id.clone(),
        status: state.status.clone(),
        message: format!("Run '{}' unpinned.", params.run_id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use deterministic_protocol::{PinMetadata, RunOutcome, RunPolicy};

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
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        }
    }

    fn make_pinned_state(run_id: &str, status: &str) -> RunState {
        let mut state = make_state(run_id, status);
        state.pin_metadata = Some(PinMetadata {
            reason: "primary effort".into(),
            pinned_at: "2024-01-01T00:00:00Z".into(),
        });
        state
    }

    #[test]
    fn unpin_pinned_run_succeeds() {
        let mut state = make_pinned_state("run-1", "active");
        let params = RunUnpinParams {
            run_id: "run-1".into(),
            reason: "no longer priority".into(),
        };
        let result = unpin(&params, &mut state).unwrap();
        assert_eq!(result.run_id, "run-1");
        assert!(state.pin_metadata.is_none());
    }

    #[test]
    fn unpin_finalized_pinned_run_succeeds() {
        let mut state = make_pinned_state("run-2", "finalized:completed");
        state.finalized_outcome = Some(RunOutcome {
            outcome_kind: "completed".into(),
            summary: "Done".into(),
            reason: None,
            finalized_at: "2024-01-01T01:00:00Z".into(),
        });
        let params = RunUnpinParams {
            run_id: "run-2".into(),
            reason: "work complete".into(),
        };
        let result = unpin(&params, &mut state).unwrap();
        assert_eq!(result.run_id, "run-2");
        // Status must not change.
        assert_eq!(state.status, "finalized:completed");
        // Finalized outcome must be preserved.
        assert!(state.finalized_outcome.is_some());
        assert!(state.pin_metadata.is_none());
    }

    #[test]
    fn unpin_non_pinned_run_rejected() {
        let mut state = make_state("run-3", "active");
        let params = RunUnpinParams {
            run_id: "run-3".into(),
            reason: "no longer priority".into(),
        };
        let err = unpin(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("not pinned"));
    }

    #[test]
    fn unpin_empty_reason_rejected() {
        let mut state = make_pinned_state("run-4", "active");
        let params = RunUnpinParams {
            run_id: "run-4".into(),
            reason: "".into(),
        };
        let err = unpin(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn unpin_whitespace_only_reason_rejected() {
        let mut state = make_pinned_state("run-5", "active");
        let params = RunUnpinParams {
            run_id: "run-5".into(),
            reason: "   ".into(),
        };
        let err = unpin(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn unpin_reason_too_long_rejected() {
        let mut state = make_pinned_state("run-6", "active");
        let params = RunUnpinParams {
            run_id: "run-6".into(),
            reason: "x".repeat(PIN_REASON_MAX_LEN + 1),
        };
        let err = unpin(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("exceeds maximum length"));
    }

    #[test]
    fn unpin_does_not_change_status() {
        let mut state = make_pinned_state("run-7", "awaiting-approval");
        let params = RunUnpinParams {
            run_id: "run-7".into(),
            reason: "resolved".into(),
        };
        unpin(&params, &mut state).unwrap();
        assert_eq!(state.status, "awaiting-approval");
    }

    #[test]
    fn unpin_updates_updated_at() {
        let mut state = make_pinned_state("run-8", "active");
        let params = RunUnpinParams {
            run_id: "run-8".into(),
            reason: "deprioritized".into(),
        };
        unpin(&params, &mut state).unwrap();
        assert_ne!(state.updated_at, "2024-01-01T00:00:00Z");
    }

    #[test]
    fn unpin_result_message_contains_run_id() {
        let mut state = make_pinned_state("run-9", "active");
        let params = RunUnpinParams {
            run_id: "run-9".into(),
            reason: "done".into(),
        };
        let result = unpin(&params, &mut state).unwrap();
        assert!(result.message.contains("run-9"));
    }

    #[test]
    fn unpin_clears_pin_metadata_only() {
        let mut state = make_pinned_state("run-10", "active");
        // Set some other metadata to ensure it's not cleared.
        state.last_action = Some("some action".into());
        let params = RunUnpinParams {
            run_id: "run-10".into(),
            reason: "done".into(),
        };
        unpin(&params, &mut state).unwrap();
        assert!(state.pin_metadata.is_none());
        // Other fields preserved.
        assert_eq!(state.last_action.as_deref(), Some("some action"));
    }
}
