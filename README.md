# 🚀 Gemini Proxy (Rust) - Work in Progress

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/Rust-1.70+-orange.svg)](https://rustup.rs/)
[![Docker](https://img.shields.io/badge/Docker-Ready-blue.svg)](https://hub.docker.com)

**STATUS: ACTIVE DEVELOPMENT / EXPERIMENTAL**

⚠️ **WARNING: This code is currently undergoing active refactoring and may contain bugs. Use at your own risk.**

## 1. The Problem

This project was created to solve a personal infrastructure challenge: bypassing the API rate limits of Google Gemini for my own R&D in large-scale AI agent systems.

## 2. The Solution (Architecture)

A high-performance, asynchronous proxy server written in Rust.
Designed for efficient key rotation to scale API requests.

## 3. Current State & Known Issues

The core logic is functional, but the latest commits have introduced several bugs that I am currently in the process of fixing.
This repository is a snapshot of my live R&D process. It is raw, unpolished, and reflects a work-in-progress.

## 4. Why am I sharing this?

- As a proof-of-concept for my system architecture skills.
- To demonstrate my ability to rapidly prototype complex, high-performance tools.

## ✨ Core Features (Working)

- 🔄 **Smart Key Rotation**: Round-robin with health-aware selection
- 🛡️ **Circuit Breaker**: Automatic failover protection
- 📊 **Health Monitoring**: Real-time key performance tracking
- 🔒 **Rate Limiting**: IP-based protection
- 🐳 **Docker Ready**: Optimized containers for deployment
- 🧪 **Comprehensive Tests**: 100+ tests covering core functionality

## 📚 Quick Navigation

- [🚀 Quick Start](#-quick-start)
- [🔧 Configuration](#-configuration)
- [🏗️ Architecture](#-architecture)
- [🧪 Testing](#-testing)
- [🐳 Docker Deployment](#-docker-deployment)
- [⚠️ Known Issues](#️-known-issues)

## 🎯 What This Proxy Does

### Core Functionality
- **API Key Rotation**: Automatically cycles through multiple Gemini API keys
- **Rate Limit Bypass**: Distributes requests across keys to avoid quotas
- **OpenAI Compatibility**: Drop-in replacement for OpenAI API endpoints
- **Health Monitoring**: Tracks key performance and automatically disables failing keys
- **Circuit Breaker**: Prevents cascade failures with automatic recovery

### Technical Architecture
- **Async Rust**: Built on Tokio for high-performance concurrent request handling
- **Smart Routing**: Health-aware key selection with round-robin fallback
- **State Persistence**: Optional Redis backend for distributed deployments
- **Comprehensive Logging**: Structured logging with request tracing
- **Docker Optimized**: Multi-stage builds with minimal runtime images (~50MB)

## 🏗️ Architecture

This is a high-performance async proxy built with Rust's Tokio runtime. The architecture is designed for scalability and reliability:

### Core Components

- **`main.rs`**: Application entry point with graceful shutdown handling
- **`key_manager.rs`**: Smart key rotation with health tracking
- **`proxy.rs`**: HTTP request forwarding with error handling
- **`circuit_breaker.rs`**: Automatic failover protection
- **`config/`**: YAML-based configuration with validation
- **`handlers/`**: Request processing pipeline
- **`storage/`**: Redis and in-memory state persistence

### Request Flow

```
Client → Axum Router → Key Manager → Circuit Breaker → Gemini API
   ↑                                                        ↓
   ← Response Handler ← Error Handler ← Health Monitor ←────┘
```

### Key Features

- **Async Processing**: Non-blocking I/O for high throughput
- **Health Scoring**: Real-time key performance metrics (0.0-1.0)
- **Automatic Recovery**: Failed keys re-enter rotation when healthy
- **State Persistence**: Survives restarts with Redis backend

## 🚀 Quick Start

### Prerequisites

- **Rust 1.70+**: Install from [rustup.rs](https://rustup.rs/)
- **Docker** (optional): For containerized deployment
- **Google Gemini API Keys**: Get them from [Google AI Studio](https://aistudio.google.com/app/apikey)

### Installation

```bash
# Clone the repository
git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git
cd gemini-proxy-key-rotation-rust

# Build the project
make build

# Set up configuration
make setup-config
# Edit config.yaml with your API keys
nano config.yaml
```

### Running the Proxy

**Option 1: Direct Binary**
```bash
make run
```

**Option 2: Docker (Recommended)**
```bash
make docker-run
```

The proxy will start on `http://localhost:4806` by default.

## 🔧 Configuration

### Basic Configuration

Edit `config.yaml` with your API keys:

```yaml
# config.yaml - Minimal setup
server:
  port: 4806

groups:
  - name: "default"
    target_url: "https://generativelanguage.googleapis.com/v1beta/openai/"
    api_keys:
      - "your-gemini-api-key-1"
      - "your-gemini-api-key-2"
      - "your-gemini-api-key-3"
```

### Advanced Configuration

```yaml
server:
  port: 4806
  admin_token: "your-secure-admin-token"  # For admin dashboard

# Redis for persistence (optional)
redis_url: "redis://localhost:6379"

# Circuit breaker settings
circuit_breaker:
  failure_threshold: 5
  recovery_timeout_secs: 60

# Rate limiting
max_failures_threshold: 3
temporary_block_minutes: 5
```

### Testing Your Setup

```bash
# Health check
curl http://localhost:4806/health

# Test chat completion
curl http://localhost:4806/v1/chat/completions \
  -H "Authorization: Bearer dummy-key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gemini-1.5-flash-latest",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

### Personal Persistent Development Container (for Active Development)

This method starts a single, persistent, and isolated container for your development work. It will not be affected by other agents or standard `make` commands.

1.  **Start the Container:**
    *   Run the following command. It will build the image and start a container with a unique name and a random, free port on your local machine.
    ```bash
    make start-dev
    ```

2.  **Check the Output:**
    *   The script will print the container ID and the exact address (e.g., `127.0.0.1:49155`) you can use to connect to your personal proxy.

## 🐳 Docker Deployment

### Quick Docker Setup

```bash
# Start with Docker Compose
make docker-run

# Development mode with hot-reload
make docker-run-dev

# With Redis UI and monitoring tools
make docker-run-with-tools
```

### Docker Commands

```bash
# Build optimized image
make docker-build

# View logs
make docker-logs

# Stop services
make docker-stop

# Clean up
make docker-clean
```

### End-to-End Testing

```bash
# Run comprehensive UAT
make uat
```

Expected result:
- Docker images build successfully
- Services start and pass health checks
- API endpoints respond correctly

## 🧪 Testing

### Running Tests

```bash
# Run all tests
make test

# Run with coverage
make test-coverage

# Run critical tests only
make test-critical
```

### Test Coverage

The project includes comprehensive tests covering:
- Core functionality (key rotation, health monitoring)
- Error handling and recovery
- Security features (rate limiting, authentication)
- Integration scenarios

## ⚠️ Known Issues

### Current Limitations

- **Admin Dashboard**: Web interface needs UI polish
- **Metrics Export**: Prometheus integration partially implemented
- **Documentation**: Some advanced features lack detailed docs
- **Error Recovery**: Some edge cases in circuit breaker logic

### Troubleshooting

**Health Check Failures:**
```bash
# Check container health
docker compose exec gemini-proxy ls -l /app/busybox

# Verify port availability
netstat -tulpn | grep 4806
```

**Port Conflicts:**
- Edit `server.port` in `config.yaml`
- Or set `PORT` environment variable
- Restart with `make docker-restart`

## 📡 API Reference

### Health Endpoints

```bash
# Basic health check
curl http://localhost:4806/health

# Detailed health with key validation
curl http://localhost:4806/health/detailed

# Prometheus metrics
curl http://localhost:4806/metrics
```

### Proxy Endpoints

All `/v1/*` requests are proxied to Gemini API:
- `/v1/chat/completions` - Chat completions
- `/v1/models` - List available models
- `/v1/embeddings` - Text embeddings

The proxy automatically:
1. Selects a healthy API key
2. Adds proper authentication headers
3. Forwards to Google Gemini API
4. Returns the response to client

## 🔧 Advanced Configuration

### Environment Variables

```bash
# Logging level
export RUST_LOG=info  # debug, info, warn, error

# Override config file location
export CONFIG_PATH=/path/to/config.yaml

# Redis connection (overrides config.yaml)
export REDIS_URL=redis://localhost:6379
```

### Circuit Breaker Settings

```yaml
circuit_breaker:
  failure_threshold: 5
  recovery_timeout_secs: 60
  success_threshold: 3
```

### Rate Limiting

```yaml
max_failures_threshold: 3
temporary_block_minutes: 5
```

## 📊 Monitoring & Performance

### Basic Monitoring

```bash
# View logs
make logs

# Check service status
make status

# Health check
make health
```

### Performance Notes

- **Throughput**: Handles 1000+ RPS on modest hardware
- **Memory Usage**: ~100MB base memory footprint
- **Latency**: <10ms proxy overhead
- **Key Switching**: Sub-millisecond key rotation

### Error Handling

The proxy handles Gemini API errors intelligently:
- **400/404**: Returns immediately (client error)
- **403**: Marks key as invalid, tries next key
- **429**: Temporarily disables key, retries with another
- **500/503**: Retries with same key, then switches

## 🛠️ Development

### Development Setup

```bash
# Set up development environment
make dev-setup

# Run in development mode
make run-dev

# Run tests
make test

# Code quality checks
make check  # Runs lint, format, and tests
```

### Available Commands

| Command | Purpose |
|---------|---------|
| `make build` | Build release binary |
| `make test` | Run all tests |
| `make format` | Format code with rustfmt |
| `make lint` | Run clippy linter |
| `make docker-build` | Build Docker image |

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- Built with [Rust](https://www.rust-lang.org/) and [Tokio](https://tokio.rs/)
- HTTP framework: [Axum](https://github.com/tokio-rs/axum)
- Redis integration: [deadpool-redis](https://github.com/bikeshedder/deadpool)
- Security: [secrecy](https://github.com/iqlusioninc/crates/tree/main/secrecy)

## 📚 Additional Resources

- [🏗️ Architecture Guide](ARCHITECTURE.md) - Detailed system design
- [📊 Monitoring Guide](MONITORING.md) - Observability setup
- [🤝 Contributing](CONTRIBUTING.md) - Development guidelines

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

Built with [Rust](https://www.rust-lang.org/) and [Tokio](https://tokio.rs/) for high-performance async processing.

---

**Note**: This is an experimental project reflecting active R&D work. The code is functional but may contain rough edges as it evolves.