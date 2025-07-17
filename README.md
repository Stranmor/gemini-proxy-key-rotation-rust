# Gemini Proxy Key Rotation (Rust) - OpenAI Compatibility

[![CI](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml/badge.svg)](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
<!-- Add Docker Hub badge if applicable: [![Docker Hub](https://img.shields.io/docker/pulls/your_dockerhub_user/your_repo)](https://hub.docker.com/r/your_dockerhub_user/your_repo) -->

**A lightweight, high-performance asynchronous HTTP proxy specifically designed to use Google Gemini models via their OpenAI-compatible API layer.** This proxy rotates Google Gemini API keys, distributes load, and manages rate limits effectively when interacting with the `generativelanguage.googleapis.com/v1beta/openai/` endpoint. Built with Rust, Axum, and Tokio.

**Note:** This proxy is intended *only* for use with Google's OpenAI compatibility layer. It does not support native Gemini API endpoints like `:generateContent`.

## Overview

This proxy acts as a middleman between your OpenAI-compatible application (like clients using OpenAI libraries or tools like Roo Code/Cline) and the Google Gemini API's OpenAI compatibility endpoint. You provide it with multiple Gemini API keys via a `config.yaml` file. The proxy automatically rotates through them for outgoing requests, handling authentication and rate limits.

**Key Benefits:**

*   **Avoid Rate Limits:** Distributes requests across many Gemini keys.
*   **Increased Availability:** If one key hits its limit, the proxy automatically switches to another.
*   **Simplified Configuration:** All settings are managed in a single `config.yaml` file, making configuration straightforward and predictable.
*   **Simplified Client Configuration:** Point your OpenAI client's base URL to this proxy; no need to manage Gemini keys in the client.
*   **Group-Specific Routing:** Use different upstream proxies (`http`, `https`, `socks5`) for different sets of keys.
*   **State Persistence:** Remembers rate-limited keys between restarts, avoiding checks on known limited keys until their reset time (daily midnight Pacific Time by default).

## Features

*   Proxies requests specifically to Google's OpenAI compatibility endpoint (`https://generativelanguage.googleapis.com/v1beta/openai/` by default).
*   Supports multiple **groups** of Gemini API keys with optional upstream proxies (`http`, `https`, `socks5`) per group, all configured in `config.yaml`.
*   **Group Round-Robin Key Rotation:** Selects the next available key by iterating through key groups sequentially (round-robin between groups) and then iterating through keys within the selected group. This ensures fairer distribution across groups compared to rotating through all keys flattened.
*   Handles `429 Too Many Requests` responses from the target API by temporarily disabling the rate-limited key.
*   **Rate Limit Reset:** Limited keys are automatically considered available again after the next **daily midnight in the Pacific Time zone (America/Los_Angeles)** by default.
*   **Persists Rate Limit State:** Saves the limited status and UTC reset time of keys to `key_states.json` (located in the current working directory, or `/app/` in Docker), allowing the proxy to skip known limited keys on startup.
*   Configuration is managed entirely through the `config.yaml` file.
*   Correctly adds the required `x-goog-api-key` and `Authorization: Bearer <key>` headers, replacing any client-sent `Authorization` headers.
*   High performance asynchronous request handling using Axum and Tokio.
*   Graceful shutdown handling (`SIGINT`, `SIGTERM`).
*   Configurable logging using `tracing` and the `RUST_LOG` environment variable.
*   Basic health check endpoint (`/health`).

## Architecture

The Gemini Proxy Key Rotation service is built with a modular architecture, leveraging Rust's ownership and concurrency features to ensure high performance and reliability. Below are the core components and their interactions:

*   [`main.rs`](src/main.rs): The entry point of the application. It initializes logging, loads the configuration, sets up the `KeyManager` and `AppState`, and starts the Axum HTTP server.
*   [`config.rs`](src/config.rs): Handles loading and validating the application's configuration from the `config.yaml` file. It defines how API key groups, proxy URLs, and target URLs are parsed and structured.
*   [`key_manager.rs`](src/key_manager.rs): Manages the lifecycle of Gemini API keys. It's responsible for loading keys, selecting the next available key using a group round-robin strategy, tracking rate limits, and persisting key states to `key_states.json`.
*   [`state.rs`](src/state.rs): Defines the shared application state (`AppState`) that is accessible across different request handlers. This includes the `KeyManager`, configuration, and other shared resources.
*   [`handler.rs`](src/handler.rs): Contains the Axum request handlers. It processes incoming HTTP requests, interacts with the `KeyManager` to get an API key, and prepares the request for forwarding.
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

## Requirements

*   **Docker:** The easiest and **most secure** way to run the proxy. ([Install Docker](https://docs.docker.com/engine/install/)).
*   **Google Gemini API Keys:** Obtain these from [Google AI Studio](https://aistudio.google.com/app/apikey).
*   **(Optional) Rust & Cargo:** Only needed if you want to build or develop locally without Docker. ([Install Rust](https://rustup.rs/)) (Uses Rust 2021 Edition or later).

## Getting Started

### Running with Docker (Recommended)

This method uses a simple script to run the proxy in a Docker container, mounting the necessary configuration files.

1.  **Clone the Repository:**
    ```bash
    git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git
    cd gemini-proxy-key-rotation-rust
    ```

2.  **Build the Docker Image:**
    You only need to do this once, or whenever you pull new changes.
    ```bash
    docker build -t gemini-proxy-key-rotation:latest .
    ```

3.  **Prepare Configuration File (`config.yaml`):**
    *   Copy the example configuration file:
        ```bash
        cp config.example.yaml config.yaml
        ```
    *   **Edit `config.yaml`:** This is the **only file you need to edit** to configure the proxy.
        *   Set the `server.port`. The port defined here will be the one the proxy listens on and will be exposed on your host machine.
        *   Define one or more `groups`, each with a `name` and a list of `api_keys`.
        *   Optionally, add `proxy_url` or `target_url` to any group.
        *   **Refer to `config.example.yaml`** for detailed examples.
        *   **Security:** Ensure `config.yaml` is **NOT** committed to Git if it contains sensitive API keys. (It is included in `.gitignore` by default).

4.  **Prepare State File (Optional but Recommended):**
    *   For persistence of rate-limited key states across restarts, create an empty file:
        ```bash
        touch key_states.json
        ```

5.  **Run the Proxy:**
    *   The provided `run.sh` script handles everything: it stops any old container, reads the port from your `config.yaml`, and starts a new container.
    ```bash
    ./run.sh
    ```

6.  **Verify:**
    *   Check the output of the script. It should say `Container gemini-proxy-openai-compose started.`.
    *   Check logs: `docker logs -f gemini-proxy-openai-compose`.
    *   Check health: `curl http://localhost:YOUR_PORT/health` (use the port you set in `config.yaml`).
    *   Test with an OpenAI client pointed to `http://localhost:YOUR_PORT`.
    *   Check if `key_states.json` was created/updated in your local directory.

7.  **Applying `config.yaml` Changes:**
    *   If you modify `config.yaml` after the container is running, you **must restart** the container for the changes to take effect.
    *   Simply run the script again:
        ```bash
        ./run.sh
        ```

8.  **Stopping:**
    ```bash
    docker stop gemini-proxy-openai-compose
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

5.  **Verify:**
    *   Check terminal logs.
    *   Check health: `curl http://localhost:YOUR_PORT/health` (use the port from `config.yaml` or the default 8080)
    *   Test with an OpenAI client pointed to `http://localhost:YOUR_PORT`.
## Usage with OpenAI Clients

(This section remains largely the same - the client configuration depends only on the proxy's host and port)

Once the proxy is running, configure your OpenAI client (e.g., Python/JS libraries, Roo Code/Cline, etc.) as follows:

1.  **Set the Base URL / API Host:** Point the client to the proxy's address (protocol, host, port only).
    *   Example (Docker Compose): `http://localhost:8081` (or the host port you mapped in `docker-compose.yml`)
    *   Example (Local): `http://127.0.0.1:8080` (or your address from `config.yaml`)
    *   **Do NOT include `/v1` or other paths in the Base URL.**

2.  **Set the API Key:** Enter **any non-empty placeholder** (e.g., "dummy-key", "ignored"). The proxy manages the *real* Gemini keys internally and **ignores the key sent by the client**, but the field usually requires input.

3.  **Send Requests:** Make requests as you normally would using the OpenAI client library or tool (e.g., to `/v1/chat/completions`, `/v1/models`, etc.). The proxy will intercept these, add the correct Google authentication for the OpenAI compatibility layer using a rotated key, and forward them.

### Example (`curl` to proxy)

```bash
# Example request to list models via the proxy (replace 8081 with your host port from docker-compose.yml)
curl http://localhost:8081/v1/models \
  -H "Authorization: Bearer dummy-ignored-key" # This header is ignored/replaced

# Example request for chat completion via the proxy (replace 8081 with your host port from docker-compose.yml)
curl http://localhost:8081/v1/chat/completions \
  -H "Authorization: Bearer dummy-ignored-key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gemini-1.5-flash-latest",
    "messages": [{"role": "user", "content": "Explain Rust."}],
    "temperature": 0.7
  }'
```

### Using with Roo Code / Cline

1.  In API settings, select **"OpenAI Compatible"** as **API Provider**.
2.  Set **Base URL** to the proxy address (e.g., `http://localhost:8081` or your host port from `docker-compose.yml`).
3.  Set **API Key** to any non-empty placeholder (e.g., "dummy").

**Example Configuration Screenshot:**
![Roo Code Configuration Example](2025-04-13_14-02.png)

## API Reference

The proxy exposes a minimal set of HTTP endpoints designed for compatibility with OpenAI clients and basic health monitoring.

### Endpoints

*   **`GET /health`**
    *   **Purpose:** A simple health check endpoint.
    *   **Description:** Returns a `200 OK` status code if the proxy is running and responsive. This is useful for load balancers, Docker health checks, and basic monitoring.
    *   **Example:**
        ```bash
        curl http://localhost:8081/health
        # Expected Response: HTTP/1.1 200 OK (empty body)
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

## Configuration (`config.yaml`)

This file is the **single source of truth** for all configuration. It is required for both local and Docker runs.

**Behavior:**

*   The application loads all its settings—server configuration and API key groups—directly from `config.yaml`.
*   Environment variables are only used for `RUST_LOG` to control logging level.

**Recommendation:**

*   Define all your `server` settings and `groups` in this file.
*   Use `config.example.yaml` as a starting point.

```yaml
# config.yaml
server:
  port: 8080
  cache_ttl_secs: 300
  cache_max_size: 1000

# Defines the behavior when a rate limit is hit
# - "RetryNextKey" (default): Tries the next available key.
# - "BlockUntilMidnight": Disables the key until midnight PT.
rate_limit_behavior: RetryNextKey

groups:
  - name: "Default"
    api_keys:
      - "key1"
      - "key2"
    # target_url: "https://..." # Optional: Defaults to Google's endpoint
    # proxy_url: "socks5://user:pass@host:port" # Optional

  - name: "TeamX"
    api_keys: ["team_x_key1"]
    proxy_url: "http://proxy.example.com:8080"
```

## Environment Variable Configuration

The only environment variable used by the application is `RUST_LOG`.

### Log Level
*   **Purpose:** Control the logging verbosity.
*   **Variable:** `RUST_LOG`
*   **Value:** Log level (e.g., `error`, `warn`, `info`, `debug`, `trace`).

## Operation & Maintenance

(Sections on Logging, Health Check, Key State Persistence, Error Handling, Docker Commands remain largely the same but reviewed for clarity)

### Logging
*   Use the `RUST_LOG` environment variable to set the desired log level (e.g., `info`, `debug`, `trace`). Default is `info`.

### Health Check
*   `GET /health` returns `200 OK`. Use for basic monitoring. Access via the host port mapped in `docker-compose.yml` (e.g., `http://localhost:8081/health`).

### Key State Persistence (`key_states.json`)
*   **Purpose:** Remembers rate-limited keys to avoid checking them immediately after restarts.
*   **Location:** Saved as `key_states.json` in the current working directory of the application (or `/app/` inside the default Docker container). When using Docker Compose, the `docker-compose.yml` maps your local `./key_states.json` into the container for persistence. Create an empty file locally first if it doesn't exist (`touch key_states.json`).
*   **Reset Logic:** Daily midnight Pacific Time (America/Los_Angeles).
*   **Management:** Automatic. Deleting the file resets the state memory.
*   **.gitignore:** Included by default.

### Resilience and Error Handling
The proxy implements a sophisticated strategy to handle various errors from the upstream Google Gemini API, maximizing availability and resilience.

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

### Common Docker Commands
*   **Start/Run:** `./run.sh`
*   **View Logs:** `docker logs -f gemini-proxy-openai-compose`
*   **Stop:** `docker stop gemini-proxy-openai-compose`
*   **Rebuild Image:** `docker build -t gemini-proxy-key-rotation:latest .`
*   **Check Status:** `docker ps`

## Security Considerations

*   **API Keys:** Do not commit `config.yaml` to version control if it contains API keys. The file is included in `.gitignore` by default.
*   **Files:** Do not commit `key_states.json` to Git.
*   **Network:** Expose the proxy only to trusted networks. Consider a reverse proxy (Nginx/Caddy) for TLS and advanced access control if needed.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## License

MIT License - see the [LICENSE](LICENSE) file.