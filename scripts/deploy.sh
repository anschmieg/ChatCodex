#!/usr/bin/env bash
# ============================================================================
# ChatCodex Deployment Script
#
# Starts the deterministic daemon and MCP gateway as background processes.
# Use --help to see usage. Use --teardown to stop background processes.
#
# Usage:
#   ./scripts/deploy.sh                     # start both services
#   ./scripts/deploy.sh --teardown          # stop background processes
#   ./scripts/deploy.sh --source /path/to/repo --daemon-port 19281
# ============================================================================
set -euo pipefail

DAEMON_PID=""
GATEWAY_PID=""
CLEANED_UP=""

# Default configuration
SOURCE_DIR="${SOURCE_DIR:-}"
DAEMON_PORT="${DAEMON_PORT:-19280}"
DAEMON_BIND="127.0.0.1:${DAEMON_PORT}"
DAEMON_STORE_DIR="${DAEMON_STORE_DIR:-/tmp/chatcodex-daemon}"
WORKSPACE_ROOT="${WORKSPACE_ROOT:-}"
DAEMON_LOG="${DAEMON_LOG:-/tmp/chatcodex-daemon.log}"
GATEWAY_PORT="${GATEWAY_PORT:-3000}"
GATEWAY_HOST="${GATEWAY_HOST:-127.0.0.1}"
GATEWAY_LOG="${GATEWAY_LOG:-/tmp/chatcodex-mcp.log}"
MCP_TRANSPORT="${MCP_TRANSPORT:-http}"

# Hybrid mode (optional)
HYBRID_ENABLED="${CHATCODEX_HYBRID_ENABLED:-}"
HYBRID_PROVIDER_URL="${CHATCODEX_HYBRID_PROVIDER_URL:-}"
HYBRID_MODEL="${CHATCODEX_HYBRID_MODEL:-}"

usage() {
    cat <<EOF
ChatCodex Deployment Script

Starts the deterministic daemon and MCP gateway as background processes.

Usage: ./deploy.sh [options]

Options:
  --source DIR        Path to the ChatCodex repo (default: derived from script location)
  --daemon-port PORT  Port for the daemon (default: 19280)
  --daemon-bind HOST:PORT  Bind address for the daemon (default: 127.0.0.1:19280)
  --store-dir DIR     Directory for daemon SQLite store (default: /tmp/chatcodex-daemon)
  --workspace ROOT   Workspace root directory (default: none)
  --gateway-port PORT Port for the MCP HTTP gateway (default: 3000)
  --gateway-host HOST Host for the MCP gateway (default: 127.0.0.1)
  --transport TRANSPORT  Transport mode: 'stdio' or 'http' (default: http)
  --hybrid            Enable hybrid mode (requires HYBRID_PROVIDER_URL and HYBRID_MODEL)
  --hybrid-url URL    Worker LLM base URL (used when --hybrid is set)
  --hybrid-model M    Worker model name (used when --hybrid is set)
  --teardown          Stop background processes and exit
  --help              Show this help message

Environment variables also work (override CLI args):
  CHATCODEX_DAEMON_PORT, CHATCODEX_WORKSPACE_ROOT, CHATCODEX_HYBRID_ENABLED,
  CHATCODEX_HYBRID_PROVIDER_URL, CHATCODEX_HYBRID_MODEL

Examples:
  # Minimal local setup
  ./deploy.sh

  # Remote accessible (browser ChatGPT)
  ./deploy.sh --gateway-host 0.0.0.0 --gateway-port 3000

  # With hybrid mode
  ./deploy.sh --hybrid --hybrid-url http://localhost:11434/v1 --hybrid-model qwen2.5-coder

  # Docker compose production
  docker compose up

  # Teardown
  ./deploy.sh --teardown
EOF
}

