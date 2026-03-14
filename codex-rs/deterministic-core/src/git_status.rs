//! Handler logic for `git.status`.

use anyhow::{Context, Result};
use deterministic_protocol::{GitStatusParams, GitStatusResult};
use std::path::Path;

/// Return the working tree status for a workspace.
pub fn status(_params: &GitStatusParams, workspace_root: &str) -> Result<GitStatusResult> {
    let root = Path::new(workspace_root);
    anyhow::ensure!(root.is_dir(), "workspace root is not a directory: {workspace_root}");

    let branch = current_branch(root)?;
    let (dirty, untracked) = porcelain_status(root)?;

    Ok(GitStatusResult {
        branch,
        dirty_files: dirty,
        untracked_files: untracked,
    })
}

fn current_branch(root: &Path) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(root)
        .output()
        .context("failed to run git rev-parse")?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn porcelain_status(root: &Path) -> Result<(Vec<String>, Vec<String>)> {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()
        .context("failed to run git status")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut dirty = Vec::new();
    let mut untracked = Vec::new();
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        if line.starts_with("??") {
            if let Some(path) = line.get(3..) {
                untracked.push(path.trim().to_string());
            }
        } else if let Some(path) = line.get(3..) {
            dirty.push(path.trim().to_string());
        }
    }
    Ok((dirty, untracked))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_on_git_repo() {
        // Create a temp git repo
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "test"])
            .current_dir(root)
            .output()
            .unwrap();
        std::fs::write(root.join("file.txt"), "hello").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(root)
            .output()
            .unwrap();

        // Create an untracked file
        std::fs::write(root.join("new.txt"), "new").unwrap();

        let params = GitStatusParams {
            run_id: "r1".into(),
        };
        let result = status(&params, root.to_str().unwrap()).unwrap();
        assert!(result.untracked_files.contains(&"new.txt".to_string()));
    }
}
