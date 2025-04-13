# Gemini Proxy Key Rotation (Rust)

[![CI](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml/badge.svg)](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**A lightweight, asynchronous HTTP proxy for rotating Google Gemini (Generative Language API) API keys.** Built with Rust and Axum for high performance.

This proxy distributes requests across multiple Gemini API keys, helping to manage rate limits and improve availability. It supports grouping keys and routing through different upstream proxies per group.

## Features

*   Proxies requests to Google Gemini API or other target URLs.
*   Supports multiple **groups** of API keys (different targets, optional upstream proxies).
*   Automatic round-robin key rotation across **all** configured keys.
*   Handles `429 Too Many Requests` by temporarily disabling the rate-limited key.
*   Configurable via YAML file and/or environment variables (for API keys).
*   High performance asynchronous handling (Axum/Tokio).
*   Graceful shutdown.
*   Logs request details and key usage.

## Requirements

*   **Docker:** The easiest way to run the proxy. ([Install Docker](https://docs.docker.com/engine/install/))
*   **Google Gemini API Keys:** Get them from [Google AI Studio](https://aistudio.google.com/app/apikey).
*   **(Optional) Rust/Cargo:** Only needed if you want to build or develop locally without Docker. ([Install Rust](https://rustup.rs/))

## Quick Start with Docker (Recommended)

This is the simplest way to get the proxy running.

1.  **Clone the repository:**
    ```sh
    git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git
    cd gemini-proxy-key-rotation-rust
    ```

2.  **Build the Docker Image:**
    ```sh
    docker build -t gemini-proxy-key-rotation .
    ```

3.  **Prepare Configuration Base:**
    *   The proxy needs a basic configuration file for server settings (host/port) and defining key groups (names, target URLs, etc.). We'll use the provided example as a base.
    *   **Important:** Ensure `config.example.yaml` has `server.host` set to `"0.0.0.0"` for Docker usage. (It should be by default now).

4.  **Run the Container using Environment Variables for Keys:**
    *   Replace `YOUR_KEY_1,YOUR_KEY_2,...` with your actual comma-separated API keys.
    *   The example below assumes a group named `default` exists in `config.example.yaml`. Adjust the variable name (`GEMINI_PROXY_GROUP_DEFAULT_API_KEYS`) if your group has a different name (see Configuration section for details).
    *   Adjust the host port (`8081`) if it's already in use on your machine.

    ```sh
    docker run -d --name gemini-proxy \
      -p 8081:8080 \
      -v "$(pwd)/config.example.yaml:/app/config.yaml:ro" \
      -e GEMINI_PROXY_GROUP_DEFAULT_API_KEYS="YOUR_KEY_1,YOUR_KEY_2,YOUR_KEY_3" \
      gemini-proxy-key-rotation
    ```

5.  **Verify:**
    *   Check logs: `docker logs gemini-proxy` (You should see startup messages).
    *   Test the proxy (replace `localhost:8081` if you used a different host port):
        ```sh
        curl http://localhost:8081/v1beta/models
        ```
        (You should get a response from the Google API, possibly `401` if the keys are invalid, but not a connection error).

## Docker Usage Details

### Running the Container

There are two main ways to provide configuration:

**Method 1: Environment Variables for API Keys (Recommended)**

*   **How it works:** Mount a base config file (`config.example.yaml` or your own `config.yaml` *without* sensitive keys) for server settings and group definitions. Provide the actual API keys via environment variables.
*   **Why:** More secure (keys aren't stored in files), easier to manage in deployment scripts and orchestration tools.
*   **Command:**
    ```sh
    # Ensure config.example.yaml (or your base config.yaml) exists
    docker run -d --name gemini-proxy \
      -p <HOST_PORT>:8080 \
      -v "$(pwd)/config.example.yaml:/app/config.yaml:ro" \
      -e GEMINI_PROXY_GROUP_<GROUP_NAME>_API_KEYS="<KEY1,KEY2,...>" \
      # Add more -e flags for other groups if needed
      gemini-proxy-key-rotation
    ```
    *   Replace `<HOST_PORT>` with the port you want to use on your machine (e.g., `8080`, `8081`).
    *   Replace `<GROUP_NAME>` with the name of the group from your config file (uppercase, non-alphanumeric replaced by `_`). Example: `default` -> `DEFAULT`, `my-group` -> `MY_GROUP`.
    *   Replace `<KEY1,KEY2,...>` with your comma-separated API keys.

**Method 2: Mounting `config.yaml` with API Keys**

*   **How it works:** Create a complete `config.yaml` file including your sensitive API keys and mount this file into the container. **Do not** set the `GEMINI_PROXY_GROUP_..._API_KEYS` environment variables.
*   **Why:** Simpler for local testing if you prefer managing keys in the file. **Use with caution** – ensure this file is not committed to Git or exposed.
*   **Command:**
    ```sh
    # Ensure your complete config.yaml with API keys exists
    docker run -d --name gemini-proxy \
      -p <HOST_PORT>:8080 \
      -v "$(pwd)/config.yaml:/app/config.yaml:ro" \
      gemini-proxy-key-rotation
    ```
    *   Replace `<HOST_PORT>` as needed.

### Common Docker Commands

*   **View Logs:** `docker logs gemini-proxy` (replace `gemini-proxy` if you used a different `--name`)
*   **Stop Container:** `docker stop gemini-proxy`
*   **Start Container:** `docker start gemini-proxy`
*   **Remove Container:** `docker rm gemini-proxy` (stop it first)
*   **Rebuild Image (after code changes):** `docker build -t gemini-proxy-key-rotation .`

## Configuration (`config.yaml`)

This file defines server settings and key groups. It's always needed, even if keys are provided via environment variables.

```yaml
# config.yaml (or config.example.yaml)
server:
  # IMPORTANT for Docker: Use "0.0.0.0" to accept connections from the host.
  host: "0.0.0.0"
  # Port the server listens on *inside* the container.
  port: 8080

# Define your key groups here.
groups:
  - name: "default" # Unique name for this group
    # Target URL for this group (defaults to Google Gemini API if omitted)
    target_url: "https://generativelanguage.googleapis.com"
    # Optional upstream proxy for this group (http, https, socks5)
    # proxy_url: "socks5://user:pass@your-proxy.com:1080"

    # API Keys for this group.
    # OMIT this section or leave it empty if providing keys via ENV VAR:
    # GEMINI_PROXY_GROUP_DEFAULT_API_KEYS="KEY1,KEY2"
    api_keys:
       - "YOUR_GEMINI_API_KEY_1" # Ignored if ENV VAR is set
       - "YOUR_GEMINI_API_KEY_2" # Ignored if ENV VAR is set

  # Add more groups as needed
  # - name: "special-group"
  #   target_url: "https://..."
  #   api_keys: [] # Provide via GEMINI_PROXY_GROUP_SPECIAL_GROUP_API_KEYS
```

**Key Points:**

*   `server.host`: **Must be `"0.0.0.0"`** for Docker usage.
*   `server.port`: Internal container port (usually `8080`).
*   `groups`: Define at least one group.
*   `groups[].name`: Used to identify the group (and in the ENV VAR name).
*   `groups[].api_keys`: List keys here **only** if you are *not* using environment variables for this group.

**Environment Variable Naming for Keys:**

*   Format: `GEMINI_PROXY_GROUP_{GROUP_NAME}_API_KEYS`
*   Rule: Take group `name` from YAML, convert to UPPERCASE, replace non-alphanumeric characters with `_`.
    *   `default` -> `GEMINI_PROXY_GROUP_DEFAULT_API_KEYS`
    *   `no-proxy` -> `GEMINI_PROXY_GROUP_NO_PROXY_API_KEYS`
    *   `group-123` -> `GEMINI_PROXY_GROUP_GROUP_123_API_KEYS`
*   Value: Comma-separated keys (`KEY1,KEY2,KEY3`).

## API Usage

Send your API requests to the proxy's address (e.g., `http://localhost:8081` if you mapped to host port `8081`) instead of the direct Google API URL. The proxy handles adding the correct API key.

**Example (`curl`):**

```sh
# Assuming proxy is accessible at http://localhost:8081
curl http://localhost:8081/v1beta/models
```

```sh
curl -X POST \
  -H "Content-Type: application/json" \
  -d '{"contents":[{"parts":[{"text":"Explain Large Language Models"}]}]}' \
  http://localhost:8081/v1beta/models/gemini-pro:generateContent
```

## Using with Roo Code / Cline

To use this proxy with Roo Code or Cline:

1.  In the API configuration settings, select **"OpenAI Compatible"** as the **API Provider**.
2.  Set the **Base URL** to the address where the proxy is running, including the `/v1beta/openai` path. For example: `http://localhost:8080/v1beta/openai` (replace `localhost:8080` if you configured a different host or port).
3.  For the **OpenAI API Key**, you can enter **any non-empty value**. The proxy manages the actual Gemini keys internally, but the OpenAI Compatible setting usually requires a value in this field.

Example Configuration (based on the provided image):
![Roo Code Configuration Example](2025-04-13_14-02.png)

## Project Structure

```
.
├── .dockerignore               # Files ignored by Docker build
├── .github/workflows/rust.yml  # Example CI workflow
├── .gitignore
├── Cargo.lock
├── Cargo.toml
├── Dockerfile                  # Docker build instructions
├── config.example.yaml         # Example configuration
├── config.yaml                 # Your configuration (ignored by git)
├── LICENSE
├── README.md                   # This file
├── examples/                   # Directory for usage examples
├── src/
│   ├── config.rs               # Configuration loading and structs
│   ├── error.rs                # Custom application error types (AppError)
│   ├── handler.rs              # Axum request handler (receives request, calls key_manager & proxy)
│   ├── key_manager.rs          # API key storage, rotation, and state management (KeyManager)
│   ├── main.rs                 # Application entry point, setup, validation
│   ├── proxy.rs                # Core request forwarding logic to target API
│   └── state.rs                # Shared application state (AppState holding KeyManager, HttpClient)
├── target/                     # Build artifacts (ignored by git)
└── tests/                      # Directory for integration tests
```

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

Before contributing, please read the [CONTRIBUTING.md](CONTRIBUTING.md) guide (you should create this file). We also adhere to a [Code of Conduct](CODE_OF_CONDUCT.md) (you should create this file).

Key areas for contribution:
-   Adding more sophisticated key rotation strategies (e.g., based on usage or errors).
-   Implementing caching.
-   Adding metrics and monitoring endpoints.
-   Improving test coverage (especially integration tests).

## Security

-   **NEVER** commit your `config.yaml` file or expose your API keys in public repositories or client-side code. Use environment variables for keys in production/Docker environments.
-   Use strong, unique API keys.
-   Consider running the proxy in a trusted network environment.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.