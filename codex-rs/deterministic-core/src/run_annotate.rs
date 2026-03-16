//! Handler logic for `run.annotate`.
//!
//! Provides a deterministic, explicit annotation surface for ChatGPT to attach
//! compact organization metadata (labels and an operator note) to a run.
//!
//! Normalization rules:
//! - Labels are trimmed and converted to lowercase.
//! - Labels are deduplicated (first occurrence wins).
//! - Labels are sorted deterministically.
//! - Each label must be non-empty after trimming and contain only
//!   lowercase ASCII letters, digits, hyphens, or underscores.
//! - Each label is bounded to `LABEL_MAX_LEN` characters.
//! - The total label count is bounded to `LABEL_MAX_COUNT`.
//! - The operator note is bounded to `OPERATOR_NOTE_MAX_LEN` characters.
//!
//! This operation:
//! - does not execute work
//! - does not refresh, replan, reopen, finalize, archive, unarchive, or
//!   supersede the run
//! - appends a deterministic audit entry

use anyhow::{bail, Result};
use deterministic_protocol::{
    RunAnnotateParams, RunAnnotateResult, RunState, LABEL_MAX_COUNT, LABEL_MAX_LEN,
    OPERATOR_NOTE_MAX_LEN,
};

/// Normalize and validate a single label.
///
/// Accepts lowercase ASCII letters, digits, hyphens, and underscores.
/// Leading/trailing whitespace is stripped.  The result must be non-empty
/// and at most `LABEL_MAX_LEN` characters.
fn normalize_label(raw: &str) -> Result<String> {
    let trimmed = raw.trim().to_lowercase();
    if trimmed.is_empty() {
        bail!("label must not be empty after trimming");
    }
    if trimmed.len() > LABEL_MAX_LEN {
        bail!(
            "label '{trimmed}' exceeds maximum length of {LABEL_MAX_LEN} characters"
        );
    }
    for ch in trimmed.chars() {
        if !matches!(ch, 'a'..='z' | '0'..='9' | '-' | '_') {
            bail!(
                "label '{trimmed}' contains invalid character '{ch}'; only lowercase ASCII letters, digits, hyphens, and underscores are allowed"
            );
        }
    }
    Ok(trimmed)
}

/// Normalize a full label set: trim, lowercase, validate, deduplicate, sort.
pub fn normalize_labels(raw_labels: Vec<String>) -> Result<Vec<String>> {
    if raw_labels.len() > LABEL_MAX_COUNT {
        bail!(
            "too many labels: {} provided, maximum is {}",
            raw_labels.len(),
            LABEL_MAX_COUNT
        );
    }
    let mut normalized: Vec<String> = Vec::with_capacity(raw_labels.len());
    for raw in raw_labels {
        let label = normalize_label(&raw)?;
        if !normalized.contains(&label) {
            normalized.push(label);
        }
    }
    normalized.sort();
    Ok(normalized)
}

