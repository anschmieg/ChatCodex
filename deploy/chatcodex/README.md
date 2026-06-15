# ChatCodex Deployment

This directory contains the Docker image and Compose configuration for the
ChatCodex deterministic coding harness MCP server.

## Image

`deploy/chatcodex/Dockerfile` builds a multi-stage image:

- Builder stage compiles the Rust MCP server and OAuth crate.
- Runtime stage is a minimal Ubuntu 24.04 image with the harness binary and
  required tooling.

Pre-installed runtime tools include:

- `bubblewrap` — required by the Codex read-only command sandbox
- `git` and `git-lfs` — repository operations
- `gh` — GitHub CLI (outbound git operations are still blocked by policy)
- `mise` and `uv` — language toolchain and Python environment management
- `ripgrep`, `fd-find`, `jq`, `openssh-client`, and common build tools

## Compose

`deploy/chatcodex/compose.yaml` runs the harness with a hardened container
profile:

- non-root user (`10001:10001`)
- read-only root filesystem
- dropped capabilities and `no-new-privileges`
- resource limits

Required environment variables:

| Variable | Purpose |
| --- | --- |
| `CHATCODEX_WORKSPACE_BASE` | Host directory bind-mounted at `/workspaces`. Must be writable by UID `10001`. |
| `CHATCODEX_IMAGE` | Docker image to run (default: `ghcr.io/anschmieg/chatcodex:latest`). |

Optional overrides:

| Variable | Default |
| --- | --- |
| `CHATCODEX_PIDS_LIMIT` | `512` |
| `CHATCODEX_MEMORY_LIMIT` | `4g` |
| `CHATCODEX_CPU_LIMIT` | `2` |

Start the service:

```bash
export CHATCODEX_WORKSPACE_BASE=/var/chatcodex/workspaces
docker compose up -d
```

## Workspace Layout

Each MCP client gets its own isolated directory under
`/workspaces/clients/<client_id>/`:

```text
/workspaces/clients/<client_id>/
  repos/<repo-name>/       # cloned git repositories
  sandboxes/<sandbox-name>/ # persistent scratch directories with git init
```

The client ID is resolved from the `CHATCODEX_CLIENT_ID` environment variable,
falling back to `"default"`.

Before any workspace tool can be used, the client must call:

```json
{ "name": "setup_workspace", "arguments": { "source": "<git-url>" } }
```

or:

```json
{ "name": "setup_workspace", "arguments": { "source": "sandbox" } }
```

Sandbox workspaces are automatically initialized as empty git repositories so
local commits and diffs work.

## Git Tool Policy

The generic `git` MCP tool accepts any local-only git subcommand. Outbound
network operations are rejected. In addition to the generic `git` tool, there
are convenience wrappers for common local writes:

- `git push`
- `git fetch` / `git pull`
- `git clone`
- `git ls-remote`
- `git remote add` / `git remote set-url`
- `git submodule update --init`
- `gh repo create`

Use `git_status` and `git_diff` for structured read-only repository state,
and `git_commit`, `git_branch`, and `git_checkout` for local-only writes.

Read-only git tools run inside the exec-server read-only filesystem sandbox.
Writable git tools run unsandboxed because the upstream workspace-write
sandbox protects `.git/` metadata under a writable workspace root; outbound
subcommands are still blocked and network access is declared as restricted.

## Credential Mounts for Private Repositories

To let `setup_workspace` clone private repositories, uncomment and adjust the
optional bind mounts in `compose.yaml`:

- Mount `~/.ssh` for SSH-key based cloning.
- Mount a git credentials file or netrc for HTTPS cloning.

The container runs as UID `10001` with `HOME=/toolchains/home/chatcodex`, so
mounted credentials must be readable by that user.

## Health Check

The container exposes port `3000` and provides a `/healthz` endpoint for
Docker health checks.
