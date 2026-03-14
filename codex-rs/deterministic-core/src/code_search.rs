//! Handler logic for `code.search`.

use anyhow::{Context, Result};
use deterministic_protocol::{CodeSearchMatch, CodeSearchParams, CodeSearchResult};
use std::path::Path;

/// Search for text matches in the workspace.
///
/// Uses `grep -rn` under the hood.  No LLM ranking.
pub fn search(params: &CodeSearchParams, workspace_root: &str) -> Result<CodeSearchResult> {
    let root = Path::new(workspace_root);
    anyhow::ensure!(root.is_dir(), "workspace root is not a directory: {workspace_root}");

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
}
