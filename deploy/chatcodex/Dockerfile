FROM rust:1.92.0-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
  ca-certificates \
  curl \
  git \
  gnupg \
  ripgrep \
  && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
  && apt-get update \
  && apt-get install -y --no-install-recommends nodejs \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .

WORKDIR /app/apps/chatgpt-mcp
RUN npm ci && npm run build

WORKDIR /app/codex-rs
RUN cargo build --release -p deterministic-daemon

FROM rust:1.92.0-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
  ca-certificates \
  curl \
  git \
  gnupg \
  ripgrep \
  && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
  && apt-get update \
  && apt-get install -y --no-install-recommends nodejs \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app /app
COPY deploy/chatcodex/entrypoint.sh /usr/local/bin/chatcodex-entrypoint
RUN chmod 755 /usr/local/bin/chatcodex-entrypoint

ENV BOOTSTRAP_REPO_URL=https://github.com/anschmieg/ChatCodex.git
ENV BOOTSTRAP_REPO_REF=main
ENV CHATCODEX_WORKSPACE_DIR=/workspace/repo
ENV CHATCODEX_STORE_DIR=/workspace/store
ENV DAEMON_PORT=19280
ENV MCP_TRANSPORT=http
ENV HOST=0.0.0.0
ENV PORT=3000

EXPOSE 3000
VOLUME ["/workspace"]

ENTRYPOINT ["/usr/local/bin/chatcodex-entrypoint"]
