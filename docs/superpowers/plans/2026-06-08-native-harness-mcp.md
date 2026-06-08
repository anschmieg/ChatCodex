# Native Harness MCP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose Codex's native model-facing harness tools to ChatGPT through an authenticated remote MCP server without invoking any backend model or agent loop.

**Architecture:** A narrow public facade in `codex-core` constructs and dispatches the existing native tool registry. A new Rust MCP crate maps those specs and calls to MCP, bridges approvals through elicitation, confines workspaces, and serves stdio plus Streamable HTTP. Docker is the cross-platform host-isolation boundary.

**Tech Stack:** Rust 1.93, `codex-core`, `codex-protocol`, `rmcp`, Tokio, Axum/HTTP, Docker Compose, Ubuntu 24.04.

---

## File Map

- `codex-rs/core/src/harness_mcp.rs`: native catalog/session/call facade.
- `codex-rs/core/src/lib.rs`: export the facade.
- `codex-rs/core/src/tools/spec.rs`: only visibility or filtering hooks required by the facade.
- `codex-rs/core/src/tools/registry.rs`: only visibility required for direct dispatch.
- `codex-rs/native-harness-mcp/src/catalog.rs`: Codex `ToolSpec` to MCP mapping.
- `codex-rs/native-harness-mcp/src/server.rs`: MCP handler and call routing.
- `codex-rs/native-harness-mcp/src/approval.rs`: MCP elicitation bridge.
- `codex-rs/native-harness-mcp/src/config.rs`: workspace root, transport, and bearer configuration.
- `codex-rs/native-harness-mcp/src/http.rs`: Streamable HTTP and health endpoint.
- `codex-rs/native-harness-mcp/src/main.rs`: process bootstrap.
- `Dockerfile`, `docker-compose.yml`, `.dockerignore`: deployment boundary.
- `docs/IMPLEMENTATION_PLAN.md`: live status and takeover record.

## Task 1: Native Catalog Facade

- [ ] Add a `harness_catalog_excludes_agent_tools` test in `harness_mcp.rs`.
- [ ] Run `rustup run 1.93.0 cargo test -p codex-core harness_catalog_excludes_agent_tools` and confirm it fails because the facade is absent.
- [ ] Implement `HarnessToolCatalog::from_config` by reusing `ToolsConfig` and `build_specs_with_discoverable_tools`.
- [ ] Filter only agent-control tools while preserving native schemas.
- [ ] Run the focused test and confirm it passes.
- [ ] Update `docs/IMPLEMENTATION_PLAN.md` M1 status.
- [ ] Commit the catalog facade.

## Task 2: Native Direct Dispatch

- [ ] Add a failing test that executes `exec_command` with `printf native`.
- [ ] Add a failing test that applies a patch using native `apply_patch`.
- [ ] Expose the minimum existing session/turn construction needed by a `HarnessSession`.
- [ ] Dispatch through `ToolRouter`/`ToolRegistry`, not through a new executor.
- [ ] Verify both tests pass and no model client is constructed.
- [ ] Update M1 status and commit.

## Task 3: MCP Catalog Server

- [ ] Create `native-harness-mcp` crate and add it to the workspace.
- [ ] Add a failing `tools/list` parity test comparing MCP tools to the native catalog.
- [ ] Implement lossless `ToolSpec` to MCP mapping.
- [ ] Add a failing `tools/call` test for `exec_command`.
- [ ] Implement direct call dispatch and native output conversion.
- [ ] Verify catalog and call tests.
- [ ] Update M2 status and commit.

## Task 4: Process Sessions and Freeform Patch

- [ ] Add an MCP test for a yielded `exec_command` followed by `write_stdin`.
- [ ] Add an MCP test for freeform `apply_patch`.
- [ ] Preserve Codex session identifiers and freeform payload kinds in dispatch.
- [ ] Verify both workflows pass.
- [ ] Update M2 status and commit.

## Task 5: Approval Elicitation

- [ ] Add a failing command-approval test with a client elicitation responder.
- [ ] Add a failing patch-approval test.
- [ ] Adapt the existing `codex-rs/mcp-server` approval payload and response handling.
- [ ] Deny on disconnect, malformed result, or unsupported elicitation.
- [ ] Verify approve and deny paths.
- [ ] Update M3 status and commit.

## Task 6: Workspace Confinement

- [ ] Add traversal and symlink escape tests.
- [ ] Implement canonical workspace-root configuration.
- [ ] Reject projects outside the root before creating a harness session.
- [ ] Verify native commands and patches remain within the selected project.
- [ ] Update M4 status and commit.

## Task 7: HTTP, Authentication, and Health

- [ ] Add unauthorized/authorized Streamable HTTP tests.
- [ ] Add `/healthz` test.
- [ ] Implement stateless/single-user Streamable HTTP transport.
- [ ] Validate bearer authentication without logging token values.
- [ ] Add stdio bootstrap.
- [ ] Update M2 status and commit.

## Task 8: Docker and Coolify

- [ ] Add a multi-stage Ubuntu 24.04 Dockerfile.
- [ ] Add Compose security settings and mounts.
- [ ] Build the image.
- [ ] Verify non-root identity, Git availability, health, and mounted workspace access.
- [ ] Document Coolify setup and credentials.
- [ ] Update M5 status and commit.

## Task 9: Remove Deprecated Stack

- [ ] Delete deterministic crates and TypeScript gateway.
- [ ] Remove manifest and workspace references.
- [ ] Rewrite architecture, onboarding, contracts, and status docs.
- [ ] Run invariant scans.
- [ ] Update M6 status and commit.

## Task 10: Final Validation

- [ ] Run formatting, focused tests, and Clippy with Rust 1.93.
- [ ] Run MCP end-to-end tests.
- [ ] Build and smoke-test Docker Compose.
- [ ] Confirm native schema parity and forbidden-tool absence.
- [ ] Record exact evidence and remaining limitations in `docs/IMPLEMENTATION_PLAN.md`.
- [ ] Commit final validation documentation.
