# ChatCodex Deployment Guide

This guide covers all deployment options for ChatCodex: local development, single-server
with Docker, and split (daemon + gateway on separate hosts).

---

## Prerequisites

- **Docker** (20.10+) and **Docker Compose** (2.0+)
- **Rust** (1.85+) for local daemon builds — [rustup](https://rustup.rs)
- **Node.js** (18+) for local gateway builds — [nodejs.org](https://nodejs.org)

---

## Environment Variables

All settings are driven by environment variables. There are **no config files**.

### Daemon env vars

| Variable | Default | Description |
|---|---|---|
| `DETERMINISTIC_BIND` | `127.0.0.1:19280` | Listen address/port |
| `DETERMINISTIC_STORE_DIR` | `/tmp/deterministic-daemon` | SQLite + state directory |
| `DETERMINISTIC_WORKSPACE_ROOT` | *(none)* | Default workspace root for new runs |

### Gateway env vars

| Variable | Default | Description |
|---|---|---|
| `DETERMINISTIC_DAEMON_URL` | `http://127.0.0.1:19280` | Daemon RPC URL |
| `MCP_TRANSPORT` | `stdio` | `stdio` (desktop app) or `http` (browser) |
| `PORT` | `3000` | HTTP gateway port |
| `HOST` | `127.0.0.1` | HTTP gateway bind address |
| `CHATCODEX_AUTH_MODE` | `none` | `none`, `static-token`, `oauth` |
| `MCP_AUTH_TOKEN` | *(none)* | Required when `AUTH_MODE=static-token` |

### Hybrid mode env vars

| Variable | Default | Description |
|---|---|---|
| `CHATCODEX_HYBRID_ENABLED` | *(none)* | Set to `true` to enable |
| `CHATCODEX_HYBRID_PROVIDER_URL` | *(none)* | Worker LLM base URL (OpenAI-compatible) |
| `CHATCODEX_HYBRID_MODEL` | *(none)* | Worker model name |
| `CHATCODEX_HYBRID_API_KEY_ENV` | *(none)* | Name of env var holding the API key |
| `CHATCODEX_HYBRID_TIMEOUT_SECONDS` | `120` | Worker call timeout |
| `CHATCODEX_HYBRID_MAX_OUTPUT_TOKENS` | `8000` | Max tokens in worker response |
| `CHATCODEX_HYBRID_TEMPERATURE` | `0.2` | Worker sampling temperature |

---

## Deployment Options

### Option 1 — Local Development (no Docker)

Terminal 1: build and run the daemon:

```sh
cargo build -p deterministic-daemon --release
DETERMINISTIC_STORE_DIR=/tmp/chatcodex \
  DETERMINISTIC_BIND=127.0.0.1:19280 \
  cargo run -p deterministic-daemon --release
```

Terminal 2: build and run the MCP gateway:

```sh
cd apps/chatgpt-mcp
npm install && npm run build

# For ChatGPT desktop app (stdio transport):
DETERMINISTIC_DAEMON_URL=http://127.0.0.1:19280 \
  MCP_TRANSPORT=stdio \
  node dist/index.js

# For browser ChatGPT (HTTP transport):
DETERMINISTIC_DAEMON_URL=http://127.0.0.1:19280 \
  MCP_TRANSPORT=http \
  PORT=3000 \
  HOST=0.0.0.0 \
  node dist/index.js
```

One-liner that starts everything:

```sh
./scripts/deploy.sh
# or with options:
./scripts/deploy.sh --daemon-bind 127.0.0.1:19280 \
  --gateway-port 3000 --transport http
```

### Option 2 — Docker Combined (recommended for single-server)

Build and run everything in one container:

```sh
docker build -t chatcodex/chatcodex:latest .
docker run -d \
  --name chatcodex \
  -p 19280:19280 \
  -p 3000:3000 \
  -v chatcodex-data:/data \
  -v /path/to/workspace:/workspace \
  chatcodex/chatcodex:latest
```

Or use Docker Compose (always picks up local changes if you `docker compose up --build`):

```sh
# Start
docker compose up -d --build

# Verify
curl http://localhost:19280/healthz
curl http://localhost:3000/healthz

# Stop
docker compose down
```

Enable hybrid mode:

```sh
WORKSPACE_DIR=/path/to/workspace docker compose up -d --build
docker exec chatcodex env \
  CHATCODEX_HYBRID_ENABLED=true \
  CHATCODEX_HYBRID_PROVIDER_URL=http://host.docker.internal:11434/v1 \
  CHATCODEX_HYBRID_MODEL=qwen2.5-coder \
  chatcodex-daemon ...  # restart with env vars

# Or edit docker-compose.yml and uncomment the hybrid env block
docker compose up -d --build
```

### Option 3 — Docker Split (daemon + gateway on different hosts)

**Daemon host** (Rust server):

```sh
# Build only the daemon binary
docker build --target build -t chatcodex-daemon-build . && \
docker create --name daemon-build chatcodex-daemon-build && \
docker cp daemon-build:/app/deterministic-daemon ./daemon-bin && \
docker rm daemon-build

# On the daemon host:
scp ./daemon-bin user@daemon-host:/opt/chatcodex/deterministic-daemon
ssh user@daemon-host
# Run:
DETERMINISTIC_BIND=0.0.0.0:19280 \
  DETERMINISTIC_STORE_DIR=/data \
  /opt/chatcodex/deterministic-daemon
```

**Gateway host** (Node.js MCP server):

```sh
# Build the gateway only
docker build --target runtime -t chatcodex-gateway .

# On the gateway host:
scp ./gateway-bin user@gateway-host:/opt/chatcodex/gateway
ssh user@gateway-host
# Run:
DETERMINISTIC_DAEMON_URL=http://daemon-host:19280 \
  MCP_TRANSPORT=http \
  PORT=3000 \
  HOST=0.0.0.0 \
  node /opt/chatcodex/gateway
```

Or use the split compose profile:

```sh
# Start daemon
docker compose --profile split up -d daemon

# Start gateway (on a different machine, or same machine for testing)
docker compose --profile split up -d gateway

# Verify
curl http://localhost:19280/healthz      # daemon
curl http://localhost:3000/healthz       # gateway
```

---

## Health Checks

Both services expose structured JSON health check endpoints.

### Daemon health check

```sh
curl http://localhost:19280/healthz
# → {"status":"ok","version":"0.1.0","daemon":"ok","hybrid_enabled":false,"timestamp":"..."}
```

| Field | Description |
|---|---|
| `status` | `"ok"` if daemon is healthy |
| `daemon` | `"ok"` if store is accessible |
| `hybrid_enabled` | `true` if hybrid mode is active |
| `workspace_root` | Default workspace root (null if not set) |

### Gateway health check

```sh
curl http://localhost:3000/healthz
# → {"status":"ok","daemon":"ok",...}  # daemon's full response relayed
# If daemon unreachable: {"status":"degraded","daemon":"unreachable"}
```

The gateway healthz **proxies and extends** the daemon healthz — it always calls
`/healthz` on the daemon and merges the result. A 503 response means the daemon
is unreachable.

---

## Registering in ChatGPT

### Desktop app (stdio transport)

1. Open ChatGPT → Settings → MCP
2. Add server, point to the gateway binary path:

   ```sh
   # Absolute path to node + dist/index.js
   /usr/local/bin/node /path/to/ChatCodex/apps/chatgpt-mcp/dist/index.js
   ```

   Or use the deploy script to manage the process:

   ```sh
   ./scripts/deploy.sh --transport stdio
   ```

3. Save. ChatGPT auto-discovers the stdio MCP server.

### Browser (HTTP transport)

1. Open ChatGPT → Settings → MCP
2. Add server with URL:

   ```
   http://<gateway-host>:3000/mcp
   ```

   For local Docker: `http://localhost:3000/mcp`
   For remote: `http://<gateway-ip>:3000/mcp`

3. Save and verify the server shows as connected.

---

## One-Liner Deployment

### Linux/macOS

```sh
# Full stack, HTTP transport, remote accessible
./scripts/deploy.sh --gateway-host 0.0.0.0 --transport http

# Minimal local
./scripts/deploy.sh

# With hybrid mode (Ollama)
./scripts/deploy.sh --hybrid \
  --hybrid-url http://localhost:11434/v1 \
  --hybrid-model qwen2.5-coder
```

### Windows PowerShell

```powershell
.\scripts\deploy.ps1

# Remote accessible
.\scripts\deploy.ps1 -GatewayHost "0.0.0.0" -Transport http

# With hybrid mode
.\scripts\deploy.ps1 -Hybrid `
  -HybridUrl "http://localhost:11434/v1" `
  -HybridModel "qwen2.5-coder"
```

---

## Architecture

```
              ChatGPT Desktop / Browser
                        │
                        ▼
              ┌─────────────────────────┐
              │   chatgpt-mcp gateway   │
              │   Node.js / Express     │
              │   :3000 (stdio or http) │
              └────────────┬────────────┘
                           │  JSON-RPC over HTTP
                           ▼
              ┌─────────────────────────┐
              │  deterministic-daemon   │
              │  Rust / Axum            │
              │  :19280                 │
              ├─────────────────────────┤
              │  Store (SQLite)         │
              │  Run state / approvals  │
              └─────────────────────────┘

              Optional: Hybrid worker LLM
              ┌─────────────────────────┐
              │  Worker LLM (ollama,    │
              │  openai, lm studio...)  │
              └─────────────────────────┘
```

---

## Troubleshooting

**`curl` to `/healthz` returns connection refused**

Daemon is not running. Check `docker compose ps` or `ps aux | grep deterministic-daemon`.

**ChatGPT shows MCP server as disconnected**

1. Daemon healthz works? `curl http://localhost:19280/healthz`
2. Gateway healthz works? `curl http://localhost:3000/healthz`
3. If using HTTP transport: is the URL `http://<host>:3000/mcp` correct?
4. If using stdio transport: is `dist/index.js` compiled? `npm run build` in `apps/chatgpt-mcp`

**Gateway returns `degraded` / `daemon: unreachable`**

The daemon is not reachable from the gateway. Check:
- `DETERMINISTIC_DAEMON_URL` in the gateway env matches the daemon's `DETERMINISTIC_BIND`
- Network/firewall between containers
- Daemon is not OOM-killed

**Hybrid mode not working**

Check the daemon healthz for `"hybrid_enabled": true`. If false:
- `CHATCODEX_HYBRID_ENABLED=true` is set on the daemon
- `CHATCODEX_HYBRID_PROVIDER_URL` is correct (must include `/v1`)
- `CHATCODEX_HYBRID_MODEL` is set

**Build fails with `could not find sqlx` or `rusqlite`**

`cargo build --release` needs network access to fetch dependencies. For Docker builds,
the multi-stage build fetches deps in the `build` stage. Ensure Docker build has
network access and the build cache is not corrupted.