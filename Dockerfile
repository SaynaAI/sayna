# syntax=docker/dockerfile:1
ARG RUST_VERSION=1.88.0
ARG CARGO_BUILD_FEATURES="--no-default-features --features turn-detect,noise-filter"
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
RUN apt-get update -o Acquire::Retries=5 -o Acquire::http::No-Cache=true \
    && apt-get install -y --no-install-recommends \
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

# Copy OpenSSL libraries to architecture-independent location for multi-arch builds
# On amd64: /usr/lib/x86_64-linux-gnu/, on arm64: /usr/lib/aarch64-linux-gnu/
RUN mkdir -p /app/lib && \
    case "${TARGETARCH}" in \
        amd64) LIBDIR="/usr/lib/x86_64-linux-gnu" ;; \
        arm64) LIBDIR="/usr/lib/aarch64-linux-gnu" ;; \
        *) echo "Unsupported architecture: ${TARGETARCH}" && exit 1 ;; \
    esac && \
    cp "${LIBDIR}/libssl.so.3" "${LIBDIR}/libcrypto.so.3" /app/lib/

# ==================== Init Stage ====================
# Separate stage to run sayna init (downloads models)
# This allows using distroless for final runtime
FROM debian:bookworm-slim AS init
ARG RUN_SAYNA_INIT=true
WORKDIR /app

# Install runtime dependencies
# Note: webrtc-sys links dynamically to several system libraries
RUN apt-get update -o Acquire::Retries=5 -o Acquire::http::No-Cache=true \
    && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 libstdc++6 \
    libva2 libva-drm2 \
    libx11-6 libxext6 libxrandr2 libxcomposite1 libxdamage1 libxfixes3 \
    libglib2.0-0 libgbm1 libdrm2 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/sayna /app/sayna
COPY --from=builder /app/onnxruntime/lib/*.so* /app/

ENV LD_LIBRARY_PATH=/app \
    CACHE_PATH=/app/cache

RUN mkdir -p "$CACHE_PATH" \
    && if [ "${RUN_SAYNA_INIT}" = "true" ]; then /app/sayna init; fi

# ==================== Runtime ====================
# Using distroless for minimal attack surface (~20MB vs ~74MB debian-slim)
# Includes glibc, libstdc++, and ca-certificates
FROM gcr.io/distroless/cc-debian12 AS runtime
WORKDIR /app

# Binary
COPY --from=builder /app/target/release/sayna /app/sayna

# ONNX Runtime shared libraries
COPY --from=builder /app/onnxruntime/lib/*.so* /app/

# OpenSSL libraries (not included in distroless, copied to /app/lib in builder for multi-arch)
COPY --from=builder /app/lib/libssl.so.3 /app/
COPY --from=builder /app/lib/libcrypto.so.3 /app/

# Cached models from init stage
COPY --from=init /app/cache /app/cache

ENV RUST_LOG=info \
    PORT=3001 \
    CACHE_PATH=/app/cache \
    LD_LIBRARY_PATH=/app

EXPOSE 3001
ENTRYPOINT ["/app/sayna"]
