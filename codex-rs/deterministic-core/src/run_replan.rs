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
pub fn replan(params: &RunReplanParams, state: &mut RunState) -> Result<RunReplanResult> {
    let now = chrono::Utc::now().to_rfc3339();

    // Absorb new evidence into the observation log.
    if !params.new_evidence.is_empty() {
        let evidence_text = params.new_evidence.join("; ");
        state.last_observation = Some(format!("Replan evidence: {evidence_text}"));
    }

    // If failure context is provided, mark the current step as failed
    // and adjust the plan.
    if let Some(ref failure) = params.failure_context {
        // If the status isn't already failed, move to active so we can
        // recover.  A truly terminal failure should have status "failed"
        // set externally.
        if state.status == "blocked" || state.status == "awaiting_approval" {
            state.status = "active".to_string();
        }

        // Insert a recovery step at the current position.
        let recovery_step = format!("recover from failure: {}", truncate(failure, 120));
        if !state.pending_steps.contains(&recovery_step) {
            state.pending_steps.insert(0, recovery_step);
        }
    }

    // Determine recommended next action from pending steps.
    let (recommended_action, recommended_tool) = if state.pending_steps.is_empty() {
        state.status = "done".to_string();
        (
            "All steps complete — review diff and finalise.".to_string(),
            "show_diff".to_string(),
        )
    } else {
        let next = &state.pending_steps[0];
        recommend_for_step(next)
    };

    // If the run was prepared but not yet active, mark active on replan.
    if state.status == "prepared" {
        state.status = "active".to_string();
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

    Ok(RunReplanResult {
        run_id: state.run_id.clone(),
        status: state.status.clone(),
        current_step: state.current_step,
        pending_steps: state.pending_steps.clone(),
        recommended_next_action: recommended_action,
        recommended_tool,
        replan_summary: summary,
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
            warnings: vec![],
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
}
