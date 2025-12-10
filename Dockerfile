ARG RUST_VERSION=1.88.0
FROM debian:bookworm-slim AS builder

# Target architecture for multi-arch builds
ARG TARGETARCH

# ——— Build dependencies for a static musl release ———
RUN apt-get update && \
    apt-get install -y \
    clang binutils cmake pkg-config libssl-dev ca-certificates curl libva-dev libdrm-dev && \
    rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal

ENV PATH="/root/.cargo/bin:${PATH}"

# Use clang for C/asm build scripts (ring) to avoid GCC ICEs under qemu on arm64
# Keep GNU binutils for ar/ranlib to avoid missing llvm-ar in slim images
ENV CC=clang CXX=clang++ AR=ar RANLIB=ranlib

# Fine-tuned, size-optimised build flags
# Avoid strip/linker flags here because binutils on arm builds crashes when combined with version scripts
ENV RUSTFLAGS="-C opt-level=z"
# Enable ThinLTO for further optimisation
ENV CARGO_PROFILE_RELEASE_LTO=thin

WORKDIR /app

# Download ONNX Runtime (architecture-specific)
ARG ONNX_VERSION=1.23.2
RUN ONNX_ARCH=$(case "${TARGETARCH:-amd64}" in \
    amd64) echo "x64" ;; \
    arm64) echo "aarch64" ;; \
    *) echo "x64" ;; \
    esac) && \
    curl -L "https://github.com/microsoft/onnxruntime/releases/download/v${ONNX_VERSION}/onnxruntime-linux-${ONNX_ARCH}-${ONNX_VERSION}.tgz" -o onnxruntime.tgz && \
    tar -xzf onnxruntime.tgz && \
    mv onnxruntime-linux-${ONNX_ARCH}-${ONNX_VERSION} onnxruntime && \
    rm onnxruntime.tgz

# ---------- 1a. Cache dependencies ----------
# Create a dummy src so `cargo build` only fetches & compile deps.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main(){}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

# ---------- 1b. Build the real application ----------
COPY src ./src

RUN cargo build --release --all-features

############################
# 2️⃣  Runtime stage
############################
FROM debian:bookworm-slim AS runtime

WORKDIR /app

# Install OpenSSL
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    libssl-dev && \
    rm -rf /var/lib/apt/lists/*

# Root certificates for HTTPS (reqwest/rustls)
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/

# The statically-linked binary
COPY --from=builder /app/target/release/sayna /app/sayna
COPY --from=builder /app/target/release/*.so /app/
COPY --from=builder /app/target/release/*.so.* /app/
COPY --from=builder /app/target/release/*.d /app/
COPY --from=builder /app/target/release/*.rlib /app/
COPY --from=builder /app/onnxruntime/lib/libonnxruntime.so* /app/

# Default logging level & port (override with -e if needed)
ENV RUST_LOG=info \
    PORT=3001 \
    CACHE_PATH=/app/cache

EXPOSE 3001
# Init will run at container start so we don't execute target binaries during cross-build
COPY docker-entrypoint.sh /app/docker-entrypoint.sh
RUN chmod +x /app/docker-entrypoint.sh
ENTRYPOINT ["/app/docker-entrypoint.sh"]
