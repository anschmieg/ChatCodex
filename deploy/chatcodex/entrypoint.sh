#!/usr/bin/env bash
set -euo pipefail

workspace_dir="${CHATCODEX_WORKSPACE_DIR:-/workspace/repo}"
store_dir="${CHATCODEX_STORE_DIR:-/workspace/store}"
daemon_port="${DAEMON_PORT:-19280}"
repo_url="${BOOTSTRAP_REPO_URL:-https://github.com/anschmieg/ChatCodex.git}"
repo_ref="${BOOTSTRAP_REPO_REF:-main}"

mkdir -p "${store_dir}"
mkdir -p "$(dirname "${workspace_dir}")"

if [[ ! -d "${workspace_dir}/.git" ]]; then
  rm -rf "${workspace_dir}"
  git clone --depth 1 --branch "${repo_ref}" "${repo_url}" "${workspace_dir}"
fi

export DETERMINISTIC_BIND="127.0.0.1:${daemon_port}"
export DETERMINISTIC_STORE_DIR="${store_dir}"
export DETERMINISTIC_DAEMON_URL="http://127.0.0.1:${daemon_port}"
export MCP_TRANSPORT="${MCP_TRANSPORT:-http}"
export HOST="${HOST:-0.0.0.0}"
export PORT="${PORT:-3000}"

/app/codex-rs/target/release/deterministic-daemon &
daemon_pid=$!

cleanup() {
  kill "${daemon_pid}" 2>/dev/null || true
}

trap cleanup EXIT INT TERM

cd /app/apps/chatgpt-mcp
exec node dist/index.js
