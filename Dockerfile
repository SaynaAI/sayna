# syntax=docker/dockerfile:1
ARG RUST_VERSION=1.88.0
ARG CARGO_BUILD_FEATURES="--no-default-features --features stt-vad,noise-filter"
ARG ONNX_VERSION=1.23.2

# ==================== Chef Base ====================
FROM rust:${RUST_VERSION}-slim-bookworm AS chef
RUN cargo install cargo-chef
WORKDIR /app

# ==================== Planner ====================
FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
# cargo-chef needs a target file for cargo metadata; stub keeps cache stable.
RUN mkdir -p src && touch src/main.rs src/lib.rs
RUN cargo chef prepare --recipe-path recipe.json

# ==================== Builder ====================
FROM chef AS builder
ARG CARGO_BUILD_FEATURES
ARG ONNX_VERSION
ARG TARGETARCH

# Install build dependencies
# Note: webrtc-sys requires many system libraries for video/graphics support
# Using multiple mirror fallback strategies for ARM64 compatibility
RUN rm -rf /var/lib/apt/lists/* \
    && if [ -f /etc/apt/sources.list.d/debian.sources ]; then \
         sed -i 's|deb.debian.org/debian-security|security.debian.org/debian-security|g' /etc/apt/sources.list.d/debian.sources; \
       elif [ -f /etc/apt/sources.list ]; then \
         sed -i 's|deb.debian.org/debian-security|security.debian.org/debian-security|g' /etc/apt/sources.list; \
       fi \
    && apt-get update -o Acquire::Retries=5 -o Acquire::http::No-Cache=true -o Acquire::Check-Valid-Until=false \
    && apt-get install -y --no-install-recommends -o Acquire::Retries=5 \
       clang cmake pkg-config libssl-dev libzstd-dev ca-certificates curl git \
       libva-dev libdrm-dev libglib2.0-dev libgbm-dev \
       libx11-dev libxext-dev libxrandr-dev libxcomposite-dev libxdamage-dev libxfixes-dev \
    && rm -rf /var/lib/apt/lists/*

# Download ONNX Runtime (architecture-aware for multi-platform builds)
RUN case "${TARGETARCH}" in \
        amd64) ONNX_ARCH="x64" ;; \
        arm64) ONNX_ARCH="aarch64" ;; \
        *) echo "Unsupported architecture: ${TARGETARCH}" && exit 1 ;; \
    esac && \
    curl -L "https://github.com/microsoft/onnxruntime/releases/download/v${ONNX_VERSION}/onnxruntime-linux-${ONNX_ARCH}-${ONNX_VERSION}.tgz" -o onnxruntime.tgz && \
    tar -xzf onnxruntime.tgz && \
    mv "onnxruntime-linux-${ONNX_ARCH}-${ONNX_VERSION}" onnxruntime && \
    rm onnxruntime.tgz

# Build optimization flags
ENV RUSTFLAGS="-C opt-level=z -C link-arg=-s -C strip=symbols" \
    CARGO_PROFILE_RELEASE_LTO=thin \
    CARGO_NET_GIT_FETCH_WITH_CLI=true \
    ZSTD_SYS_USE_PKG_CONFIG=1 \
    CC=clang \
    CXX=clang++

# Cook dependencies (cached layer) - sharing=locked prevents cache corruption
COPY --from=planner /app/recipe.json recipe.json
COPY .cargo/config.toml .cargo/config.toml
RUN --mount=type=cache,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,target=/root/.cargo/git,sharing=locked \
    cargo chef cook --release ${CARGO_BUILD_FEATURES} --recipe-path recipe.json

# Build application
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN --mount=type=cache,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,target=/root/.cargo/git,sharing=locked \
    cargo build --release ${CARGO_BUILD_FEATURES} --locked

# ==================== Runtime Dependencies ====================
# Base image with all runtime dependencies for webrtc-sys
# Reused by both init and final runtime stages
FROM debian:bookworm-slim AS runtime-deps

# Install runtime dependencies
# Note: webrtc-sys links dynamically to several system libraries
# Using multiple mirror fallback strategies for ARM64 compatibility
RUN rm -rf /var/lib/apt/lists/* \
    && if [ -f /etc/apt/sources.list.d/debian.sources ]; then \
         sed -i 's|deb.debian.org/debian-security|security.debian.org/debian-security|g' /etc/apt/sources.list.d/debian.sources; \
       elif [ -f /etc/apt/sources.list ]; then \
         sed -i 's|deb.debian.org/debian-security|security.debian.org/debian-security|g' /etc/apt/sources.list; \
       fi \
    && apt-get update -o Acquire::Retries=5 -o Acquire::http::No-Cache=true -o Acquire::Check-Valid-Until=false \
    && apt-get install -y --no-install-recommends -o Acquire::Retries=5 \
       ca-certificates libssl3 libstdc++6 \
       libva2 libva-drm2 \
       libx11-6 libxext6 libxrandr2 libxcomposite1 libxdamage1 libxfixes3 \
       libglib2.0-0 libgbm1 libdrm2 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
ENV LD_LIBRARY_PATH=/app

# ==================== Init Stage ====================
# Runs sayna init to download models (requires shell)
FROM runtime-deps AS init
ARG RUN_SAYNA_INIT=true

COPY --from=builder /app/target/release/sayna /app/sayna
COPY --from=builder /app/onnxruntime/lib/*.so* /app/

ENV CACHE_PATH=/app/cache

RUN mkdir -p "$CACHE_PATH" \
    && if [ "${RUN_SAYNA_INIT}" = "true" ]; then /app/sayna init; fi

# ==================== Runtime ====================
# Final production image
FROM runtime-deps AS runtime

# Binary
COPY --from=builder /app/target/release/sayna /app/sayna

# ONNX Runtime shared libraries
COPY --from=builder /app/onnxruntime/lib/*.so* /app/

# Cached models from init stage
COPY --from=init /app/cache /app/cache

ENV RUST_LOG=info \
    PORT=3001 \
    CACHE_PATH=/app/cache

EXPOSE 3001
ENTRYPOINT ["/app/sayna"]
