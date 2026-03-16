//! Handler logic for `run.unarchive`.
//!
//! Provides a deterministic, explicit unarchiving surface for ChatGPT to restore
//! an archived run back to the normal visible working set.  Unarchiving does not
//! execute work, does not reopen the run, and does not change the finalized
//! outcome.  Only archived runs may be unarchived.

use anyhow::{bail, Result};
use deterministic_protocol::{RunState, RunUnarchiveParams, RunUnarchiveResult, UnarchiveMetadata};

/// Unarchive an archived run.
///
/// Deterministic lifecycle rules:
/// - Only runs with `archive_metadata` set (i.e. explicitly archived) may be unarchived.
/// - Non-archived runs are rejected.
/// - Already-unarchived runs are rejected (idempotent-safe rejection with a clear error).
/// - Unarchiving does not execute work.
/// - Unarchiving does not reopen the run or change status.
/// - Unarchiving does not clear finalized outcome, plan, completed steps, or audit history.
/// - Unarchiving does not modify lineage (supersession) metadata.
/// - Unarchive metadata is recorded on the run state.
/// - Original archive metadata remains intact for historical inspection.
///
/// Returns the updated run state.
pub fn unarchive(
    params: &RunUnarchiveParams,
    state: &mut RunState,
) -> Result<RunUnarchiveResult> {
    // Enforce: only archived runs can be unarchived.
    if state.archive_metadata.is_none() {
        bail!(
            "run '{}' cannot be unarchived: it is not archived",
            params.run_id
        );
    }

    // Reject if already unarchived.
    if state.unarchive_metadata.is_some() {
        bail!(
            "run '{}' is already unarchived",
            params.run_id
        );
    }

    let now = chrono::Utc::now().to_rfc3339();

    // Record compact unarchive metadata on the run state.
    state.unarchive_metadata = Some(UnarchiveMetadata {
        reason: params.reason.clone(),
        unarchived_at: now.clone(),
    });
    state.updated_at = now.clone();

    Ok(RunUnarchiveResult {
        run_id: params.run_id.clone(),
        status: state.status.clone(),
        unarchived_at: now,
        reason: params.reason.clone(),
        message: format!(
            "Run '{}' unarchived. It is now visible in the default active run listing.",
            params.run_id
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use deterministic_protocol::{ArchiveMetadata, RunOutcome, RunPolicy};

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
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        }
    }

    fn make_archived_state(run_id: &str, outcome_kind: &str) -> RunState {
        let mut s = make_state(run_id, &format!("finalized:{outcome_kind}"));
        s.finalized_outcome = Some(RunOutcome {
            outcome_kind: outcome_kind.into(),
            summary: "Done".into(),
            reason: None,
            finalized_at: "2024-01-01T01:00:00Z".into(),
        });
        s.archive_metadata = Some(ArchiveMetadata {
            reason: "archived for hygiene".into(),
            archived_at: "2024-01-01T02:00:00Z".into(),
        });
        s
    }

    // -----------------------------------------------------------------------
    // Happy-path: unarchive a completed run
    // -----------------------------------------------------------------------

    #[test]
    fn unarchive_completed_run_succeeds() {
        let mut state = make_archived_state("run-c", "completed");
        let params = RunUnarchiveParams {
            run_id: "run-c".into(),
            reason: "Restoring for follow-up inspection".into(),
        };
        let result = unarchive(&params, &mut state).unwrap();

        assert_eq!(result.run_id, "run-c");
        assert!(result.status.starts_with("finalized:"));
        assert!(!result.unarchived_at.is_empty());
        assert_eq!(result.reason, "Restoring for follow-up inspection");
        assert!(!result.message.is_empty());

        // State must carry unarchive_metadata.
        let meta = state.unarchive_metadata.as_ref().expect("unarchive_metadata must be set");
        assert_eq!(meta.reason, "Restoring for follow-up inspection");
        assert!(!meta.unarchived_at.is_empty());

        // Status must not change.
        assert_eq!(state.status, "finalized:completed");

        // Finalized outcome must be preserved.
        assert!(state.finalized_outcome.is_some());

        // Original archive metadata must remain intact.
        let arch = state.archive_metadata.as_ref().expect("archive_metadata must remain");
        assert_eq!(arch.reason, "archived for hygiene");
        assert_eq!(arch.archived_at, "2024-01-01T02:00:00Z");
    }

    // -----------------------------------------------------------------------
    // Happy-path: unarchive a failed run
    // -----------------------------------------------------------------------

    #[test]
    fn unarchive_failed_run_succeeds() {
        let mut state = make_archived_state("run-f", "failed");
        let params = RunUnarchiveParams {
            run_id: "run-f".into(),
            reason: "Reviewing failed build".into(),
        };
        let result = unarchive(&params, &mut state).unwrap();
        assert_eq!(result.run_id, "run-f");
        assert!(state.unarchive_metadata.is_some());
        // Original finalized outcome preserved.
        assert!(state.finalized_outcome.is_some());
        // Original archive metadata preserved.
        assert!(state.archive_metadata.is_some());
    }

    // -----------------------------------------------------------------------
    // Happy-path: unarchive an abandoned run
    // -----------------------------------------------------------------------

    #[test]
    fn unarchive_abandoned_run_succeeds() {
        let mut state = make_archived_state("run-a", "abandoned");
        let params = RunUnarchiveParams {
            run_id: "run-a".into(),
            reason: "Re-inspecting abandoned run".into(),
        };
        let result = unarchive(&params, &mut state).unwrap();
        assert_eq!(result.run_id, "run-a");
        assert!(state.unarchive_metadata.is_some());
    }

    // -----------------------------------------------------------------------
    // Reject: non-archived run cannot be unarchived
    // -----------------------------------------------------------------------

    #[test]
    fn unarchive_non_archived_run_rejected() {
        let mut state = make_state("run-na", "finalized:completed");
        // Not archived (no archive_metadata).
        let params = RunUnarchiveParams {
            run_id: "run-na".into(),
            reason: "trying".into(),
        };
        let err = unarchive(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("cannot be unarchived"));
        assert!(err.to_string().contains("not archived"));
        // State must not be mutated.
        assert!(state.unarchive_metadata.is_none());
    }

    // -----------------------------------------------------------------------
    // Reject: active run (not archived) cannot be unarchived
    // -----------------------------------------------------------------------

    #[test]
    fn unarchive_active_run_rejected() {
        let mut state = make_state("run-act", "active");
        let params = RunUnarchiveParams {
            run_id: "run-act".into(),
            reason: "trying".into(),
        };
        let err = unarchive(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("cannot be unarchived"));
        assert!(state.unarchive_metadata.is_none());
    }

    // -----------------------------------------------------------------------
    // Reject: already-unarchived run cannot be unarchived again
    // -----------------------------------------------------------------------

    #[test]
    fn unarchive_already_unarchived_run_rejected() {
        let mut state = make_archived_state("run-dup", "completed");
        state.unarchive_metadata = Some(UnarchiveMetadata {
            reason: "first unarchive".into(),
            unarchived_at: "2024-01-01T03:00:00Z".into(),
        });

        let params = RunUnarchiveParams {
            run_id: "run-dup".into(),
            reason: "unarchiving again".into(),
        };
        let err = unarchive(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("already unarchived"));
        // Original unarchive metadata must not be overwritten.
        assert_eq!(state.unarchive_metadata.as_ref().unwrap().reason, "first unarchive");
    }

    // -----------------------------------------------------------------------
    // Plan, completed steps, finalized outcome, and archive metadata are preserved
    // -----------------------------------------------------------------------

    #[test]
    fn unarchive_preserves_run_history() {
        let mut state = make_archived_state("run-hist", "completed");
        state.plan = vec!["step A".into(), "step B".into()];
        state.completed_steps = vec!["step A".into(), "step B".into()];

        let params = RunUnarchiveParams {
            run_id: "run-hist".into(),
            reason: "History preservation test".into(),
        };
        unarchive(&params, &mut state).unwrap();

        // Plan and completed steps must not be cleared.
        assert_eq!(state.plan, vec!["step A", "step B"]);
        assert_eq!(state.completed_steps, vec!["step A", "step B"]);
        // Finalized outcome must still be present.
        assert!(state.finalized_outcome.is_some());
        // Archive metadata must still be present.
        assert!(state.archive_metadata.is_some());
    }

    // -----------------------------------------------------------------------
    // Status is unchanged after unarchiving
    // -----------------------------------------------------------------------

    #[test]
    fn unarchive_does_not_change_status() {
        for outcome_kind in &["completed", "failed", "abandoned"] {
            let mut state = make_archived_state("run-st", outcome_kind);
            let expected_status = state.status.clone();
            let params = RunUnarchiveParams {
                run_id: "run-st".into(),
                reason: "status test".into(),
            };
            unarchive(&params, &mut state).unwrap();
            assert_eq!(state.status, expected_status, "status must not change after unarchiving");
        }
    }

    // -----------------------------------------------------------------------
    // Lineage metadata is preserved after unarchiving
    // -----------------------------------------------------------------------

    #[test]
    fn unarchive_preserves_supersession_lineage() {
        let mut state = make_archived_state("run-sup", "completed");
        state.superseded_by_run_id = Some("run-sup-v2".into());
        state.supersession_reason = Some("Replaced by v2".into());
        state.superseded_at = Some("2024-01-02T00:00:00Z".into());

        let params = RunUnarchiveParams {
            run_id: "run-sup".into(),
            reason: "Restoring superseded run".into(),
        };
        unarchive(&params, &mut state).unwrap();

        // Supersession lineage must be preserved.
        assert_eq!(state.superseded_by_run_id.as_deref(), Some("run-sup-v2"));
        assert_eq!(state.supersession_reason.as_deref(), Some("Replaced by v2"));
    }

    // -----------------------------------------------------------------------
    // updated_at is refreshed after unarchiving
    // -----------------------------------------------------------------------

    #[test]
    fn unarchive_updates_updated_at() {
        let mut state = make_archived_state("run-ts", "completed");
        let params = RunUnarchiveParams {
            run_id: "run-ts".into(),
            reason: "timestamp test".into(),
        };
        unarchive(&params, &mut state).unwrap();
        // updated_at must be a non-empty ISO 8601 timestamp after unarchiving.
        assert!(!state.updated_at.is_empty());
    }

    // -----------------------------------------------------------------------
    // Unarchiving does not reopen the run (status stays finalized)
    // -----------------------------------------------------------------------

    #[test]
    fn unarchive_does_not_reopen_run() {
        let mut state = make_archived_state("run-no-reopen", "completed");
        let params = RunUnarchiveParams {
            run_id: "run-no-reopen".into(),
            reason: "checking no reopen".into(),
        };
        unarchive(&params, &mut state).unwrap();
        // Status must remain finalized, not active/prepared.
        assert!(state.status.starts_with("finalized:"), "unarchiving must not reopen the run");
        // Finalized outcome must remain.
        assert!(state.finalized_outcome.is_some());
    }
}
