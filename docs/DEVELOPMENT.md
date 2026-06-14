# Development Guide

This document provides practical guidance for developers working on ChatCodex.

## Quick Start

### Prerequisites

- Rust toolchain (latest stable)

### Repository Structure

```
chatcodex/
  crates/
    mcp-server/       # Native Rust MCP server
    oauth/            # OAuth 2.1 authorization layer

codex-rs/             # upstream Codex checkout (do not modify)
```

## Local Verification

### Rust ChatCodex Crates

```bash
cd chatcodex

# Build
cargo build -p chatcodex-mcp-server -p chatcodex-oauth

# Test
cargo test -p chatcodex-mcp-server -p chatcodex-oauth -- --nocapture

# Lint
cargo clippy -p chatcodex-mcp-server -p chatcodex-oauth --all-targets -- -D warnings
```

### Full Verification

Run all checks:

```bash
cd chatcodex
cargo build -p chatcodex-mcp-server -p chatcodex-oauth
cargo test -p chatcodex-mcp-server -p chatcodex-oauth -- --nocapture
cargo clippy -p chatcodex-mcp-server -p chatcodex-oauth --all-targets -- -D warnings
```

## Architecture Constraints

When making changes, ensure you maintain these invariants:

### 1. ChatGPT is the Only LLM

- Do not add model provider SDKs (OpenAI, Anthropic, Google, etc.)
- Do not make API calls to language models
- Do not add hidden agent loops

### 2. Deterministic Backend

- All policy enforcement is server-side
- All file writes happen through `apply_patch`
- All test execution happens through `run_tests`
- `run_command` is restricted and whitelisted

### 3. No Hidden Agent Runtime

Forbidden flows:
- `turn/start`
- `turn/steer`
- `review/start`
- `codex()`
- `codex-reply()`
- `continue_run`
- `resume_thread`
- `agent_step`
- `fix_end_to_end`

## Adding a New MCP Tool

1. **Add tool spec** in `chatcodex/crates/mcp-server/src/lib.rs`
2. **Implement handler** in `chatcodex/crates/mcp-server/src/lib.rs`
3. **Add test** in `chatcodex/crates/mcp-server/tests/`
4. **Verify** with `cargo test -p chatcodex-mcp-server`
