//! Handler logic for `run.prepare`.

use anyhow::Result;
use deterministic_protocol::{RunPolicy, RunPrepareParams, RunPrepareResult, RunState};
use uuid::Uuid;

/// Validate a user-supplied `RunPolicy`, returning an error if any value is
/// out of range.  Called only when the caller explicitly provides a policy.
/// Also normalises `extra_safe_make_targets` to lowercase in-place so that
/// approval evaluation does not need repeated allocations.
fn validate_policy(policy: &mut RunPolicy) -> Result<()> {
    if policy.patch_edit_threshold == 0 {
        return Err(anyhow::anyhow!(
            "policy.patchEditThreshold must be >= 1"
        ));
    }
    for target in &policy.extra_safe_make_targets {
        if target.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "policy.extraSafeMakeTargets entries must not be empty"
            ));
        }
    }
    // Normalise to lowercase so approval evaluation can use direct equality.
    for target in &mut policy.extra_safe_make_targets {
        *target = target.to_lowercase();
    }
    Ok(())
}

/// Create a new deterministic run.
///
/// This compiles a run brief from the user goal and workspace metadata
/// and initialises the run state.  It does **not** invoke any LLM.
pub fn prepare(params: &RunPrepareParams) -> Result<(RunPrepareResult, RunState)> {
    let run_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    // Build effective policy from caller input or deterministic defaults.
    let mut effective_policy: RunPolicy = params.policy.clone().unwrap_or_default();

    // Validate any explicitly provided policy.
    if params.policy.is_some() {
        validate_policy(&mut effective_policy)?;
    }

    // If focus_paths are given at the top level but the policy's focus_paths
    // are empty, promote them into the policy (backward compat).
    if effective_policy.focus_paths.is_empty() && !params.focus_paths.is_empty() {
        effective_policy.focus_paths = params.focus_paths.clone();
    }

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
        // Keep focus_paths in RunState for backward compat, sourced from policy.
        focus_paths: effective_policy.focus_paths.clone(),
        warnings: vec![],
        retryable_action: None,
        policy_profile: effective_policy,
        created_at: now.clone(),
        updated_at: now,
    };

    Ok((result, state))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn prepare_default_policy_has_expected_values() {
        let params = RunPrepareParams {
            workspace_id: "/tmp/ws".to_string(),
            user_goal: "fix".to_string(),
            focus_paths: vec![],
            mode: None,
            policy: None,
        };
        let (result, state) = prepare(&params).unwrap();
        assert_eq!(result.effective_policy.patch_edit_threshold, 5);
        assert!(result.effective_policy.delete_requires_approval);
        assert!(result.effective_policy.sensitive_path_requires_approval);
        assert!(result.effective_policy.outside_focus_requires_approval);
        assert!(result.effective_policy.extra_safe_make_targets.is_empty());
        // Policy stored in state.
        assert_eq!(state.policy_profile, result.effective_policy);
    }

    #[test]
    fn prepare_focus_paths_promoted_to_policy() {
        let params = RunPrepareParams {
            workspace_id: "/tmp/ws".to_string(),
            user_goal: "fix".to_string(),
            focus_paths: vec!["src/".to_string()],
            mode: None,
            policy: None,
        };
        let (result, state) = prepare(&params).unwrap();
        assert_eq!(result.effective_policy.focus_paths, vec!["src/"]);
        assert_eq!(state.focus_paths, vec!["src/"]);
        assert_eq!(state.policy_profile.focus_paths, vec!["src/"]);
    }

    #[test]
    fn prepare_custom_policy_roundtrip() {
        let custom = RunPolicy {
            patch_edit_threshold: 10,
            delete_requires_approval: false,
            sensitive_path_requires_approval: true,
            outside_focus_requires_approval: true,
            extra_safe_make_targets: vec!["deploy-staging".to_string()],
            focus_paths: vec!["src/".to_string(), "tests/".to_string()],
        };
        let params = RunPrepareParams {
            workspace_id: "/tmp/ws".to_string(),
            user_goal: "fix".to_string(),
            focus_paths: vec![],
            mode: None,
            policy: Some(custom.clone()),
        };
        let (result, state) = prepare(&params).unwrap();
        assert_eq!(result.effective_policy, custom);
        assert_eq!(state.policy_profile, custom);
    }

    #[test]
    fn prepare_invalid_policy_threshold_zero_is_rejected() {
        let params = RunPrepareParams {
            workspace_id: "/tmp/ws".to_string(),
            user_goal: "fix".to_string(),
            focus_paths: vec![],
            mode: None,
            policy: Some(RunPolicy {
                patch_edit_threshold: 0,
                ..RunPolicy::default()
            }),
        };
        let err = prepare(&params).unwrap_err();
        assert!(err.to_string().contains("patchEditThreshold"));
    }

    #[test]
    fn prepare_invalid_policy_empty_make_target_is_rejected() {
        let params = RunPrepareParams {
            workspace_id: "/tmp/ws".to_string(),
            user_goal: "fix".to_string(),
            focus_paths: vec![],
            mode: None,
            policy: Some(RunPolicy {
                extra_safe_make_targets: vec!["  ".to_string()],
                ..RunPolicy::default()
            }),
        };
        let err = prepare(&params).unwrap_err();
        assert!(err.to_string().contains("extraSafeMakeTargets"));
    }

    #[test]
    fn prepare_policy_focus_paths_takes_precedence_over_params() {
        // When policy explicitly provides focus_paths, they win.
        let params = RunPrepareParams {
            workspace_id: "/tmp/ws".to_string(),
            user_goal: "fix".to_string(),
            focus_paths: vec!["other/".to_string()],
            mode: None,
            policy: Some(RunPolicy {
                focus_paths: vec!["src/".to_string()],
                ..RunPolicy::default()
            }),
        };
        let (result, state) = prepare(&params).unwrap();
        assert_eq!(result.effective_policy.focus_paths, vec!["src/"]);
        assert_eq!(state.focus_paths, vec!["src/"]);
    }
}
