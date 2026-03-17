//! Handler logic for `run.prepare`.

use anyhow::Result;
use deterministic_protocol::{RunPolicy, RunPrepareParams, RunPrepareResult, RunState};
use uuid::Uuid;

/// Create a new deterministic run.
///
/// This compiles a run brief from the user goal and workspace metadata
/// and initialises the run state.  It does **not** invoke any LLM.
///
/// Milestone 8: if `params.policy` is supplied the provided settings are
/// validated and merged with defaults to produce the effective `RunPolicy`.
/// When omitted the default policy is used (matching pre-M8 behaviour).
pub fn prepare(params: &RunPrepareParams) -> Result<(RunPrepareResult, RunState)> {
    let run_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    // Build the effective policy for this run (Milestone 8).
    let effective_policy: RunPolicy = params
        .policy
        .clone()
        .unwrap_or_default()
        .into_policy(params.focus_paths.clone());

    let plan = vec![
        "inspect workspace".to_string(),
        "read relevant files".to_string(),
        "search code if needed".to_string(),
        "apply patch".to_string(),
        "run tests".to_string(),
        "show diff".to_string(),
    ];

    // Deterministic constraints — enforced server-side, not by an LLM.
    let constraints = vec![
        "All file writes must go through apply_patch".to_string(),
        "All test execution must go through run_tests".to_string(),
        "No autonomous continuation — each step requires explicit invocation".to_string(),
    ];

    let assistant_brief = format!(
        "Goal: {}. Workspace: {}. Plan has {} steps. Start by inspecting the workspace.",
        params.user_goal,
        params.workspace_id,
        plan.len()
    );

    let result = RunPrepareResult {
        run_id: run_id.clone(),
        objective: params.user_goal.clone(),
        assistant_brief,
        constraints,
        status: "prepared".to_string(),
        plan: plan.clone(),
        current_step: 0,
        recommended_next_action: "Inspect the workspace to understand the codebase.".to_string(),
        recommended_tool: "get_workspace_summary".to_string(),
        effective_policy: effective_policy.clone(),
    };

    let state = RunState {
        run_id,
        workspace_id: params.workspace_id.clone(),
        user_goal: params.user_goal.clone(),
        status: "prepared".to_string(),
        plan: plan.clone(),
        current_step: 0,
        completed_steps: vec![],
        pending_steps: plan,
        last_action: None,
        last_observation: None,
        recommended_next_action: Some(
            "Inspect the workspace to understand the codebase.".to_string(),
        ),
        recommended_tool: Some("get_workspace_summary".to_string()),
        latest_diff_summary: None,
        latest_test_result: None,
        focus_paths: params.focus_paths.clone(),
        warnings: vec![],
        retryable_action: None,
        policy_profile: effective_policy,
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
        created_at: now.clone(),
        updated_at: now,
    };

    Ok((result, state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use deterministic_protocol::RunPolicyInput;

    #[test]
    fn prepare_creates_run() {
        let params = RunPrepareParams {
            workspace_id: "/tmp/ws".to_string(),
            user_goal: "fix the bug".to_string(),
            focus_paths: vec![],
            mode: None,
            policy: None,
        };
        let (result, state) = prepare(&params).unwrap();
        assert_eq!(result.status, "prepared");
        assert!(!result.run_id.is_empty());
        assert!(!result.assistant_brief.is_empty());
        assert!(!result.constraints.is_empty());
        assert_eq!(state.workspace_id, "/tmp/ws");
        assert_eq!(state.user_goal, "fix the bug");
    }

    #[test]
    fn prepare_uses_default_policy_when_none_provided() {
        let params = RunPrepareParams {
            workspace_id: "/tmp/ws".to_string(),
            user_goal: "fix bug".to_string(),
            focus_paths: vec![],
            mode: None,
            policy: None,
        };
        let (result, state) = prepare(&params).unwrap();
        let defaults = RunPolicy::default();
        assert_eq!(result.effective_policy.patch_edit_threshold, defaults.patch_edit_threshold);
        assert_eq!(result.effective_policy.delete_requires_approval, defaults.delete_requires_approval);
        assert_eq!(state.policy_profile.patch_edit_threshold, defaults.patch_edit_threshold);
    }

    #[test]
    fn prepare_applies_custom_policy() {
        let params = RunPrepareParams {
            workspace_id: "/tmp/ws".to_string(),
            user_goal: "big refactor".to_string(),
            focus_paths: vec!["src/".to_string()],
            mode: None,
            policy: Some(RunPolicyInput {
                patch_edit_threshold: Some(20),
                delete_requires_approval: Some(false),
                ..Default::default()
            }),
        };
        let (result, state) = prepare(&params).unwrap();
        assert_eq!(result.effective_policy.patch_edit_threshold, 20);
        assert!(!result.effective_policy.delete_requires_approval);
        assert_eq!(result.effective_policy.focus_paths, vec!["src/"]);
        assert_eq!(state.policy_profile.patch_edit_threshold, 20);
    }

    #[test]
    fn prepare_copies_focus_paths_into_policy() {
        let params = RunPrepareParams {
            workspace_id: "/tmp/ws".to_string(),
            user_goal: "fix bug".to_string(),
            focus_paths: vec!["lib/".to_string()],
            mode: None,
            policy: None,
        };
        let (result, state) = prepare(&params).unwrap();
        assert_eq!(result.effective_policy.focus_paths, vec!["lib/"]);
        assert_eq!(state.policy_profile.focus_paths, vec!["lib/"]);
    }
}