log() { echo "[chatcodex-deploy] $*" >&2; }
warn() { echo "[chatcodex-deploy] WARNING: $*" >&2; }
die() { echo "[chatcodex-deploy] ERROR: $*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

TEARDOWN=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --help) usage; exit 0 ;;
        --teardown) TEARDOWN="1"; shift ;;
        --source) SOURCE_DIR="$2"; shift 2 ;;
        --daemon-port) DAEMON_PORT="$2"; shift 2 ;;
        --daemon-bind) DAEMON_BIND="$2"; shift 2 ;;
        --store-dir) DAEMON_STORE_DIR="$2"; shift 2 ;;
        --workspace) WORKSPACE_ROOT="$2"; shift 2 ;;
        --gateway-port) GATEWAY_PORT="$2"; shift 2 ;;
        --gateway-host) GATEWAY_HOST="$2"; shift 2 ;;
        --transport) MCP_TRANSPORT="$2"; shift 2 ;;
        --hybrid) HYBRID_ENABLED="true"; shift ;;
        --hybrid-url) HYBRID_PROVIDER_URL="$2"; shift 2 ;;
        --hybrid-model) HYBRID_MODEL="$2"; shift 2 ;;
        *) die "unknown option: $1 (use --help)" ;;
    esac
done

# Resolve source directory
if [[ -z "$SOURCE_DIR" ]]; then
    SOURCE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fi

[[ -d "$SOURCE_DIR" ]] || die "source directory not found: $SOURCE_DIR"
[[ -f "$SOURCE_DIR/Cargo.toml" ]] || die "not a ChatCodex repo (no Cargo.toml): $SOURCE_DIR"

DAEMON_BIN="$SOURCE_DIR/target/release/deterministic-daemon"
GATEWAY_DIR="$SOURCE_DIR/apps/chatgpt-mcp"
GATEWAY_BIN="$GATEWAY_DIR/dist/index.js"

# ---------------------------------------------------------------------------
# Teardown
# ---------------------------------------------------------------------------

stop_daemon() {
    if [[ -n "$DAEMON_PID" ]] && kill -0 "$DAEMON_PID" 2>/dev/null; then
        log "stopping daemon (PID $DAEMON_PID)..."
        kill "$DAEMON_PID" 2>/dev/null || true
    fi
}

stop_gateway() {
    if [[ -n "$GATEWAY_PID" ]] && kill -0 "$GATEWAY_PID" 2>/dev/null; then
        log "stopping MCP gateway (PID $GATEWAY_PID)..."
        kill "$GATEWAY_PID" 2>/dev/null || true
    fi
}

if [[ -n "$TEARDOWN" ]]; then
    log "teardown requested"
    stop_daemon
    stop_gateway
    log "teardown complete"
    exit 0
fi

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

log "building daemon..."
mkdir -p "$(dirname "$DAEMON_LOG")"
mkdir -p "$DAEMON_STORE_DIR"

RUST_BUILD_LOG="$DAEMON_LOG.build"
if ! cargo build --release -p deterministic-daemon \
    --manifest-path "$SOURCE_DIR/Cargo.toml" \
    > "$RUST_BUILD_LOG" 2>&1; then
    die "daemon build failed (see $RUST_BUILD_LOG)"
fi
log "daemon built"

if [[ "$MCP_TRANSPORT" == "http" ]]; then
    log "building MCP gateway..."
    (
        cd "$GATEWAY_DIR" || die "cannot cd to $GATEWAY_DIR"
        npm install --silent \
            && npm run build \
            || die "MCP gateway build failed"
    )
    log "gateway built"
fi

# ---------------------------------------------------------------------------
# Start daemon
# ---------------------------------------------------------------------------

DAEMON_ENV=(
    "DETERMINISTIC_BIND=$DAEMON_BIND"
    "DETERMINISTIC_STORE_DIR=$DAEMON_STORE_DIR"
)

if [[ -n "$WORKSPACE_ROOT" ]]; then
    DAEMON_ENV+=("DETERMINISTIC_WORKSPACE_ROOT=$WORKSPACE_ROOT")
fi

if [[ -n "$HYBRID_ENABLED" ]]; then
    DAEMON_ENV+=("CHATCODEX_HYBRID_ENABLED=true")
    if [[ -n "$HYBRID_PROVIDER_URL" ]]; then
        DAEMON_ENV+=("CHATCODEX_HYBRID_PROVIDER_URL=$HYBRID_PROVIDER_URL")
    fi
    if [[ -n "$HYBRID_MODEL" ]]; then
        DAEMON_ENV+=("CHATCODEX_HYBRID_MODEL=$HYBRID_MODEL")
    fi
fi

log "starting daemon on $DAEMON_BIND..."
"${DAEMON_ENV[@]}" "$DAEMON_BIN" >> "$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!

# Wait briefly for daemon to bind
sleep 1
if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
    die "daemon failed to start (see $DAEMON_LOG)"
fi
log "daemon started (PID $DAEMON_PID)"

