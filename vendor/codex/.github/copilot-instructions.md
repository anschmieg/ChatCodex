You are working in a fork of `openai/codex`, but this repository must **not** preserve Codex's model-owning runtime.

Your job is to build a **deterministic control plane for ChatGPT**.

Non-negotiable requirements:

- The only LLM in the stack is ChatGPT.
- Do not add or call any model provider SDK or API.
- Do not expose or use agent-runtime endpoints such as `turn/start`, `turn/steer`, `review/start`, `codex()`, or `codex-reply()`.
- Do not build any coarse tool that continues work autonomously.
- The public surface must be fine-grained MCP tools plus a few deterministic control tools.
- Deterministic logic belongs in Rust.
- The TypeScript MCP server must be thin and should only validate inputs, call the Rust daemon, and format outputs.

Read these files first before changing code:
1. `AGENTS.md`
2. `docs/ARCHITECTURE.md`
3. `docs/IMPLEMENTATION_PLAN.md`
4. `docs/MCP_TOOL_CONTRACTS.md`
5. `docs/INTERNAL_RPC.md`
6. `docs/NO_HIDDEN_AGENT_INVARIANTS.md`

For the first implementation task, limit scope to:
- deterministic Rust daemon
- TypeScript MCP gateway
- the minimal end-to-end tool loop

Do not broaden scope.
