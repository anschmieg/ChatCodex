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
FROM --platform=$BUILDPLATFORM rust:1.95.0-bookworm AS builder

ARG TARGETPLATFORM
ARG SCCACHE_VERSION=0.15.0

# Install build dependencies.
# On amd64 hosts we need the aarch64 cross-compilation toolchain + arm64
# libcap for the vendored bubblewrap build in codex-linux-sandbox.
# On arm64 hosts we just need the native toolchain.
RUN arch="$(dpkg --print-architecture)" && \
    if [ "$arch" = "amd64" ]; then \
        dpkg --add-architecture arm64 && \
        apt-get update && \
        apt-get install -y --no-install-recommends \
            ca-certificates \
            curl \
            gcc-aarch64-linux-gnu \
            libc6-dev-arm64-cross \
            libcap-dev:arm64 \
            liblzma-dev:arm64 \
            pkg-config \
        && rm -rf /var/lib/apt/lists/* && \
        rustup target add aarch64-unknown-linux-gnu; \
    else \
        apt-get update && \
        apt-get install -y --no-install-recommends \
            ca-certificates \
            curl \
            gcc \
            libcap-dev \
            pkg-config \
        && rm -rf /var/lib/apt/lists/*; \
    fi && \
    case "$arch" in \
        amd64) \
            sccache_target="x86_64-unknown-linux-musl"; \
            sccache_sha256="782d2b5dd7ae0a55ebe368ab258114d0928d019ac2d949ab85d5d02f3926709e"; \
            ;; \
        arm64) \
            sccache_target="aarch64-unknown-linux-musl"; \
            sccache_sha256="3a6a3712b49da3d263bf2d30d702de4302793016019e800bfb81c0c69401d8f8"; \
            ;; \
        *) \
            echo "unsupported builder architecture for sccache: $arch" >&2; \
            exit 1; \
            ;; \
    esac && \
    sccache_archive="sccache-v${SCCACHE_VERSION}-${sccache_target}.tar.gz" && \
    curl --proto '=https' --tlsv1.2 -fsSLo "/tmp/${sccache_archive}" \
        "https://github.com/mozilla/sccache/releases/download/v${SCCACHE_VERSION}/${sccache_archive}" && \
    echo "${sccache_sha256}  /tmp/${sccache_archive}" | sha256sum -c - && \
    tar -xzf "/tmp/${sccache_archive}" -C /tmp && \
    install -m 0755 \
        "/tmp/sccache-v${SCCACHE_VERSION}-${sccache_target}/sccache" \
        /usr/local/bin/sccache && \
    rm -rf \
        "/tmp/${sccache_archive}" \
        "/tmp/sccache-v${SCCACHE_VERSION}-${sccache_target}" && \
    sccache --version

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
    PKG_CONFIG_PATH=/usr/lib/aarch64-linux-gnu/pkgconfig \
    RUSTC_WRAPPER=sccache \
    SCCACHE_GHA_VERSION=chatcodex-rust-v1 \
    SCCACHE_BASEDIRS=/src/codex-rs

WORKDIR /src
COPY codex-rs ./codex-rs

# Build the binary. On amd64 we cross-compile to aarch64; on arm64 we
# compile natively (the target triple happens to be the same).
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/codex-rs/target \
    --mount=type=cache,target=/src/.cache/sccache \
    --mount=type=secret,id=gha_cache_url \
    --mount=type=secret,id=gha_cache_token \
    export ACTIONS_RESULTS_URL="$(cat /run/secrets/gha_cache_url 2>/dev/null || true)" && \
    export ACTIONS_RUNTIME_TOKEN="$(cat /run/secrets/gha_cache_token 2>/dev/null || true)" && \
    if [ -n "$ACTIONS_RESULTS_URL" ] && [ -n "$ACTIONS_RUNTIME_TOKEN" ]; then \
        export SCCACHE_GHA_ENABLED=on; \
        echo "Using the GitHub Actions sccache backend"; \
    else \
        export SCCACHE_DIR=/src/.cache/sccache; \
        echo "Using the local BuildKit sccache backend"; \
    fi && \
    arch="$(dpkg --print-architecture)" && \
    if [ "$arch" = "amd64" ]; then \
        TARGET="aarch64-unknown-linux-gnu"; \
    else \
        TARGET="$(rustc -vV | grep 'host:' | awk '{print $2}')"; \
    fi && \
    echo "Building for target: $TARGET on arch: $arch" && \
    cd codex-rs && \
    cargo build --release --target "$TARGET" -p codex-native-harness-mcp && \
    cp "target/$TARGET/release/codex-native-harness-mcp" /tmp/chatcodex && \
    sccache --show-stats

# ---------- runtime: minimal Ubuntu image ----------
FROM --platform=$TARGETPLATFORM ubuntu:24.04

ARG TARGETARCH
ARG MISE_VERSION=2026.6.6
ARG UV_VERSION=0.11.18

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        cmake \
        curl \
        fd-find \
        git \
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
            echo "unsupported runtime architecture: ${TARGETARCH}" >&2; \
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
