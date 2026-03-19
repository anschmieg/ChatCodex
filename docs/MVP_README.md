# ChatCodex MVP — Quick Start

> **The only LLM in the stack is ChatGPT.** No backend model calls. No hidden agent loops. The backend is purely deterministic.

---

## What Is ChatCodex?

ChatCodex is a **deterministic coding harness control plane** that lets ChatGPT operate on a codebase with structured state, policy gates, and audit trails.

Think of it as a state machine for coding tasks:
- **You** tell ChatGPT what to do
- **ChatGPT** uses MCP tools to create runs, apply patches, run tests
- **ChatCodex** tracks state, enforces policies, persists history

### Who Is This For?

| User | Use Case |
|------|----------|
| Developers | Use ChatGPT with structured task management instead of free-form chat |
| Teams | Audit trail of AI-assisted changes |
| Operators | Queue management, priority control, intervention when needed |

### What This Is NOT

- **Not an autonomous agent** — ChatGPT must approve every action
- **Not a code review tool** — Runs are for execution, not human review
- **Not Codex CLI** — Different project (this runs on OpenAI's Codex CLI)

---

## MVP Scope

### ✅ What's Included

| Feature | Description |
|---------|-------------|
| Run Lifecycle | Create → Execute → Finalize runs with full state tracking |
| Inspection Tools | Read files, search code, get workspace summary |
| Patch Application | Apply code changes with policy-gated approvals |
| Test Execution | Run tests with approval gates |
| Queue Management | List, filter, prioritize, assign ownership |
| Queue Views | Save and recall filtered queue views |
| Intervention | Reopen, supersede, archive, snooze runs |
| Policy Controls | Edit thresholds, path restrictions, approval requirements |
| Audit Trail | Full history of state changes per run |

### ❌ What's NOT Included (MVP)

- Multi-run parallel execution
- Web UI or dashboard
- Team/permission system
- Scheduled/automated runs
- External integrations (GitHub, Jira)
- Run templates

---

## Fastest Path to First Use

### Prerequisites

```bash
# 1. Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# 2. Install Node.js 18+
brew install node  # macOS

# 3. Clone the repo
git clone https://github.com/anschmieg/ChatCodex.git
cd ChatCodex
```

### Build

```bash
# Build Rust daemon
cd codex-rs
cargo build --release

# Build MCP gateway
cd ../apps/chatgpt-mcp
npm ci
npm run build
```

### Run

```bash
# Terminal 1: Start daemon
cd codex-rs
./target/release/deterministic-daemon --port 3100 --data-dir ./runs

# Terminal 2: Start gateway
cd apps/chatgpt-mcp
DAEMON_URL=http://localhost:3100 node dist/index.js
```

### Connect ChatGPT

Add to your ChatGPT MCP configuration:

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

### Verify It Works

Ask ChatGPT:

> Create a run with goal "Add a hello world function to main.rs" and plan ["Add function"].

Expected: ChatGPT creates a run and shows the `runId`.

---

## Documentation Map

| Document | When to Read |
|----------|--------------|
| [ONBOARDING.md](./ONBOARDING.md) | First-time setup |
| [FIRST_RUN_WORKFLOW.md](./FIRST_RUN_WORKFLOW.md) | Understanding the workflow |
| [EXAMPLE_PROMPTS.md](./EXAMPLE_PROMPTS.md) | What to ask ChatGPT |
| [TOOLS_OVERVIEW.md](./TOOLS_OVERVIEW.md) | All available tools |
| [VALIDATION_PLAN.md](./VALIDATION_PLAN.md) | How to verify it works |
| [MANUAL_VALIDATION_WALKTHROUGH.md](./MANUAL_VALIDATION_WALKTHROUGH.md) | Step-by-step validation |
| [OPERATOR_GUIDE.md](./OPERATOR_GUIDE.md) | Running in production |
| [INTERVENTION_PATTERNS.md](./INTERVENTION_PATTERNS.md) | Recovery playbooks |

---

## Architecture

```
┌─────────────────┐
│  ChatGPT (LLM) │
│   MCP Client    │
└────────┬────────┘
         │ MCP protocol
         ▼
┌─────────────────┐
│ TypeScript MCP │
│    Gateway      │
└────────┬────────┘
         │ JSON-RPC
         ▼
┌─────────────────┐
│   Rust Daemon  │
│  (Deterministic)│
└────────┬────────┘
         ▼
┌─────────────────┐
│   Filesystem   │
│ Git / Patch / Test │
└─────────────────┘
```

**The only LLM is ChatGPT.** All other components are deterministic.

---

## Known Limitations

1. **Single workspace** — Each daemon instance manages one project/workspace
2. **No concurrent runs** — Only one active run at a time per daemon
3. **Manual intervention** — Operators must approve certain actions
4. **No web UI** — All interaction through ChatGPT MCP client
5. **SQLite only** — No other database backends

---

## Next Steps

1. **Set up the system** — Follow the Fastest Path above
2. **Validate it works** — See [VALIDATION_PLAN.md](./VALIDATION_PLAN.md)
3. **Try a workflow** — See [FIRST_RUN_WORKFLOW.md](./FIRST_RUN_WORKFLOW.md)
4. **Learn operator tasks** — See [OPERATOR_GUIDE.md](./OPERATOR_GUIDE.md)

---

**Questions?** Open an issue at https://github.com/anschmieg/ChatCodex/issues