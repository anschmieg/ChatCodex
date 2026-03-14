//! Handler logic for `tests.run`.

use anyhow::{Context, Result};
use deterministic_protocol::{TestsRunParams, TestsRunResult};
use std::path::Path;

/// Resolve a semantic scope to a concrete test command.
///
/// The scope can be either a well-known framework name (e.g. "cargo",
/// "npm", "pytest", "make") **or** a semantic label (e.g. "unit",
/// "integration", "all").  When a semantic label is used the daemon
/// inspects the workspace to determine the correct framework command
/// deterministically.
///
/// Only whitelisted commands are allowed — this prevents the daemon
/// from executing arbitrary shell commands.
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

        other => anyhow::bail!(
            "unsupported test scope: {other}. \
             Use a framework name (cargo, npm, pytest, make) or a semantic scope (unit, integration, all)."
        ),
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
pub fn run(params: &TestsRunParams, workspace_root: &str) -> Result<TestsRunResult> {
    let root = Path::new(workspace_root);
    anyhow::ensure!(root.is_dir(), "workspace root is not a directory: {workspace_root}");

    let cmd_parts = resolve_command(&params.scope, params.target.as_deref(), root)?;
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
    fn run_echo_test() {
        // We test with "make" scope pointing to a command that will
        // succeed in a directory with a trivial Makefile.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Makefile"),
            "test:\n\t@echo 'all tests passed'\n",
        )
        .unwrap();

        let params = TestsRunParams {
            run_id: "r1".into(),
            scope: "make".into(),
            target: None,
            reason: "verify tests pass".into(),
        };
        let result = run(&params, dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("all tests passed"));
    }
}
