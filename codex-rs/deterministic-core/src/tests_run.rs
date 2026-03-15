//! Handler logic for `tests.run`.
//!
//! # Contract
//!
//! The `scope` parameter accepts **semantic test scopes** that are resolved
//! to concrete commands deterministically based on workspace detection.
//!
//! ## Accepted scope values
//!
//! ### Framework names (explicit)
//! - `"cargo"` → `cargo test [target]`
//! - `"npm"` → `npm test [-- target]`
//! - `"pytest"` → `python -m pytest [target]`
//! - `"make"` → `make [target|test]`
//!
//! ### Semantic labels (auto-resolved)
//! - `"unit"` → resolved via workspace detection
//! - `"integration"` → resolved via workspace detection
//! - `"all"` → resolved via workspace detection
//!
//! ## Resolution order
//!
//! 1. If scope is a known framework name, use it directly
//! 2. If scope is a semantic label, detect framework via workspace files:
//!    - `Cargo.toml` exists → "cargo"
//!    - `package.json` exists → "npm"
//!    - `setup.py` or `pyproject.toml` exists → "pytest"
//!    - `Makefile` exists → "make"
//! 3. If no framework detected, return error
//!
//! ## Validation
//!
//! - Scope must be non-empty
//! - Reason must be non-empty (for audit trail)
//! - Target is optional and passed through to the framework command
//!
//! ## Errors
//!
//! - Returns error for unsupported scope values
//! - Returns error if workspace framework cannot be auto-detected
//! - Returns error if test command fails to execute

use anyhow::{Context, Result};
use deterministic_protocol::{TestsRunParams, TestsRunResult};
use std::path::Path;

/// Well-known framework scopes that map directly to commands.
const FRAMEWORK_SCOPES: &[&str] = &["cargo", "npm", "pytest", "make"];

/// Semantic scopes that require workspace detection.
const SEMANTIC_SCOPES: &[&str] = &["unit", "integration", "all"];

/// Resolve a semantic scope to a concrete test command.
///
/// See module-level documentation for the full contract.
fn resolve_command(scope: &str, target: Option<&str>, workspace_root: &Path) -> Result<Vec<String>> {
    match scope {
        // --- well-known framework scopes ---
        "cargo" => {
            let mut cmd = vec!["cargo".to_string(), "test".to_string()];
            if let Some(t) = target {
                cmd.push(t.to_string());
            }
            Ok(cmd)
        }
        "npm" => {
            let mut cmd = vec!["npm".to_string(), "test".to_string()];
            if let Some(t) = target {
                cmd.push("--".to_string());
                cmd.push(t.to_string());
            }
            Ok(cmd)
        }
        "pytest" => {
            let mut cmd = vec!["python".to_string(), "-m".to_string(), "pytest".to_string()];
            if let Some(t) = target {
                cmd.push(t.to_string());
            }
            Ok(cmd)
        }
        "make" => {
            let mut cmd = vec!["make".to_string()];
            cmd.push(target.unwrap_or("test").to_string());
            Ok(cmd)
        }

        // --- semantic scopes resolved via workspace detection ---
        "unit" | "integration" | "all" => {
            // Detect the workspace tooling and delegate to the
            // appropriate framework.
            let detected = detect_framework(workspace_root)?;
            resolve_command(&detected, target, workspace_root)
        }

        other => {
            let is_framework = FRAMEWORK_SCOPES.contains(&other);
            let is_semantic = SEMANTIC_SCOPES.contains(&other);

            if is_framework {
                // This shouldn't happen since we handle all frameworks above,
                // but include for completeness
                anyhow::bail!("framework scope '{other}' not properly handled")
            } else if is_semantic {
                anyhow::bail!("semantic scope '{other}' not properly handled")
            } else {
                anyhow::bail!(
                    "unsupported test scope: '{other}'. \
                     Accepted values:\n\
                     - Framework names: cargo, npm, pytest, make\n\
                     - Semantic scopes: unit, integration, all"
                )
            }
        }
    }
}

/// Heuristic workspace framework detection.
fn detect_framework(workspace_root: &Path) -> Result<String> {
    if workspace_root.join("Cargo.toml").exists() {
        Ok("cargo".to_string())
    } else if workspace_root.join("package.json").exists() {
        Ok("npm".to_string())
    } else if workspace_root.join("setup.py").exists()
        || workspace_root.join("pyproject.toml").exists()
    {
        Ok("pytest".to_string())
    } else if workspace_root.join("Makefile").exists() {
        Ok("make".to_string())
    } else {
        anyhow::bail!(
            "cannot auto-detect test framework in {}. \
             Use an explicit framework scope (cargo, npm, pytest, make) instead.",
            workspace_root.display()
        )
    }
}

