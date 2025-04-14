# Gemini Proxy Key Rotation (Rust) - OpenAI Compatibility

[![CI](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml/badge.svg)](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
<!-- Add Docker Hub badge if applicable: [![Docker Hub](https://img.shields.io/docker/pulls/your_dockerhub_user/your_repo)](https://hub.docker.com/r/your_dockerhub_user/your_repo) -->

**A lightweight, high-performance asynchronous HTTP proxy specifically designed to use Google Gemini models via their OpenAI-compatible API layer.** This proxy rotates Google Gemini API keys, distributes load, and manages rate limits effectively when interacting with the `generativelanguage.googleapis.com/v1beta/openai/` endpoint. Built with Rust, Axum, and Tokio.

**Note:** This proxy is intended *only* for use with Google's OpenAI compatibility layer. It does not support native Gemini API endpoints like `:generateContent`.

## Overview (TL;DR)

This proxy acts as a middleman between your OpenAI-compatible application (like clients using OpenAI libraries or tools like Roo Code/Cline) and the Google Gemini API's OpenAI compatibility endpoint. You provide it with multiple Gemini API keys, and it automatically rotates through them for outgoing requests, handling the necessary authentication.

**Key Benefits:**

*   **Avoid Rate Limits:** Distributes requests across many Gemini keys.
*   **Increased Availability:** If one key hits its limit, the proxy automatically switches to another.
*   **Centralized Key Management:** Manage Gemini keys in one place (config file or environment variables).
*   **Simplified Client Configuration:** Point your OpenAI client's base URL to this proxy; no need to manage Gemini keys in the client.
*   **Group-Specific Routing:** Use different upstream proxies (e.g., SOCKS5) for different sets of keys if needed.
*   **Security:** Handles Google API authentication securely, hiding keys from the client application.

## Why Use This Proxy?

Google Gemini API keys often have rate limits. For applications making frequent calls via the OpenAI compatibility layer, hitting these limits is common. This proxy solves that by pooling multiple keys and automatically switching when a limit is encountered. It allows you to use standard OpenAI clients while benefiting from Gemini key rotation.

## Features

*   Proxies requests specifically to Google's OpenAI compatibility endpoint (`https://generativelanguage.googleapis.com/v1beta/openai/` by default).
*   Supports multiple **groups** of Gemini API keys with optional upstream proxies per group.
*   Automatic round-robin key rotation across **all** configured keys (from all groups combined).
*   Handles `429 Too Many Requests` responses from the target API by temporarily disabling the rate-limited key (resets daily at 10:00 AM Moscow Time by default).
*   Configurable via a single YAML file (`config.yaml`).
*   API keys can be securely provided using **environment variables** (recommended).
*   Correctly adds the required `x-goog-api-key` and `Authorization: Bearer <key>` headers required by the OpenAI compatibility layer, replacing any client-sent `Authorization` headers.
*   Supports `http`, `https`, and `socks5` upstream proxies per key group.
*   High performance asynchronous request handling using Axum and Tokio.
*   Graceful shutdown handling (`SIGINT`, `SIGTERM`).
*   Configurable logging using `tracing` and the `RUST_LOG` environment variable.
*   Basic health check endpoint (`/health`).

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
        *   Set `server.host` to `"0.0.0.0"`.
        *   Set `server.port` (e.g., `8080`).
        *   Define your `groups`. The `target_url` will default correctly to the OpenAI compatibility endpoint, so you usually don't need to set it.
        *   Configure `proxy_url` per group only if you need an upstream proxy.
        *   **Use environment variables for API keys (recommended):** Leave `api_keys: []` empty or omit it. See [API Key Environment Variables](#api-key-environment-variables-recommended-method).

3.  **Build the Docker Image:**
    ```bash
    docker build -t gemini-proxy-openai .
    ```
    *(Note: Image tag changed slightly for clarity)*

4.  **Run the Container (Single Command - Build & Run):**

    Replace `<YOUR_GEMINI_KEYS_FOR_DEFAULT>` and adjust ports/environment variables as needed.

    ```bash
    docker build -t gemini-proxy-openai . && docker run -d --name gemini-proxy -p 8081:8080 -v "$(pwd)/config.yaml:/app/config.yaml:ro" -e RUST_LOG="info" -e GEMINI_PROXY_GROUP_DEFAULT_API_KEYS="<YOUR_GEMINI_KEYS_FOR_DEFAULT>" gemini-proxy-openai
    ```
    *   **Explanation:**
        *   `-p 8081:8080`: Maps host port 8081 to container port 8080. Adjust `8081` if busy.
        *   `-v ...`: Mounts your local `config.yaml`.
        *   `-e RUST_LOG="info"`: Optional log level.
        *   `-e GEMINI_PROXY_GROUP_DEFAULT_API_KEYS="..."`: **Replace** with your comma-separated Gemini keys. Add more `-e` flags for other groups.

5.  **Run the Container (Separate Build and Run Steps):**

    *   **Build:** `docker build -t gemini-proxy-openai .`
    *   **Run:**
        ```bash
        docker run -d --name gemini-proxy -p 8081:8080 -v "$(pwd)/config.yaml:/app/config.yaml:ro" -e RUST_LOG="info" -e GEMINI_PROXY_GROUP_DEFAULT_API_KEYS="<YOUR_GEMINI_KEYS_FOR_DEFAULT>" gemini-proxy-openai
        ```

6.  **Verify:**
    *   Check logs: `docker logs gemini-proxy`
    *   Check health: `curl http://localhost:8081/health`
    *   Test with an OpenAI client pointed to `http://localhost:8081`.

### Option 2: Building and Running Locally (Without Docker)

Requires Rust and Cargo installed ([rustup.rs](https://rustup.rs/)).

1.  **Clone Repository:** (If needed)
    ```bash
    git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git
    cd gemini-proxy-key-rotation-rust
    ```
2.  **Prepare `config.yaml`:**
    *   Copy `config.example.yaml` to `config.yaml`.
    *   Edit `server.host` (`127.0.0.1` or `0.0.0.0`) and `server.port`.
    *   Define `groups`. Use environment variables for keys.
3.  **Build:**
    ```bash
    cargo build --release
    ```
4.  **Run:**
    *   **Using Environment Variables:**
        ```bash
        export RUST_LOG="info" # Optional
        export GEMINI_PROXY_GROUP_DEFAULT_API_KEYS="key1,key2"
        # export GEMINI_PROXY_GROUP_ANOTHER_GROUP_API_KEYS="key3"

        ./target/release/gemini-proxy-key-rotation-rust --config config.yaml
        ```
5.  **Verify:**
    *   Check terminal logs.
    *   Check health: `curl http://<HOST>:<PORT>/health`
    *   Test with an OpenAI client pointed to `http://<HOST>:<PORT>`.

## Usage with OpenAI Clients

Once the proxy is running, configure your OpenAI client (e.g., Python/JS libraries, Roo Code/Cline, etc.) as follows:

1.  **Set the Base URL / API Host:** Point the client to the proxy's address (protocol, host, port only).
    *   Example (Docker): `http://localhost:8081`
    *   Example (Local): `http://127.0.0.1:8080` (or your configured address)
    *   **Do NOT include `/v1` or other paths in the Base URL.**

2.  **Set the API Key:** Enter **any non-empty placeholder** (e.g., "dummy-key", "ignored", your Gemini key). The proxy manages the *real* Gemini keys internally and **ignores the key sent by the client**, but the field usually requires input. The proxy adds the necessary `x-goog-api-key` and `Authorization: Bearer <key>` headers itself using a rotated key from its pool.

3.  **Send Requests:** Make requests as you normally would using the OpenAI client library or tool (e.g., to `/v1/chat/completions`, `/v1/models`, etc.). The proxy will intercept these, add the correct Google authentication for the OpenAI compatibility layer, and forward them to `https://generativelanguage.googleapis.com/v1beta/openai/`.

### Example (`curl` to proxy simulating OpenAI client)

```bash
# Example request to list models via the proxy
curl http://localhost:8081/v1/models \
  -H "Authorization: Bearer dummy-ignored-key" # This header is ignored/replaced by the proxy

# Example request for chat completion via the proxy
curl http://localhost:8081/v1/chat/completions \
  -H "Authorization: Bearer dummy-ignored-key" \ # This header is ignored/replaced
  -H "Content-Type: application/json" \
  -d '{
    "model": "gemini-1.5-flash-latest", # Use a valid Gemini model name
    "messages": [{"role": "user", "content": "Explain Rust."}],
    "temperature": 0.7
  }'
```

### Using with Roo Code / Cline

1.  In API settings, select **"OpenAI Compatible"** as **API Provider**.
2.  Set **Base URL** to the proxy address (e.g., `http://localhost:8081`).
3.  Set **API Key** to any non-empty placeholder (e.g., "dummy").

**Example Configuration Screenshot:**
*(Illustrates settings for Base URL and API Key within an OpenAI-compatible tool)*
![Roo Code Configuration Example](2025-04-13_14-02.png)

## Configuration Details (`config.yaml`)

```yaml
# config.yaml
server:
  host: "0.0.0.0" # Use "0.0.0.0" for Docker, "127.0.0.1" for local-only
  port: 8080     # Port the proxy listens on

groups:
  - name: "default"
    # target_url: Defaults to Google's OpenAI endpoint, usually no need to set.
    # proxy_url: "socks5://user:pass@your-proxy.com:1080" # Optional upstream proxy
    api_keys: [] # Use env var: GEMINI_PROXY_GROUP_DEFAULT_API_KEYS
```
*   See `config.example.yaml` for more details and options.

## API Key Environment Variables (Recommended Method)

*   **Format:** `GEMINI_PROXY_GROUP_{SANITIZED_GROUP_NAME}_API_KEYS`
*   **Sanitization:** Group name from `config.yaml` -> UPPERCASE, non-alphanumeric -> `_`.
*   **Value:** Comma-separated Gemini API keys (e.g., `"key1,key2,key3"`).

**Examples:**

| Group Name (`config.yaml`) | Environment Variable                           | Example Value      |
| :------------------------- | :--------------------------------------------- | :----------------- |
| `default`                  | `GEMINI_PROXY_GROUP_DEFAULT_API_KEYS`          | `"keyA,keyB"`      |
| `socks-proxy`              | `GEMINI_PROXY_GROUP_SOCKS_PROXY_API_KEYS`      | `"keyC, keyD"`     |
| `Group 1!`                 | `GEMINI_PROXY_GROUP_GROUP_1__API_KEYS`         | `"keyE"`           |

## Operation & Maintenance

### Logging

*   Use `RUST_LOG` env var (e.g., `info`, `debug`, `trace`, `warn`, `error`). Default: `info`.
*   Example: `export RUST_LOG=debug` or `-e RUST_LOG=debug` in `docker run`.

### Health Check

*   `GET /health` endpoint returns `200 OK`. Use for basic monitoring.
*   Example: `curl http://localhost:8081/health`

### Error Handling

*   **`400 Bad Request` (from Target):** Usually indicates the **request body/format sent by your client** is invalid for the Google OpenAI compatibility endpoint. Check the request structure against OpenAI API specs.
*   **`401/403` (from Target):** Invalid/revoked Gemini API key provided to the proxy. Check your keys.
*   **`429 Too Many Requests` (from Target):** Key rate-limited. Proxy logs warning, marks key, retries with next key.
*   **`503 Service Unavailable` (from Proxy):** All keys currently rate-limited.
*   **`502 Bad Gateway` (from Proxy):** Network error connecting to Google or upstream proxy. Check `target_url` and `proxy_url`.
*   **Configuration Errors:** Logged on startup, proxy exits.

### Common Docker Commands

*   Logs: `docker logs gemini-proxy` / `docker logs -f gemini-proxy`
*   Stop: `docker stop gemini-proxy`
*   Start: `docker start gemini-proxy`
*   Remove: `docker rm gemini-proxy` (stop first)
*   Rebuild: `docker build --no-cache -t gemini-proxy-openai .`

### Security

*   **Use environment variables for keys.**
*   Do not commit `config.yaml` with keys.
*   Use strong Gemini keys.
*   Secure your network.

## Project Structure
(See previous sections, structure remains the same)

## Contributing
(See CONTRIBUTING.md and CODE_OF_CONDUCT.md)

## License
MIT License - see [LICENSE](LICENSE) file.