# ---------------------------------------------------------------------------
# Start MCP gateway
# ---------------------------------------------------------------------------

if [[ "$MCP_TRANSPORT" == "http" ]]; then
    GATEWAY_ENV=(
        "NODE_ENV=production"
        "DETERMINISTIC_DAEMON_URL=http://127.0.0.1:${DAEMON_PORT}"
        "MCP_TRANSPORT=http"
        "PORT=$GATEWAY_PORT"
        "HOST=$GATEWAY_HOST"
    )

    log "starting MCP gateway on ${GATEWAY_HOST}:${GATEWAY_PORT}..."
    "${GATEWAY_ENV[@]}" node "$GATEWAY_BIN" >> "$GATEWAY_LOG" 2>&1 &
    GATEWAY_PID=$!

    sleep 1
    if ! kill -0 "$GATEWAY_PID" 2>/dev/null; then
        warn "gateway may have failed to start (see $GATEWAY_LOG)"
    else
        log "MCP gateway started (PID $GATEWAY_PID)"
    fi

    GATEWAY_URL="http://${GATEWAY_HOST}:${GATEWAY_PORT}/mcp"
    HEALTHZ_URL="http://${GATEWAY_HOST}:${GATEWAY_PORT}/healthz"
else
    GATEWAY_ENV=(
        "DETERMINISTIC_DAEMON_URL=http://127.0.0.1:${DAEMON_PORT}"
        "MCP_TRANSPORT=stdio"
    )

    log "starting MCP gateway in stdio mode..."
    "${GATEWAY_ENV[@]}" node "$GATEWAY_BIN" >> "$GATEWAY_LOG" 2>&1 &
    GATEWAY_PID=$!

    sleep 1
    if ! kill -0 "$GATEWAY_PID" 2>/dev/null; then
        warn "gateway may have failed to start (see $GATEWAY_LOG)"
    else
        log "MCP gateway started in stdio mode (PID $GATEWAY_PID)"
    fi

    GATEWAY_URL="stdio (process PID $GATEWAY_PID)"
    HEALTHZ_URL=""
fi

# ---------------------------------------------------------------------------
# Print summary
# ---------------------------------------------------------------------------

echo ""
echo "============================================"
echo "  ChatCodex stack is running"
echo "============================================"
echo ""
echo "  Daemon PID:     $DAEMON_PID"
echo "  Daemon URL:     http://127.0.0.1:${DAEMON_PORT}"
echo "  Daemon log:    $DAEMON_LOG"
echo "  Store dir:     $DAEMON_STORE_DIR"
echo ""
if [[ -n "$GATEWAY_PID" ]]; then
    echo "  Gateway PID:    $GATEWAY_PID"
    echo "  Gateway URL:    $GATEWAY_URL"
    echo "  Gateway log:    $GATEWAY_LOG"
    echo ""
fi
if [[ -n "$HYBRID_ENABLED" ]]; then
    echo "  Hybrid mode:    ENABLED (provider: $HYBRID_PROVIDER_URL, model: $HYBRID_MODEL)"
    echo ""
fi
echo "--------------------------------------------"
echo "  To verify:"
echo "    curl http://127.0.0.1:${DAEMON_PORT}/healthz"
if [[ -n "$HEALTHZ_URL" ]]; then
    echo "    curl $HEALTHZ_URL"
fi
echo ""
echo "  To register in ChatGPT Desktop:"
if [[ "$MCP_TRANSPORT" == "http" ]]; then
    echo "    MCP URL: $GATEWAY_URL"
else
    echo "    Run: cd $GATEWAY_DIR && MCP_TRANSPORT=stdio node dist/index.js"
    echo "    (ChatGPT Desktop auto-discovers stdio servers)"
fi
echo ""
echo "  To teardown:"
echo "    ./scripts/deploy.sh --teardown"
echo "============================================"
echo ""

# Wait for both processes (or either exits)
if [[ -n "$GATEWAY_PID" ]]; then
    tail --pid="$DAEMON_PID" -f /dev/null 2>/dev/null &
    WAIT_PID=$!
    wait "$DAEMON_PID" "$GATEWAY_PID" 2>/dev/null || true
    kill "$WAIT_PID" 2>/dev/null || true
else
    wait "$DAEMON_PID" 2>/dev/null || true
fi

log "a background process exited, shutting down..."
stop_daemon
stop_gateway
log "shutdown complete"