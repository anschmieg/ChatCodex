# ChatCodex Onboarding Guide

## What is ChatCodex?

ChatCodex is a **deterministic coding harness control plane** that lets ChatGPT operate on a codebase without any backend LLM. Think of it as a structured state machine for coding tasks — ChatGPT drives, the backend executes deterministically.

### Core Principle

> **The only LLM in the stack is ChatGPT.**

No backend model calls. No hidden agent loops. No autonomous continuation. The backend is purely deterministic.

## Architecture Overview

```
┌─────────────────┐
│  ChatGPT (user) │
│   MCP Client    │
└────────┬────────┘
         │ MCP protocol
         ▼
┌─────────────────┐
│ TypeScript MCP │
│    Gateway      │  ← Thin validation/mapping layer
└────────┬────────┘
         │ JSON-RPC
         ▼
┌─────────────────┐
│   Rust Daemon   │
│  (Deterministic)│  ← State machine, persistence, policy
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ Filesystem/Git │
│ Patch/Test/Approve │
└─────────────────┘
```

### Components

| Component | Language | Purpose |
|-----------|----------|---------|
| MCP Gateway | TypeScript | MCP tool registration, validation, daemon calls |
| Deterministic Daemon | Rust | State machine, SQLite persistence, policy enforcement |
| Deterministic Core | Rust | Business logic, patch validation, test resolution |
| Deterministic Protocol | Rust | Shared types and method names |

## Prerequisites

### System Requirements

- **Rust**: 1.70+ (for daemon)
- **Node.js**: 18+ (for MCP gateway)
- **SQLite**: 3.x (built into daemon)
- **Operating System**: macOS, Linux, or Windows

### Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

### Install Node.js

```bash
# macOS
brew install node

# Linux
curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
sudo apt-get install -y nodejs
```

## Building from Source

### 1. Clone the Repository

```bash
git clone https://github.com/anschmieg/ChatCodex.git
cd ChatCodex
```

### 2. Build the Rust Daemon

```bash
cd codex-rs
cargo build --release
```

The daemon binary will be at:
```
codex-rs/target/release/deterministic-daemon
```

### 3. Build the MCP Gateway

```bash
cd ../apps/chatgpt-mcp
npm ci
npm run build
```

## Starting the System

### 1. Start the Daemon

The daemon is an HTTP JSON-RPC server:

```bash
cd codex-rs
./target/release/deterministic-daemon --port 3100 --data-dir ./runs
```

Options:
- `--port`: HTTP port (default: 3100)
- `--data-dir`: Directory for SQLite database (default: `./runs`)

### 2. Start the MCP Gateway

The MCP gateway connects to the daemon:

```bash
cd apps/chatgpt-mcp
node dist/index.js
```

The gateway reads daemon URL from environment:
```bash
DAEMON_URL=http://localhost:3100 node dist/index.js
```

## Connecting ChatGPT

### MCP Configuration

ChatGPT connects to the MCP server. Configure your MCP client (ChatGPT with MCP support) to use:

```json
{
  "mcpServers": {
    "chatcodex": {
      "command": "node",
      "args": ["/path/to/ChatCodex/apps/chatgpt-mcp/dist/index.js"],
      "env": {
        "DAEMON_URL": "http://localhost:3100"
      }
    }
  }
}
```

### Verify Connection

After configuration, ChatGPT should see these MCP tools available:

**Lifecycle:**
- `codex_prepare_run` — Start a new run
- `refresh_run_state` — Get current state
- `replan_run` — Update the plan
- `finalize_run` — Close with outcome
- `reopen_run` — Continue a finalized run
- `supersede_run` — Create successor run

**Inspection:**
- `get_run_state` — Full run details
- `get_run_history` — Audit trail
- `list_runs` — Query queue
- `get_run_queue_overview` — Aggregate counts

**Execution:**
- `get_workspace_summary` — Detect tooling
- `read_file` — Read file contents
- `search_code` — Find code
- `apply_patch` — Apply changes (policy-gated)
- `run_tests` — Execute tests (policy-gated)
- `show_diff` — See changes
- `git_status` — Working tree status

**Queue Management:**
- `set_run_priority` — Set priority level
- `assign_run_owner` — Assign ownership
- `set_run_due_date` — Set deadline
- `pin_run` / `unpin_run` — Pin/unpin
- `snooze_run` / `unsnooze_run` — Defer/revisit
- `archive_run` / `unarchive_run` — Organize
- `annotate_run` — Add labels/notes

**Policy:**
- `preview_patch_policy` — Will patch need approval?
- `preview_test_policy` — Will tests need approval?
- `approve_action` — Resolve pending approvals

**Views:**
- `create_queue_view` — Save a filter configuration
- `list_queue_views` — List saved views
- `get_queue_view` — Get view definition
- `update_queue_view` / `delete_queue_view`

## Common Setup Issues

### Daemon Won't Start

**Port in use:**
```bash
lsof -i :3100
kill -9 <PID>
```

