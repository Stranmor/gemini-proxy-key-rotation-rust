# Makefile for Gemini Proxy Key Rotation

# Use .PHONY to declare targets that are not actual files.
# This prevents conflicts with files of the same name and improves performance.
.PHONY: all build up down logs restart test lint clean

# Default target that runs when `make` is called without arguments.
all: build up

# Build the Docker images for the services.
build:
	docker compose build

# Start the services in detached mode.
up:
	docker compose up -d

# Stop and remove the services.
down:
	docker compose down

# Follow the logs of the services.
logs:
	docker compose logs -f

# Restart the services.
restart:
	docker compose restart

# Run the tests inside a new container.
# The `--rm` flag ensures the container is removed after the tests complete.
test:
	docker compose run --rm gemini-proxy cargo test

# Run the linter (clippy) inside a new container.
# `-- -D warnings` escalates all warnings to errors, ensuring high code quality.
lint:
	docker compose run --rm gemini-proxy cargo clippy -- -D warnings

# Clean up the project by removing the build artifacts.
clean:
	rm -rf target