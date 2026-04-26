//! Deterministic approval policy layer.
//!
//! Classifies operations as safe, requiring approval, or rejected.
//! All rules are explicit and deterministic — no LLM reasoning.
//!
//! Milestone 8: policy knobs are taken from the per-run `RunPolicy` profile
//! instead of being hardcoded constants.  Callers pass the effective policy
//! for the current run so that custom thresholds and target lists apply.

use deterministic_protocol::{PatchApplyParams, RunPolicy, TestsRunParams};

/// Outcome of a policy evaluation.
#[derive(Debug, Clone, PartialEq)]
pub enum PolicyDecision {
    /// The operation may proceed immediately.
    Proceed,
    /// The operation requires explicit approval before execution.
    RequiresApproval {
        /// Human-readable summary of the action being gated.
        action_summary: String,
        /// Why this is considered risky.
        risk_reason: String,
        /// Which policy rule triggered the gate.
        policy_rationale: String,
    },
}

// ---------------------------------------------------------------------------
// Patch policy
// ---------------------------------------------------------------------------

/// Sensitive file name patterns that require approval before modification.
const SENSITIVE_PATTERNS: &[&str] = &[
    ".env",
    ".ssh",
    ".git/",
    "id_rsa",
    "id_ed25519",
    "secrets",
    ".secret",
    ".credentials",
    ".key",
    ".pem",
];

/// Evaluate whether a patch request requires approval using the per-run policy.
///
/// Rules (evaluated in order — first match wins):
/// 1. Any delete operation AND `policy.deleteRequiresApproval` is true → requires approval
/// 2. More than `policy.patchEditThreshold` edits → requires approval
/// 3. Any path matching a sensitive pattern AND `policy.sensitivePathRequiresApproval` is true → requires approval
/// 4. Any path outside declared focus paths (when non-empty) AND `policy.outsideFocusRequiresApproval` is true → requires approval
/// 5. Otherwise → proceed
pub fn evaluate_patch(params: &PatchApplyParams, policy: &RunPolicy) -> PolicyDecision {
    let focus_paths = &policy.focus_paths;

    // Rule 1: delete operations
    if policy.delete_requires_approval {
        for edit in &params.edits {
            if edit.operation == "delete" {
                return PolicyDecision::RequiresApproval {
                    action_summary: format!("Delete file: {}", edit.path),
                    risk_reason: "File deletion is destructive and irreversible".into(),
                    policy_rationale: "Policy: file deletion requires approval".into(),
                };
            }
        }
    }

    // Rule 2: large patch (too many edits)
    if params.edits.len() > policy.patch_edit_threshold {
        return PolicyDecision::RequiresApproval {
            action_summary: format!(
                "Patch with {} edits across {} file(s)",
                params.edits.len(),
                unique_paths(&params.edits),
            ),
            risk_reason: format!(
                "Patch touches {} edits (threshold: {})",
                params.edits.len(),
                policy.patch_edit_threshold,
            ),
            policy_rationale: format!(
                "Policy: large patch (>{} edits) requires approval",
                policy.patch_edit_threshold
            ),
        };
    }

    // Rule 3: sensitive file paths
    if policy.sensitive_path_requires_approval {
        for edit in &params.edits {
            if let Some(pattern) = matches_sensitive_pattern(&edit.path) {
                return PolicyDecision::RequiresApproval {
                    action_summary: format!("Edit sensitive file: {}", edit.path),
                    risk_reason: format!(
                        "Path '{}' matches sensitive pattern '{}'",
                        edit.path, pattern
                    ),
                    policy_rationale: "Policy: sensitive file path requires approval".into(),
                };
            }
        }
    }

    // Rule 4: outside focus paths
    if policy.outside_focus_requires_approval && !focus_paths.is_empty() {
        for edit in &params.edits {
            if !is_within_focus_paths(&edit.path, focus_paths) {
                return PolicyDecision::RequiresApproval {
                    action_summary: format!("Edit outside focus: {}", edit.path),
                    risk_reason: format!(
                        "Path '{}' is outside declared focus paths: {:?}",
                        edit.path, focus_paths
                    ),
                    policy_rationale: "Policy: edit outside declared focus paths requires approval"
                        .into(),
                };
            }
        }
    }

    PolicyDecision::Proceed
}

/// Check if a path matches any sensitive pattern.
fn matches_sensitive_pattern(path: &str) -> Option<&'static str> {
    let lower = path.to_lowercase();
    SENSITIVE_PATTERNS
        .iter()
        .find(|&&pattern| lower.contains(pattern))
        .copied()
}

/// Check if a path is within any of the declared focus paths.
fn is_within_focus_paths(path: &str, focus_paths: &[String]) -> bool {
    focus_paths.iter().any(|fp| path.starts_with(fp.as_str()))
}

