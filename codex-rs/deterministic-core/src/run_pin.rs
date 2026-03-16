//! Handler logic for `run.pin`.
//!
//! Provides a deterministic, explicit pin surface for ChatGPT to mark a run
//! as prominent in the working set.
//!
//! Rules:
//! - Any run (regardless of status) may be pinned.
//! - Pinning a run that is already pinned replaces the pin metadata (idempotent).
//! - Pinning does not execute work.
//! - Pinning does not change status, plan, retryable action, lineage, archive
//!   state, or any other lifecycle field.
//! - An audit entry is appended by the daemon layer.
//! - The pin reason must be non-empty and at most `PIN_REASON_MAX_LEN` characters.

use anyhow::{bail, Result};
use deterministic_protocol::{PinMetadata, RunPinParams, RunPinResult, RunState, PIN_REASON_MAX_LEN};

/// Pin a run, recording compact pin metadata.
///
/// Deterministic rules:
/// - Any run may be pinned regardless of current status.
/// - If already pinned, the metadata is replaced (idempotent re-pin).
/// - Only `pin_metadata` and `updated_at` are mutated on `state`.
///
/// Returns the updated run state (via mutation) and a result DTO.
pub fn pin(params: &RunPinParams, state: &mut RunState) -> Result<RunPinResult> {
    let reason = params.reason.trim();
    if reason.is_empty() {
        bail!("pin reason must not be empty");
    }
    if reason.len() > PIN_REASON_MAX_LEN {
        bail!(
            "pin reason exceeds maximum length of {PIN_REASON_MAX_LEN} characters"
        );
    }

    let pinned_at = chrono::Utc::now().to_rfc3339();

    state.pin_metadata = Some(PinMetadata {
        reason: reason.to_string(),
        pinned_at: pinned_at.clone(),
    });
    state.updated_at = pinned_at.clone();

    Ok(RunPinResult {
        run_id: params.run_id.clone(),
        status: state.status.clone(),
        pinned_at,
        reason: reason.to_string(),
        message: format!("Run '{}' pinned.", params.run_id),
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
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn pin_active_run_succeeds() {
        let mut state = make_state("run-1", "active");
        let params = RunPinParams {
            run_id: "run-1".into(),
            reason: "primary effort".into(),
        };
        let result = pin(&params, &mut state).unwrap();
        assert_eq!(result.run_id, "run-1");
        assert_eq!(result.reason, "primary effort");
        assert!(!result.pinned_at.is_empty());
        assert!(state.pin_metadata.is_some());
        assert_eq!(state.pin_metadata.as_ref().unwrap().reason, "primary effort");
    }

    #[test]
    fn pin_finalized_run_succeeds() {
        let mut state = make_state("run-2", "finalized:completed");
        state.finalized_outcome = Some(RunOutcome {
            outcome_kind: "completed".into(),
            summary: "Done".into(),
            reason: None,
            finalized_at: "2024-01-01T01:00:00Z".into(),
        });
        let params = RunPinParams {
            run_id: "run-2".into(),
            reason: "keep for reference".into(),
        };
        let result = pin(&params, &mut state).unwrap();
        assert_eq!(result.run_id, "run-2");
        // Status must not change.
        assert_eq!(state.status, "finalized:completed");
        // Finalized outcome must be preserved.
        assert!(state.finalized_outcome.is_some());
        assert!(state.pin_metadata.is_some());
    }

    #[test]
    fn pin_prepared_run_succeeds() {
        let mut state = make_state("run-3", "prepared");
        let params = RunPinParams {
            run_id: "run-3".into(),
            reason: "high priority".into(),
        };
        pin(&params, &mut state).unwrap();
        assert!(state.pin_metadata.is_some());
        assert_eq!(state.status, "prepared");
    }

    #[test]
    fn pin_replaces_existing_pin_metadata() {
        let mut state = make_state("run-4", "active");
        state.pin_metadata = Some(PinMetadata {
            reason: "old reason".into(),
            pinned_at: "2024-01-01T00:00:00Z".into(),
        });
        let params = RunPinParams {
            run_id: "run-4".into(),
            reason: "new reason".into(),
        };
        let result = pin(&params, &mut state).unwrap();
        assert_eq!(result.reason, "new reason");
        assert_eq!(state.pin_metadata.as_ref().unwrap().reason, "new reason");
    }

    #[test]
    fn pin_empty_reason_rejected() {
        let mut state = make_state("run-5", "active");
        let params = RunPinParams {
            run_id: "run-5".into(),
            reason: "".into(),
        };
        let err = pin(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn pin_whitespace_only_reason_rejected() {
        let mut state = make_state("run-6", "active");
        let params = RunPinParams {
            run_id: "run-6".into(),
            reason: "   ".into(),
        };
        let err = pin(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn pin_reason_too_long_rejected() {
        let mut state = make_state("run-7", "active");
        let params = RunPinParams {
            run_id: "run-7".into(),
            reason: "x".repeat(PIN_REASON_MAX_LEN + 1),
        };
        let err = pin(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("exceeds maximum length"));
    }

    #[test]
    fn pin_does_not_change_status() {
        let mut state = make_state("run-8", "awaiting-approval");
        let params = RunPinParams {
            run_id: "run-8".into(),
            reason: "blocking issue".into(),
        };
        pin(&params, &mut state).unwrap();
        assert_eq!(state.status, "awaiting-approval");
    }

    #[test]
    fn pin_updates_updated_at() {
        let mut state = make_state("run-9", "active");
        let params = RunPinParams {
            run_id: "run-9".into(),
            reason: "main task".into(),
        };
        pin(&params, &mut state).unwrap();
        assert_ne!(state.updated_at, "2024-01-01T00:00:00Z");
    }

    #[test]
    fn pin_reason_trimmed() {
        let mut state = make_state("run-10", "active");
        let params = RunPinParams {
            run_id: "run-10".into(),
            reason: "  trimmed reason  ".into(),
        };
        let result = pin(&params, &mut state).unwrap();
        assert_eq!(result.reason, "trimmed reason");
        assert_eq!(state.pin_metadata.as_ref().unwrap().reason, "trimmed reason");
    }

    #[test]
    fn pin_result_message_contains_run_id() {
        let mut state = make_state("run-11", "active");
        let params = RunPinParams {
            run_id: "run-11".into(),
            reason: "test".into(),
        };
        let result = pin(&params, &mut state).unwrap();
        assert!(result.message.contains("run-11"));
    }
}
