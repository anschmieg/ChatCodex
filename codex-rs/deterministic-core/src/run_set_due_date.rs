//! Deterministic run due-date update (Milestone 20).

use anyhow::{bail, Result};
use deterministic_protocol::types::{
    RunSetDueDateParams, RunSetDueDateResult, RunState,
};

/// Validate an ISO `YYYY-MM-DD` date string.
///
/// Rules:
/// - Exactly 10 characters.
/// - Format `YYYY-MM-DD` where Y/M/D are decimal digits.
/// - Month in `01..=12`, day in `01..=31` (calendar precision beyond that is
///   not enforced — the backend records the string, not a computed instant).
pub fn validate_due_date(s: &str) -> Result<String> {
    let trimmed = s.trim();
    if trimmed.len() != 10 {
        bail!(
            "due_date must be exactly 10 characters in YYYY-MM-DD format, got: '{trimmed}'"
        );
    }
    let bytes = trimmed.as_bytes();
    // Validate digit positions: 0-3, 5-6, 8-9; separators at 4 and 7.
    if bytes[4] != b'-' || bytes[7] != b'-' {
        bail!("due_date must use '-' separators in YYYY-MM-DD format, got: '{trimmed}'");
    }
    for idx in [0, 1, 2, 3, 5, 6, 8, 9] {
        if !bytes[idx].is_ascii_digit() {
            bail!("due_date contains non-digit character at position {idx} in '{trimmed}'");
        }
    }
    // Basic range checks.
    let month: u32 = trimmed[5..7].parse().unwrap_or(0);
    let day: u32 = trimmed[8..10].parse().unwrap_or(0);
    if !(1..=12).contains(&month) {
        bail!("due_date month must be 01–12, got '{trimmed}'");
    }
    if !(1..=31).contains(&day) {
        bail!("due_date day must be 01–31, got '{trimmed}'");
    }
    Ok(trimmed.to_string())
}

