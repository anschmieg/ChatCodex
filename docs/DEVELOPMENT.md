# Development Guide

This document provides practical guidance for developers working on the deterministic ChatGPT control plane.

## Quick Start

### Prerequisites

- Rust toolchain (latest stable)
- Node.js 22+
- npm

### Repository Structure

```
codex-rs/
  deterministic-protocol/   # Shared DTOs and method names
  deterministic-core/       # Deterministic logic, policy, handlers
  deterministic-daemon/     # HTTP JSON-RPC transport, SQLite persistence

apps/chatgpt-mcp/           # TypeScript MCP gateway
```

## Local Verification

### Rust Deterministic Crates

```bash
cd codex-rs

# Build
cargo build -p deterministic-protocol -p deterministic-core -p deterministic-daemon

# Test
cargo test -p deterministic-protocol -p deterministic-core -p deterministic-daemon -- --nocapture

# Lint
cargo clippy -p deterministic-protocol -p deterministic-core -p deterministic-daemon --all-targets -- -D warnings
```

### TypeScript MCP Gateway

```bash
cd apps/chatgpt-mcp

# Install dependencies
npm ci

# Build
npm run build

# Test
npm test
```

### Full Verification

Run all checks:

```bash
# Rust
cd codex-rs
cargo build -p deterministic-protocol -p deterministic-core -p deterministic-daemon
cargo test -p deterministic-protocol -p deterministic-core -p deterministic-daemon -- --nocapture
cargo clippy -p deterministic-protocol -p deterministic-core -p deterministic-daemon --all-targets -- -D warnings

# TypeScript
cd ../apps/chatgpt-mcp
npm ci && npm run build && npm test
```

## Architecture Constraints

When making changes, ensure you maintain these invariants:

### 1. ChatGPT is the Only LLM

- Do not add model provider SDKs (OpenAI, Anthropic, Google, etc.)
- Do not make API calls to language models
- Do not add hidden agent loops

### 2. Deterministic Backend

- All logic in Rust must be rule-based and predictable
- No probabilistic reasoning in the daemon
- No autonomous iteration

### 3. Thin TypeScript Gateway

The MCP gateway should only:
- Validate inputs (Zod schemas)
- Map to daemon JSON-RPC calls
- Format responses

It must NOT:
- Contain core planning logic
- Make decisions about what actions to take
- Call model APIs

### 4. Approval Policy

Risky operations must be gated by the approval policy:
- File deletions
- Large patches (>5 edits)
- Sensitive file paths
- Out-of-focus edits
- Non-standard make targets

## Adding a New MCP Tool

1. **Define the contract** in `docs/MCP_TOOL_CONTRACTS.md`
2. **Add types** in `codex-rs/deterministic-protocol/src/types.rs`
3. **Add method** in `codex-rs/deterministic-protocol/src/methods.rs` (if new internal method)
4. **Implement handler** in `codex-rs/deterministic-core/src/` or `deterministic-daemon/src/handlers.rs`
5. **Add schema** in `apps/chatgpt-mcp/src/schemas.ts`
6. **Register tool** in `apps/chatgpt-mcp/src/tools.ts`
7. **Update tests** as needed
8. **Verify** all checks pass

## Adding a New Daemon Method

1. **Add to `Method` enum** in `deterministic-protocol/src/methods.rs`
2. **Add types** in `deterministic-protocol/src/types.rs`
3. **Wire handler** in `deterministic-daemon/src/handlers.rs`
4. **Implement logic** in `deterministic-core/src/` (if complex)
5. **Add tests**

## Database Schema Changes

The SQLite persistence layer automatically migrates older databases using `ALTER TABLE ADD COLUMN`.

When adding new columns:
1. Add to `migrate()` in `deterministic-daemon/src/persistence.rs`
2. Add to the `migrations` array with appropriate defaults
3. Update `save_run()` and `get_run()` if needed
4. Add test for migration from older schema

## Testing

### Rust Tests

```bash
cd codex-rs
cargo test -p deterministic-core -- --nocapture
cargo test -p deterministic-daemon -- --nocapture
```

### TypeScript Tests

```bash
cd apps/chatgpt-mcp
npm test
```

### Integration Testing

The CI workflow (`.github/workflows/milestone-deterministic.yml`) runs:
- Build and test for all deterministic crates
- TypeScript build and test
- Invariant checks (forbidden methods, tools, model SDKs)

## Debugging

### Daemon Logs

The daemon logs to stderr. When running locally:

```bash
cd codex-rs
cargo run -p deterministic-daemon -- --data-dir /tmp/daemon-data
```

### SQLite Inspection

```bash
sqlite3 /tmp/daemon-data/runs.db
.tables
.schema runs
.schema approvals
SELECT * FROM runs;
```

### MCP Gateway Debugging

Enable debug logging:

```bash
DEBUG=mcp* npm start
```

## Common Issues

### "table runs has no column named X"

The database was created with an older schema. The persistence layer should automatically migrate it. If not, delete the database file and restart:

```bash
rm /path/to/runs.db
```

### Forbidden method/tool errors

Check that you're not using names in `FORBIDDEN_METHODS` or `FORBIDDEN_TOOL_NAMES`.

### Model SDK dependency errors

Ensure no AI provider packages are in `Cargo.toml` `[dependencies]` sections or `package.json`.

## Code Style

- Rust: Follow standard Rust conventions, clippy-clean
- TypeScript: Use strict mode, explicit types
- Documentation: Keep docs in sync with implementation
- Comments: Explain "why", not "what"

## Getting Help

- Review `AGENTS.md` for architecture constraints
- Review `docs/ARCHITECTURE.md` for system overview
- Review `docs/MCP_TOOL_CONTRACTS.md` for tool behavior
- Review `docs/INTERNAL_RPC.md` for daemon method details