**Data directory permissions:**
```bash
mkdir -p ./runs
chmod 755 ./runs
```

### MCP Tools Not Visible in ChatGPT

1. Verify daemon is running: `curl http://localhost:3100/healthz`
2. Verify gateway starts without errors
3. Check MCP configuration path is absolute
4. Restart ChatGPT session after configuration changes

### SQLite Errors

The daemon auto-migrates the database. If you see migration errors:
```bash
rm -rf ./runs/runs.db
./target/release/deterministic-daemon --port 3100 --data-dir ./runs
```

## Quick Start Checklist

- [ ] Rust installed (`rustc --version`)
- [ ] Node.js installed (`node --version`)
- [ ] Repository cloned
- [ ] Daemon builds (`cargo build --release`)
- [ ] Gateway builds (`npm ci && npm run build`)
- [ ] Daemon starts (`./target/release/deterministic-daemon`)
- [ ] Gateway starts (`node dist/index.js`)
- [ ] ChatGPT sees MCP tools

## Smoke Test Checklist

After setup, verify the system works by completing this checklist:

### Daemon Health

```bash
# 1. Check daemon is running
curl http://localhost:3100/healthz
# Expected: {"status":"ok"}
```

### MCP Connection

Ask ChatGPT to verify tools are available:

> List the available ChatCodex tools.

Expected: ChatGPT should list the MCP tools (codex_prepare_run, refresh_run_state, etc.)

### Run Creation

> Create a run with the goal "Add a comment to the README file" and a simple plan.

Expected:
- ChatGPT calls `codex_prepare_run`
- Response includes `runId` and `status: "prepared"`

### State Inspection

> What's the current state of my run?

Expected:
- ChatGPT calls `refresh_run_state` or `get_run_state`
- Response shows run details

### Queue Overview

> Show me my queue overview.

Expected:
- ChatGPT calls `get_run_queue_overview`
- Response shows counts (totalVisible, readyCount, etc.)

### Run a Test Operation (Read-Only)

> Get the workspace summary for this project.

Expected:
- ChatGPT calls `get_workspace_summary`
- Response shows detected tooling

### Mutation Test (Safe)

> Read the README file and add a single-line comment at the top.

Expected:
- ChatGPT calls `read_file`
- ChatGPT calls `apply_patch` (may require approval depending on policy)
- Patch is applied

### Finalize Run

> Finalize this run as completed.

Expected:
- ChatGPT calls `finalize_run`
- Response shows `status: "finalized:completed"`

---

If all steps pass, the system is working correctly.

## Smoke Test Checklist

After setup, validate the system works end-to-end:

### Daemon Health

```bash
# Start daemon
./target/release/deterministic-daemon --port 3100 --data-dir ./runs &

# Verify health endpoint
curl http://localhost:3100/healthz
# Expected: {"status": "ok"}
```

### MCP Tool Registration

In your MCP client (ChatGPT), verify tools are visible:

```
Ask ChatGPT: "List the available MCP tools."
```

Expected: Should list 45+ tools including:
- `codex_prepare_run`
- `refresh_run_state`
- `get_run_state`
- `list_runs`
- `apply_patch`
- `run_tests`

### Run Creation Test

Create a test run:

```
Ask ChatGPT: "Create a run with goal 'Test setup' and plan ['Verify setup']."
```

Expected:
- Returns a `runId`
- Status is `prepared` or `active`

### Read-Only Test

Test inspection tools:

```
Ask ChatGPT: "Get the workspace summary."
```

Expected:
- Returns workspace information
- Detected tooling

### Mutation Test (Optional)

Test a safe mutation:

```
Ask ChatGPT: "Create a run to add a comment to README.md, then apply the patch."
```

Expected:
- Patch is applied (may require approval)
- Diff is visible
- Tests pass (if run)

### Queue Test

Test queue management:

```
Ask ChatGPT: "Show me the queue overview."
```

Expected:
- Returns aggregate counts
- Lists active runs

### Full Smoke Test Checklist

- [ ] Daemon starts without errors
- [ ] Health endpoint responds
- [ ] MCP gateway starts without errors
- [ ] ChatGPT can see MCP tools
- [ ] `get_workspace_summary` returns data
- [ ] `codex_prepare_run` creates a run
- [ ] `refresh_run_state` returns run state
- [ ] `list_runs` returns run list
- [ ] `get_run_queue_overview` returns counts
- [ ] (Optional) `apply_patch` applies changes
- [ ] (Optional) `finalize_run` closes a run

## Next Steps

- **First run workflow**: See [FIRST_RUN_WORKFLOW.md](./FIRST_RUN_WORKFLOW.md)
- **Example prompts**: See [EXAMPLE_PROMPTS.md](./EXAMPLE_PROMPTS.md)
- **API reference**: See [MCP_TOOL_CONTRACTS.md](./MCP_TOOL_CONTRACTS.md)
- **Architecture details**: See [ARCHITECTURE.md](./ARCHITECTURE.md)