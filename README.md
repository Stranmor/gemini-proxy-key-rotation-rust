# Gemini Proxy Key Rotation (Rust)

[![CI](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml/badge.svg)](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
<!-- Add Docker Hub badge if applicable: [![Docker Hub](https://img.shields.io/docker/pulls/your_dockerhub_user/your_repo)](https://hub.docker.com/r/your_dockerhub_user/your_repo) -->

**A lightweight, high-performance asynchronous HTTP proxy to rotate Google Gemini (Generative Language API) keys, distribute load, and manage rate limits effectively.** Built with Rust, Axum, and Tokio.

## Overview (TL;DR)

This proxy acts as a middleman between your application and the Google Gemini API (or other compatible APIs). You provide it with multiple API keys, and it automatically rotates through them for outgoing requests.

**Key Benefits:**

*   **Avoid Rate Limits:** Distributes requests across many keys.
*   **Increased Availability:** If one key hits its limit, the proxy automatically switches to another.
*   **Centralized Key Management:** Manage keys in one place (config file or environment variables).
*   **Group-Specific Routing:** Use different target APIs or upstream proxies for different sets of keys.
*   **Security:** Handles authentication (`x-goog-api-key`, `Authorization: Bearer`) securely, hiding keys from the client application.

## Why Use This Proxy?

Google Gemini API keys often have relatively strict rate limits (e.g., requests per day). For applications making frequent calls, hitting these limits is common. This proxy solves that by pooling multiple keys and automatically switching when a limit is encountered, ensuring smoother operation. It also simplifies client configuration by abstracting away key management and authentication details.

## Features

*   Proxies requests to Google Gemini API or other specified target URLs.
*   Supports multiple **groups** of API keys with potentially different target URLs and optional upstream proxies per group.
*   Automatic round-robin key rotation across **all** configured keys (from all groups combined).
*   Handles `429 Too Many Requests` responses from the target API by temporarily disabling the rate-limited key (resets daily at 10:00 AM Moscow Time by default).
*   Configurable via a single YAML file (`config.yaml`).
*   API keys can be securely provided using **environment variables** (recommended), avoiding the need to store them directly in the configuration file.
*   Correctly adds the required `x-goog-api-key` and `Authorization: Bearer <key>` headers, replacing any client-sent `Authorization` headers.
*   Supports `http`, `https`, and `socks5` upstream proxies per key group.
*   High performance asynchronous request handling using Axum and Tokio.
*   Graceful shutdown handling (`SIGINT`, `SIGTERM`).
*   Configurable logging using `tracing` and the `RUST_LOG` environment variable.

## Requirements

*   **Docker:** The easiest and recommended way to run the proxy. ([Install Docker](https://docs.docker.com/engine/install/))
*   **Google Gemini API Keys:** Obtain these from [Google AI Studio](https://aistudio.google.com/app/apikey).
*   **(Optional) Rust & Cargo:** Only needed if you want to build or develop locally without Docker. ([Install Rust](https://rustup.rs/)) (Uses Rust 2021 Edition or later).

## Getting Started

Choose one of the following methods:

### Option 1: Running with Docker (Recommended)

1.  **Clone the Repository:**
    ```bash
    git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git
    cd gemini-proxy-key-rotation-rust
    ```

2.  **Prepare Configuration (`config.yaml`):**
    *   Copy `config.example.yaml` to `config.yaml`:
        ```bash
        cp config.example.yaml config.yaml
        ```
    *   Edit `config.yaml`:
        *   **Crucially:** Set `server.host` to `"0.0.0.0"` to accept connections from outside the container.
        *   Set `server.port` to the desired port *inside* the container (e.g., `8080`).
        *   Define your `groups`. **If using environment variables (recommended), leave `api_keys: []` empty or omit the line.**
        *   Adjust `target_url` or `proxy_url` per group if needed. See [Configuration Details](#configuration-details) below.

3.  **Build the Docker Image:**
    ```bash
    docker build -t gemini-proxy-key-rotation .
    ```

4.  **Run the Container:**
    *   Replace `<YOUR_KEYS_FOR_DEFAULT>` with your actual comma-separated API keys for the `default` group.
    *   Adjust the environment variable name (`GEMINI_PROXY_GROUP_DEFAULT_API_KEYS`) if your group `name` in `config.yaml` is different. Add more `-e` flags for other groups.
    *   Adjust the host port mapping (`8081:8080`) if port `8081` is busy on your host. Format: `<HOST_PORT>:<CONTAINER_PORT>`.

    ```bash
    docker run -d --name gemini-proxy \
      -p 8081:8080 \                                    # Map host port 8081 to container port 8080 (adjust host port if needed)
      -v "$(pwd)/config.yaml:/app/config.yaml:ro" \     # Mount local config file (read-only)
      -e RUST_LOG="info" \                              # Optional: Set log level (e.g., info, debug, trace)
      -e GEMINI_PROXY_GROUP_DEFAULT_API_KEYS="<YOUR_KEYS_FOR_DEFAULT>" \ # Provide API keys for the 'default' group
      # -e GEMINI_PROXY_GROUP_ANOTHER_GROUP_API_KEYS="<KEYS_FOR_ANOTHER_GROUP>" \ # Add more env vars for other groups if needed
      gemini-proxy-key-rotation
    ```

5.  **Verify:**
    *   Check container logs: `docker logs gemini-proxy`
    *   Test the proxy (replace `localhost:8081` if you used a different host port):
        ```bash
        # Example using direct Gemini generateContent endpoint
        curl -X POST \
          -H "Content-Type: application/json" \
          -d '{"contents":[{"parts":[{"text":"Explain Large Language Models in simple terms"}]}]}' \
          http://localhost:8081/v1beta/models/gemini-pro:generateContent
        ```
        You should receive a valid JSON response from the Gemini API.

### Option 2: Building and Running Locally (Without Docker)

Requires Rust and Cargo installed ([rustup.rs](https://rustup.rs/)).

1.  **Clone the Repository:** (If not already done)
    ```bash
    git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git
    cd gemini-proxy-key-rotation-rust
    ```

2.  **Prepare Configuration (`config.yaml`):**
    *   Copy `config.example.yaml` to `config.yaml`.
    *   Edit `config.yaml`:
        *   Set `server.host` to `"127.0.0.1"` (for local-only access) or `"0.0.0.0"` (for network access).
        *   Set `server.port` (e.g., `8080`).
        *   Define `groups` and provide API keys (either directly in the file under `api_keys: [...]` or leave it empty/omit and use environment variables). See [Configuration Details](#configuration-details).

3.  **Build:**
    ```bash
    cargo build --release
    ```
    (The `--release` flag enables optimizations).

4.  **Run:**
    *   **Using Keys from `config.yaml`:**
        ```bash
        # Optional: Set log level
        export RUST_LOG="info"
        # Ensure config.yaml is in the current directory
        ./target/release/gemini-proxy-key-rotation-rust --config config.yaml
        ```
    *   **Using Environment Variables:**
        ```bash
        # Set environment variables BEFORE running the command
        export RUST_LOG="info" # Optional log level
        export GEMINI_PROXY_GROUP_DEFAULT_API_KEYS="key1,key2"
        # export GEMINI_PROXY_GROUP_ANOTHER_GROUP_API_KEYS="key3"

        ./target/release/gemini-proxy-key-rotation-rust --config config.yaml
        ```

5.  **Verify:**
    *   Check the terminal output for logs.
    *   Send requests to `http://<HOST>:<PORT>` as configured (e.g., `http://127.0.0.1:8080`), using the `curl` example from the Docker section.

## Usage

Once the proxy is running, configure your client application to send API requests **to the proxy's address** (`http://<PROXY_HOST>:<PROXY_PORT>`), **not** directly to the Gemini API URL.

**Important:** Your client **should NOT send** any API key or `Authorization` header when talking *to the proxy*. The proxy handles the authentication with the actual target API internally:
*   It selects an available API key from its pool.
*   It adds the correct `x-goog-api-key: <selected_key>` header.
*   It adds the `Authorization: Bearer <selected_key>` header.
*   Any `Authorization` header sent by your client will be ignored and replaced.

### Example (`curl`)

Assuming the proxy runs at `http://localhost:8081`:

```bash
curl -X POST \
  -H "Content-Type: application/json" \
  -d '{"contents":[{"parts":[{"text":"Explain LLMs"}]}]}' \
  http://localhost:8081/v1beta/models/gemini-pro:generateContent # Send request TO THE PROXY
```

The proxy receives this request, adds the necessary Gemini authentication headers using a rotated key, and forwards it to `https://generativelanguage.googleapis.com` (or the `target_url` configured for the key's group).

### Using OpenAI Compatible Endpoints / Clients

If your client sends requests formatted for the OpenAI API (e.g., to `/v1/chat/completions` or similar), the proxy will forward them correctly, handling the Gemini authentication. Just point your client's **Base URL** to the proxy address.

```bash
# Example OpenAI-formatted request sent TO THE PROXY
curl --request POST \
  --url http://localhost:8081/v1/chat/completions \ # Note: Path doesn't matter much here, proxy targets based on config
  --header 'Authorization: Bearer any_dummy_key_will_be_ignored' \ # This header is ignored/replaced
  --header 'Content-Type: application/json' \
  --data '{
      "model": "gemini-pro",
      "messages": [
          {"role": "user", "content": "hi"}
      ]
  }'
```

### Using with Roo Code / Cline

This proxy is compatible with tools that support the OpenAI API format, like Roo Code / Cline.

1.  In your tool's API settings, select **"OpenAI Compatible"** as the **API Provider**.
2.  Set the **Base URL** to the proxy's address (protocol, host, port only, **without** any specific path like `/v1`).
    *   Example (Docker): `http://localhost:8081`
    *   Example (Local): `http://127.0.0.1:8080`
3.  For the **API Key** field, enter **any non-empty placeholder** (e.g., "dummy-key", "ignored"). The proxy manages the real keys and ignores this value, but the field usually requires input.

**Example Configuration Screenshot:**
*(Illustrates settings for Base URL and API Key within an OpenAI-compatible tool)*
![Roo Code Configuration Example](2025-04-13_14-02.png)

## Configuration Details (`config.yaml`)

The proxy's behavior is controlled by a `config.yaml` file.

```yaml
# config.yaml
server:
  # Host address the proxy listens on.
  # IMPORTANT for Docker: Use "0.0.0.0".
  # For local runs: "127.0.0.1" is usually sufficient.
  host: "0.0.0.0"
  # Port the server listens on *inside* the container or on the local machine.
  port: 8080

# Define one or more groups of API keys.
# The proxy rotates through keys from ALL groups combined.
groups:
  - name: "default" # A unique, descriptive name for this group.
                    # Used for logs and environment variable construction (see below).

    # Target URL for requests using keys from this group.
    # If omitted, defaults to Google's Generative Language API endpoint:
    # target_url: "https://generativelanguage.googleapis.com"
    target_url: "https://generativelanguage.googleapis.com"

    # Optional: Specify an upstream proxy for requests using keys from this group.
    # Supports http, https, and socks5 protocols.
    # Examples:
    # proxy_url: "http://user:pass@proxyserver:port"
    # proxy_url: "https://proxyserver:port"
    # proxy_url: "socks5://user:pass@your-proxy.com:1080"
    proxy_url: null # Or omit the line entirely if no proxy is needed

    # --- API Key Configuration ---
    # Option 1 (Recommended): Use Environment Variable
    # The proxy will look for: GEMINI_PROXY_GROUP_DEFAULT_API_KEYS
    # See the "API Key Environment Variables" section below for naming rules.
    # Leave api_keys empty or omit it when using environment variables.
    api_keys: []

    # Option 2 (Less Secure): Define keys directly here
    # Use this ONLY if you are NOT setting the corresponding environment variable.
    # Ensure this file is NOT committed to version control if it contains keys.
    # api_keys:
    #   - "YOUR_GEMINI_API_KEY_1_FOR_DEFAULT_GROUP"
    #   - "YOUR_GEMINI_API_KEY_2_FOR_DEFAULT_GROUP"


  # Add more groups as needed...
  # - name: "special-project"
  #   target_url: "https://another-api.example.com"
  #   proxy_url: "socks5://project-proxy:1080"
  #   api_keys: [] # Provide keys via GEMINI_PROXY_GROUP_SPECIAL_PROJECT_API_KEYS

  # - name: "no-proxy-group"
  #   api_keys: [] # Provide keys via GEMINI_PROXY_GROUP_NO_PROXY_GROUP_API_KEYS
```

**Key Points:**

*   **`server.host`**: **Must be `"0.0.0.0"` when running inside Docker**. Use `"127.0.0.1"` for local-only access if running directly without Docker.
*   **`server.port`**: The port the proxy listens on (e.g., `8080`).
*   **`groups`**: You must define at least one group.
*   **`groups[].name`**: A unique identifier for the group.
*   **`groups[].target_url`**: The API endpoint for this group. Defaults to `https://generativelanguage.googleapis.com` if omitted.
*   **`groups[].proxy_url`**: Optional upstream proxy URL (`http`, `https`, `socks5`).
*   **`groups[].api_keys`**: Define keys here **only if not** using environment variables for this group.

### API Key Environment Variables (Recommended Method)

This is the most secure way to provide API keys. If a valid environment variable is found for a group, it **completely overrides** the `api_keys` list in `config.yaml` for that group.

*   **Variable Name Format:** `GEMINI_PROXY_GROUP_{SANITIZED_GROUP_NAME}_API_KEYS`
*   **Sanitization Rule:** Convert the group `name` from `config.yaml` to **UPPERCASE** and replace any non-alphanumeric character (not A-Z, 0-9) with an **underscore (`_`)**.
*   **Value:** A **comma-separated** string of your API keys (e.g., `"key1,key2,key3"`). Spaces around commas are automatically trimmed.

**Examples:**

| Group Name in `config.yaml` | Corresponding Environment Variable                     | Example Value          |
| :-------------------------- | :------------------------------------------------------ | :--------------------- |
| `default`                   | `GEMINI_PROXY_GROUP_DEFAULT_API_KEYS`                   | `"keyA,keyB,keyC"`     |
| `special-project`           | `GEMINI_PROXY_GROUP_SPECIAL_PROJECT_API_KEYS`           | `"keyD"`               |
| `no-proxy-group`            | `GEMINI_PROXY_GROUP_NO_PROXY_GROUP_API_KEYS`            | `"keyE, keyF"`         |
| `Group 1!`                  | `GEMINI_PROXY_GROUP_GROUP_1__API_KEYS` (*Note double `_`*) | `"keyG,keyH"`          |

*   If the environment variable is set but empty or contains only whitespace/commas, the keys from the file (if any) will be used, and a warning will be logged.

## Operation & Maintenance

### Logging

*   Logging uses the `tracing` crate.
*   Control verbosity via the `RUST_LOG` environment variable.
*   **Default:** `info` (if `RUST_LOG` is not set).
*   **Examples:**
    *   `RUST_LOG=debug` : Show debug messages for all crates.
    *   `RUST_LOG=gemini_proxy_key_rotation_rust=debug`: Show debug messages only for this proxy.
    *   `RUST_LOG=warn,gemini_proxy_key_rotation_rust=trace`: Show trace messages for the proxy, warn for others.
    *   `RUST_LOG=error`: Show only errors.
*   Set the environment variable before running (e.g., `export RUST_LOG=debug` locally, or add `-e RUST_LOG=debug` to your `docker run` command).

### Error Handling

*   **Target API Errors (e.g., 400, 500):** Forwards the status code and body from the target API whenever possible.
*   **`429 Too Many Requests` (from Target):** Logs a warning, marks the used key as rate-limited (until 10:00 AM Moscow Time next day), and automatically retries the request with the next available key.
*   **`503 Service Unavailable` (from Proxy):** Returned by the proxy itself if *all* configured keys are currently rate-limited and no more keys are available to retry.
*   **`502 Bad Gateway` (from Proxy):** Returned if there's a network error connecting to the target API or the configured upstream proxy (e.g., connection refused, DNS error).
*   **Configuration Errors:** Logged on startup, causing the proxy to exit if validation fails.
*   **Authentication Errors from Target (e.g., `401`, `403`):** Usually indicate an invalid or revoked API key provided *to the proxy* (via config or env var). Check your keys.

### Common Docker Commands

*   **View Logs:** `docker logs gemini-proxy`
*   **Follow Logs:** `docker logs -f gemini-proxy`
*   **Stop Container:** `docker stop gemini-proxy`
*   **Start Container:** `docker start gemini-proxy`
*   **Remove Container:** `docker rm gemini-proxy` (stop it first)
*   **Rebuild Image (after code changes):** `docker build -t gemini-proxy-key-rotation .` (then stop/remove/run the new image)

### Security

*   **NEVER commit `config.yaml` files containing real API keys** to version control. Use `.gitignore` (it already lists `config.yaml`).
*   **Prioritize using environment variables** for API keys, especially in production or shared environments.
*   Use strong, unique API keys obtained from Google AI Studio.
*   Secure your network environment; consider firewalls if exposing the proxy beyond localhost.

## Project Structure

```
.
├── .dockerignore               # Files ignored by Docker build
├── .github/workflows/rust.yml  # Example CI workflow
├── .gitignore
├── 2025-04-13_14-02.png        # Example config screenshot for Roo Code
├── Cargo.lock
├── Cargo.toml
├── CODE_OF_CONDUCT.md
├── config.example.yaml         # Example configuration
├── config.yaml                 # Your configuration (ignored by git)
├── CONTRIBUTING.md
├── Dockerfile                  # Docker build instructions
├── LICENSE                     # Project License (MIT)
├── README.md                   # This file
└── src/
    ├── config.rs               # Configuration loading, validation, structs (AppConfig, KeyGroup)
    ├── error.rs                # Custom error types (AppError), HTTP response conversion
    ├── handler.rs              # Axum request handler (request entry point, retry logic)
    ├── key_manager.rs          # Key storage, rotation, state management (KeyManager, KeyState)
    ├── main.rs                 # Application entry point, CLI args, setup, validation call
    ├── proxy.rs                # Core request forwarding, header manipulation, upstream proxy logic
    └── state.rs                # Shared application state (AppState: HttpClient, KeyManager)

(target/ directory is generated during build and ignored by git)
```

## Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) and adhere to the [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

Potential areas for improvement:
*   More sophisticated key rotation strategies (e.g., least recently used, priority groups).
*   Health check endpoint (`/health`).
*   Metrics endpoint (`/metrics`) for Prometheus monitoring.
*   Expanded test coverage, especially for error conditions and proxy interactions.
*   Dynamic configuration reloading without restart.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.