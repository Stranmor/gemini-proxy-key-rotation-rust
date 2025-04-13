# Gemini Proxy Key Rotation (Rust)

[![CI](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml/badge.svg)](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
<!-- [![Crates.io](https://img.shields.io/crates/v/your-crate-name.svg)](https://crates.io/crates/your-crate-name) -->
<!-- [![Docs.rs](https://docs.rs/your-crate-name/badge.svg)](https://docs.rs/your-crate-name) -->

**A lightweight, asynchronous HTTP proxy for rotating Google Gemini (Generative Language API) API keys.** Built with Rust, Axum, and Tokio for high performance and reliability.

This proxy allows you to distribute requests across multiple Gemini API keys (organized in groups) using a round-robin strategy, helping to manage rate limits and improve availability. It also supports routing requests through different upstream proxies on a per-group basis.

## Features

-   Proxy requests to the Google Gemini API (`generativelanguage.googleapis.com`) or other configured target URLs.
-   Supports multiple **groups** of API keys, each with its own target URL and optional upstream proxy.
-   Automatic round-robin key rotation for each incoming request across **all keys** in the configuration.
-   Handles `429 Too Many Requests` errors by temporarily disabling the rate-limited key until the next day (10:00 Moscow Time).
-   Configurable host and port binding.
-   Asynchronous handling of requests using Axum and Tokio.
-   Simple YAML configuration.
-   Graceful shutdown handling.

## Requirements

-   Rust (latest stable version recommended, check `rust-toolchain.toml` or install via [rustup](https://rustup.rs/))
-   Cargo (comes with Rust)
-   Google Gemini API Keys (get them from [Google AI Studio](https://aistudio.google.com/app/apikey))

## Quick Start

1.  **Clone the repository:**
    ```sh
    git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git
    cd gemini-proxy-key-rotation-rust
    ```

2.  **Create your configuration file:**
    -   Copy the example:
        ```sh
        cp config.example.yaml config.yaml
        ```
    -   Edit `config.yaml` and add your Gemini API keys under the `api_keys` list within at least one group. Configure `target_url` and optional `proxy_url` for each group as needed.
    -   **Important:** `config.yaml` is listed in `.gitignore` to prevent accidental commits of your keys. **Never commit your API keys to version control.**

3.  **Build:**
    ```sh
    cargo build --release
    ```

4.  **Run:**
    ```sh
    ./target/release/gemini-proxy-key-rotation-rust
    # Or using cargo:
    # cargo run --release
    ```
    The proxy will start listening on the host and port specified in `config.yaml` (default: `127.0.0.1:8080`).

## Configuration

Configuration is managed through the `config.yaml` file (refer to `config.example.yaml` for detailed comments).

```yaml
# config.yaml
server:
  host: "127.0.0.1"  # IP address to bind the server to
  port: 8080        # Port to listen on

# List of key groups. Requires at least one group.
groups:
  - name: "default-gemini" # Unique name for this group
    # Target API URL for this group. Defaults to Google's global endpoint if omitted.
    target_url: "https://generativelanguage.googleapis.com"
    # Optional outgoing proxy URL for this group (http, https, socks5 supported)
    # proxy_url: "socks5://user:pass@your-proxy.com:1080"
    # List of API keys for this group. Rotation happens across keys from ALL groups.
    api_keys:
      - "YOUR_GEMINI_API_KEY_1"
      - "YOUR_GEMINI_API_KEY_2"
      # Add more keys as needed

  # Example of another group targeting a different endpoint or using a different proxy
  # - name: "special-endpoint"
  #   target_url: "https://special-regional-api.googleapis.com"
  #   proxy_url: "http://another-proxy.example.com:8888"
  #   api_keys:
  #     - "YOUR_SPECIAL_API_KEY_3"
```

-   `server.host`: The hostname or IP address the proxy server will bind to.
-   `server.port`: The port the proxy server will listen on.
-   `groups`: A list of key groups. Each group requires:
    -   `name`: A unique identifier for the group.
    -   `api_keys`: A list of API keys associated with this group.
    -   `target_url` (Optional): The upstream API endpoint URL for this group. Defaults to `https://generativelanguage.googleapis.com`.
    -   `proxy_url` (Optional): An upstream proxy URL (http, https, or socks5) to use for requests made with keys from this group.

## API Usage

Once the proxy is running (e.g., on `http://127.0.0.1:8080`), send your API requests to the proxy **instead of** directly to the target API (e.g., `https://generativelanguage.googleapis.com`).

The proxy will automatically:
1.  Receive your request.
2.  Select the next available API key from **all configured keys** across all groups (round-robin), skipping any keys currently marked as rate-limited.
3.  Determine the correct `target_url` and optional `proxy_url` based on the group the selected key belongs to.
4.  Add the `x-goog-api-key` header (and `Authorization: Bearer` for compatibility) with the selected key.
5.  Forward the request (including path, query parameters, modified headers, and body) to the determined `target_url`, potentially via the group's `proxy_url`.
6.  Stream the response back to you.
7.  If the target API returns `429 Too Many Requests`, the proxy marks the used key as rate-limited for the day.

**Example using `curl`:**

Assuming the proxy is running on `http://localhost:8080`:

```sh
curl --request GET \
  --url http://localhost:8080/v1beta/openai/models \
  --header 'Authorization: Bearer GEMINI_API_KEY'
```

```sh
curl --request POST \
  --url http://localhost:8080/v1beta/openai/chat/completions \
  --header 'Authorization: Bearer GEMINI_API_KEY' \
  --header 'Content-Type: application/json' \
  --data '{
    "model": "gemini-2.0-flash",
    "messages": [
      {"role": "user", "content": "hi"}
    ]
  }'
```

The proxy handles adding the necessary `x-goog-api-key` header. Do **not** include your own API key header when sending requests to the proxy, except for compatibility modes like OpenAI where a dummy `Authorization` header might be needed by the client.

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
├── .github/workflows/rust.yml  # Example CI workflow
├── .gitignore
├── Cargo.lock
├── Cargo.toml
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

-   **NEVER** commit your `config.yaml` file or expose your API keys in public repositories or client-side code.
-   Use strong, unique API keys.
-   Consider running the proxy in a trusted network environment.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.