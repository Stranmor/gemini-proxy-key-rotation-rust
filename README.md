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
*   API keys can be securely overridden using **environment variables**, avoiding the need to store them directly in the configuration file.
*   Correctly adds the required `x-goog-api-key` header for Gemini API authentication and ignores/replaces client-sent `Authorization` headers.
*   Supports `http`, `https`, and `socks5` upstream proxies per key group.
*   High performance asynchronous request handling using Axum and Tokio.
*   Graceful shutdown handling (responds to `SIGINT` and `SIGTERM`).
*   Configurable logging using `tracing` and `RUST_LOG`.

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
groups:
  - name: "default" # A unique, descriptive name for this group.
                    # Used to construct the environment variable name for key overriding.
                    # While names like "Group 1!" are allowed in the config,
                    # they will be sanitized for environment variables (see below).

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
    # Provide API keys either directly here OR via environment variable.
    # Using environment variables is STRONGLY RECOMMENDED for security.

    # OPTION 1: Define keys directly in the file (Less Secure)
    # Use this ONLY if you are NOT setting the corresponding environment variable.
    # Ensure this file is NOT committed to version control if it contains keys.
    # api_keys:
    #   - "YOUR_GEMINI_API_KEY_1_FOR_DEFAULT_GROUP"
    #   - "YOUR_GEMINI_API_KEY_2_FOR_DEFAULT_GROUP"

    # OPTION 2: Provide keys via Environment Variable (Recommended)
    # Leave api_keys empty or omit it entirely if using the environment variable.
    # The proxy will look for: GEMINI_PROXY_GROUP_DEFAULT_API_KEYS
    api_keys: []

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
*   **`groups`**: You must define at least one group. The proxy rotates through keys from *all* groups combined.
*   **`groups[].name`**: A unique identifier for the group. Used for logging and constructing the environment variable name.
*   **`groups[].target_url`**: The API endpoint for this group. Defaults to `https://generativelanguage.googleapis.com` if omitted.
*   **`groups[].proxy_url`**: Optional upstream proxy URL (supports `http`, `https`, `socks5`).
*   **`groups[].api_keys`**: Define keys here **only if not** using the environment variable method for this specific group.

### API Key Environment Variables (Recommended)

This is the most secure way to provide API keys.

*   **Format:** `GEMINI_PROXY_GROUP_{GROUP_NAME}_API_KEYS`
*   **Sanitization Rule:**
    1.  Take the group `name` from `config.yaml`.
    2.  Convert it to **UPPERCASE**.
    3.  Replace any character that is **not** alphanumeric (A-Z, 0-9) with an **underscore (`_`)**.
*   **Value:** A **comma-separated** string of your API keys (e.g., `"key1,key2,key3"`). Spaces around commas are trimmed.
*   **Examples:**
    *   Group name `default` -> `GEMINI_PROXY_GROUP_DEFAULT_API_KEYS`
    *   Group name `special-project` -> `GEMINI_PROXY_GROUP_SPECIAL_PROJECT_API_KEYS`
    *   Group name `no-proxy-group` -> `GEMINI_PROXY_GROUP_NO_PROXY_GROUP_API_KEYS`
    *   Group name `Group 1!` -> `GEMINI_PROXY_GROUP_GROUP_1__API_KEYS` (Note the double underscore from ` ` and `!`)

If a valid environment variable is found for a group, it **completely overrides** the `api_keys` list specified in the `config.yaml` for that group. If the environment variable is set but empty or contains only whitespace/commas, the keys from the file (if any) will be used, and a warning will be logged.

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
        *   Define your desired `groups`. You can remove the example `api_keys` list within the groups if you plan to use environment variables (recommended). Adjust `target_url` or `proxy_url` if needed. Ensure at least one group (e.g., `default`) exists.

3.  **Build the Docker Image:**
    ```bash
    docker build -t gemini-proxy-key-rotation .
    ```

