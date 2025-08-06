# Gemini Proxy Key Rotation - Makefile
# Convenient commands for development and deployment

.PHONY: help install build test run clean docker-build docker-run docker-stop setup-config

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
run: build setup-config ## Run the proxy directly
	@echo "🚀 Starting Gemini Proxy..."
	@echo "📝 Make sure you've configured your API keys in config.yaml"
	RUST_LOG=info ./target/release/gemini-proxy-key-rotation-rust

run-dev: build-dev setup-config ## Run in development mode with debug logging
	@echo "🚀 Starting Gemini Proxy (dev mode)..."
	RUST_LOG=debug ./target/debug/gemini-proxy-key-rotation-rust

# Docker commands
docker-build: ## Build Docker image
	@echo "🐳 Building Docker image..."
	docker build -t gemini-proxy:latest .

docker-run: setup-config setup-env ## Start with Docker Compose
	@echo "🐳 Starting with Docker Compose..."
	@echo "📝 Make sure you've configured your API keys in config.yaml"
	docker-compose up -d
	@echo "✅ Services started!"
	@echo "🔗 Proxy: http://localhost:8081"
	@echo "📊 Health: http://localhost:8081/health"
	@echo "📋 Logs: make docker-logs"

docker-run-with-tools: setup-config setup-env ## Start with Docker Compose including Redis UI
	@echo "🐳 Starting with Docker Compose (with tools)..."
	docker-compose --profile tools up -d
	@echo "✅ Services started!"
	@echo "🔗 Proxy: http://localhost:8081"
	@echo "🔧 Redis UI: http://localhost:8082"

docker-stop: ## Stop Docker services
	@echo "🛑 Stopping Docker services..."
	docker-compose down

docker-restart: docker-stop docker-run ## Restart Docker services

docker-logs: ## Show Docker logs
	docker-compose logs -f

docker-clean: ## Clean up Docker resources
	@echo "🧹 Cleaning up Docker resources..."
	docker-compose down -v
	docker system prune -f

# Monitoring and Health
health: ## Check proxy health
	@echo "🏥 Checking proxy health..."
	@curl -s http://localhost:8081/health && echo "✅ Proxy is healthy" || echo "❌ Proxy is not responding"

health-detailed: ## Check detailed proxy health
	@echo "🏥 Checking detailed proxy health..."
	@curl -s http://localhost:8081/health/detailed | jq . || curl -s http://localhost:8081/health/detailed

status: ## Show service status
	@echo "📊 Service Status:"
	@echo "===================="
	@if docker-compose ps | grep -q "Up"; then \
		echo "🐳 Docker services:"; \
		docker-compose ps; \
	else \
		echo "🐳 Docker services: Not running"; \
	fi
	@echo ""
	@if pgrep -f "gemini-proxy-key-rotation-rust" > /dev/null; then \
		echo "🔧 Direct process: Running"; \
	else \
		echo "🔧 Direct process: Not running"; \
	fi

logs: ## Show application logs (auto-detect Docker or direct)
	@if docker-compose ps | grep -q "Up"; then \
		echo "📋 Showing Docker logs..."; \
		docker-compose logs -f gemini-proxy; \
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
	@echo "   2. Run 'make run' or 'make docker-run'"

quick-start: dev-setup ## Quick start for new users
	@echo ""
	@echo "🎉 Quick start complete!"
	@echo ""
	@echo "📝 IMPORTANT: Edit config.yaml and add your Gemini API keys"
	@echo "🚀 Then run: make docker-run"

# CI/CD helpers
ci-test: ## Run tests suitable for CI
	cargo test --all-features --no-fail-fast

ci-build: ## Build for CI
	cargo build --release --all-features