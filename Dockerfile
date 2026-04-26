# syntax=docker/dockerfile:1
#
# Build stage: compile the Rust daemon + Node.js gateway
#
FROM --platform=$BUILDPLATFORM rust:1-bookworm AS build

ARG TARGETARCH
ARG BUILDPLATFORM

WORKDIR /src/chatcodex

# Cache cargo dependencies between builds
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/chatcodex/target,sharing=locked \
    cargo fetch

# Install Node.js for gateway build
RUN apt-get update && apt-get install -y --no-install-recommends \
    nodejs npm \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml ./
COPY rust-toolchain.toml ./

# Copy only the crates needed for building (not tests / benches)
COPY crates/deterministic-protocol/ crates/deterministic-protocol/
COPY crates/deterministic-core/ crates/deterministic-core/
COPY crates/deterministic-daemon/ crates/deterministic-daemon/

# Pre-build dependencies so incremental builds are fast
RUN --mount=type=cache,target=/src/chatcodex/target,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo build --release -p deterministic-daemon

COPY apps/chatgpt-mcp/package.json apps/chatgpt-mcp/tsconfig.json apps/chatgpt-mcp/
COPY apps/chatgpt-mcp/src/ apps/chatgpt-mcp/src/

WORKDIR /src/chatcodex/apps/chatgpt-mcp
RUN npm install && npm run build

#
# Runtime stage: minimal image with only the binaries
#
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    nodejs \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy daemon binary
COPY --from=build /src/chatcodex/target/release/deterministic-daemon /app/deterministic-daemon

# Copy gateway
COPY --from=build /src/chatcodex/apps/chatgpt-mcp/dist/ /app/gateway/dist/
COPY --from=build /src/chatcodex/apps/chatgpt-mcp/package.json /app/gateway/

ENV DETERMINISTIC_BIND="0.0.0.0:19280" \
    DETERMINISTIC_STORE_DIR="/data" \
    MCP_TRANSPORT="http" \
    PORT="3000" \
    HOST="0.0.0.0" \
    DETERMINISTIC_DAEMON_URL="http://127.0.0.1:19280"

EXPOSE 19280 3000

# Default: run both services in separate processes
# Use /usr/bin/env wrapper so the MCP gateway runs in foreground too
ENTRYPOINT ["sh", "-c", "deterministic-daemon & node /app/gateway/dist/index.js"]

# For separate containers, override the entrypoint:
#   docker run chatcodex/daemon   → deterministic-daemon
#   docker run chatcodex/gateway  → node /app/gateway/dist/index.js