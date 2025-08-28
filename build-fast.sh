#!/bin/bash

# Fast Docker build script for Gemini Proxy
set -e

echo "🚀 Starting optimized Docker build..."

# Use BuildKit for better caching and parallel builds
export DOCKER_BUILDKIT=1

# Build with optimized Dockerfile
docker build \
    --file Dockerfile.optimized \
    --target runtime \
    --build-arg RUST_VERSION=1.75 \
    --build-arg APP_NAME=gemini-proxy \
    --tag gemini-proxy:latest \
    --tag gemini-proxy:optimized \
    .

echo "✅ Build completed successfully!"
echo "📦 Image: gemini-proxy:latest"
echo "🏃 Run with: docker run -p 4806:4806 gemini-proxy:latest"