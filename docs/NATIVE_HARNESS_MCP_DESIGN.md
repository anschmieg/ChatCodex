# Native Harness MCP Design

## Purpose

ChatCodex exposes the official Codex coding harness to ChatGPT. ChatGPT supplies
the only model and owns the agentic loop. The server supplies the same
model-facing tools and deterministic execution behavior that Codex supplies to
its own model.

## Architecture

The implementation is a Rust MCP server linked directly to the vendored
`codex-core` crate. The server obtains tool names, descriptions, and JSON
schemas from Codex `ToolSpec` values and dispatches calls through Codex tool
handlers.

No backend component calls a model, submits a user prompt to a Codex thread, or
continues an agent turn. Existing `codex` and `codex-reply` MCP tools are not
used because they start and continue Codex-owned model loops.

## Harness Facade

A small `codex-core` facade provides two operations:

1. Build a configured deterministic tool catalog and registry.
2. Execute one explicit tool call in a harness context.

The facade reuses native configuration, sessions, turn context, sandbox policy,
approval policy, process management, patch application, command parsing, and
output formatting. It must not duplicate these mechanisms.

Tools that control sub-agents are omitted. Ordinary harness tools remain
configuration-driven so upstream Codex changes naturally propagate to the MCP
surface.

## MCP Mapping

Function tools map directly to MCP tools. Their input schemas are serialized
from Codex `JsonSchema`. Freeform tools such as `apply_patch` retain freeform
input semantics through an MCP wrapper field only where the protocol requires
an object; the wrapper is generated and tested centrally rather than copied
into handwritten TypeScript.

`tools/list` reflects the active Codex catalog. `tools/call` creates exactly one
native tool invocation and returns its native output. Long-running commands use
the native `exec_command`/`write_stdin` session mechanism.

## Approvals

Codex command and patch approvals suspend the exact operation. The MCP server
converts the request into `elicitation/create`, then feeds the client decision
back into that suspended operation. Disconnects, malformed responses, and
clients without elicitation support result in denial.

There is no separate approval database or retry token.

## Workspace and Sandbox

The server accepts projects only beneath one canonical workspace root,
`/workspaces` by default. The Docker deployment mounts the host project
directory there.

Codex runs with `workspace-write` sandbox policy inside the container. Docker
adds a second boundary:

- non-root runtime user;
- dropped Linux capabilities;
- `no-new-privileges`;
- read-only container filesystem;
- explicit writable `/workspaces` and `/data` mounts;
- bounded CPU, memory, and process count;
- no Docker socket.

The project workspace is trusted to run its own build and test code. The
container boundary protects the VPS host and unrelated host files.

## Git

Git is installed in the runtime image and invoked through `exec_command`, just
as it is in Codex. Codex command parsing, safe-command classification,
approvals, and sandbox policy apply unchanged. Existing convenience behavior
inside Codex may invoke Git internally.

SSH credentials may be mounted read-only. HTTPS credentials may be supplied
through deployment secrets. Credentials are never persisted in project state.

## Transport and Authentication

The server supports:

- stdio for local development and protocol tests;
- Streamable HTTP for ChatGPT and Coolify;
- `/healthz` for deployment probes;
- the OAuth 2.1 / MCP 2025-11-25 layer implemented in
  `codex-rs/native-harness-mcp-auth`. The same origin mints and validates
  access tokens, exposes discovery + JWKS, and serves the streamable MCP
  transport behind a JWT bearer middleware. Cloudflare Access is the
  upstream IdP; the consent step reads the team's `CF_Authorization` cookie
  and verifies it against `<team>/cdn-cgi/access/certs`.

Multi-user tenancy remains out of scope. The current release is a single
operator deployment authenticated by the Cloudflare Access team.

## Migration

The custom deterministic daemon and TypeScript MCP gateway remain only until
native parity tests pass. They are then deleted from this branch. Their
run-queue and lifecycle features are not migrated because they are unrelated to
the Codex harness goal.

## Success Criteria

- MCP tool schemas match the native Codex configured schemas.
- Git, shell, patch, process sessions, sandboxing, and approvals use Codex code.
- Agent-owned tools and model calls are absent.
- A Docker-hosted ChatGPT connection can inspect, edit, test, and review a
  mounted repository.
