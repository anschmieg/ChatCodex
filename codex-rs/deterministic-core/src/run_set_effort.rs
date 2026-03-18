//! Deterministic run effort update (Milestone 24).
//!
//! Provides a deterministic, explicit effort-bucket surface for ChatGPT to
//! classify runs by expected execution size.
//!
//! Rules:
//! - Any run (regardless of status) may have its effort updated or cleared.
//! - The effort value must be one of the bounded enum variants: `small`, `medium`, `large`.
//! - Passing `effort: Some(None)` (JSON `null`) clears the effort estimate.
//! - Setting effort does not execute work.
//! - Setting effort does not change status, plan, retryable action, lineage,
//!   archive state, pin state, snooze state, priority, ownership, due date,
//!   dependencies, or any other lifecycle field.
//! - An audit entry is appended by the daemon layer.

use anyhow::Result;
use deterministic_protocol::{RunEffort, RunSetEffortParams, RunSetEffortResult, RunState};

/// Set or clear the effort estimate of a run.
///
/// Deterministic rules:
/// - Any run may have its effort updated regardless of current status.
/// - Only `effort` and `updated_at` are mutated on `state`.
///
/// Returns the updated run state (via mutation) and a result DTO.
pub fn set_effort(
    params: &RunSetEffortParams,
    state: &mut RunState,
) -> Result<RunSetEffortResult> {
    let previous_effort = state.effort;

    let new_effort: Option<RunEffort> = match &params.effort {
        None => {
            // The field was absent — treat as no change (preserve current value).
            state.effort
        }
        Some(None) => None,          // explicit JSON null → clear
        Some(Some(e)) => Some(*e),   // explicit value → set
    };

    let now = chrono::Utc::now().to_rfc3339();
    state.effort = new_effort;
    state.updated_at = now.clone();

    let message = match (previous_effort, new_effort) {
        (None, None) => "effort unchanged (no estimate set)".to_string(),
        (None, Some(e)) => format!("effort set to {e}"),
        (Some(prev), None) => format!("effort {} cleared", prev.as_str()),
        (Some(prev), Some(next)) if prev == next => {
            format!("effort unchanged: {next}")
        }
        (Some(prev), Some(next)) => {
            format!("effort updated from {} to {}", prev.as_str(), next.as_str())
        }
    };

    Ok(RunSetEffortResult {
        run_id: params.run_id.clone(),
        status: state.status.clone(),
        previous_effort,
        effort: new_effort,
        updated_at: now,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use deterministic_protocol::{RunEffort, RunPolicy, RunPriority};

    fn make_state(id: &str, status: &str) -> RunState {
        RunState {
            run_id: id.to_string(),
            workspace_id: "/tmp/ws".to_string(),
            status: status.to_string(),
            user_goal: "test goal".to_string(),
            plan: vec![],
            current_step: 0,
            completed_steps: vec![],
            pending_steps: vec![],
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
            assignee: None,
            ownership_note: None,
            due_date: None,
            blocked_by_run_ids: vec![],
            effort: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    fn params_set(run_id: &str, effort: RunEffort) -> RunSetEffortParams {
        RunSetEffortParams {
            run_id: run_id.into(),
            effort: Some(Some(effort)),
        }
    }

    fn params_clear(run_id: &str) -> RunSetEffortParams {
        RunSetEffortParams {
            run_id: run_id.into(),
            effort: Some(None),
        }
    }

    #[test]
    fn set_effort_to_small() {
        let mut state = make_state("r1", "active");
        let result = set_effort(&params_set("r1", RunEffort::Small), &mut state).unwrap();
        assert_eq!(result.effort, Some(RunEffort::Small));
        assert_eq!(result.previous_effort, None);
        assert_eq!(state.effort, Some(RunEffort::Small));
    }

    #[test]
    fn set_effort_to_medium() {
        let mut state = make_state("r2", "active");
        let result = set_effort(&params_set("r2", RunEffort::Medium), &mut state).unwrap();
        assert_eq!(result.effort, Some(RunEffort::Medium));
        assert_eq!(state.effort, Some(RunEffort::Medium));
    }

    #[test]
    fn set_effort_to_large() {
        let mut state = make_state("r3", "active");
        let result = set_effort(&params_set("r3", RunEffort::Large), &mut state).unwrap();
        assert_eq!(result.effort, Some(RunEffort::Large));
        assert_eq!(state.effort, Some(RunEffort::Large));
    }

    #[test]
    fn replace_existing_effort() {
        let mut state = make_state("r4", "active");
        state.effort = Some(RunEffort::Small);
        let result = set_effort(&params_set("r4", RunEffort::Large), &mut state).unwrap();
        assert_eq!(result.effort, Some(RunEffort::Large));
        assert_eq!(result.previous_effort, Some(RunEffort::Small));
        assert_eq!(state.effort, Some(RunEffort::Large));
    }

    #[test]
    fn clear_effort() {
        let mut state = make_state("r5", "active");
        state.effort = Some(RunEffort::Medium);
        let result = set_effort(&params_clear("r5"), &mut state).unwrap();
        assert_eq!(result.effort, None);
        assert_eq!(result.previous_effort, Some(RunEffort::Medium));
        assert_eq!(state.effort, None);
    }

    #[test]
    fn clear_when_already_none() {
        let mut state = make_state("r6", "active");
        let result = set_effort(&params_clear("r6"), &mut state).unwrap();
        assert_eq!(result.effort, None);
        assert_eq!(result.previous_effort, None);
        assert!(result.message.contains("unchanged"), "{}", result.message);
    }

    #[test]
    fn set_same_effort_unchanged_message() {
        let mut state = make_state("r7", "active");
        state.effort = Some(RunEffort::Medium);
        let result = set_effort(&params_set("r7", RunEffort::Medium), &mut state).unwrap();
        assert_eq!(result.effort, Some(RunEffort::Medium));
        assert!(result.message.contains("unchanged"), "{}", result.message);
    }

    #[test]
    fn set_effort_does_not_change_status() {
        let mut state = make_state("r8", "finalized:completed");
        set_effort(&params_set("r8", RunEffort::Small), &mut state).unwrap();
        assert_eq!(state.status, "finalized:completed");
    }

    #[test]
    fn set_effort_updates_updated_at() {
        let mut state = make_state("r9", "active");
        set_effort(&params_set("r9", RunEffort::Large), &mut state).unwrap();
        assert_ne!(state.updated_at, "2024-01-01T00:00:00Z");
    }

    #[test]
    fn set_effort_on_finalized_run_succeeds() {
        let mut state = make_state("r10", "finalized:completed");
        let result = set_effort(&params_set("r10", RunEffort::Small), &mut state).unwrap();
        assert_eq!(result.effort, Some(RunEffort::Small));
        assert_eq!(state.status, "finalized:completed");
    }

    #[test]
    fn message_set() {
        let mut state = make_state("r11", "active");
        let result = set_effort(&params_set("r11", RunEffort::Large), &mut state).unwrap();
        assert!(result.message.contains("large"), "{}", result.message);
    }

    #[test]
    fn message_cleared() {
        let mut state = make_state("r12", "active");
        state.effort = Some(RunEffort::Large);
        let result = set_effort(&params_clear("r12"), &mut state).unwrap();
        assert!(result.message.contains("cleared"), "{}", result.message);
    }

    #[test]
    fn message_updated() {
        let mut state = make_state("r13", "active");
        state.effort = Some(RunEffort::Small);
        let result = set_effort(&params_set("r13", RunEffort::Medium), &mut state).unwrap();
        assert!(result.message.contains("small"), "{}", result.message);
        assert!(result.message.contains("medium"), "{}", result.message);
    }

    #[test]
    fn result_includes_updated_at() {
        let mut state = make_state("r14", "active");
        let result = set_effort(&params_set("r14", RunEffort::Small), &mut state).unwrap();
        assert!(!result.updated_at.is_empty());
    }

    #[test]
    fn effort_enum_ordering() {
        assert!(RunEffort::Small < RunEffort::Medium);
        assert!(RunEffort::Medium < RunEffort::Large);
    }

    #[test]
    fn effort_sort_keys_are_monotone() {
        let keys: Vec<i64> = RunEffort::all().iter().map(|e| e.sort_key()).collect();
        for i in 0..keys.len() - 1 {
            assert!(keys[i] < keys[i + 1], "sort_key not strictly increasing");
        }
    }

    #[test]
    fn effort_roundtrip_parse() {
        for e in RunEffort::all() {
            assert_eq!(RunEffort::parse(e.as_str()), Some(*e));
        }
    }

    #[test]
    fn effort_parse_invalid_returns_none() {
        assert_eq!(RunEffort::parse("tiny"), None);
        assert_eq!(RunEffort::parse(""), None);
        assert_eq!(RunEffort::parse("LARGE"), None);
        assert_eq!(RunEffort::parse("extra-large"), None);
    }

    #[test]
    fn no_op_absent_effort_preserves_current() {
        let mut state = make_state("r15", "active");
        state.effort = Some(RunEffort::Medium);
        // params.effort = None means "absent from JSON" — treat as no-op
        let p = RunSetEffortParams {
            run_id: "r15".into(),
            effort: None,
        };
        let result = set_effort(&p, &mut state).unwrap();
        // Should preserve the current value
        assert_eq!(result.effort, Some(RunEffort::Medium));
        assert_eq!(state.effort, Some(RunEffort::Medium));
    }
}
