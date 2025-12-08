#!/bin/bash
set -e

echo "============================================="
echo "  Cross-Platform Build Verification Script"
echo "============================================="
echo ""
echo "NOTE: ARM64 Docker builds require QEMU emulation which is"
echo "      slow and can be unstable locally. The GitHub Actions"
echo "      workflow handles ARM64 builds reliably."
echo ""

# Test 1: Docker AMD64 build (fast, native)
echo "=== [1/3] Testing Docker AMD64 build ==="
docker buildx build --platform linux/amd64 -t saynaai/sayna:test-amd64 --load .
docker run --rm saynaai/sayna:test-amd64 --help
echo "✓ Docker AMD64 build successful"
echo ""

# Test 2: Binary x86_64 Linux (native cargo)
echo "=== [2/3] Testing binary x86_64-linux ==="
rustup target add x86_64-unknown-linux-gnu 2>/dev/null || true
cargo build --release --target x86_64-unknown-linux-gnu --no-default-features
echo "✓ Binary x86_64-linux build successful"
echo ""

# Test 3: Binary aarch64 Linux (using cross)
echo "=== [3/3] Testing binary aarch64-linux (via cross) ==="
if ! command -v cross &> /dev/null; then
    echo "Installing cross..."
    cargo install cross --git https://github.com/cross-rs/cross
fi
cross build --release --target aarch64-unknown-linux-gnu --no-default-features
echo "✓ Binary aarch64-linux build successful"
echo ""

echo "============================================="
echo "  All local builds passed!"
echo "============================================="
echo ""
echo "Skipped tests (run in GitHub Actions):"
echo "  - Docker ARM64 build (requires QEMU, slow/unstable locally)"
echo "  - macOS x86_64 and ARM64 builds (requires macOS runners)"
echo ""
echo "To test Docker ARM64 locally (slow, ~30+ min):"
echo "  docker run --rm --privileged multiarch/qemu-user-static --reset -p yes"
echo "  docker buildx build --platform linux/arm64 -t saynaai/sayna:test-arm64 ."
