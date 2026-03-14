//! Handler logic for `file.read`.

use anyhow::{Context, Result};
use deterministic_protocol::{FileReadParams, FileReadResult};
use std::path::Path;

/// Read a file or a bounded range of lines from it.
pub fn read(params: &FileReadParams, workspace_root: &str) -> Result<FileReadResult> {
    let base = Path::new(workspace_root);
    let file_path = base.join(&params.path);

    // Prevent directory traversal outside workspace.
    let canonical = file_path
        .canonicalize()
        .with_context(|| format!("cannot resolve path: {}", params.path))?;
    let canonical_base = base
        .canonicalize()
        .with_context(|| format!("cannot resolve workspace root: {workspace_root}"))?;
    anyhow::ensure!(
        canonical.starts_with(&canonical_base),
        "path escapes workspace: {}",
        params.path
    );

    let content = std::fs::read_to_string(&canonical)
        .with_context(|| format!("cannot read file: {}", params.path))?;
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len() as u64;

    let start = params.start_line.unwrap_or(1).max(1);
    let end = params.end_line.unwrap_or(total_lines).min(total_lines);

    let selected: String = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            let line_no = (*i as u64) + 1;
            line_no >= start && line_no <= end
        })
        .map(|(_, l)| *l)
        .collect::<Vec<_>>()
        .join("\n");

    Ok(FileReadResult {
        path: params.path.clone(),
        content: selected,
        start_line: start,
        end_line: end,
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
}