/// Annotate a run with labels and/or an operator note.
///
/// Deterministic rules:
/// - `params.labels`, if provided, replaces the existing label set entirely.
/// - `params.operator_note`, if provided, replaces the existing note.
///   An empty string clears the note.
/// - At least one of `labels` or `operator_note` must be provided.
/// - The annotation update does not change run status, plan, or any other
///   lifecycle field.
///
/// Returns the updated run state (via mutation) and a result DTO.
pub fn annotate(params: &RunAnnotateParams, state: &mut RunState) -> Result<RunAnnotateResult> {
    // At least one field must be provided.
    if params.labels.is_none() && params.operator_note.is_none() {
        bail!(
            "run.annotate requires at least one of 'labels' or 'operatorNote'"
        );
    }

    let mut annotation = state.annotation.clone().unwrap_or_default();

    // Apply label update if provided.
    if let Some(raw_labels) = &params.labels {
        annotation.labels = normalize_labels(raw_labels.clone())?;
    }

    // Apply operator note update if provided.
    if let Some(note) = &params.operator_note {
        if note.len() > OPERATOR_NOTE_MAX_LEN {
            bail!(
                "operator note exceeds maximum length of {OPERATOR_NOTE_MAX_LEN} characters"
            );
        }
        annotation.operator_note = if note.is_empty() {
            None
        } else {
            Some(note.clone())
        };
    }

    state.annotation = Some(annotation.clone());
    state.updated_at = chrono::Utc::now().to_rfc3339();

    Ok(RunAnnotateResult {
        run_id: params.run_id.clone(),
        annotation,
        message: format!("Run '{}' annotation updated.", params.run_id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use deterministic_protocol::{RunAnnotation, RunOutcome, RunPolicy};

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
            snooze_metadata: None,
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        }
    }

    // -------------------------------------------------------------------------
    // label normalization tests
    // -------------------------------------------------------------------------

    #[test]
    fn normalize_label_lowercase() {
        let labels = normalize_labels(vec!["Auth".into(), "INFRA".into()]).unwrap();
        assert_eq!(labels, vec!["auth", "infra"]);
    }

    #[test]
    fn normalize_label_trimmed() {
        let labels = normalize_labels(vec!["  auth  ".into()]).unwrap();
        assert_eq!(labels, vec!["auth"]);
    }

    #[test]
    fn normalize_label_deduplication() {
        let labels = normalize_labels(vec!["auth".into(), "auth".into(), "infra".into()]).unwrap();
        assert_eq!(labels, vec!["auth", "infra"]);
    }

    #[test]
    fn normalize_labels_sorted() {
        let labels = normalize_labels(vec!["zeta".into(), "alpha".into(), "beta".into()]).unwrap();
        assert_eq!(labels, vec!["alpha", "beta", "zeta"]);
    }

    #[test]
    fn normalize_label_valid_chars() {
        let labels =
            normalize_labels(vec!["my-label".into(), "my_label".into(), "label123".into()])
                .unwrap();
        assert_eq!(labels, vec!["label123", "my-label", "my_label"]);
    }

    #[test]
    fn normalize_label_empty_rejected() {
        let err = normalize_labels(vec!["".into()]).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn normalize_label_whitespace_only_rejected() {
        let err = normalize_labels(vec!["   ".into()]).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn normalize_label_invalid_char_rejected() {
        let err = normalize_labels(vec!["bad label".into()]).unwrap_err();
        assert!(err.to_string().contains("invalid character"));
    }

    #[test]
    fn normalize_label_special_chars_rejected() {
        for bad in &["lab@el", "lab!el", "lab/el", "lab.el"] {
            let err = normalize_labels(vec![bad.to_string()]).unwrap_err();
            assert!(
                err.to_string().contains("invalid character"),
                "expected invalid character error for '{bad}'"
            );
        }
    }

    #[test]
    fn normalize_label_too_long_rejected() {
        let long = "a".repeat(LABEL_MAX_LEN + 1);
        let err = normalize_labels(vec![long]).unwrap_err();
        assert!(err.to_string().contains("exceeds maximum length"));
    }

    #[test]
    fn normalize_labels_too_many_rejected() {
        let labels: Vec<String> = (0..=LABEL_MAX_COUNT).map(|i| format!("label{i}")).collect();
        let err = normalize_labels(labels).unwrap_err();
        assert!(err.to_string().contains("too many labels"));
    }

    // -------------------------------------------------------------------------
    // annotate happy path
    // -------------------------------------------------------------------------

    #[test]
    fn annotate_sets_labels() {
        let mut state = make_state("run-1", "active");
        let params = RunAnnotateParams {
            run_id: "run-1".into(),
            labels: Some(vec!["auth".into(), "infra".into()]),
            operator_note: None,
        };
        let result = annotate(&params, &mut state).unwrap();
        assert_eq!(result.run_id, "run-1");
        assert_eq!(result.annotation.labels, vec!["auth", "infra"]);
        assert!(result.annotation.operator_note.is_none());
        assert_eq!(state.annotation.as_ref().unwrap().labels, vec!["auth", "infra"]);
    }

    #[test]
    fn annotate_sets_operator_note() {
        let mut state = make_state("run-2", "active");
        let params = RunAnnotateParams {
            run_id: "run-2".into(),
            labels: None,
            operator_note: Some("This run addresses the auth regression.".into()),
        };
        let result = annotate(&params, &mut state).unwrap();
        assert_eq!(
            result.annotation.operator_note.as_deref(),
            Some("This run addresses the auth regression.")
        );
        assert!(result.annotation.labels.is_empty());
    }

    #[test]
    fn annotate_sets_both() {
        let mut state = make_state("run-3", "active");
        let params = RunAnnotateParams {
            run_id: "run-3".into(),
            labels: Some(vec!["blocked".into()]),
            operator_note: Some("Waiting for CI approval.".into()),
        };
        let result = annotate(&params, &mut state).unwrap();
        assert_eq!(result.annotation.labels, vec!["blocked"]);
        assert_eq!(
            result.annotation.operator_note.as_deref(),
            Some("Waiting for CI approval.")
        );
    }

    #[test]
    fn annotate_clears_note_with_empty_string() {
        let mut state = make_state("run-4", "active");
        state.annotation = Some(RunAnnotation {
            labels: vec!["auth".into()],
            operator_note: Some("old note".into()),
        });
        let params = RunAnnotateParams {
            run_id: "run-4".into(),
            labels: None,
            operator_note: Some("".into()),
        };
        let result = annotate(&params, &mut state).unwrap();
        assert!(result.annotation.operator_note.is_none());
        // Labels unchanged
        assert_eq!(result.annotation.labels, vec!["auth"]);
    }

    #[test]
    fn annotate_replaces_existing_labels() {
        let mut state = make_state("run-5", "active");
        state.annotation = Some(RunAnnotation {
            labels: vec!["old-label".into()],
            operator_note: Some("existing note".into()),
        });
        let params = RunAnnotateParams {
            run_id: "run-5".into(),
            labels: Some(vec!["new-label".into()]),
            operator_note: None,
        };
        let result = annotate(&params, &mut state).unwrap();
        assert_eq!(result.annotation.labels, vec!["new-label"]);
        // Note unchanged
        assert_eq!(
            result.annotation.operator_note.as_deref(),
            Some("existing note")
        );
    }

    #[test]
    fn annotate_requires_at_least_one_field() {
        let mut state = make_state("run-6", "active");
        let params = RunAnnotateParams {
            run_id: "run-6".into(),
            labels: None,
            operator_note: None,
        };
        let err = annotate(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("at least one"));
    }

    #[test]
    fn annotate_note_too_long_rejected() {
        let mut state = make_state("run-7", "active");
        let long_note = "x".repeat(OPERATOR_NOTE_MAX_LEN + 1);
        let params = RunAnnotateParams {
            run_id: "run-7".into(),
            labels: None,
            operator_note: Some(long_note),
        };
        let err = annotate(&params, &mut state).unwrap_err();
        assert!(err.to_string().contains("maximum length"));
    }

    #[test]
    fn annotate_normalizes_label_case() {
        let mut state = make_state("run-8", "active");
        let params = RunAnnotateParams {
            run_id: "run-8".into(),
            labels: Some(vec!["AUTH".into(), "Infra".into()]),
            operator_note: None,
        };
        let result = annotate(&params, &mut state).unwrap();
        assert_eq!(result.annotation.labels, vec!["auth", "infra"]);
    }

    #[test]
    fn annotate_deduplicates_labels() {
        let mut state = make_state("run-9", "active");
        let params = RunAnnotateParams {
            run_id: "run-9".into(),
            labels: Some(vec!["auth".into(), "auth".into(), "infra".into()]),
            operator_note: None,
        };
        let result = annotate(&params, &mut state).unwrap();
        assert_eq!(result.annotation.labels, vec!["auth", "infra"]);
    }

    #[test]
    fn annotate_updates_updated_at() {
        let mut state = make_state("run-10", "active");
        let params = RunAnnotateParams {
            run_id: "run-10".into(),
            labels: Some(vec!["ci".into()]),
            operator_note: None,
        };
        annotate(&params, &mut state).unwrap();
        assert_ne!(state.updated_at, "2024-01-01T00:00:00Z");
    }

    #[test]
    fn annotate_does_not_change_status() {
        let mut state = make_state("run-11", "finalized:completed");
        state.finalized_outcome = Some(RunOutcome {
            outcome_kind: "completed".into(),
            summary: "Done".into(),
            reason: None,
            finalized_at: "2024-01-01T01:00:00Z".into(),
        });
        let params = RunAnnotateParams {
            run_id: "run-11".into(),
            labels: Some(vec!["completed".into()]),
            operator_note: None,
        };
        annotate(&params, &mut state).unwrap();
        // Status must not be changed.
        assert_eq!(state.status, "finalized:completed");
        // Finalized outcome must be preserved.
        assert!(state.finalized_outcome.is_some());
    }

    #[test]
    fn annotate_empty_label_set_clears_labels() {
        let mut state = make_state("run-12", "active");
        state.annotation = Some(RunAnnotation {
            labels: vec!["auth".into()],
            operator_note: None,
        });
        let params = RunAnnotateParams {
            run_id: "run-12".into(),
            labels: Some(vec![]),
            operator_note: None,
        };
        let result = annotate(&params, &mut state).unwrap();
        assert!(result.annotation.labels.is_empty());
    }
}
