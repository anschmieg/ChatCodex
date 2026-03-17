//! Deterministic run ownership assignment (Milestone 19).

use anyhow::{bail, Result};
use deterministic_protocol::types::{
    RunAssignOwnerParams, RunAssignOwnerResult, RunState, ASSIGNEE_MAX_LEN, OWNERSHIP_NOTE_MAX_LEN,
};

/// Normalize and validate an assignee string.
///
/// Rules:
/// - trim whitespace
/// - lowercase
/// - allow only `[a-z0-9._-]`
/// - max `ASSIGNEE_MAX_LEN` characters
/// - must not be empty after trim
pub fn normalize_assignee(s: &str) -> Result<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        bail!("assignee must not be empty after trimming whitespace");
    }
    let lower = trimmed.to_lowercase();
    if lower.len() > ASSIGNEE_MAX_LEN {
        bail!(
            "assignee exceeds maximum length of {ASSIGNEE_MAX_LEN} characters"
        );
    }
    if let Some(ch) = lower
        .chars()
        .find(|c| !matches!(c, 'a'..='z' | '0'..='9' | '.' | '_' | '-'))
    {
        bail!("assignee contains invalid character '{ch}'; allowed: [a-z0-9._-]");
    }
    Ok(lower)
}

/// Assign or clear ownership of a run.
pub fn assign_owner(
    params: &RunAssignOwnerParams,
    state: &mut RunState,
) -> Result<RunAssignOwnerResult> {
    let previous_assignee = state.assignee.clone();

    // Resolve new assignee.
    let new_assignee = match &params.assignee {
        None => state.assignee.clone(),                  // absent → preserve
        Some(None) => None,                              // explicit null → clear
        Some(Some(s)) => Some(normalize_assignee(s)?),  // string → normalize
    };

    // Resolve new ownership note.
    let new_note = match &params.ownership_note {
        None => state.ownership_note.clone(), // absent → preserve
        Some(None) => None,                   // explicit null → clear
        Some(Some(s)) => {
            let trimmed = s.trim();
            if trimmed.len() > OWNERSHIP_NOTE_MAX_LEN {
                bail!(
                    "ownership_note exceeds maximum length of {OWNERSHIP_NOTE_MAX_LEN} characters"
                );
            }
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
    };

    let now = chrono::Utc::now().to_rfc3339();
    state.assignee = new_assignee.clone();
    state.ownership_note = new_note.clone();

    let message = match (&previous_assignee, &new_assignee) {
        (None, None) => "ownership unchanged (no assignee)".to_string(),
        (None, Some(a)) => format!("run assigned to {a}"),
        (Some(prev), None) => format!("assignee {prev} cleared"),
        (Some(prev), Some(next)) if prev == next => format!("assignee unchanged: {next}"),
        (Some(prev), Some(next)) => format!("run reassigned from {prev} to {next}"),
    };

    Ok(RunAssignOwnerResult {
        run_id: params.run_id.clone(),
        status: state.status.clone(),
        previous_assignee,
        assignee: new_assignee,
        ownership_note: new_note,
        updated_at: now,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use deterministic_protocol::{RunPolicy, RunPriority};

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
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    fn params_set(run_id: &str, assignee: &str) -> RunAssignOwnerParams {
        RunAssignOwnerParams {
            run_id: run_id.into(),
            assignee: Some(Some(assignee.to_string())),
            ownership_note: None,
        }
    }

    fn params_clear(run_id: &str) -> RunAssignOwnerParams {
        RunAssignOwnerParams {
            run_id: run_id.into(),
            assignee: Some(None),
            ownership_note: None,
        }
    }

    #[test]
    fn set_assignee() {
        let mut state = make_state("r1", "active");
        let p = params_set("r1", "alice");
        let result = assign_owner(&p, &mut state).unwrap();
        assert_eq!(result.assignee.as_deref(), Some("alice"));
        assert_eq!(result.previous_assignee, None);
        assert_eq!(state.assignee.as_deref(), Some("alice"));
    }

    #[test]
    fn clear_assignee() {
        let mut state = make_state("r2", "active");
        state.assignee = Some("bob".into());
        let p = params_clear("r2");
        let result = assign_owner(&p, &mut state).unwrap();
        assert_eq!(result.assignee, None);
        assert_eq!(result.previous_assignee.as_deref(), Some("bob"));
        assert_eq!(state.assignee, None);
    }

    #[test]
    fn assignee_is_lowercased() {
        let mut state = make_state("r3", "active");
        let p = params_set("r3", "BOB");
        let result = assign_owner(&p, &mut state).unwrap();
        assert_eq!(result.assignee.as_deref(), Some("bob"));
    }

    #[test]
    fn assignee_is_trimmed() {
        let mut state = make_state("r4", "active");
        let p = params_set("r4", "  carol  ");
        let result = assign_owner(&p, &mut state).unwrap();
        assert_eq!(result.assignee.as_deref(), Some("carol"));
    }

    #[test]
    fn assignee_empty_after_trim_is_rejected() {
        let mut state = make_state("r5", "active");
        let p = params_set("r5", "   ");
        let err = assign_owner(&p, &mut state).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn assignee_too_long_is_rejected() {
        let mut state = make_state("r6", "active");
        let p = params_set("r6", &"a".repeat(ASSIGNEE_MAX_LEN + 1));
        let err = assign_owner(&p, &mut state).unwrap_err();
        assert!(err.to_string().contains("exceeds maximum length"));
    }

    #[test]
    fn assignee_invalid_char_rejected() {
        let mut state = make_state("r7", "active");
        let p = params_set("r7", "alice@example");
        let err = assign_owner(&p, &mut state).unwrap_err();
        assert!(err.to_string().contains("invalid character"));
    }

    #[test]
    fn assignee_valid_special_chars() {
        let mut state = make_state("r8", "active");
        let p = params_set("r8", "alice.b_ob-42");
        let result = assign_owner(&p, &mut state).unwrap();
        assert_eq!(result.assignee.as_deref(), Some("alice.b_ob-42"));
    }

    #[test]
    fn absent_assignee_field_preserves_current() {
        let mut state = make_state("r9", "active");
        state.assignee = Some("dave".into());
        let p = RunAssignOwnerParams {
            run_id: "r9".into(),
            assignee: None,
            ownership_note: None,
        };
        let result = assign_owner(&p, &mut state).unwrap();
        assert_eq!(result.assignee.as_deref(), Some("dave"));
        assert_eq!(state.assignee.as_deref(), Some("dave"));
    }

    #[test]
    fn set_ownership_note() {
        let mut state = make_state("r10", "active");
        let p = RunAssignOwnerParams {
            run_id: "r10".into(),
            assignee: None,
            ownership_note: Some(Some("hand off to team B".to_string())),
        };
        let result = assign_owner(&p, &mut state).unwrap();
        assert_eq!(result.ownership_note.as_deref(), Some("hand off to team B"));
        assert_eq!(state.ownership_note.as_deref(), Some("hand off to team B"));
    }

    #[test]
    fn clear_ownership_note_with_null() {
        let mut state = make_state("r11", "active");
        state.ownership_note = Some("old note".into());
        let p = RunAssignOwnerParams {
            run_id: "r11".into(),
            assignee: None,
            ownership_note: Some(None),
        };
        let result = assign_owner(&p, &mut state).unwrap();
        assert_eq!(result.ownership_note, None);
        assert_eq!(state.ownership_note, None);
    }

    #[test]
    fn ownership_note_too_long_is_rejected() {
        let mut state = make_state("r12", "active");
        let p = RunAssignOwnerParams {
            run_id: "r12".into(),
            assignee: None,
            ownership_note: Some(Some("x".repeat(OWNERSHIP_NOTE_MAX_LEN + 1))),
        };
        let err = assign_owner(&p, &mut state).unwrap_err();
        assert!(err.to_string().contains("exceeds maximum length"));
    }

    #[test]
    fn absent_ownership_note_preserves_current() {
        let mut state = make_state("r13", "active");
        state.ownership_note = Some("existing note".into());
        let p = RunAssignOwnerParams {
            run_id: "r13".into(),
            assignee: None,
            ownership_note: None,
        };
        let result = assign_owner(&p, &mut state).unwrap();
        assert_eq!(result.ownership_note.as_deref(), Some("existing note"));
        assert_eq!(state.ownership_note.as_deref(), Some("existing note"));
    }

    #[test]
    fn does_not_mutate_status() {
        let mut state = make_state("r14", "finalized:completed");
        let p = params_set("r14", "alice");
        assign_owner(&p, &mut state).unwrap();
        assert_eq!(state.status, "finalized:completed");
    }
}
