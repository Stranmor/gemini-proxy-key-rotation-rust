# Gemini Proxy Key Rotation (Rust)

[![CI](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml/badge.svg)](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**A lightweight, asynchronous HTTP proxy for rotating Google Gemini (Generative Language API) API keys.** Built with Rust and Axum/Tokio for high performance and resilience.

This proxy distributes requests across multiple Gemini API keys, helping to manage rate limits and improve availability. It supports grouping keys (e.g., for different projects or users), routing requests to different target URLs per group, and using different upstream proxies per group.

## Features

*   Proxies requests to Google Gemini API or other specified target URLs.
*   Supports multiple **groups** of API keys with potentially different target URLs and optional upstream proxies.
*   Automatic round-robin key rotation across **all** configured keys (from all groups combined).
*   Handles `429 Too Many Requests` responses from the target API by temporarily disabling the rate-limited key (resets daily at 10:00 AM Moscow Time by default).
*   Configurable via a single YAML file (`config.yaml`).
*   API keys can be securely provided using **environment variables** (recommended), avoiding the need to store them directly in the configuration file.
*   Correctly adds the required `x-goog-api-key` and `Authorization: Bearer <key>` headers for Gemini API authentication, ignoring/replacing any client-sent `Authorization` headers.
*   Supports `http`, `https`, and `socks5` upstream proxies per key group.
*   High performance asynchronous request handling using Axum and Tokio.
*   Graceful shutdown handling (responds to `SIGINT` and `SIGTERM`).
*   Configurable logging using `tracing` and the `RUST_LOG` environment variable.

## Requirements

*   **Docker:** The easiest and recommended way to run the proxy. ([Install Docker](https://docs.docker.com/engine/install/))
*   **Google Gemini API Keys:** Obtain these from [Google AI Studio](https://aistudio.google.com/app/apikey).
*   **(Optional) Rust &amp; Cargo:** Only needed if you want to build or develop locally without Docker. ([Install Rust](https://rustup.rs/))

## Configuration (`config.yaml`)

The proxy's behavior is controlled by a `config.yaml` file. You can use `config.example.yaml` as a starting point.

```yaml
# config.yaml
server:
  # Host address the proxy listens on.
  # IMPORTANT for Docker: Use "0.0.0.0" to accept connections from outside the container.
  # For local runs, "127.0.0.1" is usually sufficient.
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
    proxy_url: null # Or omit the line entirely if no proxy is needed for this group

    # --- API Key Configuration ---
    # Provide API keys using the Environment Variable (Recommended)
    # The proxy will look for: GEMINI_PROXY_GROUP_DEFAULT_API_KEYS
    # See the "API Key Environment Variables" section below for naming rules.
    # Leave api_keys empty or omit it when using environment variables.
    api_keys: []

    # OPTIONALLY, define keys directly here (Less Secure)
    # Use this ONLY if you are NOT setting the corresponding environment variable.
    # Ensure this file is NOT committed to version control if it contains keys.
    # api_keys:
    #   - "YOUR_GEMINI_API_KEY_1_FOR_DEFAULT_GROUP"
    #   - "YOUR_GEMINI_API_KEY_2_FOR_DEFAULT_GROUP"


  # Add more groups as needed, for example, for different users or projects.
  # - name: "special-project"
  #   target_url: "https://another-api.example.com"
  #   proxy_url: "socks5://project-proxy:1080"
  #   api_keys: [] # Provide keys via GEMINI_PROXY_GROUP_SPECIAL_PROJECT_API_KEYS

  # - name: "no-proxy-group"
  #   api_keys: [] # Provide keys via GEMINI_PROXY_GROUP_NO_PROXY_GROUP_API_KEYS
```

**Key Points:**

*   **`server.host`**: **Must be `"0.0.0.0"` when running inside Docker** to be accessible from your host machine. Use `"127.0.0.1"` for local-only access if running directly without Docker.
*   **`server.port`**: The port the proxy listens on (e.g., `8080`).
*   **`groups`**: You must define at least one group.
*   **`groups[].name`**: A unique identifier for the group.
*   **`groups[].target_url`**: The API endpoint for this group. Defaults to `https://generativelanguage.googleapis.com` if omitted.
*   **`groups[].proxy_url`**: Optional upstream proxy URL (`http`, `https`, `socks5`).
*   **`groups[].api_keys`**: Define keys here **only if not** using environment variables for this group.

### API Key Environment Variables (Recommended)

This is the most secure way to provide API keys. If a valid environment variable is found for a group, it **completely overrides** the `api_keys` list in `config.yaml` for that group.

*   **Variable Name Format:** `GEMINI_PROXY_GROUP_{SANITIZED_GROUP_NAME}_API_KEYS`
*   **Sanitization Rule:** Convert the group `name` from `config.yaml` to **UPPERCASE** and replace any non-alphanumeric character (not A-Z, 0-9) with an **underscore (`_`)**.
*   **Value:** A **comma-separated** string of your API keys (e.g., `"key1,key2,key3"`). Spaces around commas are trimmed.

**Examples:**

| Group Name in `config.yaml` | Corresponding Environment Variable                     |
| :-------------------------- | :------------------------------------------------------ |
| `default`                   | `GEMINI_PROXY_GROUP_DEFAULT_API_KEYS`                   |
| `special-project`           | `GEMINI_PROXY_GROUP_SPECIAL_PROJECT_API_KEYS`           |
| `no-proxy-group`            | `GEMINI_PROXY_GROUP_NO_PROXY_GROUP_API_KEYS`            |
| `Group 1!`                  | `GEMINI_PROXY_GROUP_GROUP_1__API_KEYS` (*Note double `_`*) |

If the environment variable is set but empty or contains only whitespace/commas, the keys from the file (if any) will be used, and a warning will be logged.

## Quick Start with Docker (Recommended)

1.  **Clone the Repository:**
    ```bash
    git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git
    cd gemini-proxy-key-rotation-rust
    ```

2.  **Prepare Configuration:**
    *   Copy `config.example.yaml` to `config.yaml`:
        ```bash
        cp config.example.yaml config.yaml
        ```
    *   Edit `config.yaml`:
        *   Ensure `server.host` is set to `"0.0.0.0"`.
        *   Define your desired `groups`. Remove or leave the `api_keys` list empty within the groups if using environment variables (recommended).
        *   Adjust `target_url` or `proxy_url` if needed.

3.  **Build the Docker Image:**
    ```bash
    docker build -t gemini-proxy-key-rotation .
    ```

4.  **Run the Container:**
    *   Replace `<YOUR_KEYS_FOR_DEFAULT>` with your actual comma-separated API keys for the `default` group.
    *   Adjust the environment variable name if your group name in `config.yaml` is different. Add more `-e` flags for other groups.
    *   Adjust the host port mapping (`8081:8080`) if port `8081` is busy on your host. Format: `<HOST_PORT>:<CONTAINER_PORT>`.

    ```bash
    docker run -d --name gemini-proxy \
      -p 8081:8080 \                                    # Map host port 8081 to container port 8080
      -v "$(pwd)/config.yaml:/app/config.yaml:ro" \     # Mount local config file (read-only)
      -e GEMINI_PROXY_GROUP_DEFAULT_API_KEYS="<YOUR_KEYS_FOR_DEFAULT>" \ # Provide API keys for the 'default' group
      # -e GEMINI_PROXY_GROUP_ANOTHER_GROUP_API_KEYS="<KEYS_FOR_ANOTHER_GROUP>" \ # Add more env vars for other groups if needed
      gemini-proxy-key-rotation
    ```

5.  **Verify:**
    *   Check container logs:
        ```bash
        docker logs gemini-proxy
        ```
    *   Test the proxy (replace `localhost:8081` if you used a different host port):
        ```bash
        # Example using direct Gemini generateContent endpoint
        curl -X POST \
          -H "Content-Type: application/json" \
          -d '{"contents":[{"parts":[{"text":"Explain Large Language Models in simple terms"}]}]}' \
          http://localhost:8081/v1beta/models/gemini-pro:generateContent
        ```
        You should receive a valid JSON response from the Gemini API.

### Common Docker Commands

*   **View Logs:** `docker logs gemini-proxy`
*   **Stop Container:** `docker stop gemini-proxy`
*   **Start Container:** `docker start gemini-proxy`
*   **Remove Container:** `docker rm gemini-proxy` (stop it first)
*   **Rebuild Image (after code changes):** `docker build -t gemini-proxy-key-rotation .`

## Building and Running Locally (Without Docker)

Requires Rust and Cargo installed ([rustup.rs](https://rustup.rs/)).

1.  **Clone the Repository:** (If not already done)
    ```bash
    git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git
    cd gemini-proxy-key-rotation-rust
    ```

2.  **Prepare Configuration:**
    *   Copy `config.example.yaml` to `config.yaml`.
    *   Edit `config.yaml`:
        *   Set `server.host` to `"127.0.0.1"` (or `"0.0.0.0"` for network access).
        *   Set `server.port` (e.g., `8080`).
        *   Define `groups` and provide API keys (in file or via environment variables).

3.  **Build:**
    ```bash
    cargo build --release
    ```
    (The `--release` flag enables optimizations).

4.  **Run:**
    *   **Using Keys from `config.yaml`:**
        ```bash
        # Ensure config.yaml is in the current directory
        ./target/release/gemini-proxy-key-rotation-rust --config config.yaml
        ```
    *   **Using Environment Variables:**
        ```bash
        # Set environment variables BEFORE running the command
        export GEMINI_PROXY_GROUP_DEFAULT_API_KEYS="key1,key2"
        # export GEMINI_PROXY_GROUP_ANOTHER_GROUP_API_KEYS="key3"

        ./target/release/gemini-proxy-key-rotation-rust --config config.yaml
        ```

5.  **Verify:**
    *   Check the terminal output for logs.
    *   Send requests to `http://<HOST>:<PORT>` as configured (e.g., `http://127.0.0.1:8080`), using the `curl` example from the Docker section.

## Client Usage

Once the proxy is running, configure your client application to send API requests **to the proxy's address** (`http://<PROXY_HOST>:<PROXY_PORT>`).

**Important:** Your client **should NOT send** any API key or `Authorization` header when talking *to the proxy*. The proxy handles the authentication with the actual target API internally:
*   It selects an available Gemini API key from its pool.
*   It adds the correct `x-goog-api-key: <selected_key>` header.
*   It adds the `Authorization: Bearer <selected_key>` header (required by some Gemini API endpoints/clients).
*   Any `Authorization` header sent by your client will be ignored and replaced by the proxy.

**Example (`curl`):**

Assuming the proxy runs at `http://localhost:8081`:

```bash
curl -X POST \
  -H "Content-Type: application/json" \
  -d '{"contents":[{"parts":[{"text":"Explain Large Language Models in simple terms"}]}]}' \
  http://localhost:8081/v1beta/models/gemini-pro:generateContent # Send request to the proxy
```

The proxy receives this request, adds the necessary Gemini authentication headers using a rotated key, and forwards it to `https://generativelanguage.googleapis.com` (or the `target_url` configured for the key's group).

**Using OpenAI Compatible Endpoints / Clients:**

If your client sends requests formatted for the OpenAI API (e.g., to `/v1beta/openai/chat/completions`), the proxy will forward them correctly, handling the Gemini authentication.

```bash
# Example OpenAI-formatted request sent TO THE PROXY
curl --request POST \
  --url http://localhost:8081/v1beta/openai/chat/completions \
  --header 'Authorization: Bearer any_dummy_key_will_be_ignored' \ # This header is ignored/replaced by the proxy
  --header 'Content-Type: application/json' \
  --data '{
      "model": "gemini-pro",
      "messages": [
          {"role": "user", "content": "hi"}
      ]
  }'
```

## Using with Roo Code / Cline

This proxy is compatible with tools that support the OpenAI API format, like Roo Code / Cline.

1.  In your tool's API settings, select **"OpenAI Compatible"** as the **API Provider**.
2.  Set the **Base URL** to the proxy's address (protocol, host, port only, **without** any specific path like `/v1beta/openai`).
    *   Example (Docker): `http://localhost:8081`
    *   Example (Local): `http://127.0.0.1:8080`
3.  For the **API Key** field, enter **any non-empty placeholder** (e.g., "dummy-key", "ignored"). The proxy manages the real keys and ignores this value, but the field usually requires input.

**Example Configuration Screenshot:**
*(Illustrates settings for Base URL and API Key within an OpenAI-compatible tool)*
![Roo Code Configuration Example](2025-04-13_14-02.png)

## Logging

*   Logging uses the `tracing` crate.
*   Control verbosity via the `RUST_LOG` environment variable.
*   **Default:** `info` (if `RUST_LOG` is not set).
*   **Examples:**
    *   `RUST_LOG=debug`: Show debug messages for all crates.
    *   `RUST_LOG=gemini_proxy_key_rotation_rust=debug`: Show debug messages only for this proxy.
    *   `RUST_LOG=warn,gemini_proxy_key_rotation_rust=trace`: Show trace messages for the proxy, warn for others.
*   Set the environment variable before running (e.g., `export RUST_LOG=debug` locally, or `-e RUST_LOG=debug` in `docker run`).

## Error Handling

*   Forwards underlying HTTP status codes from the target API (e.g., 400, 500) when possible.
*   `429 Too Many Requests` from target: Logs a warning, marks the key as rate-limited, and retries with the next available key. Limited keys reset daily (10:00 Moscow Time).
*   `503 Service Unavailable`: Returned by the proxy if *all* configured keys are currently rate-limited.
*   `502 Bad Gateway`: Returned if there's a network error connecting to the target API or upstream proxy.
*   Configuration errors: Logged on startup, causing the proxy to exit.
*   Authentication errors from target (e.g., `401`, `403`): Usually indicate an invalid or revoked API key provided to the proxy. Check your keys.

## Project Structure

```
.
├── .dockerignore               # Files ignored by Docker build
├── .github/workflows/rust.yml  # Example CI workflow
├── .gitignore
├── 2025-04-13_14-02.png        # Example config screenshot for Roo Code
├── Cargo.lock
├── Cargo.toml
├── CODE_OF_CONDUCT.md          # Code of Conduct
├── config.example.yaml         # Example configuration
├── config.yaml                 # Your configuration (ignored by git)
├── CONTRIBUTING.md             # Contribution guidelines
├── Dockerfile                  # Docker build instructions
├── LICENSE                     # Project License (MIT)
├── README.md                   # This file
└── src/
    ├── config.rs               # Configuration loading, validation, structs
    ├── error.rs                # Custom error types (AppError), HTTP response conversion
    ├── handler.rs              # Axum request handler (request entry point)
    ├── key_manager.rs          # Key storage, rotation, state management (KeyManager)
    ├── main.rs                 # Application entry point, setup, validation call
    ├── proxy.rs                # Core request forwarding, header manipulation, proxy logic
    └── state.rs                # Shared application state (AppState: HttpClient, KeyManager)

(target/ directory is generated during build and ignored by git)
```

## Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

Potential areas:
*   More sophisticated key rotation strategies.
*   Health check endpoint.
*   Metrics endpoint (Prometheus).
*   Expanded test coverage.

## Security

*   **NEVER commit `config.yaml` files containing real API keys** to version control. Use `.gitignore` (it already lists `config.yaml`).
*   **Prioritize using environment variables** for API keys.
*   Use strong, unique API keys from Google AI Studio.
*   Secure your network environment; consider firewalls.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.