//! Handler logic for `run.set_priority`.
//!
//! Provides a deterministic, explicit priority-update surface for ChatGPT to
//! classify runs by urgency within the visible working set.
//!
//! Rules:
//! - Any run (regardless of status) may have its priority updated.
//! - The priority value must be one of the bounded enum variants: `low`, `normal`, `high`, `urgent`.
//! - The reason must be non-empty and at most `PRIORITY_REASON_MAX_LEN` characters.
//! - Setting priority does not execute work.
//! - Setting priority does not change status, plan, retryable action, lineage, archive state,
//!   pin state, snooze state, or any other lifecycle field.
//! - An audit entry is appended by the daemon layer.

use anyhow::{bail, Result};
use deterministic_protocol::{
    RunSetPriorityParams, RunSetPriorityResult, RunState, PRIORITY_REASON_MAX_LEN,
};

/// Set the priority of a run.
///
/// Deterministic rules:
/// - Any run may have its priority updated regardless of current status.
/// - Only `priority` and `updated_at` are mutated on `state`.
///
/// Returns the updated run state (via mutation) and a result DTO.
pub fn set_priority(
    params: &RunSetPriorityParams,
    state: &mut RunState,
) -> Result<RunSetPriorityResult> {
    let reason = params.reason.trim();
    if reason.is_empty() {
        bail!("priority reason must not be empty");
    }
    if reason.len() > PRIORITY_REASON_MAX_LEN {
        bail!(
            "priority reason exceeds maximum length of {PRIORITY_REASON_MAX_LEN} characters"
        );
    }

    let previous_priority = state.priority;
    let new_priority = params.priority;
    let set_at = chrono::Utc::now().to_rfc3339();

    state.priority = new_priority;
    state.updated_at = set_at.clone();

    Ok(RunSetPriorityResult {
        run_id: params.run_id.clone(),
        status: state.status.clone(),
        previous_priority,
        priority: new_priority,
        reason: reason.to_string(),
        set_at,
        message: format!(
            "Run '{}' priority set to '{}'.",
            params.run_id,
            new_priority.as_str()
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use deterministic_protocol::{RunOutcome, RunPolicy, RunPriority, SnoozeMetadata, PinMetadata};

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
            priority: RunPriority::Normal,
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn set_priority_to_urgent() {
        let mut state = make_state("run-1", "active");
        let params = RunSetPriorityParams {
            run_id: "run-1".into(),
            priority: RunPriority::Urgent,
            reason: "blocks release".into(),
        };
        let result = set_priority(&params, &mut state).unwrap();
        assert_eq!(result.priority, RunPriority::Urgent);
        assert_eq!(result.previous_priority, RunPriority::Normal);
        assert_eq!(result.run_id, "run-1");
        assert_eq!(state.priority, RunPriority::Urgent);
    }

    #[test]
    fn set_priority_to_high() {
        let mut state = make_state("run-2", "active");
        let params = RunSetPriorityParams {
            run_id: "run-2".into(),
            priority: RunPriority::High,
            reason: "important feature".into(),
        };
        let result = set_priority(&params, &mut state).unwrap();
        assert_eq!(result.priority, RunPriority::High);
        assert_eq!(state.priority, RunPriority::High);
    }

    #[test]
    fn set_priority_to_low() {
        let mut state = make_state("run-3", "active");
        state.priority = RunPriority::High;
        let params = RunSetPriorityParams {
            run_id: "run-3".into(),
            priority: RunPriority::Low,
            reason: "exploratory work".into(),
        };
        let result = set_priority(&params, &mut state).unwrap();
        assert_eq!(result.priority, RunPriority::Low);
        assert_eq!(result.previous_priority, RunPriority::High);
        assert_eq!(state.priority, RunPriority::Low);
    }

    #[test]
    fn set_priority_to_normal() {
        let mut state = make_state("run-4", "active");
        state.priority = RunPriority::Urgent;
        let params = RunSetPriorityParams {
            run_id: "run-4".into(),
            priority: RunPriority::Normal,
            reason: "urgency resolved".into(),
        };
        let result = set_priority(&params, &mut state).unwrap();
        assert_eq!(result.priority, RunPriority::Normal);
        assert_eq!(state.priority, RunPriority::Normal);
    }

    #[test]
    fn set_priority_on_finalized_run_succeeds() {
        let mut state = make_state("run-5", "finalized:completed");
        state.finalized_outcome = Some(RunOutcome {
            outcome_kind: "completed".into(),
            summary: "Done".into(),
            reason: None,
            finalized_at: "2024-01-01T01:00:00Z".into(),
        });
        let params = RunSetPriorityParams {
            run_id: "run-5".into(),
            priority: RunPriority::Low,
            reason: "archive priority".into(),
        };
        let result = set_priority(&params, &mut state).unwrap();
        assert_eq!(result.priority, RunPriority::Low);
        // Status must not change.
        assert_eq!(state.status, "finalized:completed");
        // Finalized outcome must be preserved.
        assert!(state.finalized_outcome.is_some());
    }

    #[test]
    fn set_priority_empty_reason_rejected() {
        let mut state = make_state("run-6", "active");
        let params = RunSetPriorityParams {
            run_id: "run-6".into(),
            priority: RunPriority::Urgent,
            reason: "".into(),
        };
        let err = set_priority(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn set_priority_whitespace_only_reason_rejected() {
        let mut state = make_state("run-7", "active");
        let params = RunSetPriorityParams {
            run_id: "run-7".into(),
            priority: RunPriority::Urgent,
            reason: "   ".into(),
        };
        let err = set_priority(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn set_priority_reason_too_long_rejected() {
        let mut state = make_state("run-8", "active");
        let params = RunSetPriorityParams {
            run_id: "run-8".into(),
            priority: RunPriority::Urgent,
            reason: "x".repeat(PRIORITY_REASON_MAX_LEN + 1),
        };
        let err = set_priority(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("exceeds maximum length"));
    }

    #[test]
    fn set_priority_does_not_change_status() {
        let mut state = make_state("run-9", "awaiting-approval");
        let params = RunSetPriorityParams {
            run_id: "run-9".into(),
            priority: RunPriority::Urgent,
            reason: "critical".into(),
        };
        set_priority(&params, &mut state).unwrap();
        assert_eq!(state.status, "awaiting-approval");
    }

    #[test]
    fn set_priority_updates_updated_at() {
        let mut state = make_state("run-10", "active");
        let params = RunSetPriorityParams {
            run_id: "run-10".into(),
            priority: RunPriority::High,
            reason: "elevated".into(),
        };
        set_priority(&params, &mut state).unwrap();
        assert_ne!(state.updated_at, "2024-01-01T00:00:00Z");
    }

    #[test]
    fn set_priority_reason_trimmed() {
        let mut state = make_state("run-11", "active");
        let params = RunSetPriorityParams {
            run_id: "run-11".into(),
            priority: RunPriority::High,
            reason: "  trimmed reason  ".into(),
        };
        let result = set_priority(&params, &mut state).unwrap();
        assert_eq!(result.reason, "trimmed reason");
    }

    #[test]
    fn set_priority_result_message_contains_run_id_and_priority() {
        let mut state = make_state("run-12", "active");
        let params = RunSetPriorityParams {
            run_id: "run-12".into(),
            priority: RunPriority::Urgent,
            reason: "test".into(),
        };
        let result = set_priority(&params, &mut state).unwrap();
        assert!(result.message.contains("run-12"));
        assert!(result.message.contains("urgent"));
    }

    #[test]
    fn set_priority_does_not_change_pin_metadata() {
        let mut state = make_state("run-13", "active");
        state.pin_metadata = Some(PinMetadata {
            reason: "important".into(),
            pinned_at: "2024-01-01T00:00:00Z".into(),
        });
        let params = RunSetPriorityParams {
            run_id: "run-13".into(),
            priority: RunPriority::Urgent,
            reason: "urgent".into(),
        };
        set_priority(&params, &mut state).unwrap();
        assert!(state.pin_metadata.is_some());
        assert_eq!(state.pin_metadata.as_ref().unwrap().reason, "important");
    }

    #[test]
    fn set_priority_does_not_change_snooze_metadata() {
        let mut state = make_state("run-14", "active");
        state.snooze_metadata = Some(SnoozeMetadata {
            reason: "deferred".into(),
            snoozed_at: "2024-01-01T00:00:00Z".into(),
        });
        let params = RunSetPriorityParams {
            run_id: "run-14".into(),
            priority: RunPriority::Low,
            reason: "downgrade".into(),
        };
        set_priority(&params, &mut state).unwrap();
        assert!(state.snooze_metadata.is_some());
        assert_eq!(state.snooze_metadata.as_ref().unwrap().reason, "deferred");
    }

    #[test]
    fn set_priority_result_includes_set_at_timestamp() {
        let mut state = make_state("run-15", "active");
        let params = RunSetPriorityParams {
            run_id: "run-15".into(),
            priority: RunPriority::High,
            reason: "elevated".into(),
        };
        let result = set_priority(&params, &mut state).unwrap();
        assert!(!result.set_at.is_empty());
    }

    #[test]
    fn priority_enum_ordering() {
        assert!(RunPriority::Low < RunPriority::Normal);
        assert!(RunPriority::Normal < RunPriority::High);
        assert!(RunPriority::High < RunPriority::Urgent);
    }

    #[test]
    fn priority_sort_keys_are_monotone() {
        let keys: Vec<i64> = RunPriority::all().iter().map(|p| p.sort_key()).collect();
        for i in 0..keys.len() - 1 {
            assert!(keys[i] < keys[i + 1], "sort_key not strictly increasing");
        }
    }

    #[test]
    fn priority_roundtrip_parse() {
        for p in RunPriority::all() {
            assert_eq!(RunPriority::parse(p.as_str()), Some(*p));
        }
    }

    #[test]
    fn priority_default_is_normal() {
        assert_eq!(RunPriority::default(), RunPriority::Normal);
    }

    #[test]
    fn priority_parse_invalid_returns_none() {
        assert_eq!(RunPriority::parse("critical"), None);
        assert_eq!(RunPriority::parse(""), None);
        assert_eq!(RunPriority::parse("URGENT"), None);
    }
}
