//! Deterministic run dependency-link update (Milestone 21).
//!
//! This module implements the pure business logic for setting, replacing, or
//! clearing the explicit blocking relationships on a run.  All validation is
//! deterministic — no model calls, no autonomous scheduling, no background
//! wakeups.

use anyhow::{bail, Result};
use deterministic_protocol::types::{
    RunSetDependenciesParams, RunSetDependenciesResult, RunState, MAX_DEPENDENCY_COUNT,
};

/// Set, replace, or clear the dependency (blocked-by) list of a run.
///
/// Validation:
/// - Self-dependency is rejected.
/// - Duplicate IDs are normalized away deterministically (deduplication + sort).
/// - The list length must not exceed [`MAX_DEPENDENCY_COUNT`].
/// - Unknown run IDs are validated externally by the daemon handler against
///   persisted runs; the pure core function receives a pre-validated
///   `known_run_ids` set for that check.
///
/// The function does **not** mutate lifecycle status, plan, approvals,
/// archive/snooze/pin/priority/owner/due-date state, or lineage.
pub fn set_dependencies(
    params: &RunSetDependenciesParams,
    state: &mut RunState,
    known_run_ids: &[String],
) -> Result<RunSetDependenciesResult> {
    // Validate each candidate blocker ID.
    let mut normalized: Vec<String> = Vec::with_capacity(params.blocked_by_run_ids.len());
    for id in &params.blocked_by_run_ids {
        let trimmed = id.trim().to_string();
        if trimmed.is_empty() {
            bail!("blocked_by_run_ids contains an empty string");
        }
        if trimmed == params.run_id {
            bail!(
                "run cannot depend on itself: '{}' appears in blocked_by_run_ids",
                params.run_id
            );
        }
        if !known_run_ids.contains(&trimmed) {
            bail!("unknown run ID in blocked_by_run_ids: '{trimmed}'");
        }
        normalized.push(trimmed);
    }

    // Deduplicate — keep the first occurrence order then sort for determinism.
    normalized.sort();
    normalized.dedup();

    if normalized.len() > MAX_DEPENDENCY_COUNT {
        bail!(
            "blocked_by_run_ids exceeds maximum of {MAX_DEPENDENCY_COUNT} entries (got {})",
            normalized.len()
        );
    }

    let previous_blocked_by_run_ids = state.blocked_by_run_ids.clone();
    let now = chrono::Utc::now().to_rfc3339();
    state.blocked_by_run_ids = normalized.clone();

    let message = match (previous_blocked_by_run_ids.is_empty(), normalized.is_empty()) {
        (true, true) => "dependencies unchanged (no dependencies set)".to_string(),
        (true, false) => format!(
            "dependencies set: blocked by {}",
            normalized.join(", ")
        ),
        (false, true) => "all dependencies cleared".to_string(),
        (false, false) if previous_blocked_by_run_ids == normalized => {
            format!("dependencies unchanged: blocked by {}", normalized.join(", "))
        }
        (false, false) => format!(
            "dependencies updated: now blocked by {}",
            normalized.join(", ")
        ),
    };

    Ok(RunSetDependenciesResult {
        run_id: params.run_id.clone(),
        status: state.status.clone(),
        blocked_by_run_ids: normalized,
        previous_blocked_by_run_ids,
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

    fn known(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    // ---- happy path ----

    #[test]
    fn set_single_dependency() {
        let mut state = make_state("run-a", "active");
        let params = RunSetDependenciesParams {
            run_id: "run-a".to_string(),
            blocked_by_run_ids: vec!["run-b".to_string()],
        };
        let result = set_dependencies(&params, &mut state, &known(&["run-b"])).unwrap();
        assert_eq!(result.blocked_by_run_ids, vec!["run-b"]);
        assert!(result.previous_blocked_by_run_ids.is_empty());
        assert_eq!(state.blocked_by_run_ids, vec!["run-b"]);
    }

    #[test]
    fn set_multiple_dependencies_sorted() {
        let mut state = make_state("run-a", "active");
        let params = RunSetDependenciesParams {
            run_id: "run-a".to_string(),
            blocked_by_run_ids: vec!["run-z".to_string(), "run-b".to_string(), "run-m".to_string()],
        };
        let result = set_dependencies(&params, &mut state, &known(&["run-b", "run-m", "run-z"])).unwrap();
        assert_eq!(result.blocked_by_run_ids, vec!["run-b", "run-m", "run-z"]);
        assert_eq!(state.blocked_by_run_ids, vec!["run-b", "run-m", "run-z"]);
    }

    #[test]
    fn clear_dependencies() {
        let mut state = make_state("run-a", "active");
        state.blocked_by_run_ids = vec!["run-b".to_string()];
        let params = RunSetDependenciesParams {
            run_id: "run-a".to_string(),
            blocked_by_run_ids: vec![],
        };
        let result = set_dependencies(&params, &mut state, &[]).unwrap();
        assert!(result.blocked_by_run_ids.is_empty());
        assert_eq!(result.previous_blocked_by_run_ids, vec!["run-b"]);
        assert!(state.blocked_by_run_ids.is_empty());
        assert!(result.message.contains("cleared"));
    }

    #[test]
    fn replace_dependencies() {
        let mut state = make_state("run-a", "active");
        state.blocked_by_run_ids = vec!["run-b".to_string()];
        let params = RunSetDependenciesParams {
            run_id: "run-a".to_string(),
            blocked_by_run_ids: vec!["run-c".to_string()],
        };
        let result = set_dependencies(&params, &mut state, &known(&["run-c"])).unwrap();
        assert_eq!(result.blocked_by_run_ids, vec!["run-c"]);
        assert_eq!(result.previous_blocked_by_run_ids, vec!["run-b"]);
    }

    #[test]
    fn does_not_mutate_status() {
        let mut state = make_state("run-a", "finalized:completed");
        let params = RunSetDependenciesParams {
            run_id: "run-a".to_string(),
            blocked_by_run_ids: vec!["run-b".to_string()],
        };
        let result = set_dependencies(&params, &mut state, &known(&["run-b"])).unwrap();
        assert_eq!(result.status, "finalized:completed");
        assert_eq!(state.status, "finalized:completed");
    }

    #[test]
    fn idempotent_same_list() {
        let mut state = make_state("run-a", "active");
        state.blocked_by_run_ids = vec!["run-b".to_string()];
        let params = RunSetDependenciesParams {
            run_id: "run-a".to_string(),
            blocked_by_run_ids: vec!["run-b".to_string()],
        };
        let result = set_dependencies(&params, &mut state, &known(&["run-b"])).unwrap();
        assert_eq!(result.blocked_by_run_ids, vec!["run-b"]);
        assert!(result.message.contains("unchanged"));
    }

    // ---- validation ----

    #[test]
    fn reject_self_dependency() {
        let mut state = make_state("run-a", "active");
        let params = RunSetDependenciesParams {
            run_id: "run-a".to_string(),
            blocked_by_run_ids: vec!["run-a".to_string()],
        };
        let err = set_dependencies(&params, &mut state, &known(&["run-a"])).unwrap_err();
        assert!(
            err.to_string().contains("cannot depend on itself"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn reject_unknown_run_id() {
        let mut state = make_state("run-a", "active");
        let params = RunSetDependenciesParams {
            run_id: "run-a".to_string(),
            blocked_by_run_ids: vec!["ghost-run".to_string()],
        };
        let err = set_dependencies(&params, &mut state, &known(&[])).unwrap_err();
        assert!(
            err.to_string().contains("unknown run ID"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn reject_empty_id_string() {
        let mut state = make_state("run-a", "active");
        let params = RunSetDependenciesParams {
            run_id: "run-a".to_string(),
            blocked_by_run_ids: vec!["  ".to_string()],
        };
        let err = set_dependencies(&params, &mut state, &known(&[])).unwrap_err();
        assert!(
            err.to_string().contains("empty string"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn reject_too_many_dependencies() {
        let mut state = make_state("run-a", "active");
        let ids: Vec<String> = (0..=MAX_DEPENDENCY_COUNT)
            .map(|i| format!("run-{i:03}"))
            .collect();
        let known_ids: Vec<String> = ids.clone();
        let params = RunSetDependenciesParams {
            run_id: "run-a".to_string(),
            blocked_by_run_ids: ids,
        };
        let err = set_dependencies(&params, &mut state, &known_ids).unwrap_err();
        assert!(
            err.to_string().contains("maximum"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn deduplicates_and_sorts() {
        let mut state = make_state("run-a", "active");
        let params = RunSetDependenciesParams {
            run_id: "run-a".to_string(),
            blocked_by_run_ids: vec![
                "run-z".to_string(),
                "run-b".to_string(),
                "run-z".to_string(), // duplicate
                "run-b".to_string(), // duplicate
            ],
        };
        let result = set_dependencies(&params, &mut state, &known(&["run-b", "run-z"])).unwrap();
        assert_eq!(result.blocked_by_run_ids, vec!["run-b", "run-z"]);
    }

    #[test]
    fn allows_exactly_max_dependencies() {
        let mut state = make_state("run-a", "active");
        let ids: Vec<String> = (0..MAX_DEPENDENCY_COUNT)
            .map(|i| format!("run-{i:03}"))
            .collect();
        let known_ids = ids.clone();
        let params = RunSetDependenciesParams {
            run_id: "run-a".to_string(),
            blocked_by_run_ids: ids,
        };
        let result = set_dependencies(&params, &mut state, &known_ids).unwrap();
        assert_eq!(result.blocked_by_run_ids.len(), MAX_DEPENDENCY_COUNT);
    }

    #[test]
    fn message_set_from_empty() {
        let mut state = make_state("run-a", "active");
        let params = RunSetDependenciesParams {
            run_id: "run-a".to_string(),
            blocked_by_run_ids: vec!["run-b".to_string()],
        };
        let result = set_dependencies(&params, &mut state, &known(&["run-b"])).unwrap();
        assert!(result.message.contains("blocked by"), "{}", result.message);
    }

    #[test]
    fn message_cleared() {
        let mut state = make_state("run-a", "active");
        state.blocked_by_run_ids = vec!["run-b".to_string()];
        let params = RunSetDependenciesParams {
            run_id: "run-a".to_string(),
            blocked_by_run_ids: vec![],
        };
        let result = set_dependencies(&params, &mut state, &[]).unwrap();
        assert!(result.message.contains("cleared"), "{}", result.message);
    }

    #[test]
    fn message_unchanged_empty_to_empty() {
        let mut state = make_state("run-a", "active");
        let params = RunSetDependenciesParams {
            run_id: "run-a".to_string(),
            blocked_by_run_ids: vec![],
        };
        let result = set_dependencies(&params, &mut state, &[]).unwrap();
        assert!(result.message.contains("unchanged"), "{}", result.message);
    }
}
