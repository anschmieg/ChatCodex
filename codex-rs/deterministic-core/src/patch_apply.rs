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
        approval_required: None,
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

    #[test]
    fn reject_traversal() {
        let dir = tempfile::tempdir().unwrap();

        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![PatchEdit {
                path: "../../etc/passwd".into(),
                operation: "create".into(),
                start_line: None,
                end_line: None,
                old_text: None,
                new_text: "malicious".into(),
                anchor_text: None,
                reason: Some("attempt traversal".into()),
            }],
        };
        let result = apply(&params, dir.path().to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn reject_replace_nonexistent_file() {
        let dir = tempfile::tempdir().unwrap();

        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![PatchEdit {
                path: "does_not_exist.txt".into(),
                operation: "replace".into(),
                start_line: None,
                end_line: None,
                old_text: Some("old".into()),
                new_text: "new".into(),
                anchor_text: None,
                reason: Some("replace nonexistent".into()),
            }],
        };
        let result = apply(&params, dir.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cannot resolve path"));
    }

    #[test]
    fn reject_delete_nonexistent_file() {
        let dir = tempfile::tempdir().unwrap();

        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![PatchEdit {
                path: "does_not_exist.txt".into(),
                operation: "delete".into(),
                start_line: None,
                end_line: None,
                old_text: None,
                new_text: String::new(),
                anchor_text: None,
                reason: Some("delete nonexistent".into()),
            }],
        };
        let result = apply(&params, dir.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cannot resolve path"));
    }

    #[test]
    fn reject_old_text_not_found() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.txt"), "hello world").unwrap();

        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![PatchEdit {
                path: "test.txt".into(),
                operation: "replace".into(),
                start_line: None,
                end_line: None,
                old_text: Some("nonexistent text".into()),
                new_text: "replacement".into(),
                anchor_text: None,
                reason: Some("replace with wrong old_text".into()),
            }],
        };
        let result = apply(&params, dir.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("old_text not found"));
    }

    #[test]
    fn reject_unsupported_operation() {
        let dir = tempfile::tempdir().unwrap();

        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![PatchEdit {
                path: "test.txt".into(),
                operation: "move".into(), // unsupported
                start_line: None,
                end_line: None,
                old_text: None,
                new_text: String::new(),
                anchor_text: None,
                reason: Some("unsupported op".into()),
            }],
        };
        let result = apply(&params, dir.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unsupported patch operation"));
    }

    #[test]
    fn replace_by_line_range() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.txt"), "line1\nline2\nline3\nline4\n").unwrap();

        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![PatchEdit {
                path: "test.txt".into(),
                operation: "replace".into(),
                start_line: Some(2),
                end_line: Some(3),
                old_text: None,
                new_text: "replaced\n".into(),
                anchor_text: None,
                reason: Some("replace lines 2-3".into()),
            }],
        };
        let result = apply(&params, dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.changed_files, vec!["test.txt"]);
        assert_eq!(result.diff_stats, "+1 -2"); // 1 line added, 2 lines deleted

        let content = std::fs::read_to_string(dir.path().join("test.txt")).unwrap();
        // Note: join("\n") doesn't add trailing newline, so result is "line1\nreplaced\nline4"
        assert_eq!(content, "line1\nreplaced\nline4");
    }

    #[test]
    fn replace_requires_old_text_or_range() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.txt"), "content").unwrap();

        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![PatchEdit {
                path: "test.txt".into(),
                operation: "replace".into(),
                start_line: None,
                end_line: None,
                old_text: None, // no old_text
                new_text: "new".into(),
                anchor_text: None,
                reason: Some("replace without old_text or range".into()),
            }],
        };
        let result = apply(&params, dir.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("replace requires either old_text or start_line/end_line"));
    }

    #[test]
    fn create_nested_directories() {
        let dir = tempfile::tempdir().unwrap();

        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![PatchEdit {
                path: "deep/nested/dir/file.txt".into(),
                operation: "create".into(),
                start_line: None,
                end_line: None,
                old_text: None,
                new_text: "nested content\n".into(),
                anchor_text: None,
                reason: Some("create nested file".into()),
            }],
        };
        let result = apply(&params, dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.changed_files, vec!["deep/nested/dir/file.txt"]);

        let content = std::fs::read_to_string(
            dir.path().join("deep/nested/dir/file.txt")
        ).unwrap();
        assert_eq!(content, "nested content\n");
    }

    #[test]
    fn multiple_edits_same_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.txt"), "hello world\n").unwrap();

        let params = PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![
                PatchEdit {
                    path: "test.txt".into(),
                    operation: "replace".into(),
                    start_line: None,
                    end_line: None,
                    old_text: Some("hello".into()),
                    new_text: "hi".into(),
                    anchor_text: None,
                    reason: Some("first edit".into()),
                },
                PatchEdit {
                    path: "test.txt".into(),
                    operation: "replace".into(),
                    start_line: None,
                    end_line: None,
                    old_text: Some("world".into()),
                    new_text: "earth".into(),
                    anchor_text: None,
                    reason: Some("second edit".into()),
                },
            ],
        };
        let result = apply(&params, dir.path().to_str().unwrap()).unwrap();
        // File should only appear once in changed_files
        assert_eq!(result.changed_files, vec!["test.txt"]);

        let content = std::fs::read_to_string(dir.path().join("test.txt")).unwrap();
        assert_eq!(content, "hi earth\n");
    }
}
