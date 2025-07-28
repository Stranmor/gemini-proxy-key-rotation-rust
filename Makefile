# Makefile for Gemini Proxy Key Rotation
# This Makefile acts as a convenient wrapper around Docker Compose,
# ensuring all development and testing tasks run inside a consistent, containerized environment.

# Use .PHONY to declare targets that are not actual files.
# This prevents conflicts with files of the same name and improves performance.
.PHONY: all build run run-detached stop logs restart test lint coverage shell clean

# Default target that runs when `make` is called without arguments.
all: build run

# Build the Docker images for the services.
build:
	docker-compose build

# Start the services in the foreground.
run:
	docker-compose up

# Start the services in detached mode.
run-detached:
	docker-compose up -d

# Stop and remove the services.
stop:
	docker-compose down

# Follow the logs of the services.
logs:
	docker-compose logs -f

# Restart the services.
restart:
	docker-compose restart

# Run tests inside the container.
# Run tests using a dedicated Docker build stage.
test:
	docker build --target tester .

# Run the linter (clippy) using a dedicated Docker build stage.
lint:
	docker build --target linter .

# Generate the code coverage report using a dedicated Docker build stage.
coverage:
	docker build --target coverage_runner .
# Clean up Docker resources.
clean:
	docker-compose down -v --rmi all