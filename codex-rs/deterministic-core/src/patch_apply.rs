//! Handler logic for `patch.apply`.

use anyhow::{Context, Result};
use deterministic_protocol::{PatchApplyParams, PatchApplyResult};
use std::path::Path;

/// Apply a set of edits to files in the workspace.
///
/// All file mutations go through this function — this is a design
/// invariant of the no-hidden-agent architecture.
pub fn apply(params: &PatchApplyParams, workspace_root: &str) -> Result<PatchApplyResult> {
    let root = Path::new(workspace_root);
    anyhow::ensure!(root.is_dir(), "workspace root is not a directory: {workspace_root}");

    let mut changed_files: Vec<String> = Vec::new();
    let mut total_additions: usize = 0;
    let mut total_deletions: usize = 0;

    for edit in &params.edits {
        let file_path = root.join(&edit.path);
        // Validate the path does not escape the workspace.
        let canonical_root = root.canonicalize().context("cannot resolve workspace root")?;

        match edit.operation.as_str() {
            "create" => {
                if let Some(parent) = file_path.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("cannot create parent dirs for {}", edit.path))?;
                }
                std::fs::write(&file_path, &edit.new_text)
                    .with_context(|| format!("cannot create file: {}", edit.path))?;
                let canonical = file_path.canonicalize()?;
                anyhow::ensure!(
                    canonical.starts_with(&canonical_root),
                    "path escapes workspace: {}",
                    edit.path
                );
                total_additions += edit.new_text.lines().count();
            }
            "replace" => {
                let canonical = file_path
                    .canonicalize()
                    .with_context(|| format!("cannot resolve path: {}", edit.path))?;
                anyhow::ensure!(
                    canonical.starts_with(&canonical_root),
                    "path escapes workspace: {}",
                    edit.path
                );
                let content = std::fs::read_to_string(&canonical)
                    .with_context(|| format!("cannot read file: {}", edit.path))?;
                let new_content = if let Some(old_text) = &edit.old_text {
                    anyhow::ensure!(
                        content.contains(old_text.as_str()),
                        "old_text not found in {}: {:?}",
                        edit.path,
                        old_text
                    );
                    content.replacen(old_text.as_str(), &edit.new_text, 1)
                } else if let (Some(start), Some(end)) = (edit.start_line, edit.end_line) {
                    let lines: Vec<&str> = content.lines().collect();
                    let s = (start as usize).saturating_sub(1);
                    let e = (end as usize).min(lines.len());
                    let mut result_lines: Vec<&str> = Vec::new();
                    result_lines.extend_from_slice(&lines[..s]);
                    let new_lines: Vec<&str> = edit.new_text.lines().collect();
                    result_lines.extend_from_slice(&new_lines);
                    if e < lines.len() {
                        result_lines.extend_from_slice(&lines[e..]);
                    }
                    total_deletions += e - s;
                    result_lines.join("\n")
                } else {
                    anyhow::bail!(
                        "replace requires either old_text or start_line/end_line for {}",
                        edit.path
                    );
                };
                total_additions += edit.new_text.lines().count();
                std::fs::write(&canonical, &new_content)
                    .with_context(|| format!("cannot write file: {}", edit.path))?;
            }
            "delete" => {
                let canonical = file_path
                    .canonicalize()
                    .with_context(|| format!("cannot resolve path: {}", edit.path))?;
                anyhow::ensure!(
                    canonical.starts_with(&canonical_root),
                    "path escapes workspace: {}",
                    edit.path
                );
                let content = std::fs::read_to_string(&canonical)?;
                total_deletions += content.lines().count();
                std::fs::remove_file(&canonical)
                    .with_context(|| format!("cannot delete file: {}", edit.path))?;
            }
            other => {
                anyhow::bail!("unsupported patch operation: {other}");
            }
        }

        if !changed_files.contains(&edit.path) {
            changed_files.push(edit.path.clone());
        }
    }

    Ok(PatchApplyResult {
        changed_files,
        diff_stats: format!("+{total_additions} -{total_deletions}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use deterministic_protocol::PatchEdit;

    #[test]
    fn create_and_replace() {
        let dir = tempfile::tempdir().unwrap();

        // Create a file
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![PatchEdit {
                path: "new.txt".into(),
                operation: "create".into(),
                start_line: None,
                end_line: None,
                old_text: None,
                new_text: "hello world\n".into(),
                anchor_text: None,
                reason: Some("create test file".into()),
            }],
        };
        let result = apply(&params, dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.changed_files, vec!["new.txt"]);

        // Replace via old_text
        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![PatchEdit {
                path: "new.txt".into(),
                operation: "replace".into(),
                start_line: None,
                end_line: None,
                old_text: Some("hello world".into()),
                new_text: "hello rust".into(),
                anchor_text: None,
                reason: Some("update greeting".into()),
            }],
        };
        let result = apply(&params, dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.changed_files, vec!["new.txt"]);

        let content = std::fs::read_to_string(dir.path().join("new.txt")).unwrap();
        assert!(content.contains("hello rust"));
    }

    #[test]
    fn delete_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("del.txt"), "bye").unwrap();

        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![PatchEdit {
                path: "del.txt".into(),
                operation: "delete".into(),
                start_line: None,
                end_line: None,
                old_text: None,
                new_text: String::new(),
                anchor_text: None,
                reason: Some("remove file".into()),
            }],
        };
        let result = apply(&params, dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.changed_files, vec!["del.txt"]);
        assert!(!dir.path().join("del.txt").exists());
    }
}
