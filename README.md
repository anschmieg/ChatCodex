# ChatCodex

A deterministic coding harness control plane for ChatGPT with an optional **dual-mode architecture**.

## What is this?

ChatCodex provides a deterministic execution environment for ChatGPT-assisted coding tasks. It gives ChatGPT fine-grained tools (read file, search code, apply patch, run tests, etc.) while keeping all policy enforcement server-side.

The project has two operating modes:

### Deterministic Mode (default)

ChatGPT is the **only** LLM in the stack. The Rust daemon is purely deterministic — no model calls, no agent loops, no autonomous continuation. Every file write goes through `apply_patch` and every test execution goes through `run_tests`.

### Hybrid Mode (opt-in)

ChatGPT remains the orchestrator but may start **bounded implementation-worker runs** using a configured OpenAI-compatible external/local LLM. Workers return proposed patches only. ChatGPT reviews those patches and explicitly applies them through `apply_patch`.

v1 supports OpenAI-compatible HTTP only (OpenAI API, Ollama with OpenAI-compatible endpoint, LM Studio, etc.). Anthropic is not implemented in v1.

## Architecture

```
ChatGPT model
  → MCP server (TypeScript, thin gateway)
  → internal JSON-RPC
  → deterministic Rust harness daemon
  → filesystem / git / patch / tests / approvals / sandbox

Hybrid mode extension (opt-in):
  → optional OpenAI-compatible LLM provider
  → workers return proposed patches only
  → ChatGPT reviews → apply_patch → workspace
```

### Key principles

1. **ChatGPT is the only LLM in deterministic mode** — no backend model SDKs or API calls
2. **Hybrid workers are bounded** — they produce proposed patches, not direct file mutations
3. **Server-side policy enforcement** — approvals and restrictions are backend-owned
4. **Thin TypeScript gateway** — validation, mapping, and formatting only

## Repository structure

```
codex-rs/
  deterministic-protocol/  # Shared types and method names
  deterministic-core/      # Deterministic logic and policy
  deterministic-daemon/    # HTTP JSON-RPC transport, SQLite persistence

apps/chatgpt-mcp/          # TypeScript MCP gateway

docs/                      # Architecture and contract documentation
```

## Quick start

### Prerequisites

- Rust 1.93.0 (pinned via `codex-rs/rust-toolchain.toml`)
- Node.js 22+
- npm

### Build and test (deterministic crates)

```bash
cd codex-rs
cargo build -p deterministic-protocol -p deterministic-core -p deterministic-daemon
cargo test -p deterministic-protocol -p deterministic-core -p deterministic-daemon
cargo clippy -p deterministic-protocol -p deterministic-core -p deterministic-daemon --all-targets -- -D warnings
```

### TypeScript MCP gateway

```bash
cd apps/chatgpt-mcp
npm ci
npm run build
npm test
```

## Deterministic mode tools

| Tool | Description |
|------|-------------|
| `codex_prepare_run` | Initialize a coding run with goal and plan |
| `get_workspace_summary` | Workspace overview and detected tooling |
| `read_file` | Read file contents with optional line ranges |
| `git_status` | Working tree status |
| `search_code` | Text/symbol search with snippets |
| `apply_patch` | Apply patches (policy-gated) |
| `run_tests` | Execute whitelisted test commands (policy-gated) |
| `show_diff` | Diff summary or patch text |
| `refresh_run_state` | Read-only run state snapshot |
| `replan_run` | Deterministic rule-based replanning |
| `approve_action` | Resolve pending approvals |
| `list_runs` | List known runs with filtering |
| `get_run_state` | Get authoritative current state |
| `get_run_history` | Get audit trail for a run |
| `preview_patch_policy` | Preview patch policy decision |
| `preview_test_policy` | Preview test-run policy decision |
| `finalize_run` | Close a run with structured outcome |
| `reopen_run` | Reopen a finalized run |
| `supersede_run` | Create a successor run |
| `archive_run` | Archive a finalized run |
| `unarchive_run` | Restore an archived run |
| `annotate_run` | Attach labels/note to a run |
| `pin_run` / `unpin_run` | Pin/unpin a run |
| `snooze_run` / `unsnooze_run` | Snooze/unsnooze a run |
| `set_run_priority` | Set run priority |
| `assign_run_owner` | Assign/unassign run owner |
| `set_run_due_date` | Set/clear run due date |

## Hybrid mode tools (opt-in)

Available when `CHATCODEX_HYBRID_ENABLED=true` and per-run `harnessMode: "hybrid"`:

| Tool | Description |
|------|-------------|
| `hybrid_prepare_worker_run` | Prepare a bounded worker run |
| `hybrid_start_worker_run` | Start a worker (calls provider) |
| `hybrid_get_worker_run` | Get worker status and proposed patches |
| `hybrid_cancel_worker_run` | Cancel a prepared/running worker |
| `hybrid_list_worker_runs` | List workers for a parent run |

## Hybrid mode configuration

```bash
# Enable hybrid mode (disabled by default)
CHATCODEX_HYBRID_ENABLED=true

# Required: OpenAI-compatible endpoint
CHATCODEX_HYBRID_PROVIDER_BASE_URL=http://127.0.0.1:11434/v1
CHATCODEX_HYBRID_PROVIDER_MODEL=llama3

# Optional
CHATCODEX_HYBRID_PROVIDER_API_KEY_ENV=OLLAMA_API_KEY   # env var name for key
CHATCODEX_HYBRID_PROVIDER_TIMEOUT_SECONDS=120
CHATCODEX_HYBRID_PROVIDER_MAX_OUTPUT_TOKENS=8000
CHATCODEX_HYBRID_PROVIDER_TEMPERATURE=0.2

# Start the daemon
cargo run -p deterministic-daemon
```

## Safety model

### Hybrid workers are bounded

- Workers receive a task goal and focus paths
- Workers return `proposed_edits[]` — they never call `patch.apply`
- ChatGPT reviews proposed edits and explicitly invokes `apply_patch`
- Concurrency limits: max 3 global running workers, max 3 per parent run
- Cancellation sets a flag but does not mutate workspace files

### No hidden agent loops

- The backend never owns planning or execution through an LLM
- Forbidden tool/method names are checked at registration time
- Deterministic crates cannot depend on model SDKs

## Development

See [docs/DEVELOPMENT.md](./docs/DEVELOPMENT.md) for local verification commands.

See [docs/IMPLEMENTATION_PLAN_v2.md](./docs/IMPLEMENTATION_PLAN_v2.md) for the full dual-mode implementation plan.

## License

Apache-2.0 (see LICENSE)
