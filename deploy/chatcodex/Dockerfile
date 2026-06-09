FROM rust:1.93.0-bookworm AS builder

ENV CARGO_BUILD_JOBS=2
ENV CARGO_PROFILE_RELEASE_LTO=false
ENV CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
ENV CARGO_PROFILE_RELEASE_OPT_LEVEL=0

WORKDIR /src
COPY codex-rs ./codex-rs

# codex-linux-sandbox (transitive via codex-arg0) builds vendored bubblewrap
# at build time, which needs libcap headers via pkg-config.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        libcap-dev \
    && rm -rf /var/lib/apt/lists/*

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/codex-rs/target \
    cd codex-rs \
    && cargo build --release -p codex-native-harness-mcp \
    && cp target/release/codex-native-harness-mcp /tmp/chatcodex

FROM ubuntu:24.04

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        git \
        ripgrep \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 --shell /bin/bash chatcodex \
    && mkdir -p /workspaces /data \
    && chown -R chatcodex:chatcodex /workspaces /data

COPY --from=builder \
    /tmp/chatcodex \
    /usr/local/bin/chatcodex

ENV CHATCODEX_BIND=0.0.0.0:3000 \
    CHATCODEX_DATA_DIR=/data \
    CHATCODEX_WORKSPACE_ROOT=/workspaces \
    CHATCODEX_PUBLIC_BASE_URL=https://codex.example.invalid \
    CHATCODEX_CF_ACCESS_TEAM=https://team.cloudflareaccess.com \
    CHATCODEX_CF_ACCESS_AUD=replace-me-at-deploy-time \
    CHATCODEX_OAUTH_ACCESS_TTL_SECS=3600 \
    CHATCODEX_OAUTH_REFRESH_TTL_SECS=2592000

ENV CODEX_HOME=/data/codex
ENV HOME=/home/chatcodex

EXPOSE 3000
USER chatcodex
WORKDIR /workspaces

ENTRYPOINT ["/usr/local/bin/chatcodex"]
