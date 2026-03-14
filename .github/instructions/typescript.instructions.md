---
applyTo: "apps/chatgpt-mcp/**"
---

# TypeScript-specific instructions

## Goal

Implement a thin MCP gateway for ChatGPT.

## Responsibilities

TypeScript should only:
- define MCP tools
- validate inputs with Zod
- call the Rust daemon over internal JSON-RPC
- map daemon DTOs to MCP responses

TypeScript should not:
- contain core deterministic planning logic
- inspect or mutate the repository directly
- execute shell commands directly
- duplicate Rust business rules

## Public MCP tools for the first slice

- `codex_prepare_run`
- `get_workspace_summary`
- `read_file`
- `git_status`
- `search_code`
- `apply_patch`
- `run_tests`
- `show_diff`

## Response shaping

Use:
- concise `structuredContent` for model-visible state
- optional text content for short human-readable summaries
- `_meta` only for large data that should stay out of the model context

## Important

Do not expose any coarse “continue the agent” tool.
