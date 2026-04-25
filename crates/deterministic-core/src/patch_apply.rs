//! Handler logic for `patch.apply`.

use anyhow::{Context, Result};
use deterministic_protocol::{PatchApplyParams, PatchApplyResult, PatchEdit};
use std::path::Path;

/// Apply a set of edits to files in the workspace.
///
/// All file mutations go through this function — this is a design
/// invariant of the no-hidden-agent architecture.
pub fn apply(params: &PatchApplyParams, workspace_root: &str) -> Result<PatchApplyResult> {
    let root = Path::new(workspace_root);
    anyhow::ensure!(
        root.is_dir(),
        "workspace root is not a directory: {workspace_root}"
    );

    let mut changed_files: Vec<String> = Vec::new();
    let mut total_additions: usize = 0;
    let mut total_deletions: usize = 0;

    for edit in &params.edits {
        let file_path = root.join(&edit.path);
        // Validate the path does not escape the workspace.
        let canonical_root = root
            .canonicalize()
            .context("cannot resolve workspace root")?;

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
                // If oldText is provided, remove only the matched content.
                // Otherwise remove the entire file.
                if let Some(ref old_text) = edit.old_text {
                    let new_content = content.replacen(old_text.as_str(), &edit.new_text, 1);
                    std::fs::write(&canonical, &new_content)
                        .with_context(|| format!("cannot write file: {}", edit.path))?;
                } else {
                    std::fs::remove_file(&canonical)
                        .with_context(|| format!("cannot delete file: {}", edit.path))?;
                }
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

    fn make_patch(params: PatchApplyParams) -> PatchApplyParams {
        params
    }

    #[test]
    fn create_and_replace() {
        let dir = tempfile::tempdir().unwrap();

        // Create a file
        let params = make_patch(PatchApplyParams {
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
            hybrid_worker_run_id: None,
            skip_policy: false,
        });
        let result = apply(&params, dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.changed_files, vec!["new.txt"]);

        // Replace via old_text
        let params = make_patch(PatchApplyParams {
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
            hybrid_worker_run_id: None,
            skip_policy: false,
        });
        let result = apply(&params, dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.changed_files, vec!["new.txt"]);
    }

    #[test]
    fn delete_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("del.txt"), "bye").unwrap();

        let params = make_patch(PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![PatchEdit {
                path: "del.txt".into(),
                operation: "delete".into(),
                start_line: None,
                end_line: None,
                old_text: None,
                new_text: String::new(),
                anchor_text: None,
                reason: Some("delete test file".into()),
            }],
            hybrid_worker_run_id: None,
            skip_policy: false,
        });
        let result = apply(&params, dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.changed_files, vec!["del.txt"]);
        assert!(!dir.path().join("del.txt").exists());
    }

    #[test]
    fn reject_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let params = make_patch(PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![PatchEdit {
                path: "no_such.txt".into(),
                operation: "replace".into(),
                start_line: None,
                end_line: None,
                old_text: Some("old".into()),
                new_text: "new".into(),
                anchor_text: None,
                reason: None,
            }],
            hybrid_worker_run_id: None,
            skip_policy: false,
        });
        let result = apply(&params, dir.path().to_str().unwrap());
        assert!(result.is_err(), "expected error for missing file, got: {:?}", result);
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("not found") || err.contains("cannot read") || err.contains("not a file") || err.contains("cannot resolve"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn reject_old_text_not_found() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), "actual content").unwrap();

        let params = make_patch(PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![PatchEdit {
                path: "file.txt".into(),
                operation: "replace".into(),
                start_line: None,
                end_line: None,
                old_text: Some("not present".into()),
                new_text: "new content".into(),
                anchor_text: None,
                reason: None,
            }],
            hybrid_worker_run_id: None,
            skip_policy: false,
        });
        let result = apply(&params, dir.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("old_text") || err.contains("not found"));
    }

    #[test]
    fn reject_replace_without_old_text_or_range() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), "hello world").unwrap();

        let params = make_patch(PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![PatchEdit {
                path: "file.txt".into(),
                operation: "replace".into(),
                start_line: None,
                end_line: None,
                old_text: None,
                new_text: "new".into(),
                anchor_text: None,
                reason: None,
            }],
            hybrid_worker_run_id: None,
            skip_policy: false,
        });
        let result = apply(&params, dir.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("replace requires either old_text or start_line/end_line"));
    }

    #[test]
    fn create_nested_directories() {
        let dir = tempfile::tempdir().unwrap();
        let params = make_patch(PatchApplyParams {
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
            hybrid_worker_run_id: None,
            skip_policy: false,
        });
        let result = apply(&params, dir.path().to_str().unwrap()).unwrap();
        assert!(dir.path().join("deep/nested/dir/file.txt").exists());
    }

    #[test]
    fn replace_with_start_end_lines() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("multi.txt"), "line1\nline2\nline3\nline4\n").unwrap();

        let params = make_patch(PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![PatchEdit {
                path: "multi.txt".into(),
                operation: "replace".into(),
                start_line: Some(2),
                end_line: Some(3),
                old_text: None,
                new_text: "new line\n".into(),
                anchor_text: None,
                reason: Some("replace lines 2-3".into()),
            }],
            hybrid_worker_run_id: None,
            skip_policy: false,
        });
        let result = apply(&params, dir.path().to_str().unwrap()).unwrap();
        let content = std::fs::read_to_string(dir.path().join("multi.txt")).unwrap();
        assert!(content.contains("line1"));
        assert!(content.contains("new line"));
        assert!(content.contains("line4"));
        assert!(!content.contains("line2"));
        assert!(!content.contains("line3"));
    }

    #[test]
    fn delete_with_old_text() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("sample.txt"), "hello world\nrust\n").unwrap();

        let params = make_patch(PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![PatchEdit {
                path: "sample.txt".into(),
                operation: "delete".into(),
                start_line: None,
                end_line: None,
                old_text: Some("hello world\n".into()),
                new_text: String::new(),
                anchor_text: None,
                reason: None,
            }],
            hybrid_worker_run_id: None,
            skip_policy: false,
        });
        let result = apply(&params, dir.path().to_str().unwrap()).unwrap();
        // After delete, the file should still exist with remaining content
        let content = std::fs::read_to_string(dir.path().join("sample.txt")).unwrap_or_default();
        // The delete replaces "hello world\n" with "" in the file content
        // so the remaining file should contain "rust\n"
        assert!(
            content.contains("rust"),
            "expected 'rust' in remaining file, got: {:?}",
            content
        );
        assert!(!content.contains("hello world"));
    }

    #[test]
    fn replace_preserves_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("trailing.txt"), "hello world\n").unwrap();

        let params = make_patch(PatchApplyParams {
            run_id: "r1".into(),
            edits: vec![PatchEdit {
                path: "trailing.txt".into(),
                operation: "replace".into(),
                start_line: None,
                end_line: None,
                old_text: Some("hello world".into()),
                new_text: "hi earth".into(),
                anchor_text: None,
                reason: None,
            }],
            hybrid_worker_run_id: None,
            skip_policy: false,
        });
        let result = apply(&params, dir.path().to_str().unwrap()).unwrap();
        let content = std::fs::read_to_string(dir.path().join("trailing.txt")).unwrap();
        assert_eq!(content, "hi earth\n");
    }
}