4.  **Run the Container:**
    *   Replace `<YOUR_KEYS_FOR_DEFAULT>` with your actual comma-separated API keys for the `default` group (or whichever group name you are configuring).
    *   Adjust the environment variable name (`GEMINI_PROXY_GROUP_DEFAULT_API_KEYS`) if your group name in `config.yaml` is different. Add more `-e` flags for other groups if needed.
    *   Adjust the host port mapping (`8081:8080`) if port `8081` is already in use on your machine. The format is `<HOST_PORT>:<CONTAINER_PORT>`.

    ```bash
    docker run -d --name gemini-proxy \
      -p 8081:8080 \
      -v "$(pwd)/config.yaml:/app/config.yaml:ro" \
      -e GEMINI_PROXY_GROUP_DEFAULT_API_KEYS="<YOUR_KEYS_FOR_DEFAULT>" \
      # -e GEMINI_PROXY_GROUP_ANOTHER_GROUP_API_KEYS="<YOUR_KEYS_FOR_ANOTHER_GROUP>" \
      gemini-proxy-key-rotation
    ```
    *   The `-v "$(pwd)/config.yaml:/app/config.yaml:ro"` part mounts your local `config.yaml` into the container at `/app/config.yaml` in read-only mode.
    *   The `-e` flag sets the environment variable inside the container.

5.  **Verify:**
    *   Check container logs for startup messages and confirmation of key loading:
        ```bash
        docker logs gemini-proxy
        ```
    *   Test the proxy by sending a request to the host port you mapped (e.g., `8081`):
        ```bash
        # Example using direct Gemini generateContent endpoint
        curl -X POST \
          -H "Content-Type: application/json" \
          -d '{"contents":[{"parts":[{"text":"Explain Large Language Models in simple terms"}]}]}' \
          http://localhost:8081/v1beta/models/gemini-2.0-flash:generateContent
        ```
        *(Replace `localhost:8081` if needed)*. You should receive a valid JSON response from the Gemini API.

### Common Docker Commands

*   **View Logs:** `docker logs gemini-proxy`
*   **Stop Container:** `docker stop gemini-proxy`
*   **Start Container:** `docker start gemini-proxy`
*   **Remove Container:** `docker rm gemini-proxy` (stop it first)
*   **Rebuild Image (after code changes):** `docker build -t gemini-proxy-key-rotation .`

## Building and Running Locally (Without Docker)

