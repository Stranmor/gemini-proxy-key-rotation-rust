#!/bin/bash
set -e

# Respect .dev.lock to avoid interfering with user's manual work
if [ -f ".dev.lock" ]; then
  echo "⛔ Environment is locked by user via .dev.lock. Aborting."
  exit 2
fi

echo "Building the Docker image for the dev container..."
# Build only if image missing to speed up cycles
if ! docker image inspect gemini-proxy-dev-image >/dev/null 2>&1; then
  docker build -t gemini-proxy-dev-image .
else
  echo "Image gemini-proxy-dev-image already exists. Skipping build."
fi

echo "Starting the persistent dev container..."
# Use -p 127.0.0.1::12345 to securely map to a random available port on the host,
# matching the port the application actually listens on.
NAME="gemini-proxy-dev-$(date +%s)"
CONTAINER_ID=$(docker run -d -p 127.0.0.1::12345 --restart unless-stopped --name "$NAME" -v "$(pwd)/config.yaml:/app/config.yaml:ro" -e "RUST_BACKTRACE=1" gemini-proxy-dev-image)

echo "Waiting for the container to start and port to be assigned..."
sleep 3

# Check the correct port (12345) for the mapping.
PORT_MAPPING=$(docker port "$CONTAINER_ID" 12345 || true)

echo "✅ Your personal dev container is running!"
echo "   Name: $NAME"
echo "   Container ID: ${CONTAINER_ID:0:12}"
echo "   Now listening on: $PORT_MAPPING"
# Try a quick health probe (best effort)
if command -v curl >/dev/null 2>&1; then
  host_port="$PORT_MAPPING"
  echo "   Health probe (best effort):"
  curl -fsS "http://${host_port}/health" || echo "   (health endpoint not ready yet)"
fi
echo "This container will restart automatically and will not be affected by other agents."