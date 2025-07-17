#!/bin/bash
# A script to build and run the Rust application using Podman or Docker.
set -e # Exit immediately if a command exits with a non-zero status.

# Auto-detect container runtime
if command -v podman &> /dev/null; then
    CONTAINER_RUNTIME="podman"
elif command -v docker &> /dev/null; then
    CONTAINER_RUNTIME="docker"
else
    echo "Error: Neither podman nor docker is installed."
    exit 1
fi
echo "Using $CONTAINER_RUNTIME as container runtime."


IMAGE_NAME="localhost/gemini-proxy-key-rotation:latest"
CONTAINER_NAME="gemini-proxy-container"

# Stop and remove the existing container if it's running
echo "Stopping and removing existing container..."
$CONTAINER_RUNTIME stop $CONTAINER_NAME >/dev/null 2>&1 || true
$CONTAINER_RUNTIME rm $CONTAINER_NAME >/dev/null 2>&1 || true

# Build the Docker image
echo "Building Docker image: $IMAGE_NAME"
$CONTAINER_RUNTIME build -t $IMAGE_NAME .

# Read port from config.yaml
PORT=$(grep 'port:' config.yaml | sed 's/.*: //')

if [ -z "$PORT" ]; then
    echo "Error: Port not found in config.yaml"
    exit 1
fi

echo "Starting container on port $PORT..."

# Run the new container
# The state file (key_states.json) is now ephemeral and will live only inside the container.
# This avoids all host filesystem permission issues.
$CONTAINER_RUNTIME run -d --name $CONTAINER_NAME \
    -e RUST_BACKTRACE=1 \
    -p $PORT:8080 \
    -v "$(pwd)/config.yaml:/app/config.yaml:ro" \
    $IMAGE_NAME

echo "Container $CONTAINER_NAME started successfully."