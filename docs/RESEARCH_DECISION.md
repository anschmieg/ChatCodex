# Research decision

## Chosen base

Fork upstream `openai/codex`.

## Why

We want to preserve deterministic harness behavior where possible, while removing all backend-owned inference.

## Why not use the existing Codex MCP server

Because that exposes Codex as a tool for another agent, which is exactly the architecture we must avoid.

## Why not start from OpenCode / Goose / wrappers

Those are either:

* a different harness philosophy,
* too autonomous,
* too far from Codex semantics,
* or the wrong abstraction boundary for a deterministic ChatGPT control plane.

## Fallback

Only if extracting deterministic subsystems from upstream Codex proves substantially harder than expected should we evaluate `ymichael/open-codex` as an implementation fallback.
