//! Handler logic for `workspace.summary`.

use anyhow::{Context, Result};
use deterministic_protocol::{WorkspaceSummaryParams, WorkspaceSummaryResult};
use std::path::Path;

/// Produce a deterministic summary of the workspace.
pub fn summary(params: &WorkspaceSummaryParams) -> Result<WorkspaceSummaryResult> {
    let root = Path::new(&params.workspace_id);
    anyhow::ensure!(root.is_dir(), "workspace root is not a directory: {}", params.workspace_id);

    let detected_languages = detect_languages(root);
    let dirty_files = git_dirty_files(root).unwrap_or_default();

    let relevant_paths = if params.focus_paths.is_empty() {
        list_top_level(root)
    } else {
        params.focus_paths.clone()
    };

    Ok(WorkspaceSummaryResult {
        root: params.workspace_id.clone(),
        detected_languages,
        dirty_files,
        relevant_paths,
    })
}

fn detect_languages(root: &Path) -> Vec<String> {
    let mut langs = Vec::new();
    let markers: &[(&str, &str)] = &[
        ("Cargo.toml", "Rust"),
        ("package.json", "TypeScript/JavaScript"),
        ("go.mod", "Go"),
        ("pyproject.toml", "Python"),
        ("setup.py", "Python"),
        ("requirements.txt", "Python"),
        ("Makefile", "Make"),
        ("CMakeLists.txt", "C/C++"),
        ("pom.xml", "Java"),
        ("build.gradle", "Java/Kotlin"),
    ];
    for (file, lang) in markers {
        if root.join(file).exists() {
            langs.push((*lang).to_string());
        }
    }
    langs
}

fn git_dirty_files(root: &Path) -> Result<Vec<String>> {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()
        .context("failed to run git status")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.trim().to_string())
        .collect())
}

fn list_top_level(root: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if !name.starts_with('.') {
                    paths.push(name.to_string());
                }
            }
        }
    }
    paths.sort();
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_on_tempdir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let params = WorkspaceSummaryParams {
            workspace_id: dir.path().to_str().unwrap().to_string(),
            focus_paths: vec![],
        };
        let result = summary(&params).unwrap();
        assert!(result.detected_languages.contains(&"Rust".to_string()));
        assert!(result.relevant_paths.contains(&"Cargo.toml".to_string()));
    }
}
