# Gemini Proxy Key Rotation - Makefile
# Convenient commands for development and deployment

.PHONY: help install build test run clean docker-build docker-run docker-stop setup-config lock unlock lock-check docker-logs-tail logs-tail docker-ps docker-health docker-up-quiet uat

# Default target
help: ## Show this help message
	@echo "Gemini Proxy Key Rotation - Available Commands:"
	@echo ""
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-20s\033[0m %s\n", $$1, $$2}'
	@echo ""
	@echo "Quick Start:"
	@echo "  1. make setup-config    # Create config.yaml from example"
	@echo "  2. Edit config.yaml     # Add your Gemini API keys"
	@echo "  3. make docker-run      # Start with Docker (recommended)"
	@echo "  or make run             # Run directly"

# Installation and Setup
install: ## Install dependencies and build the project
	@echo "🔧 Installing Rust dependencies..."
	cargo build --release
	@echo "✅ Installation complete!"

setup-config: ## Create config.yaml from example if it doesn't exist
	@if [ ! -f config.yaml ]; then \
		cp config.example.yaml config.yaml; \
		echo "📝 Created config.yaml from example"; \
		echo "⚠️  Please edit config.yaml and add your Gemini API keys!"; \
	else \
		echo "✅ config.yaml already exists"; \
	fi

setup-env: ## Create .env from example if it doesn't exist
	@if [ ! -f .env ]; then \
		cp .env.example .env; \
		echo "📝 Created .env from example"; \
	else \
		echo "✅ .env already exists"; \
	fi

# Development
build: ## Build the project in release mode
	@echo "🔨 Building project..."
	cargo build --release

build-dev: ## Build the project in development mode
	@echo "🔨 Building project (dev mode)..."
	cargo build

test: ## Run all tests
	@echo "🧪 Running tests..."
	cargo test

test-critical: ## Run only critical tests (security, monitoring, error handling)
	@echo "🧪 Running critical tests..."
	cargo test --test security_tests --test monitoring_tests --test error_handling_tests

test-coverage: ## Run tests with coverage (requires cargo-tarpaulin)
	@echo "📊 Running tests with coverage..."
	@if command -v cargo-tarpaulin >/dev/null 2>&1; then \
		cargo tarpaulin --out Html --output-dir coverage; \
		echo "📊 Coverage report generated in coverage/"; \
	else \
		echo "❌ cargo-tarpaulin not installed. Install with: cargo install cargo-tarpaulin"; \
	fi

lint: ## Run clippy linter
	@echo "🔍 Running clippy..."
	cargo clippy -- -D warnings

format: ## Format code with rustfmt
	@echo "🎨 Formatting code..."
	cargo fmt

check: lint format test ## Run all checks (lint, format, test)

# Running
run: build setup-config ## Run the proxy directly (foreground)
	@echo "🚀 Starting Gemini Proxy..."
	@echo "📝 Make sure you've configured your API keys in config.yaml"
	RUST_LOG=info ./target/release/gemini-proxy

run-dev: build-dev setup-config ## Run in development mode with debug logging (foreground)
	@echo "🚀 Starting Gemini Proxy (dev mode)..."
	RUST_LOG=debug ./target/debug/gemini-proxy

run-dev-bg: build-dev setup-config ## Run in development mode (background, logs to /tmp/gemini-dev.log)
	@echo "🚀 Starting Gemini Proxy (dev mode, background)..."
	@( (RUST_LOG=debug ./target/debug/gemini-proxy > /tmp/gemini-dev.log 2>&1 & echo $$! > /tmp/gemini-dev.pid) && echo "PID: $$(cat /tmp/gemini-dev.pid); log: /tmp/gemini-dev.log" )

# Docker commands
docker-build: ## Build optimized Docker image
	@echo "🐳 Building optimized Docker image..."
	docker build --target runtime -t gemini-proxy:latest .
	@echo "✅ Build complete!"

docker-build-dev: ## Build development Docker image
	@echo "🐳 Building development Docker image..."
	docker build --target development -t gemini-proxy:dev .

docker-run: lock-check setup-config setup-env ## Start with Docker Compose (production)
	@echo "🐳 Starting with Docker Compose (production)..."
	@echo "📝 Make sure you've configured your API keys in config.yaml"
	docker compose up -d
	@echo "✅ Services started!"
	@echo "🔗 Proxy: http://localhost:4806"
	@echo "📊 Health: http://localhost:4806/health"
	@echo "📋 Logs: make docker-logs"

docker-run-dev: lock-check setup-config setup-env ## Start development environment
	@echo "🐳 Starting development environment..."
	docker compose --profile dev up -d
	@echo "✅ Development environment started!"
	@echo "🔗 Proxy: http://localhost:4807"
	@echo "📊 Health: http://localhost:4807/health"

docker-run-with-tools: lock-check setup-config setup-env ## Start with Redis UI and monitoring tools
	@echo "🐳 Starting with monitoring tools..."
	docker compose --profile tools up -d
	@echo "✅ Services with tools started!"
	@echo "🔗 Proxy: http://localhost:4806"
	@echo "🔧 Redis UI: http://localhost:8082"

docker-test: ## Run tests in Docker
	@echo "🧪 Running tests in Docker..."
	docker compose --profile test run --rm test-runner

docker-coverage: ## Generate coverage report in Docker
	@echo "📊 Generating coverage report..."
	docker compose --profile coverage run --rm coverage-runner
	@echo "📊 Coverage report generated in coverage_report/"

docker-stop: lock-check ## Stop Docker services
	@echo "🛑 Stopping Docker services..."
	docker compose down

docker-restart: lock-check docker-stop docker-run ## Restart Docker services

docker-logs: ## Show Docker logs (follow - blocking)
	docker compose logs -f gemini-proxy

docker-logs-tail: ## Show last 200 lines of Docker logs (non-blocking)
	docker compose logs --since=5m gemini-proxy | tail -n 200 || true

docker-logs-all: ## Show all Docker logs (follow - blocking)
	docker compose logs -f

docker-clean: lock-check ## Clean up Docker resources
	@echo "🧹 Cleaning up Docker resources..."
	docker compose down -v --remove-orphans
	docker system prune -f

docker-clean-all: lock-check ## Clean up all Docker resources including images
	@echo "🧹 Cleaning up all Docker resources..."
	docker compose down -v --remove-orphans
	docker system prune -af
	docker volume prune -f

# Monitoring and Health
health: ## Check proxy health
	@echo "🏥 Checking proxy health..."
	@curl -s http://localhost:4806/health && echo "✅ Proxy is healthy" || echo "❌ Proxy is not responding"

health-detailed: ## Check detailed proxy health
	@echo "🏥 Checking detailed proxy health..."
	@curl -s http://localhost:4806/health/detailed | jq . || curl -s http://localhost:4806/health/detailed

status: ## Show service status
	@echo "📊 Service Status:"
	@echo "===================="
	@if docker compose ps | grep -q "Up"; then \
		echo "🐳 Docker services:"; \
		docker compose ps; \
	else \
		echo "🐳 Docker services: Not running"; \
	fi
	@echo ""
	@if pgrep -f "gemini-proxy" > /dev/null; then \
		echo "🔧 Direct process: Running"; \
	else \
		echo "🔧 Direct process: Not running"; \
	fi

logs: ## Show application logs (auto-detect Docker or direct, follow - blocking)
	@if docker compose ps | grep -q "Up"; then \
		echo "📋 Showing Docker logs..."; \
		docker compose logs -f gemini-proxy; \
	else \
		echo "📋 No Docker services running. Use 'journalctl -f' for systemd logs"; \
	fi

# Maintenance
clean: ## Clean build artifacts
	@echo "🧹 Cleaning build artifacts..."
	cargo clean

update: ## Update dependencies
	@echo "📦 Updating dependencies..."
	cargo update

backup-config: ## Backup current configuration
	@echo "💾 Backing up configuration..."
	@timestamp=$$(date +%Y%m%d_%H%M%S); \
	cp config.yaml "config.yaml.backup.$$timestamp" && \
	echo "✅ Configuration backed up to config.yaml.backup.$$timestamp"

# Security
security-scan: ## Run security audit
	@echo "🔒 Running security audit..."
	@if command -v cargo-audit >/dev/null 2>&1; then \
		cargo audit; \
	else \
		echo "❌ cargo-audit not installed. Install with: cargo install cargo-audit"; \
	fi

generate-admin-token: ## Generate a secure admin token
	@echo "🔐 Generated admin token:"
	@openssl rand -hex 32 || echo "❌ openssl not available. Use any secure random string generator."

# Development helpers
dev-setup: ## Complete development setup
	@echo "🛠️ Setting up development environment..."
	@make setup-config
	@make setup-env
	@make install
	@echo "✅ Development environment ready!"
	@echo "📝 Next steps:"
	@echo "   1. Edit config.yaml with your API keys"
	@echo "   2. Run 'make run' or 'make docker-run' (add 'lock' to protect your environment during manual work)"

quick-start: dev-setup ## Quick start for new users
	@echo ""
	@echo "🎉 Quick start complete!"
	@echo ""
	@echo "📝 IMPORTANT: Edit config.yaml and add your Gemini API keys"
	@echo "🚀 Then run: make docker-run"
	@echo "🔒 Tip: Use 'make lock' to place a .dev.lock while you are debugging manually"

# Lock/Unlock to protect user's environment
lock: ## Create .dev.lock to prevent destructive operations
	@touch .dev.lock && echo "🔒 .dev.lock created"

unlock: ## Remove .dev.lock to allow operations
	@if [ -f .dev.lock ]; then rm .dev.lock && echo "🔓 .dev.lock removed"; else echo "ℹ️ .dev.lock not present"; fi

lock-check: ## Guard: fail if .dev.lock exists
	@if [ -f .dev.lock ]; then echo "⛔ Environment is locked by user via .dev.lock. Operation aborted."; exit 2; fi

# Helpful non-blocking helpers
docker-ps: ## Show docker compose ps
	docker compose ps

docker-health: ## Print container health for gemini-proxy
	@ID=$$(docker compose ps -q gemini-proxy); \
	if [ -n "$$ID" ]; then docker inspect "$$ID" --format 'Status={{.State.Status}} Health={{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}'; else echo "Container not found"; fi

docker-up-quiet: lock-check ## docker compose up -d with short status
	@docker compose up -d >/dev/null 2>&1 || true
	@echo "✅ compose up -d issued"; \
	ID=$$(docker compose ps -q gemini-proxy); \
	if [ -n "$$ID" ]; then docker inspect "$$ID" --format 'Status={{.State.Status}} Health={{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}'; fi

# CI/CD helpers
ci-test: ## Run tests suitable for CI
	cargo test --all-features --no-fail-fast

ci-build: ## Build for CI
	cargo build --release --all-features

# UAT: build, up, wait for health, test endpoints (non-interactive)
uat: lock-check setup-config setup-env ## Run end-to-end UAT verification (non-interactive)
	@echo "🧪 UAT: building images..."
	@( (docker compose build > /tmp/uat_build.log 2>&1 & echo $$! > /tmp/uat_build.pid) && while [ -f /tmp/uat_build.pid ] && kill -0 $$(cat /tmp/uat_build.pid) 2>/dev/null; do sleep 2; done; true )
	@echo "🧪 UAT: starting services..."
	@docker compose up -d >/tmp/uat_up.log 2>&1 || true
	@echo "🧪 UAT: waiting for health (up to 90s)..."
	@i=0; ok=0; \
	while [ $$i -lt 90 ]; do \
	  if curl -fsS http://localhost:4806/health >/dev/null 2>&1; then ok=1; break; fi; \
	  sleep 1; i=$$((i+1)); \
	done; \
	if [ $$ok -ne 1 ]; then \
	  echo "❌ UAT: health endpoint not responding on :4806 within 90s"; \
	  echo "Last logs:"; docker compose logs --since=2m gemini-proxy | tail -n 200 || true; \
	  exit 1; \
	fi
	@echo "✅ UAT: /health OK on :4806"
	@echo "🧪 UAT: optional API probe (models)"
	@curl -fsS http://localhost:4806/v1/models >/dev/null 2>&1 || true
	@echo "✅ UAT completed"