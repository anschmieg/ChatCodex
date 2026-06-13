# ChatCodex Implementation Plan

## Completed Architecture

- Track the stable upstream Codex Rust release.
- Keep ChatCodex harness and OAuth code in independent crates.
- Use public upstream execution, sandbox, protocol, filesystem, and patch APIs.
- Expose a strict five-tool deterministic MCP allowlist.
- Keep commands read-only and route all workspace writes through `apply_patch`.
- Avoid all upstream session, turn, review, model, and agent-loop APIs.
- Avoid source patches to `codex-core` and other upstream implementation crates.

## Verification Gates

1. `cargo check -p codex-native-harness-mcp`
2. `cargo test -p codex-native-harness-mcp --lib`
3. `cargo test -p codex-native-harness-mcp-auth`
4. `cargo fmt --all -- --check`
5. Confirm the diff from the selected upstream tag contains only:
   - the two ChatCodex crates;
   - workspace registration and lockfile changes;
   - ChatCodex deployment, workflow, and documentation files.

## Deployment

The Docker image builds with the Rust toolchain pinned by upstream. It includes
Git, GitHub CLI, SSH, common development utilities, `mise`, and `uv`. Runtime
state is stored beneath `/toolchains`; projects are mounted beneath
`/workspaces`. The host Docker socket and host `/data` are not mounted.

Coolify consumes the prebuilt GHCR image. Container restrictions remain defined
by the image and deployment configuration: non-root user, read-only root
filesystem, dropped capabilities, `no-new-privileges`, bounded resources, and
only the required persistent mounts.

## Follow-Up

- Add explicit approval-backed environment preparation tools before permitting
  package or toolchain installation.
- Add a dedicated `run_tests` tool if tests need controlled writable build
  directories; do not weaken `exec_command` into a general write-capable shell.
- Re-evaluate upstream public APIs on each release and retain the no-core-patch
  boundary unless a documented, unavoidable capability gap appears.
