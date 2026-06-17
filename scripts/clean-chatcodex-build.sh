#!/usr/bin/env bash
set -euo pipefail

CHATCODEX_DIR="$(cd "$(dirname "$0")/../chatcodex" && pwd)"

echo "=== ChatCodex Build Cleanup ==="

# 1. Clean workspace target dir (used when direnv is not active)
if [ -d "$CHATCODEX_DIR/target" ]; then
    echo "Cleaning $CHATCODEX_DIR/target..."
    rm -rf "$CHATCODEX_DIR/target"
    echo "  done"
fi

# 2. Clean CARGO_TARGET_DIR if set
if [ -n "${CARGO_TARGET_DIR:-}" ] && [ -d "$CARGO_TARGET_DIR" ]; then
    echo "Cleaning CARGO_TARGET_DIR ($CARGO_TARGET_DIR)..."
    rm -rf "$CARGO_TARGET_DIR"
    echo "  done"
fi

# 3. Clean default sccache cache
SCCACHE_DIR="${SCCACHE_DIR:-$HOME/.cache/sccache}"
if [ -d "$SCCACHE_DIR" ]; then
    echo "Cleaning sccache at $SCCACHE_DIR..."
    if command -v sccache &>/dev/null; then
        sccache --clear 2>/dev/null || true
    fi
    rm -rf "$SCCACHE_DIR"
    echo "  done"
fi

# 4. Run cargo-cache autoclean
if command -v cargo-cache &>/dev/null; then
    echo "Running cargo cache autoclean..."
    cargo cache -a 2>/dev/null || true
    echo "  done"
fi

echo "=== All cleaned ==="
echo ""
echo "Next build will recompile from scratch (sccache will cache dependencies)."