/// Count unique paths in edits.
fn unique_paths(edits: &[deterministic_protocol::PatchEdit]) -> usize {
    let mut seen = std::collections::HashSet::new();
    for edit in edits {
        seen.insert(edit.path.as_str());
    }
    seen.len()
}

// ---------------------------------------------------------------------------
// Test-run policy
// ---------------------------------------------------------------------------

/// Built-in safe make targets that never require approval.
const SAFE_MAKE_TARGETS: &[&str] = &[
    "test", "check", "lint", "build", "clean", "all", "verify", "fmt", "format",
];

/// Evaluate whether a test-run request requires approval using the per-run policy.
///
/// Rules:
/// 1. `make` scope with a target not in the safe-target list AND not in
///    `policy.extraSafeMakeTargets` → requires approval
/// 2. Otherwise → proceed
pub fn evaluate_test_run(params: &TestsRunParams, policy: &RunPolicy) -> PolicyDecision {
    let scope_lower = params.scope.to_lowercase();

    // Rule 1: make with non-standard target
    if scope_lower == "make"
        && let Some(ref target) = params.target
    {
        let target_lower = target.to_lowercase();
        let is_builtin_safe = SAFE_MAKE_TARGETS.contains(&target_lower.as_str());
        let is_extra_safe = policy
            .extra_safe_make_targets
            .iter()
            .any(|t| t == &target_lower);
        if !is_builtin_safe && !is_extra_safe {
            return PolicyDecision::RequiresApproval {
                action_summary: format!("Run make target: {target}"),
                risk_reason: format!("Make target '{target}' is not in the safe-target list",),
                policy_rationale: "Policy: non-standard make target requires approval".into(),
            };
        }
    }

    PolicyDecision::Proceed
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------


#[cfg(test)]
mod tests {
    use super::*;

    fn default_policy() -> RunPolicy {
        RunPolicy {
            patch_edit_threshold: 5,
            delete_requires_approval: true,
            sensitive_path_requires_approval: true,
            outside_focus_requires_approval: true,
            extra_safe_make_targets: vec![],
            focus_paths: vec![],
        }
    }

    fn policy_with_focus(focus_paths: Vec<String>) -> RunPolicy {
        RunPolicy {
            focus_paths,
            ..default_policy()
        }
    }

    fn policy(extra_safe: Vec<&str>) -> RunPolicy {
        RunPolicy {
            extra_safe_make_targets: extra_safe.iter().map(|s| s.to_string()).collect(),
            ..default_policy()
        }
    }

    fn patch_params(run_id: &str, edits: Vec<PatchEdit>) -> PatchApplyParams {
        PatchApplyParams {
            run_id: run_id.into(),
            edits,
            hybrid_worker_run_id: None,
            skip_policy: false,
        }
    }

    fn test_params(run_id: &str, scope: &str) -> TestsRunParams {
        TestsRunParams {
            run_id: run_id.into(),
            scope: scope.into(),
            target: None,
            reason: "test".into(),
        }
    }

    fn test_params_with_target(run_id: &str, scope: &str, target: &str) -> TestsRunParams {
        TestsRunParams {
            run_id: run_id.into(),
            scope: scope.into(),
            target: Some(target.into()),
            reason: "test".into(),
        }
    }

    #[test]
    fn delete_requires_approval() {
        let params = patch_params("r1", vec![PatchEdit {
            path: "src/lib.rs".into(),
            operation: "delete".into(),
            start_line: None,
            end_line: None,
            old_text: None,
            new_text: String::new(),
            anchor_text: None,
            reason: None,
        }]);
        assert_eq!(
            evaluate_patch(&params, &default_policy()),
            PolicyDecision::RequiresApproval {
                action_summary: "Delete file: src/lib.rs".into(),
                risk_reason: "File deletion is destructive and irreversible".into(),
                policy_rationale: "Policy: file deletion requires approval".into(),
            }
        );
    }

    #[test]
    fn create_is_approved_by_default() {
        let params = patch_params("r1", vec![PatchEdit {
            path: "new.txt".into(),
            operation: "create".into(),
            start_line: None,
            end_line: None,
            old_text: None,
            new_text: "hello\n".into(),
            anchor_text: None,
            reason: None,
        }]);
        assert_eq!(evaluate_patch(&params, &default_policy()), PolicyDecision::Proceed);
    }

    #[test]
    fn small_replace_is_approved_by_default() {
        let params = patch_params("r1", vec![PatchEdit {
            path: "src/lib.rs".into(),
            operation: "replace".into(),
            start_line: None,
            end_line: None,
            old_text: Some("old".into()),
            new_text: "new".into(),
            anchor_text: None,
            reason: None,
        }]);
        assert_eq!(evaluate_patch(&params, &default_policy()), PolicyDecision::Proceed);
    }

    #[test]
    fn large_patch_requires_approval() {
        let edits: Vec<PatchEdit> = (0..10).map(|i| PatchEdit {
            path: format!("file{i}.txt"),
            operation: "create".into(),
            start_line: None,
            end_line: None,
            old_text: None,
            new_text: "content\n".into(),
            anchor_text: None,
            reason: None,
        }).collect();
        let params = patch_params("r1", edits);
        let result = evaluate_patch(&params, &default_policy());
        match result {
            PolicyDecision::RequiresApproval { action_summary, .. } => {
                assert!(action_summary.contains("10 edits"));
            }
            _ => panic!("expected RequiresApproval"),
        }
    }

    #[test]
    fn sensitive_path_requires_approval() {
        let params = patch_params("r1", vec![PatchEdit {
            path: "src/.env".into(),
            operation: "replace".into(),
            start_line: None,
            end_line: None,
            old_text: Some("old".into()),
            new_text: "new".into(),
            anchor_text: None,
            reason: None,
        }]);
        let result = evaluate_patch(&params, &default_policy());
        match result {
            PolicyDecision::RequiresApproval { action_summary, .. } => {
                assert!(action_summary.contains(".env"));
            }
            _ => panic!("expected RequiresApproval"),
        }
    }

    #[test]
    fn safe_make_target_no_approval() {
        let params = test_params("r1", "cargo");
        assert_eq!(evaluate_test_run(&params, &default_policy()), PolicyDecision::Proceed);
    }

    #[test]
    fn unsafe_make_target_requires_approval() {
        let params = test_params_with_target("r1", "make", "deploy");
        let result = evaluate_test_run(&params, &default_policy());
        match result {
            PolicyDecision::RequiresApproval { .. } => {}
            PolicyDecision::Proceed => panic!("expected RequiresApproval for 'make deploy'"),
        }
    }

    #[test]
    fn extra_safe_target_no_approval() {
        let params = test_params_with_target("r1", "make", "deploy");
        let custom_policy = policy(vec!["deploy"]);
        assert_eq!(evaluate_test_run(&params, &custom_policy), PolicyDecision::Proceed);
    }

    #[test]
    #[test]
    fn pytest_proceeds_by_default() {
        // pytest does not require approval by default (only make with non-safe targets does)
        let params = test_params("r1", "pytest");
        assert_eq!(evaluate_test_run(&params, &default_policy()), PolicyDecision::Proceed);
    }

    #[test]
    fn outside_focus_requires_approval() {
        let params = patch_params("r1", vec![PatchEdit {
            path: "src/other.rs".into(),
            operation: "replace".into(),
            start_line: None,
            end_line: None,
            old_text: Some("old".into()),
            new_text: "new".into(),
            anchor_text: None,
            reason: None,
        }]);
        let focus_policy = policy_with_focus(vec!["src/main.rs".into()]);
        let result = evaluate_patch(&params, &focus_policy);
        match result {
            PolicyDecision::RequiresApproval { action_summary, .. } => {
                assert!(action_summary.contains("outside focus"));
            }
            PolicyDecision::Proceed => panic!("expected RequiresApproval for path outside focus"),
        }
    }

    #[test]
    fn within_focus_no_approval() {
        let params = patch_params("r1", vec![PatchEdit {
            path: "src/main.rs".into(),
            operation: "replace".into(),
            start_line: None,
            end_line: None,
            old_text: Some("old".into()),
            new_text: "new".into(),
            anchor_text: None,
            reason: None,
        }]);
        let focus_policy = policy_with_focus(vec!["src/main.rs".into()]);
        assert_eq!(evaluate_patch(&params, &focus_policy), PolicyDecision::Proceed);
    }

    #[test]
    fn multiple_edits_in_focus_all_allowed() {
        let params = patch_params("r1", vec![
            PatchEdit {
                path: "src/a.rs".into(),
                operation: "replace".into(),
                start_line: None,
                end_line: None,
                old_text: Some("a".into()),
                new_text: "A".into(),
                anchor_text: None,
                reason: None,
            },
            PatchEdit {
                path: "src/b.rs".into(),
                operation: "replace".into(),
                start_line: None,
                end_line: None,
                old_text: Some("b".into()),
                new_text: "B".into(),
                anchor_text: None,
                reason: None,
            },
        ]);
        let focus_policy = policy_with_focus(vec!["src/a.rs".into(), "src/b.rs".into()]);
        assert_eq!(evaluate_patch(&params, &focus_policy), PolicyDecision::Proceed);
    }
}
