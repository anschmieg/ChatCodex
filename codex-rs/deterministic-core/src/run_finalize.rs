//! Handler logic for `run.finalize`.
//!
//! Provides a deterministic, explicit closure surface for ChatGPT to mark
//! a run as completed, failed, or abandoned.  Persists a structured outcome
//! record.  No autonomous work is triggered.

use anyhow::{bail, Result};
use deterministic_protocol::{
    RunFinalizeParams, RunFinalizeResult, RunOutcome, RunState, VALID_OUTCOME_KINDS,
};

/// Finalize a run with a structured outcome record.
///
/// Deterministic lifecycle rules:
/// - `outcome_kind` must be one of: `completed`, `failed`, `abandoned`.
/// - A run that is already finalized cannot be finalized again.
/// - The run status is set to `finalized:<outcome_kind>`.
/// - No autonomous follow-up work is triggered.
pub fn finalize(params: &RunFinalizeParams, state: &mut RunState) -> Result<RunFinalizeResult> {
    // Validate outcome kind.
    if !VALID_OUTCOME_KINDS.contains(&params.outcome_kind.as_str()) {
        bail!(
            "invalid outcome_kind '{}': must be one of {}",
            params.outcome_kind,
            VALID_OUTCOME_KINDS.join(", ")
        );
    }

    // Reject if already finalized.
    if state.finalized_outcome.is_some() {
        bail!("run '{}' is already finalized", params.run_id);
    }

    let now = chrono::Utc::now().to_rfc3339();

    let outcome = RunOutcome {
        outcome_kind: params.outcome_kind.clone(),
        summary: params.summary.clone(),
        reason: params.reason.clone(),
        finalized_at: now.clone(),
    };

    state.finalized_outcome = Some(outcome);
    state.status = format!("finalized:{}", params.outcome_kind);
    state.updated_at = now.clone();

    // Deterministic guidance after finalization (no inference, no model calls).
    let recommended_next_action = match params.outcome_kind.as_str() {
        "completed" => "Run is complete. Review results or prepare a new run.",
        "failed" => "Run failed. Inspect the audit trail or prepare a new run.",
        "abandoned" => "Run was abandoned. Prepare a new run if work should continue.",
        _ => "Run finalized.",
    };

    Ok(RunFinalizeResult {
        run_id: params.run_id.clone(),
        outcome_kind: params.outcome_kind.clone(),
        finalized_at: now,
        status: state.status.clone(),
        recommended_next_action: recommended_next_action.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use deterministic_protocol::RunPolicy;

    fn make_state(run_id: &str) -> RunState {
        RunState {
            run_id: run_id.into(),
            workspace_id: "/tmp/ws".into(),
            user_goal: "fix bug".into(),
            status: "active".into(),
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
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn finalize_completed_success() {
        let mut state = make_state("r1");
        let params = RunFinalizeParams {
            run_id: "r1".into(),
            outcome_kind: "completed".into(),
            summary: "All steps done".into(),
            reason: None,
        };
        let result = finalize(&params, &mut state).unwrap();
        assert_eq!(result.run_id, "r1");
        assert_eq!(result.outcome_kind, "completed");
        assert_eq!(result.status, "finalized:completed");
        assert!(!result.finalized_at.is_empty());
        assert!(result.recommended_next_action.contains("complete"));
        // State reflects the outcome.
        let outcome = state.finalized_outcome.as_ref().unwrap();
        assert_eq!(outcome.outcome_kind, "completed");
        assert_eq!(outcome.summary, "All steps done");
        assert!(outcome.reason.is_none());
        assert_eq!(state.status, "finalized:completed");
    }

    #[test]
    fn finalize_failed_with_reason() {
        let mut state = make_state("r2");
        let params = RunFinalizeParams {
            run_id: "r2".into(),
            outcome_kind: "failed".into(),
            summary: "Tests did not pass".into(),
            reason: Some("build error in step 3".into()),
        };
        let result = finalize(&params, &mut state).unwrap();
        assert_eq!(result.outcome_kind, "failed");
        assert_eq!(result.status, "finalized:failed");
        assert!(result.recommended_next_action.contains("failed"));
        let outcome = state.finalized_outcome.as_ref().unwrap();
        assert_eq!(outcome.reason.as_deref(), Some("build error in step 3"));
    }

    #[test]
    fn finalize_abandoned() {
        let mut state = make_state("r3");
        let params = RunFinalizeParams {
            run_id: "r3".into(),
            outcome_kind: "abandoned".into(),
            summary: "No longer needed".into(),
            reason: Some("goal changed".into()),
        };
        let result = finalize(&params, &mut state).unwrap();
        assert_eq!(result.outcome_kind, "abandoned");
        assert_eq!(result.status, "finalized:abandoned");
        assert!(result.recommended_next_action.contains("abandoned"));
    }

    #[test]
    fn finalize_invalid_kind_rejected() {
        let mut state = make_state("r4");
        let params = RunFinalizeParams {
            run_id: "r4".into(),
            outcome_kind: "unknown_kind".into(),
            summary: "Done".into(),
            reason: None,
        };
        let err = finalize(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("invalid outcome_kind"));
        // State must not be mutated.
        assert!(state.finalized_outcome.is_none());
        assert_eq!(state.status, "active");
    }

    #[test]
    fn finalize_duplicate_rejected() {
        let mut state = make_state("r5");
        let params = RunFinalizeParams {
            run_id: "r5".into(),
            outcome_kind: "completed".into(),
            summary: "Done".into(),
            reason: None,
        };
        // First finalization succeeds.
        finalize(&params, &mut state).unwrap();
        // Second finalization must be rejected.
        let params2 = RunFinalizeParams {
            run_id: "r5".into(),
            outcome_kind: "abandoned".into(),
            summary: "Trying again".into(),
            reason: None,
        };
        let err = finalize(&params2, &mut state).unwrap_err();
        assert!(err.to_string().contains("already finalized"));
        // Original outcome must be preserved.
        let outcome = state.finalized_outcome.as_ref().unwrap();
        assert_eq!(outcome.outcome_kind, "completed");
    }

    #[test]
    fn valid_outcome_kinds_constants() {
        assert!(VALID_OUTCOME_KINDS.contains(&"completed"));
        assert!(VALID_OUTCOME_KINDS.contains(&"failed"));
        assert!(VALID_OUTCOME_KINDS.contains(&"abandoned"));
        assert!(!VALID_OUTCOME_KINDS.contains(&"done"));
    }
}
