//! Handler logic for `tests.run`.

use anyhow::{Context, Result};
use deterministic_protocol::{TestsRunParams, TestsRunResult};
use std::path::Path;

/// Whitelisted test commands by scope.
///
/// Only these well-known commands are allowed.  This prevents the
/// daemon from executing arbitrary shell commands.
fn resolve_command(scope: &str, target: Option<&str>) -> Result<Vec<String>> {
    match scope {
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
        other => anyhow::bail!("unsupported test scope: {other}"),
    }
}

/// Execute a whitelisted test command and capture the output.
pub fn run(params: &TestsRunParams, workspace_root: &str) -> Result<TestsRunResult> {
    let root = Path::new(workspace_root);
    anyhow::ensure!(root.is_dir(), "workspace root is not a directory: {workspace_root}");

    let cmd_parts = resolve_command(&params.scope, params.target.as_deref())?;
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
        stdout.clone()
    };
    let stderr_truncated = if stderr.len() > max_len {
        format!("{}... (truncated)", &stderr[..max_len])
    } else {
        stderr.clone()
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
        let cmd = resolve_command("cargo", Some("my_test")).unwrap();
        assert_eq!(cmd, vec!["cargo", "test", "my_test"]);
    }

    #[test]
    fn reject_unknown_scope() {
        let result = resolve_command("bash", None);
        assert!(result.is_err());
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
