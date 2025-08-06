#!/bin/bash
# A script to build and run the Rust application using Podman or Docker.
set -e # Exit immediately if a command exits with a non-zero status.

# Respect .dev.lock to avoid interfering with user's manual work
if [ -f ".dev.lock" ]; then
  echo "⛔ Environment is locked by user via .dev.lock. Aborting."
  exit 2
fi

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

# --- Configuration Setup ---
CONFIG_FILE="config.yaml"
EXAMPLE_CONFIG_FILE="config.example.yaml"

# Check if config.yaml exists, if not, copy from example
if [ ! -f "$CONFIG_FILE" ]; then
    echo "Configuration file $CONFIG_FILE not found. Copying from $EXAMPLE_CONFIG_FILE..."
    cp "$EXAMPLE_CONFIG_FILE" "$CONFIG_FILE"
    echo "Please edit $CONFIG_FILE to add your Gemini API keys."
    # Optional: Interactive prompt for API keys
    # read -p "Enter your Gemini API Key (e.g., your-gemini-api-key-1): " API_KEY_1
    # sed -i "s/your-gemini-api-key-1/$API_KEY_1/" "$CONFIG_FILE"
    # echo "API key added to $CONFIG_FILE. You can add more keys by editing the file."
fi

IMAGE_NAME="localhost/gemini-proxy-key-rotation:latest"
CONTAINER_NAME="gemini-proxy-container"

# Stop and remove the existing container if it's running
echo "Stopping and removing existing container..."
$CONTAINER_RUNTIME stop $CONTAINER_NAME >/dev/null 2>&1 || true
$CONTAINER_RUNTIME rm $CONTAINER_NAME >/dev/null 2>&1 || true

# Check if the Docker image exists. If not, build it.
if [ -z "$($CONTAINER_RUNTIME images -q $IMAGE_NAME)" ]; then
    echo "Docker image $IMAGE_NAME not found. Building it now..."
    $CONTAINER_RUNTIME build -t $IMAGE_NAME .
else
    echo "Docker image $IMAGE_NAME already exists. Skipping build."
fi

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
    -p $PORT:$PORT \
    -v "$(pwd)/config.yaml:/app/config.yaml:ro" \
    $IMAGE_NAME

echo "Container $CONTAINER_NAME started successfully."