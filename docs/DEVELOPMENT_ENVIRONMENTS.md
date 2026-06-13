# Development Environments

ChatCodex separates its immutable control plane from project-specific
development environments.

## Responsibilities

- The ChatCodex image provides the deterministic harness, basic shell tools,
  Git, ripgrep, a baseline Python 3 interpreter, `mise`, and `uv`.
- `mise` provisions and activates project runtime toolchains such as Python,
  Node.js, Go, Rust, Java, and Terraform.
- `uv` manages Python dependencies, virtual environments, lockfiles, and
  Python command execution.
- Project manifests and lockfiles declare the environment. ChatCodex must not
  make global package-manager state part of a project's hidden prerequisites.

## Storage

The runtime uses three writable roots:

| Path | Purpose |
| --- | --- |
| `/workspaces` | Project source trees |
| `/data` | ChatCodex state |
| `/toolchains` | Persistent mise and uv downloads/caches |

Production should mount all three paths persistently. The image configures:

```text
MISE_DATA_DIR=/toolchains/mise/data
MISE_CACHE_DIR=/toolchains/mise/cache
MISE_STATE_DIR=/toolchains/mise/state
UV_CACHE_DIR=/toolchains/uv/cache
UV_PYTHON_INSTALL_DIR=/toolchains/uv/python
UV_TOOL_DIR=/toolchains/uv/tools
UV_PYTHON_DOWNLOADS=never
```

`UV_PYTHON_DOWNLOADS=never` keeps Python runtime ownership with `mise`.
The mounted directories must be writable by the container user, UID `10001`.

## Detection Order

A future deterministic environment-inspection operation will check, in order:

1. `mise.toml` and `.mise.toml`
2. `mise.lock`
3. `pyproject.toml`
4. `uv.lock`
5. `.python-version` and `.python-versions`
6. `flake.nix` and `flake.lock`
7. `.devcontainer/devcontainer.json` and `devcontainer.json`

Detection is read-only and produces a structured environment plan. It does not
install tools, trust configuration, or execute lifecycle scripts.

## Execution Policy

Already-prepared environments are activated with:

```bash
mise exec -- <command>
```

Python projects use `uv` inside the mise environment:

```bash
mise exec -- uv sync --locked
mise exec -- uv run python path/to/script.py
mise exec -- uv run pytest
```

Generic command execution must not perform environment mutation. In
particular, it must not run:

- `apt`, `sudo`, Homebrew, or another system package manager;
- `pip install` into the system interpreter;
- global npm installations;
- `mise use`, `mise install`, or `mise trust`;
- unlocked dependency synchronization.

If required tooling is absent, command execution reports that deterministic
environment preparation is required.

## Preparation Policy

Environment preparation will be a separate deterministic operation. It will:

1. Hash the relevant manifests and lockfiles.
2. Produce the exact proposed tool and dependency actions.
3. Evaluate server-side policy.
4. Request explicit approval for trust, network downloads, unlocked inputs, or
   lifecycle scripts.
5. Install only after approval.
6. Record versions, sources, checksums, manifest hashes, and audit events.

Locked and previously verified environments may be reused without repeated
approval. No preparation operation may start a model or hidden agent loop.

## Future Backends

- Nix flakes may provide native system dependencies for projects that need
  more than language runtimes.
- Devcontainers require a separate restricted runner service.
- The ChatCodex MCP container must never receive the host Docker socket.
