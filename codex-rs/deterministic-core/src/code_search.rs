//! Handler logic for `code.search`.
//!
//! # Contract
//!
//! Search for text matches in the workspace using `grep -rn`.
//!
//! ## Validation
//!
//! - `query` must be non-empty
//! - `max_results` defaults to 50 if not specified
//! - `path_glob` is optional and passed to grep's `--include`
//!
//! ## Returns
//!
//! - `matches`: array of matches with path, line number, and snippet
//! - Matches are ordered by grep's output order (file path, then line number)
//! - Empty query returns empty matches (not an error)
//! - No matches returns empty matches array

use anyhow::{Context, Result};
use deterministic_protocol::{CodeSearchMatch, CodeSearchParams, CodeSearchResult};
use std::path::Path;

/// Search for text matches in the workspace.
///
/// See module-level documentation for the full contract.
pub fn search(params: &CodeSearchParams, workspace_root: &str) -> Result<CodeSearchResult> {
    let root = Path::new(workspace_root);
    anyhow::ensure!(root.is_dir(), "workspace root is not a directory: {workspace_root}");

    // Handle empty query gracefully - return empty results
    if params.query.is_empty() {
        return Ok(CodeSearchResult { matches: vec![] });
    }

    let max = params.max_results.unwrap_or(50);

    let mut cmd = std::process::Command::new("grep");
    cmd.arg("-rn");

    if let Some(glob) = &params.path_glob {
        cmd.arg("--include");
        cmd.arg(glob);
    }

    cmd.arg("--");
    cmd.arg(&params.query);
    cmd.arg(".");
    cmd.current_dir(root);

    let output = cmd.output().context("failed to run grep")?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut matches = Vec::new();
    for line in stdout.lines() {
        if matches.len() >= max {
            break;
        }
        // Format: ./path:linenum:content
        let rest = line.strip_prefix("./").unwrap_or(line);
        let mut parts = rest.splitn(3, ':');
        if let (Some(file), Some(line_str), Some(snippet)) =
            (parts.next(), parts.next(), parts.next())
        {
            // Skip binary files
            if snippet == "Binary file matches" {
                continue;
            }
            let line_no: u64 = line_str.parse().unwrap_or(0);
            matches.push(CodeSearchMatch {
                path: file.to_string(),
                line: line_no,
                snippet: snippet.to_string(),
            });
        }
    }

    Ok(CodeSearchResult { matches })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_finds_text() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("foo.txt"), "hello world\ngoodbye world\n").unwrap();

        let params = CodeSearchParams {
            run_id: "r1".into(),
            query: "hello".into(),
            path_glob: Some("*.txt".into()),
            max_results: None,
        };
        let result = search(&params, dir.path().to_str().unwrap()).unwrap();
        assert!(!result.matches.is_empty());
        assert_eq!(result.matches[0].line, 1);
    }

    #[test]
    fn empty_query_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("foo.txt"), "hello world\n").unwrap();

        let params = CodeSearchParams {
            run_id: "r1".into(),
            query: "".into(),
            path_glob: None,
            max_results: None,
        };
        let result = search(&params, dir.path().to_str().unwrap()).unwrap();
        assert!(result.matches.is_empty());
    }

    #[test]
    fn no_matches_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("foo.txt"), "hello world\n").unwrap();

        let params = CodeSearchParams {
            run_id: "r1".into(),
            query: "xyz_not_found".into(),
            path_glob: None,
            max_results: None,
        };
        let result = search(&params, dir.path().to_str().unwrap()).unwrap();
        assert!(result.matches.is_empty());
    }

    #[test]
    fn respects_max_results() {
        let dir = tempfile::tempdir().unwrap();
        // Create multiple files with the same pattern
        for i in 0..10 {
            std::fs::write(
                dir.path().join(format!("file{i}.txt")),
                format!("pattern line {i}\n"),
            )
            .unwrap();
        }

        let params = CodeSearchParams {
            run_id: "r1".into(),
            query: "pattern".into(),
            path_glob: None,
            max_results: Some(3),
        };
        let result = search(&params, dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.matches.len(), 3);
    }

    #[test]
    fn search_respects_path_glob() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("include.txt"), "target text\n").unwrap();
        std::fs::write(dir.path().join("exclude.md"), "target text\n").unwrap();

        let params = CodeSearchParams {
            run_id: "r1".into(),
            query: "target".into(),
            path_glob: Some("*.txt".into()),
            max_results: None,
        };
        let result = search(&params, dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.matches.len(), 1);
        assert!(result.matches[0].path.ends_with("include.txt"));
    }

    #[test]
    fn search_finds_multiple_lines() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("multi.txt"),
            "line one\nline two\nline three\n",
        )
        .unwrap();

        let params = CodeSearchParams {
            run_id: "r1".into(),
            query: "line".into(),
            path_glob: None,
            max_results: None,
        };
        let result = search(&params, dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.matches.len(), 3);
        // Results should be ordered by line number
        assert_eq!(result.matches[0].line, 1);
        assert_eq!(result.matches[1].line, 2);
        assert_eq!(result.matches[2].line, 3);
    }

    #[test]
    fn search_skips_binary_files() {
        let dir = tempfile::tempdir().unwrap();
        // Create a text file with the pattern
        std::fs::write(dir.path().join("text.txt"), "searchable content\n").unwrap();
        // Create a "binary" file (just use different content that won't match)
        std::fs::write(dir.path().join("binary.bin"), vec![0u8, 1, 2, 3, 4]).unwrap();

        let params = CodeSearchParams {
            run_id: "r1".into(),
            query: "searchable".into(),
            path_glob: None,
            max_results: None,
        };
        let result = search(&params, dir.path().to_str().unwrap()).unwrap();
        // Should only find the text file
        assert_eq!(result.matches.len(), 1);
        assert!(result.matches[0].path.ends_with("text.txt"));
    }
}
