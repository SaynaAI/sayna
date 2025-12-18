ARG RUST_VERSION=1.88.0
FROM debian:bookworm-slim AS builder

# ——— Build dependencies for release build ———
# Note: webrtc-sys requires many system libraries for video/graphics support
RUN apt-get update && \
    apt-get install -y \
    clang cmake pkg-config libssl-dev ca-certificates curl git \
    libva-dev libdrm-dev libglib2.0-dev libgbm-dev \
    libx11-dev libxext-dev libxrandr-dev libxcomposite-dev libxdamage-dev libxfixes-dev && \
    rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal

ENV PATH="/root/.cargo/bin:${PATH}"

# Fine-tuned, size-optimised build flags
ENV RUSTFLAGS="-C opt-level=z -C link-arg=-s -C strip=symbols"
# Enable ThinLTO for further optimisation
ENV CARGO_PROFILE_RELEASE_LTO=thin

WORKDIR /app

# Download ONNX Runtime (architecture-aware for multi-platform builds)
ARG ONNX_VERSION=1.23.2
ARG TARGETARCH
RUN case "${TARGETARCH}" in \
        amd64) ONNX_ARCH="x64" ;; \
        arm64) ONNX_ARCH="aarch64" ;; \
        *) echo "Unsupported architecture: ${TARGETARCH}" && exit 1 ;; \
    esac && \
    curl -L "https://github.com/microsoft/onnxruntime/releases/download/v${ONNX_VERSION}/onnxruntime-linux-${ONNX_ARCH}-${ONNX_VERSION}.tgz" -o onnxruntime.tgz && \
    tar -xzf onnxruntime.tgz && \
    mv "onnxruntime-linux-${ONNX_ARCH}-${ONNX_VERSION}" onnxruntime && \
    rm onnxruntime.tgz

# ---------- 1a. Cache dependencies ----------
# Create a dummy src so `cargo build` only fetches & compile deps.
COPY Cargo.toml Cargo.lock ./
COPY .cargo/config.toml .cargo/config.toml
RUN mkdir src && echo "fn main(){}" > src/main.rs
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
RUN cargo build --release --all-features --locked
RUN rm -rf src

# ---------- 1b. Build the real application ----------
COPY src ./src

RUN cargo build --release --all-features --locked

############################
# 2️⃣  Runtime stage
############################
FROM debian:bookworm-slim AS runtime

WORKDIR /app

# Install runtime dependencies
# Note: webrtc-sys links dynamically to several system libraries
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    libssl3 \
    libva2 libva-drm2 \
    libx11-6 libxext6 libxrandr2 libxcomposite1 libxdamage1 libxfixes3 \
    libglib2.0-0 libgbm1 libdrm2 && \
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
RUN mkdir -p "$CACHE_PATH" && /app/sayna init
ENTRYPOINT ["/app/sayna"]
