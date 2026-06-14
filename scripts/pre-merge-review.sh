#!/usr/bin/env bash
set -euo pipefail

ABORT=0

echo "== Review flow for completed milestone =="

echo "[1/8] Verify branch state"
git status -sb || ABORT=1

echo "[2/8] Refresh refs"
git fetch origin || ABORT=1

echo "[3/8] Confirm main is current"
# fail fast if working tree is not clean
if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "ABORT=1 -- working tree is dirty. Commit, stash, or discard changes first."
  exit 1
fi
git checkout main || ABORT=1
git pull --ff-only || ABORT=1

echo "[4/8] Verify Rust build/test/lint"
(
  cd chatcodex && \
  cargo build -p chatcodex-mcp-server -p chatcodex-oauth && \
  cargo test -p chatcodex-mcp-server -p chatcodex-oauth -- --nocapture && \
  cargo clippy -p chatcodex-mcp-server -p chatcodex-oauth --all-targets -- -D warnings
) || ABORT=1

echo "[5/8] Verify ChatCodex does not modify upstream source"
(
  git diff --name-only HEAD | grep -qE '^codex-rs/' && {
    echo "ABORT=1 -- ChatCodex changes should not modify codex-rs/ files"
    exit 1
  } || true
) || ABORT=1

echo "[6/8] Re-run invariant greps"
(
  grep -RInE 'turn/start|turn/steer|review/start|codex\(|codex-reply\(|continue_run|resume_thread|resume_codex_thread|agent_step|fix_end_to_end' \
    chatcodex/crates .github/workflows || true
  grep -RInE 'openai|anthropic|gemini|ollama|xai|responses api|chat completions|model provider' \
    chatcodex/crates || true
) || ABORT=1

echo "[7/8] Merge/delete branch if everything passed"
if [[ "$ABORT" -ne 0 ]]; then
  echo "ABORT=1 -- at least one review step failed. Do NOT merge. Fix failures first."
  exit 1
fi

echo "[8/8] Safe merge flow"
# Example:
# git merge --ff-only <milestone-branch>
# git push origin main
# git branch -d <milestone-branch>
# git push origin --delete <milestone-branch>
# gh pr close <pr-number> --comment "Merged into main."

echo "All review steps passed. Safe to merge."
