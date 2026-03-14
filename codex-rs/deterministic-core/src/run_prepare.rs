//! Handler logic for `run.prepare`.

use anyhow::Result;
use deterministic_protocol::{RunPrepareParams, RunPrepareResult, RunState};
use uuid::Uuid;

/// Create a new deterministic run.
///
/// This compiles a run brief from the user goal and workspace metadata
/// and initialises the run state.  It does **not** invoke any LLM.
pub fn prepare(params: &RunPrepareParams) -> Result<(RunPrepareResult, RunState)> {
    let run_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let plan = vec![
        "inspect workspace".to_string(),
        "read relevant files".to_string(),
        "search code if needed".to_string(),
        "apply patch".to_string(),
        "run tests".to_string(),
        "show diff".to_string(),
    ];

    let state = RunState {
        run_id: run_id.clone(),
        workspace_id: params.workspace_id.clone(),
        user_goal: params.user_goal.clone(),
        status: "prepared".to_string(),
        plan: plan.clone(),
        current_step: 0,
        created_at: now.clone(),
        updated_at: now,
    };

    let result = RunPrepareResult {
        run_id: state.run_id.clone(),
        objective: params.user_goal.clone(),
        status: state.status.clone(),
        plan,
        current_step: 0,
        recommended_next_action: "Inspect the workspace to understand the codebase.".to_string(),
        recommended_tool: "get_workspace_summary".to_string(),
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
        };
        let (result, state) = prepare(&params).unwrap();
        assert_eq!(result.status, "prepared");
        assert!(!result.run_id.is_empty());
        assert_eq!(state.workspace_id, "/tmp/ws");
        assert_eq!(state.user_goal, "fix the bug");
    }
}