Requires Rust and Cargo to be installed ([rustup.rs](https://rustup.rs/)).

1.  **Clone the Repository:** (If not already done)
    ```bash
    git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git
    cd gemini-proxy-key-rotation-rust
    ```

2.  **Prepare Configuration:**
    *   Copy `config.example.yaml` to `config.yaml`.
    *   Edit `config.yaml`:
        *   Set `server.host` to `"127.0.0.1"` (or `"0.0.0.0"` if you need access from other machines on your network).
        *   Set `server.port` to your desired listening port (e.g., `8080`).
        *   Define `groups` and provide API keys either directly in the file or via environment variables (see Configuration section).

3.  **Build:**
    ```bash
    cargo build --release
    ```
    (The `--release` flag enables optimizations for better performance).

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
    *   Send requests to `http://<HOST>:<PORT>` as configured (e.g., `http://127.0.0.1:8080`), using the direct Gemini API example shown in the Docker section.

## Client Usage

Once the proxy is running, send your API requests to the proxy's address. The proxy will automatically select an available API key, add the necessary `x-goog-api-key` header (and `Authorization: Bearer <key>` header for Gemini compatibility), and forward the request to the configured `target_url` for that key's group.

**Important:** You **do not need to** add any API key or `Authorization` header to your client request when talking *to the proxy*. The proxy handles the authentication with the actual target API. If your client sends an `Authorization` header, the proxy will ignore and replace it.

**Example (`curl` for direct Gemini endpoint):**

Assuming the proxy is running and accessible at `http://localhost:8081`:

```bash
curl -X POST \
  -H "Content-Type: application/json" \
  -d '{"contents":[{"parts":[{"text":"Explain Large Language Models in simple terms"}]}]}' \
  http://localhost:8081/v1beta/models/gemini-2.0-flash:generateContent
```

*   Replace `http://localhost:8081` with your proxy's actual address and port.
*   The proxy handles adding the `x-goog-api-key` and `Authorization` headers internally.

**Using OpenAI Compatible Endpoints:**

Some clients might send requests formatted for the OpenAI API. The proxy will forward these requests as well.

```bash
# Example if your client uses OpenAI format (ensure backslashes for line continuation if copying)
curl --request POST \
  --url http://localhost:8081/v1beta/openai/chat/completions \
  --header 'Authorization: Bearer ignored' \
  --header 'Content-Type: application/json' \
  --data '{
      "model": "gemini-2.0-flash",
      "messages": [
          {"role": "user", "content": "hi"}
      ]
  }'

# Single-line equivalent (safer for copy-paste):
# curl --request POST --url http://localhost:8081/v1beta/openai/chat/completions --header 'Authorization: Bearer ignored' --header 'Content-Type: application/json' --data '{"model": "gemini-2.0-flash", "messages": [{"role": "user", "content": "hi"}]}'
```
You should receive a valid response, as the proxy manages the actual authentication with the target API using the rotated keys.


## Using with Roo Code / Cline

This proxy can be used as a backend for tools compatible with the OpenAI API format, like Roo Code / Cline.

1.  In the API configuration settings of your tool (e.g., Roo Code), select **"OpenAI Compatible"** as the **API Provider**.
2.  Set the **Base URL** to the address where the proxy is running. Use *only* the base address (protocol, host, port), **without** any specific path like `/v1beta/openai`. The proxy forwards the path it receives from the client.
    *   Example (Docker): `http://localhost:8081` (if mapped to host port 8081)
    *   Example (Local): `http://127.0.0.1:8080` (if running on port 8080)
3.  For the **API Key** field, enter **any non-empty value** (e.g., "dummy-key", "ignored"). The proxy manages the actual Gemini keys internally and will ignore/replace this value, but the setting usually requires some input.

**Example Configuration Screenshot:**
*(Illustrates settings for Base URL and API Key within an OpenAI-compatible tool)*
![Roo Code Configuration Example](2025-04-13_14-02.png)

## Logging

*   Logging is implemented using the `tracing` crate.
*   Log levels can be controlled via the `RUST_LOG` environment variable.
*   **Default:** If `RUST_LOG` is not set, it defaults to the `info` level (equivalent to `RUST_LOG=info`).
*   **Examples:**
    *   `RUST_LOG=debug`: Show debug messages for all crates.
    *   `RUST_LOG=gemini_proxy_key_rotation_rust=debug`: Show debug messages only for this proxy.
    *   `RUST_LOG=warn,gemini_proxy_key_rotation_rust=trace`: Show trace messages for the proxy, warn for others.
*   Set the environment variable before running the application (either locally or via `-e RUST_LOG=debug` in `docker run`).

## Error Handling

*   The proxy attempts to forward underlying HTTP status codes from the target API (e.g., 400 Bad Request, 500 Internal Server Error).
*   If a key results in a `429 Too Many Requests` response, the proxy logs a warning, marks that specific key as rate-limited, and automatically tries the next available key for subsequent requests. The limited key becomes available again after its reset time (daily 10:00 Moscow Time).
*   If *all* keys are currently rate-limited, the proxy returns a `503 Service Unavailable` status code.
*   Configuration errors (invalid YAML, invalid URLs, no keys found) are logged during startup, and the proxy will exit.
*   Network errors during forwarding (e.g., connection refused to the target API or upstream proxy) result in a `502 Bad Gateway` status code.
*   Authentication errors (`401 Unauthorized` or similar) from the target API usually indicate an invalid or revoked API key being used by the proxy. Double-check that your API keys are valid and correctly entered in the configuration or environment variables.

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
    ├── config.rs               # Configuration loading, validation, and structs (AppConfig, KeyGroup)
    ├── error.rs                # Custom application error types (AppError) and IntoResponse implementation
    ├── handler.rs              # Axum request handler (entry point for requests)
    ├── key_manager.rs          # API key storage, rotation logic, and state management (KeyManager, KeyState)
    ├── main.rs                 # Application entry point, CLI args, setup, validation call
    ├── proxy.rs                # Core request forwarding logic (building requests, handling proxies, sending)
    └── state.rs                # Shared application state (AppState holding KeyManager, HttpClient)

(target/ directory is generated during build and ignored by git)
```

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

Please review the [CONTRIBUTING.md](CONTRIBUTING.md) and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) files for guidelines.

Key areas for potential contributions:
*   More sophisticated key rotation strategies (e.g., usage-based, least-recently-used).
*   Health check endpoint.
*   Metrics endpoint (e.g., Prometheus).
*   Expanded test coverage (integration tests for proxying, rate limiting).
*   Support for different API authentication methods if needed.

## Security

*   **NEVER commit `config.yaml` files containing real API keys** to version control (like Git). Use the `.gitignore` file (which already lists `config.yaml`).
*   **Prioritize using environment variables** for API keys, especially in production or shared environments.
*   Use strong, unique API keys obtained from Google AI Studio.
*   Consider network security: run the proxy in a trusted network environment and potentially restrict access using firewalls if necessary.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.