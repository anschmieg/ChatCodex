# MVP Release Checklist

This checklist verifies ChatCodex is ready for MVP evaluation.

---

## Build & Test Status

### Rust Daemon

```bash
cd codex-rs

# Build all packages
cargo build -p deterministic-protocol -p deterministic-core -p deterministic-daemon

# Run tests
cargo test -p deterministic-protocol -p deterministic-core -p deterministic-daemon

# Run clippy
cargo clippy -p deterministic-protocol -p deterministic-core -p deterministic-daemon --all-targets -- -D warnings
```

- [ ] `cargo build` succeeds without errors
- [ ] `cargo test` passes all tests (203+ daemon tests, 347+ core tests)
- [ ] `cargo clippy` passes with no warnings

### TypeScript Gateway

```bash
cd apps/chatgpt-mcp

# Install dependencies
npm ci

# Build
npm run build

# Test
npm test
```

- [ ] `npm ci` succeeds
- [ ] `npm run build` succeeds
- [ ] `npm test` passes

---

## Documentation Complete

- [ ] **Quick start**: [MVP_README.md](./MVP_README.md) exists and is clear
- [ ] **Onboarding**: [ONBOARDING.md](./ONBOARDING.md) covers setup
- [ ] **Workflow**: [FIRST_RUN_WORKFLOW.md](./FIRST_RUN_WORKFLOW.md) explains usage
- [ ] **Examples**: [EXAMPLE_PROMPTS.md](./EXAMPLE_PROMPTS.md) provides prompts
- [ ] **Tools**: [TOOLS_OVERVIEW.md](./TOOLS_OVERVIEW.md) lists all tools
- [ ] **Validation**: [VALIDATION_PLAN.md](./VALIDATION_PLAN.md) defines tests
- [ ] **Walkthrough**: [MANUAL_VALIDATION_WALKTHROUGH.md](./MANUAL_VALIDATION_WALKTHROUGH.md) exists
- [ ] **Operator**: [OPERATOR_GUIDE.md](./OPERATOR_GUIDE.md) covers production use
- [ ] **Intervention**: [INTERVENTION_PATTERNS.md](./INTERVENTION_PATTERNS.md) has recovery playbooks

---

## Onboarding Path Present

- [ ] Prerequisites (Rust, Node.js) documented
- [ ] Build steps work as documented
- [ ] Start daemon command works
- [ ] Start gateway command works
- [ ] MCP configuration example provided
- [ ] Smoke test verifies end-to-end

See [ONBOARDING.md](./ONBOARDING.md) for full setup guide.

---

## Validation Path Present

- [ ] Validation plan defines core workflows (V1-V7)
- [ ] Manual walkthrough exists
- [ ] Happy-path test case documented
- [ ] Approval-gated test case documented
- [ ] Recovery test case documented

See [VALIDATION_PLAN.md](./VALIDATION_PLAN.md) and [MANUAL_VALIDATION_WALKTHROUGH.md](./MANUAL_VALIDATION_WALKTHROUGH.md).

---

## Operator Guidance Present

- [ ] Daily operations documented
- [ ] Intervention decision tree exists
- [ ] Queue shaping tools explained
- [ ] Lifecycle actions documented
- [ ] Approval handling documented
- [ ] Error messages include recovery hints

See [OPERATOR_GUIDE.md](./OPERATOR_GUIDE.md) and [INTERVENTION_PATTERNS.md](./INTERVENTION_PATTERNS.md).

---

## Architecture Constraints Satisfied

### Hard Constraint (Must Pass)

> **The only LLM in the stack is ChatGPT.**

Verification:

```bash
# Search for any backend LLM calls
grep -rn "openai\." --include="*.rs" codex-rs/deterministic-daemon/src codex-rs/deterministic-core/src
grep -rn "anthropic\." --include="*.rs" codex-rs/deterministic-daemon/src codex-rs/deterministic-core/src
grep -rn "continue_run\|resume_thread\|resume_codex_thread\|agent_step" --include="*.ts" apps/chatgpt-mcp/src
grep -rn "continue_run\|resume_thread\|resume_codex_thread\|agent_step" --include="*.rs" codex-rs/deterministic-daemon/src
```

- [ ] No backend model calls in daemon
- [ ] No backend model calls in MCP gateway
- [ ] No autonomous continuation tools exposed

### Codebase Invariants

```bash
# Verify no autonomy keywords exist in the codebase
git grep -nE 'continue_run|resume_thread|resume_codex_thread|agent_step|fix_end_to_end|turn/start|turn/steer|review/start' -- \
  apps/chatgpt-mcp/src \
  codex-rs/deterministic-daemon/src \
  .github/workflows
```

- [ ] No autonomous agent patterns in code

---

## Known Limitations Recorded

- [ ] **Single workspace**: Each daemon manages one project
- [ ] **No concurrent runs**: Only one active run at a time
- [ ] **Manual intervention**: Operators must approve certain actions
- **No web UI**: All interaction through ChatGPT MCP client
- **SQLite only**: No other database backends

See [MVP_README.md](./MVP_README.md) for full limitations list.

---

## Pre-Release Verification Commands

Run these before releasing:

```bash
# 1. Full build
cd codex-rs && cargo build --release

# 2. Full test
cargo test -p deterministic-protocol -p deterministic-core -p deterministic-daemon

# 3. Clippy
cargo clippy -p deterministic-protocol -p deterministic-core -p deterministic-daemon --all-targets -- -D warnings

# 4. TypeScript build
cd ../apps/chatgpt-mcp && npm ci && npm run build

# 5. Invariant grep
cd ../.. && git grep -nE 'continue_run|resume_thread|resume_codex_thread|agent_step|fix_end_to_end|turn/start|turn/steer|review/start' -- \
  apps/chatgpt-mcp/src \
  codex-rs/deterministic-daemon/src \
  .github/workflows
```

All commands must pass with no errors.

---

## Release Sign-Off

Before declaring MVP ready:

- [ ] All build commands pass
- [ ] All tests pass
- [ ] All documentation reviewed and cross-linked
- [ ] Architecture constraints verified
- [ ] Known limitations documented
- [ ] Quick start tested by someone new to the project

---

**Document**: [MVP_README.md](./MVP_README.md) is the entry point for evaluators.