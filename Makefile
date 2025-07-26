# Makefile for Gemini Proxy Key Rotation

# Use .PHONY to declare targets that are not actual files.
# This prevents conflicts with files of the same name and improves performance.
.PHONY: all build up down logs restart test lint clean start-dev install-coverage-tool test-coverage

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

# Run tests locally.
test:
	cargo test

# Run the linter (clippy) locally.
# `-- -D warnings` escalates all warnings to errors, ensuring high code quality.
lint:
	cargo clippy -- -D warnings

# Install the code coverage tool (cargo-tarpaulin) locally.
install-coverage-tool:
	cargo install cargo-tarpaulin --version 0.28.0

# Generate the code coverage report locally.
test-coverage:
	cargo tarpaulin --all-targets --workspace --out Html --output-dir ./coverage_report --skip-clean

# Clean up the project by removing the build artifacts.
clean:
	rm -rf target coverage_report

# --- Personal Developer Container ---

# Start a persistent, isolated development container with a unique name and random port.
start-dev:
	./start-dev-container.sh