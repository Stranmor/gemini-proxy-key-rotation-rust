# Makefile for Gemini Proxy Key Rotation
# This Makefile acts as a convenient wrapper around Docker Compose,
# ensuring all development and testing tasks run inside a consistent, containerized environment.

# Use .PHONY to declare targets that are not actual files.
# This prevents conflicts with files of the same name and improves performance.
.PHONY: all build run run-detached stop logs restart test lint coverage shell clean

# Default target that runs when `make` is called without arguments.
all: build run

# Build the Docker images for the services.
# Build the Docker images for the services.
build:
	podman-compose build

# Start the services in the foreground.
run:
	podman-compose up

# Start the services in detached mode.
run-detached:
	podman-compose up -d

# Stop and remove the services.
stop:
	podman-compose down

# Follow the logs of the services.
logs:
	podman-compose logs -f

# Restart the services.
restart:
	podman-compose restart

# Run tests inside the container.
# This command builds the tester image (if needed) and runs the tests.
test:
	podman-compose run --rm tester

# Run the linter (clippy) using a dedicated Docker build stage.
lint:
	podman build --target linter .

# Generate the code coverage report using a dedicated Docker build stage.
coverage:
	# Запускаем сервис coverage_runner через docker-compose.
	# Флаг --rm удалит контейнер после выполнения.
	# Отчет будет сохранен в ./coverage_report благодаря привязке тома.
	podman-compose run --rm coverage_runner

# Clean up Docker resources.
clean:
	podman-compose down -v --rmi all