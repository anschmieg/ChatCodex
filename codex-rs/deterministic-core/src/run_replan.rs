//! Handler logic for `run.replan`.
//!
//! Deterministic replanning: recomputes the run plan based on the
//! current state plus new evidence or failure context.
//!
//! This is **rule-based only** — no LLM calls, no hidden planning loop.

use anyhow::Result;
use deterministic_protocol::{RunReplanParams, RunReplanResult, RunState};

/// Deterministically replan a run.
///
/// Takes the current persisted state plus optional new evidence /
/// failure context and recomputes plan fields.  The result is a new
/// set of pending steps and a recommended next action.
///
/// Milestone 6: handles retryable action validity.  If failure context
/// is provided, stale retryable actions are invalidated.  If still
/// valid, they are preserved.  A concise replan delta is emitted.
pub fn replan(params: &RunReplanParams, state: &mut RunState) -> Result<RunReplanResult> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut delta_parts: Vec<String> = Vec::new();

    // Absorb new evidence into the observation log.
    if !params.new_evidence.is_empty() {
        let evidence_text = params.new_evidence.join("; ");
        state.last_observation = Some(format!("Replan evidence: {evidence_text}"));
        delta_parts.push(format!("absorbed {} new evidence item(s)", params.new_evidence.len()));
    }

    // If failure context is provided, mark the current step as failed
    // and adjust the plan.
    if let Some(ref failure) = params.failure_context {
        // If the status isn't already failed, move to active so we can
        // recover.  A truly terminal failure should have status "failed"
        // set externally.
        if state.status == "blocked" || state.status == "awaiting_approval" {
            let old_status = state.status.clone();
            state.status = "active".to_string();
            delta_parts.push(format!("status {old_status} → active"));
        }

        // Insert a recovery step at the current position.
        let recovery_step = format!("recover from failure: {}", truncate(failure, 120));
        if !state.pending_steps.contains(&recovery_step) {
            state.pending_steps.insert(0, recovery_step);
            delta_parts.push("inserted recovery step".into());
        }

        // Milestone 6: invalidate retryable action on failure context.
        if let Some(ref mut ra) = state.retryable_action
            && ra.is_valid
        {
            ra.is_valid = false;
            ra.is_recommended = false;
            ra.invalidation_reason = Some(format!(
                "Invalidated by replan failure context: {}",
                truncate(failure, 80)
            ));
            delta_parts.push(format!("invalidated retryable action '{}'", ra.kind));
        }
    } else {
        // Milestone 6: no failure context — preserve valid retryable actions,
        // but if status is recovering from blocked, reconsider validity.
        if state.status == "blocked" {
            if let Some(ref mut ra) = state.retryable_action
                && ra.is_valid
            {
                // Still blocked but no failure — the action may still be
                // valid if the user just wants to replan around it.
                ra.is_recommended = false;
                delta_parts.push(format!(
                    "retryable action '{}' preserved but not recommended (blocked state replan)",
                    ra.kind
                ));
            }
            state.status = "active".to_string();
            delta_parts.push("status blocked → active".into());
        }
    }

    // Determine recommended next action from pending steps.
    let (recommended_action, recommended_tool) = if state.pending_steps.is_empty() {
        state.status = "done".to_string();
        delta_parts.push("all steps complete".into());
        (
            "All steps complete — review diff and finalize.".to_string(),
            "show_diff".to_string(),
        )
    } else {
        let next = &state.pending_steps[0];
        recommend_for_step(next)
    };

    // If the run was prepared but not yet active, mark active on replan.
    if state.status == "prepared" {
        state.status = "active".to_string();
        delta_parts.push("status prepared → active".into());
    }

    state.recommended_next_action = Some(recommended_action.clone());
    state.recommended_tool = Some(recommended_tool.clone());
    state.updated_at = now;

    let summary = format!(
        "Replanned (reason: {}): {} pending step(s). Next: {}",
        truncate(&params.reason, 80),
        state.pending_steps.len(),
        recommended_action,
    );

    let replan_delta = if delta_parts.is_empty() {
        None
    } else {
        Some(delta_parts.join("; "))
    };

    Ok(RunReplanResult {
        run_id: state.run_id.clone(),
        status: state.status.clone(),
        current_step: state.current_step,
        pending_steps: state.pending_steps.clone(),
        recommended_next_action: recommended_action,
        recommended_tool,
        replan_summary: summary,
        retryable_action: state.retryable_action.clone(),
        replan_delta,
    })
}

