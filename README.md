# Gemini Proxy Key Rotation (Rust)

[![CI](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml/badge.svg)](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
<!-- [![Crates.io](https://img.shields.io/crates/v/your-crate-name.svg)](https://crates.io/crates/your-crate-name) -->
<!-- [![Docs.rs](https://docs.rs/your-crate-name/badge.svg)](https://docs.rs/your-crate-name) -->

**A lightweight, asynchronous HTTP proxy for rotating Google Gemini (Generative Language API) API keys.** Built with Rust, Axum, and Tokio for high performance and reliability.

This proxy allows you to distribute requests across multiple Gemini API keys using a round-robin strategy, helping to manage rate limits and improve availability.

## Features

-   Proxy requests to the Google Gemini API (`generativelanguage.googleapis.com`).
-   Supports multiple API keys.
-   Automatic round-robin key rotation for each incoming request.
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
    *(Replace `stranmor/gemini-proxy-key-rotation-rust` with the actual path if forked)*

2.  **Create your configuration file:**
    -   Copy the example:
        ```sh
        cp config.example.yaml config.yaml
        ```
    -   Edit `config.yaml` and add your Gemini API keys under the `api_keys` list.
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

Configuration is managed through the `config.yaml` file (or `config.example.yaml` as a template).

```yaml
# config.yaml
server:
  host: "127.0.0.1"  # IP address to bind the server to
  port: 8080        # Port to listen on

api_keys:
  - "YOUR_API_KEY_1"
  - "YOUR_API_KEY_2"
  # Add more keys as needed
```

-   `server.host`: The hostname or IP address the proxy server will bind to.
-   `server.port`: The port the proxy server will listen on.
-   `api_keys`: A list of your Google Gemini API keys. The proxy will rotate through these keys for outgoing requests.

## API Usage

Once the proxy is running (e.g., on `http://127.0.0.1:8080`), send your Gemini API requests to the proxy instead of directly to `https://generativelanguage.googleapis.com`.

The proxy will automatically:
1.  Receive your request.
2.  Select the next API key from the `api_keys` list (round-robin).
3.  Add the `x-goog-api-key` header with the selected key.
4.  Forward the request (including path, query parameters, headers, and body) to `https://generativelanguage.googleapis.com`.
5.  Stream the response back to you.
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

The proxy handles adding the `x-goog-api-key` header. Do **not** include your own API key header when sending requests to the proxy.

## Using with Roo Code / Cline

To use this proxy with Roo Code or Cline:

1.  In the API configuration settings, select **"OpenAI Compatible"** as the **API Provider**.
2.  Set the **Base URL** to the address where the proxy is running, including the `/v1beta/openai` path. For example: `http://localhost:8080/v1beta/openai` (replace `localhost:8080` if you configured a different host or port).
3.  For the **OpenAI API Key**, you can enter **any non-empty value**. The proxy manages the actual Gemini keys internally, but the OpenAI Compatible setting usually requires a value in this field.

Example Configuration (based on the provided image):
![Roo Code Configuration Example](2025-04-13_14-02.png)

*Note: The current implementation uses a single round-robin pool for all requests. Future versions plan to include proxy endpoints specific to each API key set if multiple sets are configured.*

## Project Structure

```
.
├── .github/workflows/rust.yml  # Example CI workflow (add this!)
├── .gitignore
├── Cargo.lock
├── Cargo.toml
├── config.example.yaml         # Example configuration
├── config.yaml                 # Your configuration (ignored by git)
├── LICENSE
├── README.md                   # This file
├── src/
│   ├── config.rs               # Configuration loading
│   ├── handler.rs              # Request handling logic
│   ├── main.rs                 # Application entry point
│   └── state.rs                # Shared application state (keys, client)
└── target/                     # Build artifacts (ignored by git)
```

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

Before contributing, please read the [CONTRIBUTING.md](CONTRIBUTING.md) guide (you should create this file). We also adhere to a [Code of Conduct](CODE_OF_CONDUCT.md) (you should create this file).

Key areas for contribution:
-   Adding more sophisticated key rotation strategies (e.g., based on usage or errors).
-   Implementing caching.
-   Adding metrics and monitoring endpoints.
-   Improving test coverage.

## Security

-   **NEVER** commit your `config.yaml` file or expose your API keys in public repositories or client-side code.
-   Use strong, unique API keys.
-   Consider running the proxy in a trusted network environment.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.