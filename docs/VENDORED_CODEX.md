# Vendored Codex Policy

## Purpose

`vendor/codex/` contains the upstream Codex source snapshot used as reference
and as a source of selected reusable implementation dependencies.

ChatCodex is not developed inside this tree. First-party ChatCodex code lives in:

- `crates/`
- `apps/chatgpt-mcp/`
- `deploy/chatcodex/`
- `docs/`

## Source

- Upstream project: `https://github.com/openai/codex`
- Snapshot location: `vendor/codex/`
- Initial snapshot commit: `efb23e4ab940045be86393309f2cecbcd0155036`
- Snapshot date: 2026-04-26
- The vendor directory is tracked as a git subtree. Inside `vendor/codex/` there is a
  full git history mirroring upstream; `git fetch upstream` pulls the latest from upstream.

## Rules

- Do not add ChatChatGPT product code to `vendor/codex/`.
- Do not patch vendored Codex files to satisfy ChatCodex behavior.
- Do not put deterministic harness policy, MCP contracts, deployment logic, or
  hybrid-worker behavior in the vendored tree.
- Do not depend from first-party crates on Codex TUI, CLI, app-server, login,
  ChatGPT auth, model-provider, or autonomous turn-execution crates.

## How to Borrow Codex Behavior

Prefer this order:

1. Use a vendored Codex crate through a narrow first-party adapter.
2. If the upstream API is unstable or too broad, write a first-party wrapper that
   exposes only the deterministic behavior ChatCodex needs.
3. If the upstream implementation must diverge, copy the minimal code into
   `crates/` with attribution and treat it as first-party from then on.

Never create a patch queue for `vendor/codex/`.

## Update Procedure

### Fetch latest from upstream

```bash
# Inside vendor/codex/
git fetch upstream
# Note the upstream commit hash, e.g. abc1234
```

### Rebase vendor onto the new upstream commit

```bash
# Inside vendor/codex/
git checkout main
git rebase upstream/main
# Or: git merge upstream/main (if you prefer merge commits)
git push origin main
```

### Update parent repo

Record the new upstream commit in `docs/VENDORED_CODEX.md` in the same change
that updates the parent repo.

### Validate

```bash
cargo metadata --no-deps
cargo test -p deterministic-protocol -p deterministic-core -p deterministic-daemon
cd apps/chatgpt-mcp
npm run build
npm run typecheck
npm test
```

If ChatCodex breaks because an upstream API changed, fix the first-party
adapter or first-party copied implementation, not the vendored source.
