//! Deterministic approval policy layer.
//!
//! Classifies operations as safe, requiring approval, or rejected.
//! All rules are explicit and deterministic — no LLM reasoning.

use deterministic_protocol::{PatchApplyParams, TestsRunParams};

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

/// Maximum number of edits before approval is required.
const MAX_EDITS_WITHOUT_APPROVAL: usize = 5;

/// Evaluate whether a patch request requires approval.
///
/// Rules (evaluated in order — first match wins):
/// 1. Any delete operation → requires approval
/// 2. More than [`MAX_EDITS_WITHOUT_APPROVAL`] edits → requires approval
/// 3. Any path matching a sensitive pattern → requires approval
/// 4. Any path outside declared focus paths (when non-empty) → requires approval
/// 5. Otherwise → proceed
pub fn evaluate_patch(params: &PatchApplyParams, focus_paths: &[String]) -> PolicyDecision {
    // Rule 1: delete operations
    for edit in &params.edits {
        if edit.operation == "delete" {
            return PolicyDecision::RequiresApproval {
                action_summary: format!("Delete file: {}", edit.path),
                risk_reason: "File deletion is destructive and irreversible".into(),
                policy_rationale: "Policy: file deletion requires approval".into(),
            };
        }
    }

    // Rule 2: large patch (too many edits)
    if params.edits.len() > MAX_EDITS_WITHOUT_APPROVAL {
        return PolicyDecision::RequiresApproval {
            action_summary: format!(
                "Patch with {} edits across {} file(s)",
                params.edits.len(),
                unique_paths(&params.edits),
            ),
            risk_reason: format!(
                "Patch touches {} edits (threshold: {})",
                params.edits.len(),
                MAX_EDITS_WITHOUT_APPROVAL,
            ),
            policy_rationale: "Policy: large patch (>5 edits) requires approval".into(),
        };
    }

    // Rule 3: sensitive file paths
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

    // Rule 4: outside focus paths
    if !focus_paths.is_empty() {
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
        .any(|fp| path.starts_with(fp.as_str()) || fp.starts_with(path))
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
/// 1. `make` scope with a target not in the safe-target list → requires approval
/// 2. Otherwise → proceed
pub fn evaluate_test_run(params: &TestsRunParams) -> PolicyDecision {
    let scope_lower = params.scope.to_lowercase();

    // Rule 1: make with non-standard target
    if scope_lower == "make"
        && let Some(ref target) = params.target
    {
        let target_lower = target.to_lowercase();
        if !SAFE_MAKE_TARGETS.contains(&target_lower.as_str()) {
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
    use deterministic_protocol::{PatchApplyParams, PatchEdit, TestsRunParams};

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

    // ---- Patch policy tests ----

    #[test]
    fn patch_proceed_for_normal_edit() {
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![make_edit("src/main.rs", "replace")],
        };
        assert_eq!(evaluate_patch(&params, &[]), PolicyDecision::Proceed);
    }

    #[test]
    fn patch_requires_approval_for_delete() {
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![make_edit("src/old.rs", "delete")],
        };
        match evaluate_patch(&params, &[]) {
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
        match evaluate_patch(&params, &[]) {
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
        assert_eq!(evaluate_patch(&params, &[]), PolicyDecision::Proceed);
    }

    #[test]
    fn patch_requires_approval_for_sensitive_path() {
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![make_edit(".env.production", "create")],
        };
        match evaluate_patch(&params, &[]) {
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
        match evaluate_patch(&params, &[]) {
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
        match evaluate_patch(&params, &[]) {
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
        let focus = vec!["src/".to_string()];
        match evaluate_patch(&params, &focus) {
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
        let focus = vec!["src/".to_string()];
        assert_eq!(evaluate_patch(&params, &focus), PolicyDecision::Proceed);
    }

    #[test]
    fn patch_proceeds_when_no_focus_paths() {
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![make_edit("anywhere/file.rs", "create")],
        };
        assert_eq!(evaluate_patch(&params, &[]), PolicyDecision::Proceed);
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
        match evaluate_patch(&params, &[]) {
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
        assert_eq!(evaluate_test_run(&params), PolicyDecision::Proceed);
    }

    #[test]
    fn test_run_proceed_for_npm() {
        let params = TestsRunParams {
            run_id: "r1".into(),
            scope: "npm".into(),
            target: None,
            reason: "verify fix".into(),
        };
        assert_eq!(evaluate_test_run(&params), PolicyDecision::Proceed);
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
                evaluate_test_run(&params),
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
        match evaluate_test_run(&params) {
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
        assert_eq!(evaluate_test_run(&params), PolicyDecision::Proceed);
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
                evaluate_test_run(&params),
                PolicyDecision::Proceed,
                "semantic scope '{scope}' should be safe"
            );
        }
    }
}