/// Set or clear the due date of a run.
pub fn set_due_date(
    params: &RunSetDueDateParams,
    state: &mut RunState,
) -> Result<RunSetDueDateResult> {
    let previous_due_date = state.due_date.clone();

    let new_due_date: Option<String> = match &params.due_date {
        None => {
            // The field was absent — treat as "no change" (preserve current).
            // Note: the params type wraps Option<Option<String>> so None means
            // the outer Option was None, which can't happen via normal JSON
            // deserialization.  We preserve as a safe fallback.
            state.due_date.clone()
        }
        Some(None) => None, // explicit JSON null → clear
        Some(Some(s)) => Some(validate_due_date(s)?), // string → validate
    };

    let now = chrono::Utc::now().to_rfc3339();
    state.due_date = new_due_date.clone();

    let message = match (&previous_due_date, &new_due_date) {
        (None, None) => "due date unchanged (no due date set)".to_string(),
        (None, Some(d)) => format!("due date set to {d}"),
        (Some(prev), None) => format!("due date {prev} cleared"),
        (Some(prev), Some(next)) if prev == next => format!("due date unchanged: {next}"),
        (Some(prev), Some(next)) => format!("due date updated from {prev} to {next}"),
    };

    Ok(RunSetDueDateResult {
        run_id: params.run_id.clone(),
        status: state.status.clone(),
        previous_due_date,
        due_date: new_due_date,
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
            blocked_by_run_ids: vec![],
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    fn params_set(run_id: &str, date: &str) -> RunSetDueDateParams {
        RunSetDueDateParams {
            run_id: run_id.into(),
            due_date: Some(Some(date.to_string())),
        }
    }

    fn params_clear(run_id: &str) -> RunSetDueDateParams {
        RunSetDueDateParams {
            run_id: run_id.into(),
            due_date: Some(None),
        }
    }

    // ------------------------------------------------------------------
    // validate_due_date tests
    // ------------------------------------------------------------------

    #[test]
    fn valid_date_accepted() {
        assert_eq!(validate_due_date("2026-03-31").unwrap(), "2026-03-31");
        assert_eq!(validate_due_date("2000-01-01").unwrap(), "2000-01-01");
        assert_eq!(validate_due_date("9999-12-31").unwrap(), "9999-12-31");
    }

    #[test]
    fn date_is_trimmed() {
        assert_eq!(validate_due_date("  2026-03-31  ").unwrap(), "2026-03-31");
    }

    #[test]
    fn invalid_length_rejected() {
        let err = validate_due_date("2026-3-1").unwrap_err();
        assert!(err.to_string().contains("exactly 10 characters"), "{err}");
    }

    #[test]
    fn invalid_separators_rejected() {
        let err = validate_due_date("2026/03/31").unwrap_err();
        assert!(err.to_string().contains("'-' separators"), "{err}");
    }

    #[test]
    fn non_digit_year_rejected() {
        let err = validate_due_date("YYYY-03-31").unwrap_err();
        assert!(err.to_string().contains("non-digit"), "{err}");
    }

    #[test]
    fn month_zero_rejected() {
        let err = validate_due_date("2026-00-01").unwrap_err();
        assert!(err.to_string().contains("month"), "{err}");
    }

    #[test]
    fn month_thirteen_rejected() {
        let err = validate_due_date("2026-13-01").unwrap_err();
        assert!(err.to_string().contains("month"), "{err}");
    }

    #[test]
    fn day_zero_rejected() {
        let err = validate_due_date("2026-01-00").unwrap_err();
        assert!(err.to_string().contains("day"), "{err}");
    }

    #[test]
    fn day_32_rejected() {
        let err = validate_due_date("2026-01-32").unwrap_err();
        assert!(err.to_string().contains("day"), "{err}");
    }

    // ------------------------------------------------------------------
    // set_due_date tests
    // ------------------------------------------------------------------

    #[test]
    fn set_due_date_basic() {
        let mut state = make_state("r1", "active");
        let p = params_set("r1", "2026-03-31");
        let result = set_due_date(&p, &mut state).unwrap();
        assert_eq!(result.due_date.as_deref(), Some("2026-03-31"));
        assert_eq!(result.previous_due_date, None);
        assert_eq!(state.due_date.as_deref(), Some("2026-03-31"));
    }

    #[test]
    fn replace_existing_due_date() {
        let mut state = make_state("r2", "active");
        state.due_date = Some("2026-01-01".into());
        let p = params_set("r2", "2026-06-30");
        let result = set_due_date(&p, &mut state).unwrap();
        assert_eq!(result.due_date.as_deref(), Some("2026-06-30"));
        assert_eq!(result.previous_due_date.as_deref(), Some("2026-01-01"));
        assert_eq!(state.due_date.as_deref(), Some("2026-06-30"));
    }

    #[test]
    fn clear_due_date() {
        let mut state = make_state("r3", "active");
        state.due_date = Some("2026-03-31".into());
        let p = params_clear("r3");
        let result = set_due_date(&p, &mut state).unwrap();
        assert_eq!(result.due_date, None);
        assert_eq!(result.previous_due_date.as_deref(), Some("2026-03-31"));
        assert_eq!(state.due_date, None);
    }

    #[test]
    fn clear_when_already_none() {
        let mut state = make_state("r4", "active");
        let p = params_clear("r4");
        let result = set_due_date(&p, &mut state).unwrap();
        assert_eq!(result.due_date, None);
        assert_eq!(result.previous_due_date, None);
        assert!(result.message.contains("unchanged"));
    }

    #[test]
    fn invalid_date_rejected() {
        let mut state = make_state("r5", "active");
        let p = params_set("r5", "not-a-date");
        let err = set_due_date(&p, &mut state).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("10 characters") || msg.contains("non-digit") || msg.contains("separators"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn does_not_mutate_status() {
        let mut state = make_state("r6", "finalized:completed");
        let p = params_set("r6", "2026-12-31");
        set_due_date(&p, &mut state).unwrap();
        assert_eq!(state.status, "finalized:completed");
    }

    #[test]
    fn message_set() {
        let mut state = make_state("r7", "active");
        let p = params_set("r7", "2026-07-04");
        let result = set_due_date(&p, &mut state).unwrap();
        assert!(result.message.contains("2026-07-04"), "{}", result.message);
    }

    #[test]
    fn message_cleared() {
        let mut state = make_state("r8", "active");
        state.due_date = Some("2026-07-04".into());
        let p = params_clear("r8");
        let result = set_due_date(&p, &mut state).unwrap();
        assert!(result.message.contains("cleared"), "{}", result.message);
    }
}
