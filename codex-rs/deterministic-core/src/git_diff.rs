//! Handler logic for `git.diff`.

use anyhow::{Context, Result};
use deterministic_protocol::{GitDiffParams, GitDiffResult};
use std::path::Path;

/// Return diff summary or patch text for the workspace.
pub fn diff(_params: &GitDiffParams, workspace_root: &str) -> Result<GitDiffResult> {
    let root = Path::new(workspace_root);
    anyhow::ensure!(root.is_dir(), "workspace root is not a directory: {workspace_root}");

    let format = _params.format.as_deref().unwrap_or("summary");

    // Get changed files list
    let name_output = std::process::Command::new("git")
        .args(["diff", "--name-only"])
        .current_dir(root)
        .output()
        .context("failed to run git diff --name-only")?;
    let names = String::from_utf8_lossy(&name_output.stdout);
    let mut changed_files: Vec<String> = names
        .lines()
        .filter(|l| !l.is_empty())
        .map(std::string::ToString::to_string)
        .collect();

    // If specific paths were requested, filter to those
    if !_params.paths.is_empty() {
        changed_files.retain(|f| _params.paths.iter().any(|p| f.contains(p.as_str())));
    }

    // Get stat summary
    let stat_output = std::process::Command::new("git")
        .args(["diff", "--stat"])
        .current_dir(root)
        .output()
        .context("failed to run git diff --stat")?;
    let diff_summary = String::from_utf8_lossy(&stat_output.stdout).to_string();

    let patch_text = if format == "patch" {
        let mut args = vec!["diff".to_string()];
        for p in &_params.paths {
            args.push("--".to_string());
            args.push(p.clone());
        }
        let patch_output = std::process::Command::new("git")
            .args(&args)
            .current_dir(root)
            .output()
            .context("failed to run git diff")?;
        Some(String::from_utf8_lossy(&patch_output.stdout).to_string())
    } else {
        None
    };

    Ok(GitDiffResult {
        changed_files,
        diff_summary,
        patch_text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_on_clean_repo() {
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
        std::fs::write(root.join("f.txt"), "hello").unwrap();
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

        let params = GitDiffParams {
            run_id: "r1".into(),
            paths: vec![],
            format: None,
        };
        let result = diff(&params, root.to_str().unwrap()).unwrap();
        assert!(result.changed_files.is_empty());
    }

    #[test]
    fn diff_on_dirty_repo() {
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
        std::fs::write(root.join("f.txt"), "hello").unwrap();
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

        // Modify the file
        std::fs::write(root.join("f.txt"), "hello world").unwrap();

        let params = GitDiffParams {
            run_id: "r1".into(),
            paths: vec![],
            format: Some("patch".into()),
        };
        let result = diff(&params, root.to_str().unwrap()).unwrap();
        assert!(result.changed_files.contains(&"f.txt".to_string()));
        assert!(result.patch_text.is_some());
    }
}
