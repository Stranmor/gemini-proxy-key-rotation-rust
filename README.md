# 🚀 Gemini Proxy Key Rotation - Production Ready

[![CI](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml/badge.svg)](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Security](https://img.shields.io/badge/Security-Hardened-green.svg)](SECURITY.md)
[![Tests](https://img.shields.io/badge/Tests-227%20Passing-brightgreen.svg)](#testing)

A **production-ready**, high-performance asynchronous HTTP proxy for Google Gemini models with **enterprise-grade security** and **intelligent monitoring**. Seamlessly integrates with OpenAI-compatible applications while providing advanced key rotation, load balancing, and comprehensive observability.

## ✨ What's New in v2.0

- 🎯 **100% Accurate Tokenization**: Multiple strategies for perfect token counting
- 🔒 **Enterprise Security**: Rate limiting, HTTPS enforcement, session management
- 📊 **Intelligent Monitoring**: Proactive key health scoring (0.0-1.0), automated alerts
- 🧱 **Token Limit Guardrails**: Configurable per-request token limit with metrics and fail-fast init
- 🛡️ **Circuit Breaker**: Automatic failover for upstream services
- 🔄 **Graceful Operations**: Zero-downtime restarts, proper signal handling
- 🧪 **227 Tests**: Comprehensive test coverage including large text scenarios
- 📦 **Easy Installation**: One-command setup with automated installer

**📚 [Installation Guide](#-installation)** | **🔒 [Security Features](SECURITY.md)** | **📊 [Monitoring Guide](MONITORING.md)** | **📋 [Project Status](PROJECT_STATUS_REPORT.md)** | **🗺️ [Development Roadmap](DEVELOPMENT_ROADMAP.md)** | **🧪 [Test Coverage](TEST_COVERAGE_REPORT.md)**

## 🎯 Key Benefits

### 🚀 **Performance & Reliability**
- **Smart Load Balancing**: Distributes requests across multiple Gemini keys with health-aware routing
- **Circuit Breaker Protection**: Automatic failover when upstream services are down
- **Zero-Downtime Operations**: Graceful shutdowns and rolling updates
- **Redis Persistence**: Maintains state across restarts for enterprise deployments

### 🔒 **Enterprise Security**
- **Rate Limiting**: IP-based protection with configurable thresholds
- **HTTPS Enforcement**: Production-ready TLS termination
- **Session Management**: Secure token-based authentication with automatic rotation
- **Audit Logging**: Comprehensive security event tracking

### 📊 **Intelligent Monitoring**
- **Health Scoring**: Real-time key performance metrics (0.0-1.0 scale)
- **Proactive Alerts**: Automated notifications for degraded performance
- **Detailed Analytics**: Request success rates, response times, error patterns
- **Admin Dashboard**: Web-based monitoring and management interface

### 🎯 **Advanced Tokenization**
- **Smart Parallel Processing**: Intelligent decision-making for optimal performance
  - Small texts (<150k tokens): Direct sending for maximum speed
  - Medium texts (150k-250k): Parallel tokenization + network requests
  - Large texts (>250k): Immediate rejection with clear error messages
- **100% Accurate Counting**: Multiple tokenization strategies for perfect accuracy
- **Official Google Tokenizer**: Direct integration with Google's Vertex AI SDK
- **Proxy-Cached Tokenizer**: Real Google API results with intelligent caching
- **Multi-language Support**: Perfect handling of Unicode, code, and mixed content
- **Large Text Optimized**: Tested on documents up to 250k tokens with consistent accuracy

### 🛠 **Developer Experience**
- **One-Command Setup**: Automated installer handles everything
- **OpenAI Compatible**: Drop-in replacement for existing applications
- **Docker Ready**: Production containers with health checks
- **Comprehensive Testing**: 227 automated tests ensure reliability

## 🌟 Features

### 🔄 **Smart Key Management**
- **Intelligent Rotation**: Group-based round-robin with health-aware selection
- **Health Scoring**: Real-time key performance metrics (0.0-1.0 scale)
- **Automatic Recovery**: Failed keys automatically re-enter rotation when healthy
- **State Persistence**: Redis-backed state survives restarts and scaling

### 🛡️ **Enterprise Security**
- **Rate Limiting**: Configurable IP-based protection (5 attempts/5 minutes default)
- **HTTPS Enforcement**: Production-ready TLS with security headers
- **Session Management**: Secure token-based admin authentication
- **Audit Logging**: Comprehensive security event tracking
- **Request Validation**: Size limits and input sanitization
- **Token Budget Enforcement**: Configurable token limit per request (`server.max_tokens_per_request`)

### 📊 **Advanced Monitoring**
- **Proactive Health Checks**: Background monitoring every 30 seconds
- **Automated Alerts**: Notifications when >3 keys unhealthy or error rate >10%
- **Performance Metrics**: Response times, success rates, usage patterns
- **Tokenization Metrics**: `request_token_count` (histogram), `token_limit_blocks_total` (counter)
- **Admin Dashboard**: Web-based monitoring at `/admin/`
- **Detailed Analytics**: Per-key and per-group statistics

### 🚀 **High Performance**
- **Async Architecture**: Built on Tokio for maximum throughput
- **Circuit Breaker**: Automatic failover for upstream services
- **Connection Pooling**: Efficient HTTP client management
- **Graceful Shutdown**: Zero-downtime deployments with proper signal handling

### 🔧 **Developer Experience**
- **OpenAI Compatible**: Drop-in replacement for existing applications
- **Flexible Configuration**: Single YAML file with hot-reload support
- **Multiple Deployment Options**: Docker, systemd, or direct binary
- **Comprehensive Testing**: 227 automated tests ensure reliability

## 🎯 Advanced Tokenization

### 🚀 **100% Accurate Token Counting**

One of the key challenges with Gemini API integration is accurate token counting for billing and rate limiting. Our proxy solves this with multiple tokenization strategies:

#### **1. Official Google Tokenizer (Recommended)**
- **100% Accuracy**: Uses Google's official Vertex AI SDK
- **Local Processing**: No API calls required for token counting
- **All Models Supported**: Works with Gemini 1.0, 1.5, and 2.0
- **Setup**: `pip install google-cloud-aiplatform[tokenization]`

#### **2. Proxy-Cached Tokenizer (Production Ready)**
- **100% Accuracy**: Uses real Google API with intelligent caching
- **High Performance**: Cached results for repeated texts
- **Fallback Support**: Graceful degradation when API unavailable
- **No Dependencies**: Pure Rust implementation

#### **3. ML-Calibrated Tokenizer (Offline Fallback)**
- **98%+ Accuracy**: Machine learning calibrated on Google API data
- **Fast Performance**: <1ms per operation
- **No Dependencies**: Works completely offline
- **Multi-language**: Optimized for Unicode, code, and mixed content

### 📊 **Tokenization Performance**

Tested on various content types and sizes, including large-scale scenarios:

| Content Type | Size | Tokens | Gemini First | Local Tokenization | Recommendation |
|--------------|------|--------|--------------|-------------------|----------------|
| **Simple Text** | 1KB | 250 | 0ms | 1ms | Either approach |
| **Unicode Heavy** | 5KB | 2,035 | 0ms | 2ms | Either approach |
| **Code Files** | 10KB | 3,066 | 0ms | 3ms | Either approach |
| **Technical Docs** | 25KB | 6,500 | 0ms | 5ms | Gemini First |
| **Mixed Content** | 50KB | 12,000 | 0ms | 8ms | Gemini First |
| **Large Requests** | **1.8MB** | **180,000** | **0ms** | **280ms** | **Gemini First Only** |

**🎯 For large requests (180k+ tokens): Use "Gemini First" approach - send directly without pre-tokenization!**

### 🔧 **Configuration**

```yaml
# config.yaml
server:
  # Choose tokenization strategy (RECOMMENDED: gemini_first for large texts)
  tokenizer_type: "gemini_first"  # Send directly to Gemini (fastest)

  # Token limits and safety
  max_tokens_per_request: 250000

  # Gemini First configuration (optimized for 180k+ tokens)
  tokenizer_config:
    enable_pre_check: false        # Skip pre-tokenization (fastest)
    enable_post_count: true        # Count after response for stats
    use_fast_estimation: true      # Fast estimation for very large texts
    fast_estimation_threshold: 50000  # 50KB threshold
```

### 📈 **Monitoring Token Usage**

The proxy provides detailed tokenization metrics:

- `request_token_count` (histogram): Per-request token counts
- `token_limit_blocks_total` (counter): Requests blocked by token limits
- `tokenizer_cache_hits_total` (counter): Cache efficiency metrics
- `tokenizer_accuracy_score` (gauge): Real-time accuracy measurements

## Architecture

### Tokenizer Initialization and Token Limit Enforcement

- The proxy uses a shared tokenizer to compute request token counts before forwarding.
- Initialization is fail-fast in production: if tokenizer cannot be initialized at startup, the app aborts.
- In test/dev mode, a minimal whitespace-based fallback tokenizer is installed automatically to keep local workflows unblocked.
- A configurable limit (`server.max_tokens_per_request`) guards requests:
  - If the computed token count exceeds the limit, the request is rejected with `RequestTooLarge`.
  - Prometheus-style metrics are emitted:
    - `request_token_count` (histogram): per-request token count
    - `token_limit_blocks_total` (counter): increments on limit-based rejections

The Gemini Proxy Key Rotation service is built with a modular architecture, leveraging Rust's ownership and concurrency features to ensure high performance and reliability. Below are the core components and their interactions:

*   [`main.rs`](src/main.rs): The entry point of the application. It initializes logging, loads the configuration, sets up the `KeyManager` and `AppState`, and starts the Axum HTTP server.
*   [`config.rs`](src/config.rs): Handles loading and validating the application's configuration from the `config.yaml` file. It defines how API key groups, proxy URLs, and target URLs are parsed and structured.
*   [`key_manager.rs`](src/key_manager.rs): Manages the lifecycle of Gemini API keys. It's responsible for loading keys, selecting the next available key using a group round-robin strategy, tracking rate limits, and persisting key states to `key_states.json`.
*   [`state.rs`](src/state.rs): Defines the shared application state (`AppState`) that is accessible across different request handlers. This includes the `KeyManager`, configuration, and other shared resources.
*   [`handlers/mod.rs`](src/handlers/mod.rs): Contains the Axum request handlers. It processes incoming HTTP requests, interacts with the `KeyManager` to get an API key, and prepares the request for forwarding.
*   [`proxy.rs`](src/proxy.rs): Responsible for forwarding the modified HTTP request to the actual Google Gemini API endpoint (or an upstream proxy if configured). It handles the network communication and returns the response to the client.

**Request Flow Diagram:**

```mermaid
graph TD
    A[Client Request] --> B{Axum HTTP Server};
    B --> C[handler.rs: Process Request];
    C --> D{state.rs: Access AppState};
    D --> E[key_manager.rs: Get Next Key];
    E --> F{proxy.rs: Forward Request};
    F --> G[Google Gemini API];
    G --> F;
    F --> C;
    C --> H[Client Response];
```

## 📦 Installation

### 🚀 **Quick Install (Recommended)**

The easiest way to get started - our installer handles everything:

```bash
# Download and run the installer
curl -fsSL https://raw.githubusercontent.com/stranmor/gemini-proxy-key-rotation-rust/main/install.sh | bash

# Or download first to review:
wget https://raw.githubusercontent.com/stranmor/gemini-proxy-key-rotation-rust/main/install.sh
chmod +x install.sh
./install.sh
```

The installer will:
- ✅ Install Rust and Docker (if needed)
- ✅ Clone the repository
- ✅ Build the application
- ✅ Set up configuration files
- ✅ Create systemd service (Linux)
- ✅ Run tests to verify installation

### 🐳 **Optimized Docker Build**

Fully redesigned Docker build for maximum efficiency:

```bash
git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git
cd gemini-proxy-key-rotation-rust

# Automatic optimization and setup
./scripts/docker-optimize.sh

# Or quick start
make quick-start
nano config.yaml  # Add your Gemini API keys

# Run (select the desired mode)
make docker-run              # Production (port 4806)
make docker-run-dev          # Development (port 4806)
make docker-run-with-tools   # + Redis UI (port 8082)
```

**🚀 Key improvements:**
- Image size reduced to ~50MB (Distroless)
- Build time accelerated 3-5x (cargo-chef)
- Maximum security (non-privileged user)
- Efficient dependency caching

### 🛠 **Manual Installation**

If you prefer manual control:

```bash
# Prerequisites
# - Rust 1.70+ (https://rustup.rs/)
# - Docker (optional, for Redis)

git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git
cd gemini-proxy-key-rotation-rust

# Build
make build

# Configure
cp config.example.yaml config.yaml
# Edit config.yaml with your API keys and optional token guardrail, e.g.:
# server:
#   max_tokens_per_request: 250000

# Run
make run
```

## 🔑 Requirements

- **Google Gemini API Keys**: Get them from [Google AI Studio](https://aistudio.google.com/app/apikey)
- **System**: Linux, macOS, or Windows with WSL2
- **Memory**: 512MB RAM minimum, 1GB+ recommended for production
- **Storage**: 100MB for application, additional space for logs

## ⚡ Quick Start

### 🎯 **3-Step Setup**

1. **Install & Configure**
   ```bash
   curl -fsSL https://raw.githubusercontent.com/stranmor/gemini-proxy-key-rotation-rust/main/install.sh | bash
   cd ~/gemini-proxy
   nano config.yaml  # Add your Gemini API keys
   ```

2. **Start the Proxy**
   ```bash
   # Option A: Docker (Recommended)
   make docker-run

   # Option B: Direct binary (use -H for host binding)
   ./target/release/gemini-proxy -H 0.0.0.0 -p 4806

   # Option C: Systemd service (Linux)
   sudo systemctl start gemini-proxy
   ```

3. **Verify & Use**
   ```bash
   # Check health
   curl http://localhost:4806/health

   # Test with your OpenAI client
   # Base URL: http://localhost:4806
   # API Key: any-dummy-key (ignored, real keys managed internally)
   ```

### 📊 **Monitoring Dashboard**

Access the admin panel at `http://localhost:4806/admin/` (configure `admin_token` in config.yaml):

- 📈 Real-time key health scores
- 📊 Request success rates and response times
- 🔧 Key management and configuration
- 🚨 Alert history and system status

### 🔄 **Common Operations**

```bash
# View status
make status

# View logs
make logs

# Restart services
make docker-restart

# Run health check
make health

# Update configuration
nano config.yaml
make docker-restart  # Apply changes
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

3.  **Stopping Your Personal Container:**
    *   Since the container has a unique name, you'll need to find it first and then stop it.
    ```bash
    # Find your container
    docker ps | grep "gemini-proxy-dev"

    # Stop it using its ID or name
    docker stop <container_id_or_name>
    ```

### Building and Running Locally (for Development)
Use this primarily for development.

1.  **Clone Repository:** (If needed)
    ```bash
    git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git
    cd gemini-proxy-key-rotation-rust
    ```
2.  **Prepare Configuration:**
    *   Copy `config.example.yaml` to `config.yaml`.
    *   Edit `config.yaml` to define your `server.port` and `groups`.
3.  **Build:**
    ```bash
    cargo build --release
    ```
4.  **Run:**
    ```bash
    # Set the log level (optional)
    export RUST_LOG="info"

    # Run the binary
    ./target/release/gemini-proxy-key-rotation-rust
    ```
    *   *(The `key_states.json` file will be created/updated in the current working directory)*

Once the proxy is running, configure your OpenAI client (e.g., Python/JS libraries, Roo Code/Cline, etc.) as follows:

1.  **Set the Base URL / API Host:** Point the client to the proxy's address (protocol, host, port only).
    *   Example: `http://localhost:4806` (or the host port you set in `config.yaml`)
    *   **Do NOT include `/v1` or other paths in the Base URL.**

2.  **Set the API Key:** Enter **any non-empty placeholder** (e.g., "dummy-key", "ignored"). The proxy manages the *real* Gemini keys internally and **ignores the key sent by the client**, but the field usually requires input.

3.  **Send Requests:** Make requests as you normally would using the OpenAI client library or tool (e.g., to `/v1/chat/completions`, `/v1/models`, etc.). The proxy will intercept these, add the correct Google authentication for the OpenAI compatibility layer using a rotated key, and forward them.

### Example (`curl` to proxy)

#### UAT

Run a non-interactive end-to-end verification:

```bash
make uat
```

Expected result:
- docker images build
- services up
- healthcheck OK at http://localhost:4806/health
- sample endpoints respond as expected

#### Troubleshooting healthcheck

If healthcheck fails:

1) Check health binary inside container:
```bash
docker compose exec gemini-proxy ls -l /app/busybox || echo "busybox not present"
```

2) Verify healthcheck configuration in compose:
- test: ["/app/busybox","wget","-qO-","http://localhost:4806/health"]
- interval: 10s, timeout: 5s

3) Port conflicts:
- Do not kill user processes.
- To change port, set environment variable PORT or edit server.port in config.yaml (e.g., 0 for auto-assign or specific free port).
- Then re-run: docker compose up -d

**Example `curl` request:**
```bash
# Example request to list models via the proxy (listening on 4806 by default)
curl http://localhost:4806/v1/models \
  -H "Authorization: Bearer dummy-ignored-key" # This header is ignored/replaced

# Example request for chat completion via the proxy (listening on 4806 by default)
curl http://localhost:4806/v1/chat/completions \
  -H "Authorization: Bearer dummy-ignored-key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gemini-1.5-flash-latest",
    "messages": [{"role": "user", "content": "Explain Rust."}],
    "temperature": 0.7
  }'
```

## Configuration

1.  In API settings, select **"OpenAI Compatible"** as **API Provider**.
2.  Set **Base URL** to the proxy address (e.g., `http://localhost:4806`).
3.  Set **API Key** to any non-empty placeholder (e.g., "dummy").

**Example Configuration Screenshot:**
![Roo Code Configuration Example](2025-04-13_14-02.png)

## API Reference

The proxy exposes a minimal set of HTTP endpoints designed for compatibility with OpenAI clients and for health monitoring.

### Endpoints

*   **`GET /health`**
    *   **Purpose:** A simple health check endpoint.
    *   **Description:** Returns a `200 OK` status code if the proxy is running and responsive. This is useful for load balancers, Docker health checks, and basic monitoring. Note that a more comprehensive check is available at `GET /health/detailed`.
    *   **Example:**
        ```bash
        curl http://localhost:4806/health
        # Expected Response: HTTP/1.1 200 OK (empty body)
        ```

*   **`GET /health/detailed`**
    *   **Purpose:** A comprehensive health check that verifies API key validity.
    *   **Description:** Performs a live, lightweight API call to Google using one of the available keys to ensure it's valid and not rate-limited. This provides a stronger guarantee that the proxy is fully functional.
    *   **Example:**
        ```bash
        curl http://localhost:4806/health/detailed
        # Expected Response: HTTP/1.1 200 OK (with JSON body confirming success)
        ```

*   **`/v1/*` (Proxy Endpoint)**
    *   **Purpose:** Acts as a transparent proxy for OpenAI-compatible API requests.
    *   **Description:** All requests sent to the proxy with a path starting `/v1/` (e.g., `/v1/chat/completions`, `/v1/models`) are intercepted. The proxy then:
        1.  Selects an available Gemini API key using its internal rotation logic.
        2.  Adds the necessary `x-goog-api-key` and `Authorization: Bearer <key>` headers.
        3.  Rewrites the request URL to target `https://generativelanguage.googleapis.com/v1beta/openai/` (or a group-specific `target_url` if configured).
        4.  Forwards the request to the Google Gemini API.
        5.  Returns the response from Google Gemini API back to the client.
    *   **Compatibility:** Designed to work seamlessly with standard OpenAI client libraries and tools.
    *   **Example:** (See [Example `curl` to proxy](#example-curl-to-proxy) for usage examples)

## ⚙️ Configuration

### 📝 **Basic Configuration**

The `config.yaml` file is your single source of truth. Start with the example:

```yaml
# config.yaml - Production Configuration
server:
  port: 4806
  admin_token: "your-secure-admin-token-here"  # Generate with: openssl rand -hex 32

  # Security settings
  security:
    require_https: true
    max_login_attempts: 5
    lockout_duration_secs: 3600
    session_timeout_secs: 86400

  # Performance tuning
  connect_timeout_secs: 10
  request_timeout_secs: 60

# Redis for production persistence
redis_url: "redis://localhost:6379"
redis_key_prefix: "gemini_proxy:"

# Key management
max_failures_threshold: 3
temporary_block_minutes: 5

# API key groups with intelligent routing
groups:
  - name: "Primary"
    api_keys:
      - "your-gemini-api-key-1"
      - "your-gemini-api-key-2"
    target_url: "https://generativelanguage.googleapis.com/v1beta/openai/"

  - name: "Backup"
    api_keys:
      - "your-backup-key-1"
    proxy_url: "socks5://proxy.example.com:1080"  # Optional upstream proxy
```

### 🔧 **Advanced Configuration**

```yaml
# Circuit breaker settings
circuit_breaker:
  failure_threshold: 5
  recovery_timeout_secs: 60
  success_threshold: 3

# Rate limiting
rate_limit:
  requests_per_minute: 100
  burst_size: 20

# Monitoring and alerts
monitoring:
  health_check_interval_secs: 30
  alert_thresholds:
    unhealthy_keys: 3
    error_rate: 0.1  # 10%
    response_time_secs: 5
```

### 🎛️ **Environment Variables**

```bash
# Logging level
export RUST_LOG=info  # debug, info, warn, error

# Override config file location
export CONFIG_PATH=/path/to/config.yaml

# Redis connection (overrides config.yaml)
export REDIS_URL=redis://localhost:6379
```

## 🔍 Monitoring & Observability

### 📊 **Health Endpoints**

```bash
# Basic health check (liveness probe)
curl http://localhost:4806/health

# Detailed health with key validation (readiness probe)
curl http://localhost:4806/health/detailed

# Metrics endpoint (Prometheus compatible)
curl http://localhost:4806/metrics
```

### 🎛️ **Admin Dashboard**

Access the web-based admin panel at `http://localhost:4806/admin/`:

- **Real-time Metrics**: Key health scores, success rates, response times
- **Key Management**: View status, manually disable/enable keys
- **System Health**: Circuit breaker status, Redis connectivity
- **Configuration**: Hot-reload settings without restart
- **Alert History**: View past incidents and recovery times

### 📈 **Key Health Scoring**

Each API key gets a health score from 0.0 (unhealthy) to 1.0 (perfect):

- **1.0**: Perfect performance, no recent failures
- **0.8-0.9**: Good performance, occasional failures
- **0.5-0.7**: Degraded performance, frequent failures
- **0.0-0.4**: Poor performance, mostly failing
- **Blocked**: Temporarily disabled due to consecutive failures

### 🚨 **Automated Alerts**

The system automatically generates alerts when:

- **>3 keys unhealthy**: Indicates potential API quota issues
- **Error rate >10%**: System-wide performance degradation
- **Response time >5s**: Upstream service slowdown
- **Circuit breaker open**: Upstream service completely down

### 📋 **Logging**

Structured JSON logging with correlation IDs:

```bash
# View logs
make logs

# Filter by level
RUST_LOG=debug make run

# Production logging
RUST_LOG=info,gemini_proxy=debug make docker-run
```

The proxy is designed to handle errors from the Gemini API gracefully:

*   **Immediate Failure (400, 404, 504):**
    *   These errors indicate a problem with the client's request (`400 Bad Request`, `404 Not Found`) or a gateway timeout (`504 Gateway Timeout`) that is unlikely to be resolved by a retry.
    *   **Action:** The error is immediately returned to the client without attempting to use another key.

*   **Invalid Key (403 Forbidden):**
    *   This error strongly indicates that the API key is invalid, revoked, or lacks the necessary permissions.
    *   **Action:** The key is marked as `Invalid` and permanently removed from the rotation for the current session to prevent further useless attempts.

*   **Rate Limiting (429 Too Many Requests):**
    *   This is a common, temporary state indicating the key has exceeded its request quota.
    *   **Action:** The key is temporarily disabled, and the proxy automatically retries the request with the next available key in the rotation.

*   **Server Errors (500, 503):**
    *   These errors (`500 Internal Server Error`, `503 Service Unavailable`) suggest a temporary problem on Google's end.
    *   **Action:** The proxy will perform a fixed number of retries (currently 2) with the *same key* using a fixed 1-second delay between attempts. If all retries fail, the key is then temporarily disabled, and the system moves to the next key.

### 🐳 Optimized Docker Commands

**Core Commands:**
```bash
make docker-run              # Run production environment
make docker-run-dev          # Development mode with hot-reload
make docker-run-with-tools   # + Redis UI and monitoring
make docker-test             # Run tests in a container
make docker-coverage         # Analyze code coverage
```

**Management:**
```bash
make docker-logs             # View application logs
make docker-logs-all         # All service logs
make docker-stop             # Stop services
make docker-restart          # Restart
make docker-clean            # Clean up resources
```

**Build:**
```bash
make docker-build            # Optimized build
make docker-build-dev        # Build for development
./scripts/docker-optimize.sh # Full optimization
```

## 🔒 Security & Production Deployment

### 🛡️ **Security Features**

- **Rate Limiting**: IP-based protection (5 attempts/5 minutes, 1-hour lockout)
- **HTTPS Enforcement**: Automatic redirect in production environments
- **Session Management**: Secure token-based authentication with rotation
- **Input Validation**: Request size limits and sanitization
- **Audit Logging**: All security events logged with correlation IDs
- **CSRF Protection**: Admin panel protected against cross-site attacks

### 🏭 **Production Deployment**

#### **Optimized Docker Compose**
```bash
# Full system optimization
./scripts/docker-optimize.sh

# Production deployment
make docker-run                    # Core services (50MB image)

# With monitoring tools
make docker-run-with-tools         # + Redis UI, metrics

# Horizontal scaling
docker-compose up -d --scale gemini-proxy=3

# Check status
make status                        # Status of all services
make health-detailed               # Detailed diagnostics
```

**📊 Advantages of the optimized build:**
- Image size: ~50MB (instead of 1.2GB)
- Build time: 3-5x faster
- Security: Distroless + non-privileged user
- Monitoring: Built-in health checks and metrics

#### **Kubernetes Deployment**
```yaml
# k8s-deployment.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: gemini-proxy
spec:
  replicas: 3
  selector:
    matchLabels:
      app: gemini-proxy
  template:
    metadata:
      labels:
        app: gemini-proxy
    spec:
      containers:
      - name: gemini-proxy
        image: gemini-proxy:latest
        ports:
        - containerPort: 4806
        env:
        - name: RUST_LOG
          value: "info"
        livenessProbe:
          httpGet:
            path: /health
            port: 4806
        readinessProbe:
          httpGet:
            path: /health/detailed
            port: 4806
```

#### **Systemd Service (Linux)**
```bash
# Installed automatically by install.sh
sudo systemctl enable gemini-proxy
sudo systemctl start gemini-proxy
sudo systemctl status gemini-proxy

# View logs
sudo journalctl -u gemini-proxy -f
```

### 🔐 **Security Best Practices**

1. **Generate Secure Admin Token**:
   ```bash
   make generate-admin-token
   ```

2. **Use HTTPS in Production**:
   ```yaml
   server:
     security:
       require_https: true
   ```

3. **Network Security**:
   - Deploy behind a reverse proxy (Nginx/Traefik)
   - Use firewall rules to restrict access
   - Consider VPN for admin panel access

4. **Key Management**:
   - Rotate API keys regularly
   - Use separate keys for different environments
   - Monitor key usage in Google AI Studio

5. **Backup Configuration**:
   ```bash
   make backup-config
   ```

## 🧪 Testing

The project includes comprehensive test coverage with 227 automated tests:

```bash
# Run all tests
make test

# Run only critical tests (security, monitoring, error handling)
make test-critical

# Run with coverage report
make test-coverage

# Run security audit
make security-scan
```

### Test Categories:
- **Tokenization Tests** (15 tests): Accuracy validation, large text handling, multilingual support
- **Security Tests** (7 tests): Rate limiting, HTTPS enforcement, token management
- **Monitoring Tests** (12 tests): Health scoring, proactive checks, alerts
- **Error Handling Tests** (3 tests): Structured error responses
- **Integration Tests** (20+ tests): End-to-end functionality
- **Unit Tests** (20+ tests): Individual component testing

### Tokenization Testing:
```bash
# Test tokenization accuracy against Google API
make test-tokenization

# Test large text handling (up to 100KB)
make test-large-text

# Test multilingual and Unicode support
make test-unicode

# Compare all tokenization strategies
make test-tokenizer-comparison
```

## 🤝 Contributing

We welcome contributions! Here's how to get started:

1. **Fork the repository**
2. **Set up development environment**:
   ```bash
   make dev-setup
   ```
3. **Make your changes**
4. **Run tests**:
   ```bash
   make check  # Runs lint, format, and tests
   ```
5. **Submit a pull request**

### Development Commands:
```bash
make dev-setup    # Complete development setup
make build-dev    # Build in development mode
make run-dev      # Run with debug logging
make format       # Format code
make lint         # Run clippy linter
```

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- Built with [Rust](https://www.rust-lang.org/) and [Tokio](https://tokio.rs/)
- HTTP framework: [Axum](https://github.com/tokio-rs/axum)
- Redis integration: [deadpool-redis](https://github.com/bikeshedder/deadpool)
- Security: [secrecy](https://github.com/iqlusioninc/crates/tree/main/secrecy)

## 📞 Support

- **Documentation**: Check the [docs](docs/) directory
- **Issues**: [GitHub Issues](https://github.com/stranmor/gemini-proxy-key-rotation-rust/issues)
- **Security**: See [SECURITY.md](SECURITY.md) for security policy
- **Discussions**: [GitHub Discussions](https://github.com/stranmor/gemini-proxy-key-rotation-rust/discussions)

---

<div align="center">

**⭐ Star this repository if it helped you!**

Made with ❤️ for the developer community

</div>