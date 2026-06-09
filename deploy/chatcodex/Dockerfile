FROM rust:1.93.0-bookworm AS builder

ENV CARGO_BUILD_JOBS=2
ENV CARGO_PROFILE_RELEASE_LTO=false
ENV CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
ENV CARGO_PROFILE_RELEASE_OPT_LEVEL=0

WORKDIR /src
COPY codex-rs ./codex-rs
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

ENV CHATCODEX_BIND=0.0.0.0:3000
ENV CHATCODEX_DATA_DIR=/data
ENV CHATCODEX_WORKSPACE_ROOT=/workspaces
ENV CODEX_HOME=/data/codex
ENV HOME=/home/chatcodex

EXPOSE 3000
USER chatcodex
WORKDIR /workspaces

ENTRYPOINT ["/usr/local/bin/chatcodex"]
