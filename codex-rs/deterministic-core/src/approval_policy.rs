//! Deterministic approval policy layer.
//!
//! Classifies operations as safe, requiring approval, or rejected.
//! All rules are explicit and deterministic — no LLM reasoning.
//!
//! In Milestone 8, policy rules use the per-run `RunPolicy` profile
//! instead of hardcoded global constants.

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
/// These cannot be overridden by the per-run policy — they are always checked
/// when `sensitive_path_requires_approval` is true.
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

/// Evaluate whether a patch request requires approval.
///
/// Rules (evaluated in order — first match wins):
/// 1. Any delete operation (when `policy.delete_requires_approval` is true) → requires approval
/// 2. More than `policy.patch_edit_threshold` edits → requires approval
/// 3. Any path matching a sensitive pattern (when `policy.sensitive_path_requires_approval`) → requires approval
/// 4. Any path outside declared focus paths (when `policy.outside_focus_requires_approval`
///    and focus paths are non-empty) → requires approval
/// 5. Otherwise → proceed
pub fn evaluate_patch(params: &PatchApplyParams, policy: &RunPolicy) -> PolicyDecision {
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
                policy.patch_edit_threshold,
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
    if policy.outside_focus_requires_approval && !policy.focus_paths.is_empty() {
        for edit in &params.edits {
            if !is_within_focus_paths(&edit.path, &policy.focus_paths) {
                return PolicyDecision::RequiresApproval {
                    action_summary: format!("Edit outside focus: {}", edit.path),
                    risk_reason: format!(
                        "Path '{}' is outside declared focus paths: {:?}",
                        edit.path, policy.focus_paths
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
    focus_paths
        .iter()
        .any(|fp| path.starts_with(fp.as_str()))
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

/// Safe make targets that don't require approval.
const SAFE_MAKE_TARGETS: &[&str] = &[
    "test", "check", "lint", "build", "clean", "all", "verify", "fmt", "format",
];

/// Evaluate whether a test-run request requires approval.
///
/// Rules:
/// 1. `make` scope with a target not in the safe-target list (built-in + policy extras) → requires approval
/// 2. Otherwise → proceed
pub fn evaluate_test_run(params: &TestsRunParams, policy: &RunPolicy) -> PolicyDecision {
    let scope_lower = params.scope.to_lowercase();

    // Rule 1: make with non-standard target
    if scope_lower == "make"
        && let Some(ref target) = params.target
    {
        let target_lower = target.to_lowercase();
        // Combine built-in safe targets with any extra targets from policy.
        // `extra_safe_make_targets` are normalised to lowercase at prepare
        // time, so a direct equality check is sufficient here.
        let is_safe = SAFE_MAKE_TARGETS.contains(&target_lower.as_str())
            || policy
                .extra_safe_make_targets
                .iter()
                .any(|t| t.as_str() == target_lower);
        if !is_safe {
            return PolicyDecision::RequiresApproval {
                action_summary: format!("Run make target: {target}"),
                risk_reason: format!(
                    "Make target '{target}' is not in the safe-target list",
                ),
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
    use deterministic_protocol::{PatchApplyParams, PatchEdit, RunPolicy, TestsRunParams};

    fn make_edit(path: &str, operation: &str) -> PatchEdit {
        PatchEdit {
            path: path.into(),
            operation: operation.into(),
            start_line: None,
            end_line: None,
            old_text: None,
            new_text: "content".into(),
            anchor_text: None,
            reason: Some("test".into()),
        }
    }

    fn default_policy() -> RunPolicy {
        RunPolicy::default()
    }

    fn policy_with_focus(paths: Vec<String>) -> RunPolicy {
        RunPolicy {
            focus_paths: paths,
            ..RunPolicy::default()
        }
    }

    // ---- Patch policy tests ----

    #[test]
    fn patch_proceed_for_normal_edit() {
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![make_edit("src/main.rs", "replace")],
        };
        assert_eq!(evaluate_patch(&params, &default_policy()), PolicyDecision::Proceed);
    }

    #[test]
    fn patch_requires_approval_for_delete() {
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![make_edit("src/old.rs", "delete")],
        };
        match evaluate_patch(&params, &default_policy()) {
            PolicyDecision::RequiresApproval {
                policy_rationale, ..
            } => {
                assert!(policy_rationale.contains("file deletion"));
            }
            PolicyDecision::Proceed => panic!("expected RequiresApproval"),
        }
    }

    #[test]
    fn patch_requires_approval_for_too_many_edits() {
        let edits: Vec<PatchEdit> = (0..6)
            .map(|i| make_edit(&format!("src/file{i}.rs"), "create"))
            .collect();
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits,
        };
        match evaluate_patch(&params, &default_policy()) {
            PolicyDecision::RequiresApproval {
                policy_rationale, ..
            } => {
                assert!(policy_rationale.contains("large patch"));
            }
            PolicyDecision::Proceed => panic!("expected RequiresApproval"),
        }
    }

    #[test]
    fn patch_allows_exactly_threshold_edits() {
        let edits: Vec<PatchEdit> = (0..5)
            .map(|i| make_edit(&format!("src/file{i}.rs"), "create"))
            .collect();
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits,
        };
        assert_eq!(evaluate_patch(&params, &default_policy()), PolicyDecision::Proceed);
    }

    #[test]
    fn patch_requires_approval_for_sensitive_path() {
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![make_edit(".env.production", "create")],
        };
        match evaluate_patch(&params, &default_policy()) {
            PolicyDecision::RequiresApproval {
                policy_rationale, ..
            } => {
                assert!(policy_rationale.contains("sensitive file"));
            }
            PolicyDecision::Proceed => panic!("expected RequiresApproval"),
        }
    }

    #[test]
    fn patch_requires_approval_for_ssh_key_path() {
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![make_edit("config/.ssh/authorized_keys", "replace")],
        };
        match evaluate_patch(&params, &default_policy()) {
            PolicyDecision::RequiresApproval {
                policy_rationale, ..
            } => {
                assert!(policy_rationale.contains("sensitive file"));
            }
            PolicyDecision::Proceed => panic!("expected RequiresApproval"),
        }
    }

    #[test]
    fn patch_requires_approval_for_git_internal_path() {
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![make_edit(".git/config", "replace")],
        };
        match evaluate_patch(&params, &default_policy()) {
            PolicyDecision::RequiresApproval {
                policy_rationale, ..
            } => {
                assert!(policy_rationale.contains("sensitive file"));
            }
            PolicyDecision::Proceed => panic!("expected RequiresApproval"),
        }
    }

    #[test]
    fn patch_requires_approval_outside_focus_paths() {
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![make_edit("other/module.rs", "create")],
        };
        let policy = policy_with_focus(vec!["src/".to_string()]);
        match evaluate_patch(&params, &policy) {
            PolicyDecision::RequiresApproval {
                policy_rationale, ..
            } => {
                assert!(policy_rationale.contains("outside declared focus"));
            }
            PolicyDecision::Proceed => panic!("expected RequiresApproval"),
        }
    }

    #[test]
    fn patch_proceeds_inside_focus_paths() {
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![make_edit("src/lib.rs", "replace")],
        };
        let policy = policy_with_focus(vec!["src/".to_string()]);
        assert_eq!(evaluate_patch(&params, &policy), PolicyDecision::Proceed);
    }

    #[test]
    fn patch_proceeds_when_no_focus_paths() {
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![make_edit("anywhere/file.rs", "create")],
        };
        assert_eq!(evaluate_patch(&params, &default_policy()), PolicyDecision::Proceed);
    }

    #[test]
    fn patch_delete_takes_priority_over_count() {
        // Delete should trigger before the >5 rule
        let mut edits: Vec<PatchEdit> = (0..6)
            .map(|i| make_edit(&format!("src/file{i}.rs"), "create"))
            .collect();
        edits[0] = make_edit("src/file0.rs", "delete");
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits,
        };
        match evaluate_patch(&params, &default_policy()) {
            PolicyDecision::RequiresApproval {
                policy_rationale, ..
            } => {
                assert!(policy_rationale.contains("file deletion"));
            }
            PolicyDecision::Proceed => panic!("expected RequiresApproval"),
        }
    }

    // ---- Test-run policy tests ----

    #[test]
    fn test_run_proceed_for_cargo() {
        let params = TestsRunParams {
            run_id: "r1".into(),
            scope: "cargo".into(),
            target: Some("my_test".into()),
            reason: "verify fix".into(),
        };
        assert_eq!(evaluate_test_run(&params, &default_policy()), PolicyDecision::Proceed);
    }

    #[test]
    fn test_run_proceed_for_npm() {
        let params = TestsRunParams {
            run_id: "r1".into(),
            scope: "npm".into(),
            target: None,
            reason: "verify fix".into(),
        };
        assert_eq!(evaluate_test_run(&params, &default_policy()), PolicyDecision::Proceed);
    }

    #[test]
    fn test_run_proceed_for_safe_make_target() {
        for target in &["test", "check", "lint", "build", "clean", "all"] {
            let params = TestsRunParams {
                run_id: "r1".into(),
                scope: "make".into(),
                target: Some(target.to_string()),
                reason: "verify fix".into(),
            };
            assert_eq!(
                evaluate_test_run(&params, &default_policy()),
                PolicyDecision::Proceed,
                "make target '{target}' should be safe"
            );
        }
    }

    #[test]
    fn test_run_requires_approval_for_risky_make_target() {
        let params = TestsRunParams {
            run_id: "r1".into(),
            scope: "make".into(),
            target: Some("deploy".into()),
            reason: "deploy".into(),
        };
        match evaluate_test_run(&params, &default_policy()) {
            PolicyDecision::RequiresApproval {
                policy_rationale, ..
            } => {
                assert!(policy_rationale.contains("non-standard make target"));
            }
            PolicyDecision::Proceed => panic!("expected RequiresApproval for 'make deploy'"),
        }
    }

    #[test]
    fn test_run_make_without_target_proceeds() {
        let params = TestsRunParams {
            run_id: "r1".into(),
            scope: "make".into(),
            target: None,
            reason: "test".into(),
        };
        assert_eq!(evaluate_test_run(&params, &default_policy()), PolicyDecision::Proceed);
    }

    #[test]
    fn test_run_proceed_for_semantic_scope() {
        for scope in &["unit", "integration", "all"] {
            let params = TestsRunParams {
                run_id: "r1".into(),
                scope: scope.to_string(),
                target: None,
                reason: "verify".into(),
            };
            assert_eq!(
                evaluate_test_run(&params, &default_policy()),
                PolicyDecision::Proceed,
                "semantic scope '{scope}' should be safe"
            );
        }
    }

    // ---- Milestone 8: per-run policy tests ----

    #[test]
    fn patch_custom_threshold_allows_more_edits() {
        // With a higher threshold, 6 edits should proceed.
        let edits: Vec<PatchEdit> = (0..6)
            .map(|i| make_edit(&format!("src/file{i}.rs"), "create"))
            .collect();
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits,
        };
        let policy = RunPolicy {
            patch_edit_threshold: 10,
            ..RunPolicy::default()
        };
        assert_eq!(evaluate_patch(&params, &policy), PolicyDecision::Proceed);
    }

    #[test]
    fn patch_custom_threshold_of_1_blocks_2_edits() {
        let edits: Vec<PatchEdit> = (0..2)
            .map(|i| make_edit(&format!("src/file{i}.rs"), "create"))
            .collect();
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits,
        };
        let policy = RunPolicy {
            patch_edit_threshold: 1,
            ..RunPolicy::default()
        };
        match evaluate_patch(&params, &policy) {
            PolicyDecision::RequiresApproval { policy_rationale, risk_reason, .. } => {
                assert!(policy_rationale.contains("large patch"));
                assert!(risk_reason.contains("threshold: 1"));
            }
            PolicyDecision::Proceed => panic!("expected RequiresApproval"),
        }
    }

    #[test]
    fn patch_delete_allowed_when_policy_disabled() {
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![make_edit("src/old.rs", "delete")],
        };
        let policy = RunPolicy {
            delete_requires_approval: false,
            ..RunPolicy::default()
        };
        assert_eq!(evaluate_patch(&params, &policy), PolicyDecision::Proceed);
    }

    #[test]
    fn patch_sensitive_path_allowed_when_policy_disabled() {
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![make_edit(".env.production", "replace")],
        };
        let policy = RunPolicy {
            sensitive_path_requires_approval: false,
            ..RunPolicy::default()
        };
        assert_eq!(evaluate_patch(&params, &policy), PolicyDecision::Proceed);
    }

    #[test]
    fn patch_outside_focus_allowed_when_policy_disabled() {
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![make_edit("other/module.rs", "create")],
        };
        let policy = RunPolicy {
            outside_focus_requires_approval: false,
            focus_paths: vec!["src/".to_string()],
            ..RunPolicy::default()
        };
        assert_eq!(evaluate_patch(&params, &policy), PolicyDecision::Proceed);
    }

    #[test]
    fn test_run_extra_safe_make_target_allows_custom_target() {
        let params = TestsRunParams {
            run_id: "r1".into(),
            scope: "make".into(),
            target: Some("deploy-staging".into()),
            reason: "deploy to staging".into(),
        };
        let policy = RunPolicy {
            extra_safe_make_targets: vec!["deploy-staging".to_string()],
            ..RunPolicy::default()
        };
        assert_eq!(evaluate_test_run(&params, &policy), PolicyDecision::Proceed);
    }

    #[test]
    fn test_run_extra_safe_target_case_insensitive() {
        let params = TestsRunParams {
            run_id: "r1".into(),
            scope: "make".into(),
            target: Some("DEPLOY-STAGING".into()),
            reason: "deploy".into(),
        };
        let policy = RunPolicy {
            extra_safe_make_targets: vec!["deploy-staging".to_string()],
            ..RunPolicy::default()
        };
        assert_eq!(evaluate_test_run(&params, &policy), PolicyDecision::Proceed);
    }

    #[test]
    fn test_run_risky_target_still_blocked_without_policy_entry() {
        let params = TestsRunParams {
            run_id: "r1".into(),
            scope: "make".into(),
            target: Some("destroy-prod".into()),
            reason: "danger".into(),
        };
        let policy = RunPolicy {
            extra_safe_make_targets: vec!["deploy-staging".to_string()],
            ..RunPolicy::default()
        };
        match evaluate_test_run(&params, &policy) {
            PolicyDecision::RequiresApproval { .. } => {}
            PolicyDecision::Proceed => panic!("expected RequiresApproval"),
        }
    }

    #[test]
    fn policy_default_matches_expected_values() {
        let policy = RunPolicy::default();
        assert_eq!(policy.patch_edit_threshold, 5);
        assert!(policy.delete_requires_approval);
        assert!(policy.sensitive_path_requires_approval);
        assert!(policy.outside_focus_requires_approval);
        assert!(policy.extra_safe_make_targets.is_empty());
        assert!(policy.focus_paths.is_empty());
    }
}
