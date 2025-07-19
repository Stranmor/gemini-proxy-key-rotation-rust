#!/bin/bash
set -e

echo "Building the Docker image for the dev container..."
docker build -t gemini-proxy-dev-image .

echo "Starting the persistent dev container..."
# Use -p 127.0.0.1::12345 to securely map to a random available port on the host,
# matching the port the application actually listens on.
CONTAINER_ID=$(docker run -d -p 127.0.0.1::12345 --restart unless-stopped --name "gemini-proxy-dev-$(date +%s)" -v "$(pwd)/config.yaml:/app/config.yaml:ro" -e "RUST_BACKTRACE=1" gemini-proxy-dev-image)

echo "Waiting for the container to start and port to be assigned..."
sleep 3

# Check the correct port (12345) for the mapping.
PORT_MAPPING=$(docker port "$CONTAINER_ID" 12345)

echo "✅ Your personal dev container is running!"
echo "   Container ID: ${CONTAINER_ID:0:12}"
echo "   Now listening on: $PORT_MAPPING"
echo "This container will restart automatically and will not be affected by other agents."