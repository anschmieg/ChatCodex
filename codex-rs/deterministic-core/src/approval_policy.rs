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
    use deterministic_protocol::{PatchApplyParams, PatchEdit, RunPolicy, RunPolicyInput, TestsRunParams};

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
        let mut policy = default_policy();
        policy.focus_paths = vec!["src/".to_string()];
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
        let mut policy = default_policy();
        policy.focus_paths = vec!["src/".to_string()];
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

    // ---- Milestone 8: custom policy tests ----

    #[test]
    fn patch_custom_threshold_allows_more_edits() {
        let edits: Vec<PatchEdit> = (0..8)
            .map(|i| make_edit(&format!("src/file{i}.rs"), "create"))
            .collect();
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits,
        };
        let policy = RunPolicyInput {
            patch_edit_threshold: Some(10),
            ..Default::default()
        }
        .into_policy(vec![]);
        assert_eq!(evaluate_patch(&params, &policy), PolicyDecision::Proceed);
    }

    #[test]
    fn patch_custom_delete_not_required_skips_delete_gate() {
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![make_edit("src/old.rs", "delete")],
        };
        let policy = RunPolicyInput {
            delete_requires_approval: Some(false),
            ..Default::default()
        }
        .into_policy(vec![]);
        assert_eq!(evaluate_patch(&params, &policy), PolicyDecision::Proceed);
    }

    #[test]
    fn patch_custom_sensitive_not_required_skips_sensitive_gate() {
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![make_edit(".env.production", "create")],
        };
        let policy = RunPolicyInput {
            sensitive_path_requires_approval: Some(false),
            ..Default::default()
        }
        .into_policy(vec![]);
        assert_eq!(evaluate_patch(&params, &policy), PolicyDecision::Proceed);
    }

    #[test]
    fn patch_custom_outside_focus_disabled_skips_focus_gate() {
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![make_edit("other/module.rs", "create")],
        };
        let policy = RunPolicyInput {
            outside_focus_requires_approval: Some(false),
            ..Default::default()
        }
        .into_policy(vec!["src/".to_string()]);
        assert_eq!(evaluate_patch(&params, &policy), PolicyDecision::Proceed);
    }

    #[test]
    fn default_policy_round_trips() {
        let d = RunPolicy::default();
        assert_eq!(d.patch_edit_threshold, 5);
        assert!(d.delete_requires_approval);
        assert!(d.sensitive_path_requires_approval);
        assert!(d.outside_focus_requires_approval);
        assert!(d.extra_safe_make_targets.is_empty());
    }

    #[test]
    fn policy_input_merges_with_defaults() {
        let input = RunPolicyInput {
            patch_edit_threshold: Some(10),
            ..Default::default()
        };
        let policy = input.into_policy(vec!["src/".to_string()]);
        assert_eq!(policy.patch_edit_threshold, 10);
        assert!(policy.delete_requires_approval);  // default preserved
        assert_eq!(policy.focus_paths, vec!["src/"]);
    }

    #[test]
    fn policy_input_normalises_make_targets_to_lowercase() {
        let input = RunPolicyInput {
            extra_safe_make_targets: Some(vec!["Deploy".to_string(), "RELEASE".to_string()]),
            ..Default::default()
        };
        let policy = input.into_policy(vec![]);
        assert!(policy.extra_safe_make_targets.contains(&"deploy".to_string()));
        assert!(policy.extra_safe_make_targets.contains(&"release".to_string()));
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

    #[test]
    fn test_run_extra_safe_make_target_proceeds() {
        let params = TestsRunParams {
            run_id: "r1".into(),
            scope: "make".into(),
            target: Some("deploy".into()),
            reason: "deploy".into(),
        };
        let policy = RunPolicyInput {
            extra_safe_make_targets: Some(vec!["deploy".to_string()]),
            ..Default::default()
        }
        .into_policy(vec![]);
        assert_eq!(evaluate_test_run(&params, &policy), PolicyDecision::Proceed);
    }

    #[test]
    fn test_run_extra_safe_make_target_case_insensitive() {
        let params = TestsRunParams {
            run_id: "r1".into(),
            scope: "make".into(),
            target: Some("Deploy".into()),
            reason: "deploy".into(),
        };
        let policy = RunPolicyInput {
            extra_safe_make_targets: Some(vec!["deploy".to_string()]),
            ..Default::default()
        }
        .into_policy(vec![]);
        assert_eq!(evaluate_test_run(&params, &policy), PolicyDecision::Proceed);
    }
}
