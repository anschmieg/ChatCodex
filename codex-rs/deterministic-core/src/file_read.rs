//! Handler logic for `file.read`.
//!
//! # Contract
//!
//! Reads a file or a bounded range of lines from the workspace.
//!
//! ## Path validation
//!
//! - Path must not escape the workspace (directory traversal protection)
//! - Path must exist and be a file (not a directory)
//!
//! ## Line range validation
//!
//! - `start_line` and `end_line` are 1-indexed (first line is line 1)
//! - If `start_line` is omitted, starts from line 1
//! - If `end_line` is omitted, reads to end of file
//! - If `start_line` > `end_line`, returns error
//! - If `start_line` > total_lines, returns empty content (with adjusted bounds)
//! - If `end_line` > total_lines, clamps to total_lines
//! - Negative line numbers are treated as 1

use anyhow::{Context, Result};
use deterministic_protocol::{FileReadParams, FileReadResult};
use std::path::Path;

/// Read a file or a bounded range of lines from it.
///
/// See module-level documentation for the full contract.
pub fn read(params: &FileReadParams, workspace_root: &str) -> Result<FileReadResult> {
    // Validate path is not empty
    anyhow::ensure!(
        !params.path.is_empty(),
        "path must not be empty"
    );

    let base = Path::new(workspace_root);
    let file_path = base.join(&params.path);

    // Prevent directory traversal outside workspace.
    let canonical = file_path
        .canonicalize()
        .with_context(|| format!("file not found: {}", params.path))?;
    let canonical_base = base
        .canonicalize()
        .with_context(|| format!("cannot resolve workspace root: {workspace_root}"))?;
    anyhow::ensure!(
        canonical.starts_with(&canonical_base),
        "path escapes workspace: {}",
        params.path
    );

    // Ensure it's a file, not a directory
    anyhow::ensure!(
        canonical.is_file(),
        "path is not a file: {}",
        params.path
    );

    let content = std::fs::read_to_string(&canonical)
        .with_context(|| format!("cannot read file: {}", params.path))?;
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len() as u64;

    // Validate and normalize line range
    let start = params.start_line.unwrap_or(1).max(1);
    let end = params.end_line.unwrap_or(total_lines).max(1);

    // Handle edge cases
    if start > end {
        anyhow::bail!(
            "start_line ({start}) cannot be greater than end_line ({end})"
        );
    }

    // Clamp to file bounds
    let effective_start = start.min(total_lines.max(1));
    let effective_end = end.min(total_lines);

    // Handle case where start is beyond file length
    let selected: String = if effective_start > total_lines {
        // Return empty content when start is beyond file
        String::new()
    } else {
        lines
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                let line_no = (*i as u64) + 1;
                line_no >= effective_start && line_no <= effective_end
            })
            .map(|(_, l)| *l)
            .collect::<Vec<_>>()
            .join("\n")
    };

    Ok(FileReadResult {
        path: params.path.clone(),
        content: selected,
        start_line: effective_start,
        end_line: effective_end,
        total_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_whole_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hello.txt");
        std::fs::write(&file, "line1\nline2\nline3\n").unwrap();

        let params = FileReadParams {
            run_id: "r1".into(),
            path: "hello.txt".into(),
            start_line: None,
            end_line: None,
            purpose: None,
        };
        let result = read(&params, dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.total_lines, 3);
        assert!(result.content.contains("line1"));
        assert!(result.content.contains("line3"));
    }

    #[test]
    fn read_line_range() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hello.txt");
        std::fs::write(&file, "a\nb\nc\nd\ne\n").unwrap();

        let params = FileReadParams {
            run_id: "r1".into(),
            path: "hello.txt".into(),
            start_line: Some(2),
            end_line: Some(4),
            purpose: None,
        };
        let result = read(&params, dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.start_line, 2);
        assert_eq!(result.end_line, 4);
        assert!(result.content.contains('b'));
        assert!(result.content.contains('d'));
        assert!(!result.content.contains('a'));
        assert!(!result.content.contains('e'));
    }

    #[test]
    fn reject_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let params = FileReadParams {
            run_id: "r1".into(),
            path: "../../etc/passwd".into(),
            start_line: None,
            end_line: None,
            purpose: None,
        };
        let result = read(&params, dir.path().to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn reject_empty_path() {
        let dir = tempfile::tempdir().unwrap();
        let params = FileReadParams {
            run_id: "r1".into(),
            path: "".into(),
            start_line: None,
            end_line: None,
            purpose: None,
        };
        let result = read(&params, dir.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("path must not be empty"));
    }

    #[test]
    fn reject_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();

        let params = FileReadParams {
            run_id: "r1".into(),
            path: "subdir".into(),
            start_line: None,
            end_line: None,
            purpose: None,
        };
        let result = read(&params, dir.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("path is not a file"));
    }

    #[test]
    fn reject_nonexistent_file() {
        let dir = tempfile::tempdir().unwrap();
        let params = FileReadParams {
            run_id: "r1".into(),
            path: "does_not_exist.txt".into(),
            start_line: None,
            end_line: None,
            purpose: None,
        };
        let result = read(&params, dir.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("file not found"));
    }

    #[test]
    fn reject_start_greater_than_end() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "a\nb\nc\n").unwrap();

        let params = FileReadParams {
            run_id: "r1".into(),
            path: "test.txt".into(),
            start_line: Some(3),
            end_line: Some(1),
            purpose: None,
        };
        let result = read(&params, dir.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("start_line (3) cannot be greater than end_line (1)"));
    }

    #[test]
    fn clamp_end_beyond_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "a\nb\n").unwrap();

        let params = FileReadParams {
            run_id: "r1".into(),
            path: "test.txt".into(),
            start_line: Some(1),
            end_line: Some(100),
            purpose: None,
        };
        let result = read(&params, dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.end_line, 2); // clamped to actual file length
        assert_eq!(result.content, "a\nb");
    }

    #[test]
    fn start_beyond_file_returns_last_line() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "a\nb\n").unwrap();

        let params = FileReadParams {
            run_id: "r1".into(),
            path: "test.txt".into(),
            start_line: Some(10),
            end_line: Some(20),
            purpose: None,
        };
        let result = read(&params, dir.path().to_str().unwrap()).unwrap();
        // When start is beyond file, it clamps to file length
        assert_eq!(result.start_line, 2);
        assert_eq!(result.end_line, 2);
        // Returns the last line since that's where it clamped to
        assert_eq!(result.content, "b");
    }

    #[test]
    fn read_single_line() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "line1\nline2\nline3\n").unwrap();

        let params = FileReadParams {
            run_id: "r1".into(),
            path: "test.txt".into(),
            start_line: Some(2),
            end_line: Some(2),
            purpose: None,
        };
        let result = read(&params, dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.start_line, 2);
        assert_eq!(result.end_line, 2);
        assert_eq!(result.content, "line2");
    }

    #[test]
    fn read_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("empty.txt");
        std::fs::write(&file, "").unwrap();

        let params = FileReadParams {
            run_id: "r1".into(),
            path: "empty.txt".into(),
            start_line: None,
            end_line: None,
            purpose: None,
        };
        let result = read(&params, dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.total_lines, 0);
        assert_eq!(result.start_line, 1);
        assert_eq!(result.end_line, 0);
        assert!(result.content.is_empty());
    }
}
