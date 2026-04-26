# MCP Setup Guide

This guide explains how to run the ChatCodex deterministic stack and connect it to
ChatGPT Desktop as an MCP server.

---

## Architecture

```
ChatGPT (MCP client)  ←→  chatgpt-mcp (MCP gateway)  ←→  deterministic-daemon (Rust)
                                    :3000                       :19280
                              (Node.js)                    (Rust/Axum)
```

Both the MCP gateway and the daemon must be running before ChatGPT can use the tools.
The MCP gateway proxies requests to the daemon and translates MCP tool calls into
daemon RPC calls.

---

## Prerequisites

- **Rust** (1.75+): `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- **Node.js** (18+): `brew install node` (macOS) or use your preferred package manager
- **ChatGPT Desktop**: Available at https://chatgpt.com or via the desktop app

---

## Gap 1 — Start the Daemon

The daemon stores run state and serves the RPC API.

```sh
# Default: listens on 127.0.0.1:19280, stores data in /tmp/deterministic-daemon
cargo run -p deterministic-daemon

# With custom settings
DETERMINISTIC_BIND=0.0.0.0:19280 \
DETERMINISTIC_STORE_DIR=/data/chatcodex \
cargo run -p deterministic-daemon
```

### Daemon environment variables

| Variable | Default | Description |
|---|---|---|
| `DETERMINISTIC_BIND` | `127.0.0.1:19280` | Address and port the daemon listens on |
| `DETERMINISTIC_STORE_DIR` | `/tmp/deterministic-daemon` | Directory for SQLite database and run state |
| `DETERMINISTIC_WORKSPACE_ROOT` | *(none)* | Default workspace root for new runs (optional) |

For hybrid mode, see [docs/HYBRID_MODE.md](HYBRID_MODE.md).

---

## Gap 2 — Start the MCP Gateway

```sh
cd apps/chatgpt-mcp

# Development (auto-rebuild on file change with tsx)
npx tsx src/index.ts

# Production
npm install
npm run build
node dist/index.js
```

### Gateway environment variables

| Variable | Default | Description |
|---|---|---|
| `DETERMINISTIC_DAEMON_URL` | `http://127.0.0.1:19280` | URL of the deterministic daemon RPC endpoint |
| `MCP_TRANSPORT` | `stdio` | Transport mode: `stdio` (local) or `http` (remote) |
| `PORT` | `3000` | HTTP server port (only in `http` transport mode) |
| `HOST` | `0.0.0.0` | HTTP server bind address (only in `http` transport mode) |
| `CHATCODEX_AUTH_MODE` | `none` | Auth mode: `none`, `static-token`, or `oauth` |
| `MCP_AUTH_TOKEN` | *(none)* | Required when `CHATCODEX_AUTH_MODE=static-token` |

### HTTP transport mode

When `MCP_TRANSPORT=http` is set, the gateway starts an Express server (default
`:3000`) that accepts MCP over Streamable HTTP. This is needed when ChatGPT is
running in a browser (not the desktop app), since the desktop app uses stdio.

```sh
DETERMINISTIC_DAEMON_URL=http://127.0.0.1:19280 \
MCP_TRANSPORT=http \
PORT=3000 \
node dist/index.js
```

The MCP endpoint will be at `http://localhost:3000/mcp` and the health check at
`http://localhost:3000/healthz`.

---

## Gap 3 — Register in ChatGPT Desktop

1. Open ChatGPT → Settings → MCP (or "Model Context Protocol" depending on version)
2. Click **Add server**
3. Paste the MCP server URL:
   - **Desktop app (stdio transport)**: `stdio` (the app auto-discovers local stdio servers;
     you may need to point it at the absolute path of the `node` command running `dist/index.js`)
   - **Browser (http transport)**: `http://localhost:3000/mcp` (replace `3000` if using a different `PORT`)
4. Save and confirm the server is listed as connected

---

## Gap 4 — Verify Connectivity

Ask ChatGPT to call the `codex_prepare_run` tool:

```
Use the codex_prepare_run tool with any workspace path and goal.
Report whether you received a run ID.
```

If the connection works, you'll get back a `runId`. If ChatGPT reports the tool
is unavailable, check:

1. Is the daemon still running? (look for `deterministic daemon listening on` in stderr)
2. Is the MCP gateway still running? (look for `chatgpt-mcp: HTTP server listening` in stderr)
3. If using HTTP transport, is the URL correct? (`curl http://localhost:3000/healthz`)
4. If using stdio transport, restart ChatGPT and try again

---

## Gap 5 — Quick Start (Combined)

Start both services in one terminal (for development):

```sh
# Terminal 1: daemon
cargo run -p deterministic-daemon

# Terminal 2: MCP gateway (stdio)
cd apps/chatgpt-mcp && npx tsx src/index.ts

# Or gateway in HTTP mode (for browser ChatGPT)
cd apps/chatgpt-mcp && \
  DETERMINISTIC_DAEMON_URL=http://127.0.0.1:19280 \
  MCP_TRANSPORT=http \
  npx tsx src/index.ts
```

For a single-command deployment, see `scripts/deploy.sh`.

For Docker-based deployment, see `docs/DEPLOYMENT.md`.

---

## Troubleshooting

**`Connection refused` from gateway to daemon**

Check that `DETERMINISTIC_DAEMON_URL` in the gateway matches the `DETERMINISTIC_BIND`
address of the daemon. Both default to `127.0.0.1:19280`.

**MCP server URL not accepted in ChatGPT Desktop**

The desktop app uses stdio transport, not HTTP. Set `MCP_TRANSPORT=stdio` (default)
and ensure the node process is running with `dist/index.js`. Do not use an HTTP URL
for desktop ChatGPT.

**Browser ChatGPT can't reach the MCP server**

The gateway must bind to `0.0.0.0` (not `127.0.0.1`) for external access:

```sh
DETERMINISTIC_DAEMON_URL=http://127.0.0.1:19280 \
MCP_TRANSPORT=http \
HOST=0.0.0.0 \
PORT=3000 \
node dist/index.js
```

Then use `http://<your-machine-ip>:3000/mcp` in ChatGPT's browser session.