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
  cd /Users/adrian/Projects/ChatCodex/codex-rs &&
  cargo build -p deterministic-protocol -p deterministic-core -p deterministic-daemon &&
  cargo test -p deterministic-protocol -p deterministic-core -p deterministic-daemon -- --nocapture &&
  cargo clippy -p deterministic-protocol -p deterministic-core -p deterministic-daemon --all-targets -- -D warnings
) || ABORT=1

echo "[5/8] Verify TypeScript build/test"
(
  cd /Users/adrian/Projects/ChatCodex/apps/chatgpt-mcp &&
  npm ci &&
  npm run build &&
  npm test
) || ABORT=1

echo "[6/8] Re-run invariant greps"
(
  cd /Users/adrian/Projects/ChatCodex &&
  grep -RInE 'turn/start|turn/steer|review/start|codex\(|codex-reply\(|continue_run|resume_thread|resume_codex_thread|agent_step|fix_end_to_end' \
    codex-rs/deterministic-* apps/chatgpt-mcp/src .github/workflows || true
  grep -RInE 'openai|anthropic|gemini|ollama|xai|responses api|chat completions|model provider' \
    codex-rs/deterministic-* apps/chatgpt-mcp || true
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
