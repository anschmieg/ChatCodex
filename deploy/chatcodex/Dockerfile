# syntax=docker/dockerfile:1
#
# Multi-stage Dockerfile for ChatCodex.
#
# Architecture strategy:
#   On x86_64 CI runners: cross-compile to aarch64-unknown-linux-gnu using
#     gcc-aarch64-linux-gnu (no QEMU emulation for the Rust build).
#   On aarch64 hosts (e.g. local Mac via Colima): compile natively.
#
# The builder stage runs on BUILDPLATFORM (the host architecture). The
# runtime stage runs on TARGETPLATFORM (linux/arm64 for Coolify).

# ---------- builder: cross-compile or native compile ----------
FROM --platform=$BUILDPLATFORM rust:1.93.0-bookworm AS builder

ARG TARGETPLATFORM

# Install build dependencies.
# On amd64 hosts we need the aarch64 cross-compilation toolchain + arm64
# libcap for the vendored bubblewrap build in codex-linux-sandbox.
# On arm64 hosts we just need the native toolchain.
RUN arch="$(dpkg --print-architecture)" && \
    if [ "$arch" = "amd64" ]; then \
        dpkg --add-architecture arm64 && \
        apt-get update && \
        apt-get install -y --no-install-recommends \
            gcc-aarch64-linux-gnu \
            libc6-dev-arm64-cross \
            libcap-dev:arm64 \
            pkg-config \
        && rm -rf /var/lib/apt/lists/* && \
        rustup target add aarch64-unknown-linux-gnu; \
    else \
        apt-get update && \
        apt-get install -y --no-install-recommends \
            gcc \
            libcap-dev \
            pkg-config \
        && rm -rf /var/lib/apt/lists/*; \
    fi

# Cross-compilation environment (harmless on native aarch64 builds where
# aarch64-linux-gnu-gcc is just the native compiler).
ENV CARGO_BUILD_JOBS=2 \
    CARGO_PROFILE_RELEASE_LTO=false \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
    CARGO_PROFILE_RELEASE_OPT_LEVEL=0 \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++ \
    PKG_CONFIG_ALLOW_CROSS=1 \
    PKG_CONFIG_PATH=/usr/lib/aarch64-linux-gnu/pkgconfig

WORKDIR /src
COPY codex-rs ./codex-rs

# Build the binary. On amd64 we cross-compile to aarch64; on arm64 we
# compile natively (the target triple happens to be the same).
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/codex-rs/target \
    arch="$(dpkg --print-architecture)" && \
    if [ "$arch" = "amd64" ]; then \
        TARGET="aarch64-unknown-linux-gnu"; \
    else \
        TARGET="$(rustc -vV | grep 'host:' | awk '{print $2}')"; \
    fi && \
    echo "Building for target: $TARGET on arch: $arch" && \
    cd codex-rs && \
    cargo build --release --target "$TARGET" -p codex-native-harness-mcp && \
    cp "target/$TARGET/release/codex-native-harness-mcp" /tmp/chatcodex

# ---------- runtime: minimal Ubuntu image ----------
FROM --platform=$TARGETPLATFORM ubuntu:24.04

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
