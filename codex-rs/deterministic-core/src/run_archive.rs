//! Handler logic for `run.archive`.
//!
//! Provides a deterministic, explicit archiving surface for ChatGPT to mark a
//! finalized run as archived.  Archived runs remain fully preserved and
//! inspectable; archiving merely distinguishes historical runs from the active
//! working set.  Only finalized runs may be archived.

use anyhow::{bail, Result};
use deterministic_protocol::{ArchiveMetadata, RunArchiveParams, RunArchiveResult, RunState};

/// Archive a finalized run.
///
/// Deterministic lifecycle rules:
/// - Only finalized runs (`status` starts with `"finalized:"`) may be archived.
/// - Active, prepared, or awaiting-approval runs are rejected.
/// - Already-archived runs are rejected (idempotent-safe rejection with a clear error).
/// - Archiving does not execute work.
/// - Archiving does not clear plan, completed steps, audit history, or finalized outcome.
/// - Archive metadata is appended to the run state.
///
/// Returns the updated run state.
pub fn archive(
    params: &RunArchiveParams,
    state: &mut RunState,
) -> Result<RunArchiveResult> {
    // Enforce: only finalized runs can be archived.
    if !state.status.starts_with("finalized:") {
        bail!(
            "run '{}' cannot be archived: status is '{}' (only finalized runs may be archived)",
            params.run_id,
            state.status
        );
    }

    // Reject if already archived.
    if state.archive_metadata.is_some() {
        bail!(
            "run '{}' is already archived",
            params.run_id
        );
    }

    let now = chrono::Utc::now().to_rfc3339();

    // Record compact archive metadata on the run state.
    state.archive_metadata = Some(ArchiveMetadata {
        reason: params.reason.clone(),
        archived_at: now.clone(),
    });
    state.updated_at = now.clone();

    Ok(RunArchiveResult {
        run_id: params.run_id.clone(),
        status: state.status.clone(),
        archived_at: now,
        reason: params.reason.clone(),
        message: format!(
            "Run '{}' archived. It remains preserved and inspectable but will be excluded from the default active list.",
            params.run_id
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use deterministic_protocol::{RunOutcome, RunPolicy};

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
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        }
    }

    fn make_finalized_state(run_id: &str, outcome_kind: &str) -> RunState {
        let mut s = make_state(run_id, &format!("finalized:{outcome_kind}"));
        s.finalized_outcome = Some(RunOutcome {
            outcome_kind: outcome_kind.into(),
            summary: "Done".into(),
            reason: None,
            finalized_at: "2024-01-01T01:00:00Z".into(),
        });
        s
    }

    // -----------------------------------------------------------------------
    // Happy-path: archive a completed run
    // -----------------------------------------------------------------------

    #[test]
    fn archive_completed_run_succeeds() {
        let mut state = make_finalized_state("run-c", "completed");
        let params = RunArchiveParams {
            run_id: "run-c".into(),
            reason: "Completed long ago; archiving for hygiene".into(),
        };
        let result = archive(&params, &mut state).unwrap();

        assert_eq!(result.run_id, "run-c");
        assert!(result.status.starts_with("finalized:"));
        assert!(!result.archived_at.is_empty());
        assert_eq!(result.reason, "Completed long ago; archiving for hygiene");
        assert!(!result.message.is_empty());

        // State must carry archive_metadata.
        let meta = state.archive_metadata.as_ref().expect("archive_metadata must be set");
        assert_eq!(meta.reason, "Completed long ago; archiving for hygiene");
        assert!(!meta.archived_at.is_empty());

        // Status must not change.
        assert_eq!(state.status, "finalized:completed");

        // Finalized outcome must be preserved.
        assert!(state.finalized_outcome.is_some());
    }

    // -----------------------------------------------------------------------
    // Happy-path: archive a failed run
    // -----------------------------------------------------------------------

    #[test]
    fn archive_failed_run_succeeds() {
        let mut state = make_finalized_state("run-f", "failed");
        let params = RunArchiveParams {
            run_id: "run-f".into(),
            reason: "Failed build archived".into(),
        };
        let result = archive(&params, &mut state).unwrap();
        assert_eq!(result.run_id, "run-f");
        assert!(state.archive_metadata.is_some());
        // Original finalized outcome preserved.
        assert!(state.finalized_outcome.is_some());
    }

    // -----------------------------------------------------------------------
    // Happy-path: archive an abandoned run
    // -----------------------------------------------------------------------

    #[test]
    fn archive_abandoned_run_succeeds() {
        let mut state = make_finalized_state("run-a", "abandoned");
        let params = RunArchiveParams {
            run_id: "run-a".into(),
            reason: "Abandoned; keeping for records".into(),
        };
        let result = archive(&params, &mut state).unwrap();
        assert_eq!(result.run_id, "run-a");
        assert!(state.archive_metadata.is_some());
    }

    // -----------------------------------------------------------------------
    // Happy-path: archive a superseded run (still finalized status)
    // -----------------------------------------------------------------------

    #[test]
    fn archive_superseded_run_succeeds() {
        let mut state = make_finalized_state("run-s", "completed");
        state.superseded_by_run_id = Some("run-s-v2".into());
        state.supersession_reason = Some("Replaced by v2".into());
        state.superseded_at = Some("2024-01-02T00:00:00Z".into());

        let params = RunArchiveParams {
            run_id: "run-s".into(),
            reason: "Superseded run archived".into(),
        };
        let result = archive(&params, &mut state).unwrap();
        assert_eq!(result.run_id, "run-s");
        assert!(state.archive_metadata.is_some());
        // Supersession lineage must be preserved.
        assert_eq!(state.superseded_by_run_id.as_deref(), Some("run-s-v2"));
    }

    // -----------------------------------------------------------------------
    // Reject: active run cannot be archived
    // -----------------------------------------------------------------------

    #[test]
    fn archive_active_run_rejected() {
        let mut state = make_state("run-act", "active");
        let params = RunArchiveParams {
            run_id: "run-act".into(),
            reason: "trying".into(),
        };
        let err = archive(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("cannot be archived"));
        assert!(err.to_string().contains("active"));
        // State must not be mutated.
        assert!(state.archive_metadata.is_none());
        assert_eq!(state.status, "active");
    }

    // -----------------------------------------------------------------------
    // Reject: prepared run cannot be archived
    // -----------------------------------------------------------------------

    #[test]
    fn archive_prepared_run_rejected() {
        let mut state = make_state("run-prep", "prepared");
        let params = RunArchiveParams {
            run_id: "run-prep".into(),
            reason: "trying".into(),
        };
        let err = archive(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("cannot be archived"));
        assert!(err.to_string().contains("prepared"));
        assert!(state.archive_metadata.is_none());
    }

    // -----------------------------------------------------------------------
    // Reject: awaiting-approval run cannot be archived
    // -----------------------------------------------------------------------

    #[test]
    fn archive_awaiting_approval_run_rejected() {
        let mut state = make_state("run-aa", "awaiting_approval");
        let params = RunArchiveParams {
            run_id: "run-aa".into(),
            reason: "trying".into(),
        };
        let err = archive(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("cannot be archived"));
        assert!(state.archive_metadata.is_none());
    }

    // -----------------------------------------------------------------------
    // Reject: already-archived run cannot be archived again
    // -----------------------------------------------------------------------

    #[test]
    fn archive_already_archived_run_rejected() {
        let mut state = make_finalized_state("run-dup", "completed");
        state.archive_metadata = Some(ArchiveMetadata {
            reason: "first archive".into(),
            archived_at: "2024-01-01T02:00:00Z".into(),
        });

        let params = RunArchiveParams {
            run_id: "run-dup".into(),
            reason: "archiving again".into(),
        };
        let err = archive(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("already archived"));
        // Original archive metadata must not be overwritten.
        assert_eq!(state.archive_metadata.as_ref().unwrap().reason, "first archive");
    }

    // -----------------------------------------------------------------------
    // Plan, completed steps, and finalized outcome are preserved after archiving
    // -----------------------------------------------------------------------

    #[test]
    fn archive_preserves_run_history() {
        let mut state = make_finalized_state("run-hist", "completed");
        state.plan = vec!["step A".into(), "step B".into()];
        state.completed_steps = vec!["step A".into(), "step B".into()];

        let params = RunArchiveParams {
            run_id: "run-hist".into(),
            reason: "History preservation test".into(),
        };
        archive(&params, &mut state).unwrap();

        // Plan and completed steps must not be cleared.
        assert_eq!(state.plan, vec!["step A", "step B"]);
        assert_eq!(state.completed_steps, vec!["step A", "step B"]);
        // Finalized outcome must still be present.
        assert!(state.finalized_outcome.is_some());
    }

    // -----------------------------------------------------------------------
    // Status is unchanged after archiving
    // -----------------------------------------------------------------------

    #[test]
    fn archive_does_not_change_status() {
        for outcome_kind in &["completed", "failed", "abandoned"] {
            let mut state = make_finalized_state("run-st", outcome_kind);
            let expected_status = state.status.clone();
            let params = RunArchiveParams {
                run_id: "run-st".into(),
                reason: "status test".into(),
            };
            archive(&params, &mut state).unwrap();
            assert_eq!(state.status, expected_status, "status must not change after archiving");
        }
    }

    // -----------------------------------------------------------------------
    // updated_at is refreshed after archiving
    // -----------------------------------------------------------------------

    #[test]
    fn archive_updates_updated_at() {
        let mut state = make_finalized_state("run-ts", "completed");
        let params = RunArchiveParams {
            run_id: "run-ts".into(),
            reason: "timestamp test".into(),
        };
        archive(&params, &mut state).unwrap();
        // updated_at must be a non-empty ISO 8601 timestamp after archiving.
        assert!(!state.updated_at.is_empty());
    }
}