/// Rule-based mapping from a plan step description to a recommended
/// tool.  This is intentionally simple and deterministic.
fn recommend_for_step(step: &str) -> (String, String) {
    let lower = step.to_lowercase();
    let tool = if lower.contains("inspect") || lower.contains("workspace") || lower.contains("summary") {
        "get_workspace_summary"
    } else if lower.contains("read") || lower.contains("file") {
        "read_file"
    } else if lower.contains("search") || lower.contains("find") || lower.contains("grep") {
        "search_code"
    } else if lower.contains("patch") || lower.contains("edit") || lower.contains("write") || lower.contains("apply") {
        "apply_patch"
    } else if lower.contains("test") || lower.contains("verify") {
        "run_tests"
    } else if lower.contains("diff") || lower.contains("review") {
        "show_diff"
    } else if lower.contains("git") || lower.contains("status") {
        "git_status"
    } else {
        // Default fallback for unknown step types (including recovery steps).
        "read_file"
    };
    (
        format!("Execute step: {}", truncate(step, 100)),
        tool.to_string(),
    )
}

fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        &s[..max_len]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state() -> RunState {
        RunState {
            run_id: "r1".into(),
            workspace_id: "/tmp/ws".into(),
            user_goal: "fix bug".into(),
            status: "active".into(),
            plan: vec![
                "inspect workspace".into(),
                "read relevant files".into(),
                "apply patch".into(),
                "run tests".into(),
            ],
            current_step: 1,
            completed_steps: vec!["inspect workspace".into()],
            pending_steps: vec![
                "read relevant files".into(),
                "apply patch".into(),
                "run tests".into(),
            ],
            last_action: Some("inspected workspace".into()),
            last_observation: Some("found 3 source files".into()),
            recommended_next_action: Some("read files".into()),
            recommended_tool: Some("read_file".into()),
            latest_diff_summary: None,
            latest_test_result: None,
            focus_paths: vec![],
            warnings: vec![],
            retryable_action: None,
            policy_profile: deterministic_protocol::RunPolicy::default(),
            finalized_outcome: None,
            reopen_metadata: None,
            supersedes_run_id: None,
            superseded_by_run_id: None,
            supersession_reason: None,
            superseded_at: None,
            archive_metadata: None,
            unarchive_metadata: None,
            annotation: None,
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn replan_updates_recommendation() {
        let mut state = make_state();
        let params = RunReplanParams {
            run_id: "r1".into(),
            reason: "new info".into(),
            new_evidence: vec!["found new test failures".into()],
            failure_context: None,
        };
        let result = replan(&params, &mut state).unwrap();
        assert_eq!(result.run_id, "r1");
        assert_eq!(result.status, "active");
        assert!(!result.replan_summary.is_empty());
        assert!(!result.recommended_next_action.is_empty());
        assert!(!result.recommended_tool.is_empty());
    }

    #[test]
    fn replan_with_failure_inserts_recovery_step() {
        let mut state = make_state();
        let params = RunReplanParams {
            run_id: "r1".into(),
            reason: "tests failed".into(),
            new_evidence: vec![],
            failure_context: Some("compilation error in main.rs".into()),
        };
        let result = replan(&params, &mut state).unwrap();
        assert!(result.pending_steps[0].contains("recover from failure"));
    }

    #[test]
    fn replan_empty_pending_marks_done() {
        let mut state = make_state();
        state.pending_steps = vec![];
        let params = RunReplanParams {
            run_id: "r1".into(),
            reason: "check completion".into(),
            new_evidence: vec![],
            failure_context: None,
        };
        let result = replan(&params, &mut state).unwrap();
        assert_eq!(result.status, "done");
        assert_eq!(result.recommended_tool, "show_diff");
    }

    #[test]
    fn replan_from_prepared_to_active() {
        let mut state = make_state();
        state.status = "prepared".to_string();
        let params = RunReplanParams {
            run_id: "r1".into(),
            reason: "start working".into(),
            new_evidence: vec![],
            failure_context: None,
        };
        let result = replan(&params, &mut state).unwrap();
        assert_eq!(result.status, "active");
    }

    #[test]
    fn replan_from_blocked_with_failure_goes_active() {
        let mut state = make_state();
        state.status = "blocked".to_string();
        let params = RunReplanParams {
            run_id: "r1".into(),
            reason: "unblock".into(),
            new_evidence: vec![],
            failure_context: Some("resolved the blocking issue".into()),
        };
        let result = replan(&params, &mut state).unwrap();
        assert_eq!(result.status, "active");
    }

    #[test]
    fn recommend_for_step_maps_correctly() {
        let cases = vec![
            ("inspect workspace", "get_workspace_summary"),
            ("read the file", "read_file"),
            ("search for usage", "search_code"),
            ("apply patch fix", "apply_patch"),
            ("run tests to verify", "run_tests"),
            ("show diff for review", "show_diff"),
            ("check git status", "git_status"),
            ("do something unknown", "read_file"),
        ];
        for (step, expected_tool) in cases {
            let (_, tool) = recommend_for_step(step);
            assert_eq!(tool, expected_tool, "step '{step}' should map to '{expected_tool}'");
        }
    }

    // ---- Milestone 6: retryable action during replan ----

    fn make_retryable_action() -> deterministic_protocol::RetryableAction {
        deterministic_protocol::RetryableAction {
            kind: "patch.apply".into(),
            summary: "Edit src/main.rs".into(),
            payload: Some(r#"{"run_id":"r1","edits":[]}"#.into()),
            retryable_reason: "Blocked by approval policy".into(),
            is_valid: true,
            is_recommended: true,
            invalidation_reason: None,
            recommended_tool: "apply_patch".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn replan_with_failure_invalidates_retryable_action() {
        let mut state = make_state();
        state.retryable_action = Some(make_retryable_action());

        let params = RunReplanParams {
            run_id: "r1".into(),
            reason: "tests failed".into(),
            new_evidence: vec![],
            failure_context: Some("compilation error".into()),
        };
        let result = replan(&params, &mut state).unwrap();

        // Retryable action should be invalidated.
        let ra = result.retryable_action.as_ref().unwrap();
        assert!(!ra.is_valid);
        assert!(!ra.is_recommended);
        assert!(ra.invalidation_reason.as_deref().unwrap().contains("failure context"));

        // Replan delta should mention invalidation.
        assert!(result.replan_delta.as_deref().unwrap().contains("invalidated"));
    }

    #[test]
    fn replan_without_failure_preserves_retryable_action() {
        let mut state = make_state();
        state.retryable_action = Some(make_retryable_action());

        let params = RunReplanParams {
            run_id: "r1".into(),
            reason: "add new evidence".into(),
            new_evidence: vec!["found more context".into()],
            failure_context: None,
        };
        let result = replan(&params, &mut state).unwrap();

        // Retryable action should be preserved and still valid.
        let ra = result.retryable_action.as_ref().unwrap();
        assert!(ra.is_valid);
        assert!(ra.invalidation_reason.is_none());
    }

    #[test]
    fn replan_from_blocked_preserves_but_unrecommends_retryable() {
        let mut state = make_state();
        state.status = "blocked".to_string();
        state.retryable_action = Some(make_retryable_action());

        let params = RunReplanParams {
            run_id: "r1".into(),
            reason: "try alternative approach".into(),
            new_evidence: vec![],
            failure_context: None,
        };
        let result = replan(&params, &mut state).unwrap();
        assert_eq!(result.status, "active");

        let ra = result.retryable_action.as_ref().unwrap();
        assert!(ra.is_valid);
        assert!(!ra.is_recommended);

        assert!(result.replan_delta.as_deref().unwrap().contains("preserved but not recommended"));
    }

    #[test]
    fn replan_delta_contains_evidence_count() {
        let mut state = make_state();
        let params = RunReplanParams {
            run_id: "r1".into(),
            reason: "more info".into(),
            new_evidence: vec!["a".into(), "b".into()],
            failure_context: None,
        };
        let result = replan(&params, &mut state).unwrap();
        assert!(result.replan_delta.as_deref().unwrap().contains("2 new evidence"));
    }

    #[test]
    fn replan_no_retryable_action_still_works() {
        let mut state = make_state();
        let params = RunReplanParams {
            run_id: "r1".into(),
            reason: "plain replan".into(),
            new_evidence: vec![],
            failure_context: None,
        };
        let result = replan(&params, &mut state).unwrap();
        assert!(result.retryable_action.is_none());
    }
}