/// Execute a whitelisted test command and capture the output.
///
/// # Validation
///
/// - `scope` must be non-empty and a supported value
/// - `reason` must be non-empty (for audit trail)
/// - `workspace_root` must be a valid directory
///
/// # Output truncation
///
/// stdout/stderr are truncated to 4096 characters in the structured
/// response to prevent oversized payloads.
pub fn run(params: &TestsRunParams, workspace_root: &str) -> Result<TestsRunResult> {
    // Validate required fields
    anyhow::ensure!(
        !params.scope.is_empty(),
        "scope must not be empty"
    );
    anyhow::ensure!(
        !params.reason.is_empty(),
        "reason must not be empty (required for audit trail)"
    );

    let root = Path::new(workspace_root);
    anyhow::ensure!(root.is_dir(), "workspace root is not a directory: {workspace_root}");

    // Validate scope is supported before attempting resolution
    let scope_lower = params.scope.to_lowercase();
    let is_framework = FRAMEWORK_SCOPES.contains(&scope_lower.as_str());
    let is_semantic = SEMANTIC_SCOPES.contains(&scope_lower.as_str());

    if !is_framework && !is_semantic {
        anyhow::bail!(
            "unsupported test scope: '{}'. \
             Supported framework scopes: {}. \
             Supported semantic scopes: {}.",
            params.scope,
            FRAMEWORK_SCOPES.join(", "),
            SEMANTIC_SCOPES.join(", ")
        );
    }

    let cmd_parts = resolve_command(&scope_lower, params.target.as_deref(), root)?;
    let resolved_command = cmd_parts.join(" ");

    let program = &cmd_parts[0];
    let args = &cmd_parts[1..];

    let output = std::process::Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| format!("failed to run test command: {resolved_command}"))?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    // Truncate for structured output
    let max_len = 4096;
    let stdout_truncated = if stdout.len() > max_len {
        format!("{}... (truncated)", &stdout[..max_len])
    } else {
        stdout
    };
    let stderr_truncated = if stderr.len() > max_len {
        format!("{}... (truncated)", &stderr[..max_len])
    } else {
        stderr
    };

    let summary = if exit_code == 0 {
        "tests passed".to_string()
    } else {
        format!("tests failed with exit code {exit_code}")
    };

    Ok(TestsRunResult {
        resolved_command,
        exit_code,
        stdout: stdout_truncated,
        stderr: stderr_truncated,
        summary,
        approval_required: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_cargo_test() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = resolve_command("cargo", Some("my_test"), dir.path()).unwrap();
        assert_eq!(cmd, vec!["cargo", "test", "my_test"]);
    }

    #[test]
    fn reject_unknown_scope() {
        let dir = tempfile::tempdir().unwrap();
        let result = resolve_command("bash", None, dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn semantic_scope_detects_cargo() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
        let cmd = resolve_command("unit", None, dir.path()).unwrap();
        assert_eq!(cmd[0], "cargo");
    }

    #[test]
    fn semantic_scope_detects_npm() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        let cmd = resolve_command("all", None, dir.path()).unwrap();
        assert_eq!(cmd[0], "npm");
    }

    #[test]
    fn reject_empty_scope() {
        let dir = tempfile::tempdir().unwrap();
        let params = TestsRunParams {
            run_id: "r1".into(),
            scope: "".into(),
            target: None,
            reason: "test".into(),
        };
        let result = run(&params, dir.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("scope must not be empty"), "error should mention empty scope: {err}");
    }

    #[test]
    fn reject_empty_reason() {
        let dir = tempfile::tempdir().unwrap();
        let params = TestsRunParams {
            run_id: "r1".into(),
            scope: "cargo".into(),
            target: None,
            reason: "".into(),
        };
        let result = run(&params, dir.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("reason must not be empty"), "error should mention empty reason: {err}");
    }

    #[test]
    fn reject_unsupported_scope() {
        let dir = tempfile::tempdir().unwrap();
        let params = TestsRunParams {
            run_id: "r1".into(),
            scope: "custom_framework".into(),
            target: None,
            reason: "test".into(),
        };
        let result = run(&params, dir.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unsupported test scope"), "error should mention unsupported scope: {err}");
    }

    #[test]
    fn scope_is_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        // Test uppercase CARGO
        let params = TestsRunParams {
            run_id: "r1".into(),
            scope: "CARGO".into(),
            target: None,
            reason: "test".into(),
        };
        let result = run(&params, dir.path().to_str().unwrap());
        assert!(result.is_ok(), "CARGO scope should work: {result:?}");
    }

    #[test]
    fn semantic_scope_fails_without_detectable_framework() {
        let dir = tempfile::tempdir().unwrap();
        // No framework files in this directory
        let params = TestsRunParams {
            run_id: "r1".into(),
            scope: "unit".into(),
            target: None,
            reason: "test".into(),
        };
        let result = run(&params, dir.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("cannot auto-detect test framework"),
            "error should mention auto-detection failure: {err}"
        );
    }

    #[test]
    fn npm_with_target_passes_correctly() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();

        // Just verify the command is constructed correctly by checking resolve_command
        let cmd = resolve_command("npm", Some("specific.test"), dir.path()).unwrap();
        assert_eq!(cmd, vec!["npm", "test", "--", "specific.test"]);
    }

    #[test]
    fn make_with_custom_target() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Makefile"),
            "custom:\n\t@echo 'custom target'\n",
        )
        .unwrap();

        let params = TestsRunParams {
            run_id: "r1".into(),
            scope: "make".into(),
            target: Some("custom".into()),
            reason: "test custom target".into(),
        };
        let result = run(&params, dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.resolved_command.contains("custom"));
    }

    #[test]
    fn make_defaults_to_test_target() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Makefile"),
            "test:\n\t@echo 'default test'\n",
        )
        .unwrap();

        let params = TestsRunParams {
            run_id: "r1".into(),
            scope: "make".into(),
            target: None,
            reason: "test default target".into(),
        };
        let result = run(&params, dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.resolved_command.contains("test"));
    }
}
