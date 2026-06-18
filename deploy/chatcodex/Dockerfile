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
FROM --platform=$BUILDPLATFORM rust:1.95.0-bookworm AS builder-base

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
            liblzma-dev:arm64 \
            lld \
            pkg-config \
        && rm -rf /var/lib/apt/lists/* && \
        rustup target add aarch64-unknown-linux-gnu; \
    else \
        apt-get update && \
        apt-get install -y --no-install-recommends \
            gcc \
            libcap-dev \
            lld \
            pkg-config \
        && rm -rf /var/lib/apt/lists/*; \
    fi

# Cross-compilation environment (harmless on native aarch64 builds where
# aarch64-linux-gnu-gcc is just the native compiler).
ENV CARGO_BUILD_JOBS=4 \
    CARGO_PROFILE_RELEASE_LTO=false \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=256 \
    CARGO_PROFILE_RELEASE_OPT_LEVEL=0 \
    CARGO_TARGET_DIR=/src/chatcodex/target \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++ \
    PKG_CONFIG_ALLOW_CROSS=1 \
    PKG_CONFIG_PATH=/usr/lib/aarch64-linux-gnu/pkgconfig \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C link-arg=-fuse-ld=lld"

WORKDIR /src

# Copy upstream workspace first. ChatCodex depends on it via path deps.
COPY codex-rs ./codex-rs

# Copy ChatCodex workspace and crate manifests, then create stub sources for
# dependency caching. The real sources are overlaid in a later stage.
COPY chatcodex/Cargo.toml ./chatcodex/Cargo.toml
COPY chatcodex/crates/mcp-server/Cargo.toml ./chatcodex/crates/mcp-server/Cargo.toml
COPY chatcodex/crates/oauth/Cargo.toml ./chatcodex/crates/oauth/Cargo.toml
RUN mkdir -p \
        chatcodex/crates/mcp-server/src \
        chatcodex/crates/oauth/src && \
    printf 'pub fn dependency_cache_stub() {}\n' \
        > chatcodex/crates/oauth/src/lib.rs && \
    printf 'pub fn dependency_cache_stub() {}\n' \
        > chatcodex/crates/mcp-server/src/lib.rs && \
    printf 'fn main() {}\n' \
        > chatcodex/crates/mcp-server/src/main.rs

# Build upstream dependencies and the ChatCodex dependency closure in one
# layer. This layer is invalidated only when upstream or workspace
# manifests change, not when ChatCodex source files change.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    arch="$(dpkg --print-architecture)" && \
    if [ "$arch" = "amd64" ]; then \
        TARGET="aarch64-unknown-linux-gnu"; \
    else \
        TARGET="$(rustc -vV | grep 'host:' | awk '{print $2}')"; \
    fi && \
    echo "Caching dependencies for target: $TARGET on arch: $arch" && \
    cd chatcodex && \
    cargo build --release --target "$TARGET" -p chatcodex-mcp-server

# Overlay only ChatCodex-owned source. These named contexts are hashed
# independently from the upstream codex-rs workspace.
FROM builder-base AS builder
COPY --from=chatcodex-mcp . /src/chatcodex/crates/mcp-server
COPY --from=chatcodex-oauth . /src/chatcodex/crates/oauth

# Build the real binary. Cargo reuses the dependency artifacts from the
# previous stage and recompiles only the ChatCodex crates.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    find \
        chatcodex/crates/mcp-server \
        chatcodex/crates/oauth \
        -type f -exec touch {} + && \
    arch="$(dpkg --print-architecture)" && \
    if [ "$arch" = "amd64" ]; then \
        TARGET="aarch64-unknown-linux-gnu"; \
    else \
        TARGET="$(rustc -vV | grep 'host:' | awk '{print $2}')"; \
    fi && \
    echo "Building ChatCodex for target: $TARGET on arch: $arch" && \
    cd chatcodex && \
    cargo build --release --target "$TARGET" -p chatcodex-mcp-server && \
    cp "$CARGO_TARGET_DIR/$TARGET/release/chatcodex-mcp-server" /tmp/chatcodex

# ---------- runtime: minimal Ubuntu image ----------
FROM --platform=$TARGETPLATFORM ubuntu:24.04

ARG TARGETARCH
ARG MISE_VERSION=2026.6.6
ARG UV_VERSION=0.11.18

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bubblewrap \
        build-essential \
        ca-certificates \
        cmake \
        curl \
        fd-find \
        git \
        git-lfs \
        gh \
        jq \
        openssh-client \
        pkg-config \
        python-is-python3 \
        python3 \
        python3-venv \
        ripgrep \
        rsync \
        tar \
        unzip \
        zip \
    && rm -rf /var/lib/apt/lists/* \
    && ln -s /usr/bin/fdfind /usr/local/bin/fd \
    && useradd --create-home --uid 10001 --shell /bin/bash chatcodex \
    && mkdir -p \
        /workspaces \
        /workspaces/.uv-cache \
        /toolchains/chatcodex/codex \
        /toolchains/home/chatcodex \
        /toolchains/mise/data \
        /toolchains/mise/cache \
        /toolchains/mise/state \
        /toolchains/uv/python \
        /toolchains/uv/tools \
    && chown -R chatcodex:chatcodex /workspaces /toolchains

RUN set -eux; \
    case "${TARGETARCH}" in \
        arm64) \
            mise_arch="arm64"; \
            mise_sha256="e3482fc5dde76502c1714b85963da411698436f06fefef7b6f0ee16fd00136a0"; \
            uv_target="aarch64-unknown-linux-gnu"; \
            uv_sha256="0f03c6648df1c159557f4222c0f37250f84733fb88d6fc3c16770e17c177a8c9"; \
            ;; \
        amd64) \
            mise_arch="x64"; \
            mise_sha256="b8c25ad5e1c178bb7ba1e0fca9066153f8ef1b28bf5a8456c20a8acd2522e556"; \
            uv_target="x86_64-unknown-linux-gnu"; \
            uv_sha256="588f3e360f69ce02b6982aa99f2240e803933a6b7e176ac01617830adf955add"; \
            ;; \
        *) \
            echo "unsupported runtime architecture: ${TARGETARCH}" &&2; \
            exit 1; \
            ;; \
    esac; \
    curl --proto '=https' --tlsv1.2 -fsSLo /tmp/mise \
        "https://github.com/jdx/mise/releases/download/v${MISE_VERSION}/mise-v${MISE_VERSION}-linux-${mise_arch}"; \
    echo "${mise_sha256}  /tmp/mise" | sha256sum -c -; \
    install -m 0755 /tmp/mise /usr/local/bin/mise; \
    curl --proto '=https' --tlsv1.2 -fsSLo /tmp/uv.tar.gz \
        "https://github.com/astral-sh/uv/releases/download/${UV_VERSION}/uv-${uv_target}.tar.gz"; \
    echo "${uv_sha256}  /tmp/uv.tar.gz" | sha256sum -c -; \
    tar -xzf /tmp/uv.tar.gz -C /tmp; \
    install -m 0755 "/tmp/uv-${uv_target}/uv" /usr/local/bin/uv; \
    install -m 0755 "/tmp/uv-${uv_target}/uvx" /usr/local/bin/uvx; \
    rm -rf /tmp/mise /tmp/uv.tar.gz "/tmp/uv-${uv_target}"; \
    mise --version; \
    uv --version; \
    python3 --version

COPY --from=builder \
    /tmp/chatcodex \
    /usr/local/bin/chatcodex

ENV CHATCODEX_BIND=0.0.0.0:3000 \
    CHATCODEX_DATA_DIR=/toolchains/chatcodex \
    CHATCODEX_WORKSPACE_ROOT=/workspaces \
    CHATCODEX_PUBLIC_BASE_URL=https://codex.example.invalid \
    CHATCODEX_CF_ACCESS_TEAM=https://team.cloudflareaccess.com \
    CHATCODEX_CF_ACCESS_AUD=replace-me-at-deploy-time \
    CHATCODEX_OAUTH_ACCESS_TTL_SECS=3600 \
    CHATCODEX_OAUTH_REFRESH_TTL_SECS=2592000 \
    MISE_DATA_DIR=/toolchains/mise/data \
    MISE_CACHE_DIR=/toolchains/mise/cache \
    MISE_STATE_DIR=/toolchains/mise/state \
    UV_CACHE_DIR=/workspaces/.uv-cache \
    UV_PYTHON_INSTALL_DIR=/toolchains/uv/python \
    UV_TOOL_DIR=/toolchains/uv/tools \
    UV_PYTHON_DOWNLOADS=never

ENV CODEX_HOME=/toolchains/chatcodex/codex
ENV HOME=/toolchains/home/chatcodex

EXPOSE 3000
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["curl", "--fail", "--silent", "--show-error", "http://127.0.0.1:3000/healthz"]
USER chatcodex
WORKDIR /workspaces

ENTRYPOINT ["/usr/local/bin/chatcodex"]
