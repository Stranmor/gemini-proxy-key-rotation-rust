# Gemini Proxy Key Rotation (Rust) - OpenAI Compatibility

[![CI](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml/badge.svg)](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
<!-- Add Docker Hub badge if applicable: [![Docker Hub](https://img.shields.io/docker/pulls/your_dockerhub_user/your_repo)](https://hub.docker.com/r/your_dockerhub_user/your_repo) -->

**A lightweight, high-performance asynchronous HTTP proxy specifically designed to use Google Gemini models via their OpenAI-compatible API layer.** This proxy rotates Google Gemini API keys, distributes load, and manages rate limits effectively when interacting with the `generativelanguage.googleapis.com/v1beta/openai/` endpoint. Built with Rust, Axum, and Tokio.

**Note:** This proxy is intended *only* for use with Google's OpenAI compatibility layer. It does not support native Gemini API endpoints like `:generateContent`.

## Overview

This proxy acts as a middleman between your OpenAI-compatible application (like clients using OpenAI libraries or tools like Roo Code/Cline) and the Google Gemini API's OpenAI compatibility endpoint. You provide it with multiple Gemini API keys, either via **environment variables (recommended)** or directly in the **`config.yaml` file**. The proxy automatically rotates through them for outgoing requests, handling authentication and rate limits.

**Key Benefits:**

*   **Avoid Rate Limits:** Distributes requests across many Gemini keys.
*   **Increased Availability:** If one key hits its limit, the proxy automatically switches to another.
*   **Flexible Configuration:** Supports providing API keys and group-specific upstream proxies via environment variables (most secure for Docker) or directly in `config.yaml` (useful for local runs). Environment variables always override the config file.
*   **Simplified Client Configuration:** Point your OpenAI client's base URL to this proxy; no need to manage Gemini keys in the client.
*   **Group-Specific Routing:** Use different upstream proxies (`http`, `https`, `socks5`) for different sets of keys, configurable via environment variables or `config.yaml`.
*   **State Persistence:** Remembers rate-limited keys between restarts, avoiding checks on known limited keys until their reset time (daily midnight Pacific Time by default).

## Features

*   Proxies requests specifically to Google's OpenAI compatibility endpoint (`https://generativelanguage.googleapis.com/v1beta/openai/` by default).
*   Supports multiple **groups** of Gemini API keys with optional upstream proxies (`http`, `https`, `socks5`) per group. Groups and their settings are primarily discovered and configured via environment variables.
*   Automatic round-robin key rotation across **all** configured keys (from all groups combined).
*   Handles `429 Too Many Requests` responses from the target API by temporarily disabling the rate-limited key.
*   **Rate Limit Reset:** Limited keys are automatically considered available again after the next **daily midnight in the Pacific Time zone (America/Los_Angeles)** by default.
*   **Persists Rate Limit State:** Saves the limited status and UTC reset time of keys to `key_states.json` (located in the current working directory, or `/app/` in Docker), allowing the proxy to skip known limited keys on startup.
*   Configurable primarily via environment variables (using `.env` with Docker Compose). `config.yaml` is optional, mainly for setting non-default `target_url`s per group or server defaults.
*   **API Keys & Proxies:** Defined via environment variables (recommended). `config.yaml` can provide fallbacks or structure but is overridden by environment variables.
*   Correctly adds the required `x-goog-api-key` and `Authorization: Bearer <key>` headers, replacing any client-sent `Authorization` headers.
*   High performance asynchronous request handling using Axum and Tokio.
*   Graceful shutdown handling (`SIGINT`, `SIGTERM`).
*   Configurable logging using `tracing` and the `RUST_LOG` environment variable.
*   Basic health check endpoint (`/health`).

## Requirements

*   **Docker & Docker Compose:** The easiest and **most secure** way to run the proxy. `docker-compose` is usually included with Docker Desktop. ([Install Docker](https://docs.docker.com/engine/install/)).
*   **Google Gemini API Keys:** Obtain these from [Google AI Studio](https://aistudio.google.com/app/apikey).
*   **(Optional) Rust & Cargo:** Only needed if you want to build or develop locally without Docker. ([Install Rust](https://rustup.rs/)) (Uses Rust 2021 Edition or later).

## Getting Started

### Option 1: Running with Docker Compose (Recommended)

This method uses Docker Compose and a `.env` file to manage configuration, allowing you to securely pass API keys and configure upstream proxies via environment variables. Using `config.yaml` is **optional** with this method.

1.  **Clone the Repository:**
    ```bash
    git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git
    cd gemini-proxy-key-rotation-rust
    ```

2.  **Prepare Environment File (`.env`):**
    *   Copy the example environment file:
        ```bash
        cp .env.example .env
        ```
    *   Edit the `.env` file:
        *   Set `SERVER_PORT_HOST` to the desired port on your host machine (e.g., `8081`). The proxy will be accessible at `http://localhost:<SERVER_PORT_HOST>`.
        *   Set `SERVER_PORT_CONTAINER` to the port the proxy listens on *inside* the container (usually `8080`). This value is also used by the `healthcheck`.
        *   Set `RUST_LOG` to the desired log level (e.g., `info`, `debug`).
        *   **Add Gemini API Keys:** Fill in the `GEMINI_PROXY_GROUP_{NAME}_API_KEYS` variables for each group you want to use (e.g., `GEMINI_PROXY_GROUP_DEFAULT_API_KEYS=key1,key2`). **This is required.**
        *   **(Optional) Add Upstream Proxies:** Set `GEMINI_PROXY_GROUP_{NAME}_PROXY_URL` variables for groups that require an upstream proxy (e.g., `GEMINI_PROXY_GROUP_MY_SOCKS_GROUP_PROXY_URL=socks5://user:pass@host:port`).
        *   See `.env.example` for details on variable naming conventions (`{NAME}` should be uppercase with underscores).
        *   **Make sure the `.env` file is NOT committed to Git.** (It should be included in `.gitignore`).

3.  **Prepare State File (Optional but Recommended):**
    *   For persistence of rate-limited key states across restarts, create an empty file:
        ```bash
        touch key_states.json
        ```
    *   *Docker Compose will automatically mount this file into the container based on the `volumes` section in `docker-compose.yml`.*

4.  **(Optional) Using `config.yaml` with Docker Compose:**
    *   For most Docker Compose use cases, `config.yaml` is **not needed**. Configure everything via `.env`.
    *   You *might* mount `config.yaml` (uncomment the volume line in `docker-compose.yml`) only if you need to:
        *   Define a different default `target_url` for a specific group **and** you don't want to use the `GEMINI_PROXY_GROUP_{NAME}_TARGET_URL` environment variable.
        *   Define different default server settings (`host`/`port`) than the hardcoded ones, although these are usually handled by Docker Compose itself.
    *   **Important:** API keys, proxy URLs, and target URLs defined in `.env` **always override** any values in `config.yaml`.

5.  **Run with Docker Compose:**
    *   This single command builds the image (if necessary) and starts the service in the background.
    ```bash
    # Use 'docker compose' (V2 syntax) or 'docker-compose' (V1 syntax)
    docker compose up -d
    ```

6.  **Verify:**
    *   Check logs: `docker compose logs -f`
    *   Check health: `curl http://localhost:<SERVER_PORT_HOST>/health` (use the host port you set in `.env`, e.g., `8081`)
    *   Test with an OpenAI client pointed to `http://localhost:<SERVER_PORT_HOST>`.
    *   Check if `key_states.json` was created/updated in your local directory.

7.  **Applying `.env` Changes:**
   *   If you modify the `.env` file after the container is running, you **must restart** the container for the changes to take effect. Docker Compose reads the `.env` file only when the container starts.
   *   Use one of the following commands:
       ```bash
       # Option A: Restart the specific service (faster)
       docker compose restart gemini-proxy

       # Option B: Stop and restart all services defined in the compose file
       docker compose down && docker compose up -d
       ```

8.  **Stopping:**
   ```bash
   docker compose down
   ```
   *(Use `docker compose down -v` to also remove the anonymous volume if you used named volumes instead of bind mounts for `key_states.json`)*.

### Option 2: Building and Running Locally

Use this primarily for development. Configuration relies heavily on `config.yaml`.

1.  **Clone Repository:** (If needed)
    ```bash
    git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git
    cd gemini-proxy-key-rotation-rust
    ```
2.  **Prepare Configuration:** Choose **one** primary method:
    *   **Method A (Environment Variables):** Set `GEMINI_PROXY_GROUP_{NAME}_API_KEYS`, optionally `..._PROXY_URL`, and optionally `..._TARGET_URL` variables in your shell. You still need a minimal `config.yaml` (e.g., just `server: {host: 127.0.0.1, port: 8080}`) to pass to the `--config` flag.
    *   **Method B (`config.yaml` only):** Copy `config.example.yaml` to `config.yaml`. Edit it to define your `server` settings and `groups` including `name`, `api_keys`, `proxy_url`, and `target_url`. **Do not** set corresponding environment variables, as they would override the file.

3.  **Build:**
    ```bash
    cargo build --release
    ```
4.  **Run:**
    ```bash
    # Ensure environment variables are set if using Method A configuration
    export RUST_LOG="info" # Optional
    # export GEMINI_PROXY_GROUP_DEFAULT_API_KEYS="key1,key2"
    # ... other env vars ...

    # Run using the relative path to your config file
    ./target/release/gemini-proxy-key-rotation-rust --config config.yaml
    ```
    *   *(The `key_states.json` file will be created/updated in the same directory as `config.yaml`)*

5.  **Verify:**
    *   Check terminal logs.
    *   Check health: `curl http://<HOST>:<PORT>/health`
    *   Test with an OpenAI client pointed to `http://<HOST>:<PORT>`.

## Usage with OpenAI Clients

(This section remains largely the same - the client configuration depends only on the proxy's host and port)

Once the proxy is running, configure your OpenAI client (e.g., Python/JS libraries, Roo Code/Cline, etc.) as follows:

1.  **Set the Base URL / API Host:** Point the client to the proxy's address (protocol, host, port only).
    *   Example (Docker Compose): `http://localhost:8081` (or the `SERVER_PORT_HOST` you set in `.env`)
    *   Example (Local): `http://127.0.0.1:8080` (or your manually configured address)
    *   **Do NOT include `/v1` or other paths in the Base URL.**

2.  **Set the API Key:** Enter **any non-empty placeholder** (e.g., "dummy-key", "ignored"). The proxy manages the *real* Gemini keys internally and **ignores the key sent by the client**, but the field usually requires input.

3.  **Send Requests:** Make requests as you normally would using the OpenAI client library or tool (e.g., to `/v1/chat/completions`, `/v1/models`, etc.). The proxy will intercept these, add the correct Google authentication for the OpenAI compatibility layer using a rotated key, and forward them.

### Example (`curl` to proxy)

```bash
# Example request to list models via the proxy (replace 8081 with your SERVER_PORT_HOST from .env)
curl http://localhost:8081/v1/models \
  -H "Authorization: Bearer dummy-ignored-key" # This header is ignored/replaced

# Example request for chat completion via the proxy (replace 8081 with your SERVER_PORT_HOST from .env)
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
2.  Set **Base URL** to the proxy address (e.g., `http://localhost:8081` or your `SERVER_PORT_HOST` from `.env`).
3.  Set **API Key** to any non-empty placeholder (e.g., "dummy").

**Example Configuration Screenshot:**
![Roo Code Configuration Example](2025-04-13_14-02.png)

## Configuration (`config.yaml`)

This file is **optional for Docker Compose** (if using `.env` for everything) but **required for local runs** via `cargo run -- --config config.yaml`.

**Behavior:**
*   **Environment variables define groups and settings:** The application discovers groups based on `GEMINI_PROXY_GROUP_{NAME}_API_KEYS` variables. It uses `{NAME}` as the group name. API keys, proxy URLs, and target URLs are taken from the corresponding `..._API_KEYS`, `..._PROXY_URL`, and `..._TARGET_URL` environment variables.
*   **YAML for defaults:** `config.yaml` is used *only* as a fallback for `target_url` (if the `..._TARGET_URL` environment variable is not set) and for default `server` settings. If `config.yaml` is missing or empty, hardcoded defaults are used.
*   **Environment variables for server/log settings:** `SERVER_PORT_CONTAINER` and `RUST_LOG` in `.env` (used by Docker Compose) control the container's internal port and logging.

**Recommendation:**
*   **For Docker:** Use `.env` for everything (API keys, proxy URLs, target URLs, ports, log level). You usually **do not need** `config.yaml` at all.
*   **For Local:** Use `config.yaml` for convenience, or set environment variables directly.

```yaml
# config.yaml (Example: Only used to set a custom target_url for GROUP1)
server: # Optional: Defaults to host: 0.0.0.0, port: 8080 if omitted
  host: "0.0.0.0"
  port: 8080
groups:
  # Keys, proxy, and target_url for GROUP1 come from env vars:
  # GEMINI_PROXY_GROUP_GROUP1_API_KEYS=...
  # GEMINI_PROXY_GROUP_GROUP1_PROXY_URL=...
  # GEMINI_PROXY_GROUP_GROUP1_TARGET_URL=...
  # This entry in YAML is only needed if you want to provide a fallback target_url
  # in case the environment variable is not set.
  - name: "GROUP1" # Must match the {NAME} used in env vars
    target_url: "https://fallback-target-if-env-missing.com" # Fallback target

  # Other groups like 'DEFAULT' will be created from env vars
  # and use the default target_url unless specified here.
  - name: "DEFAULT"
    # No target_url specified, uses default
```
*   **Priority:** Environment variables (`..._API_KEYS`, `..._PROXY_URL`, `..._TARGET_URL`) define the groups and their settings. `config.yaml` only provides fallbacks for `target_url` and server settings.

## Environment Variable Configuration

Environment variables provide the primary way to configure the proxy, especially for Docker deployments using a `.env` file.

### API Keys
*   **Purpose:** Defines a group and provides its API keys. **This is mandatory for each group you want to use.**
*   **Variable:** `GEMINI_PROXY_GROUP_{NAME}_API_KEYS`
*   **Value:** Comma-separated string of API keys (e.g., `"key1,key2,key3"`).

### Upstream Proxy URL (Per Group)
*   **Purpose:** (Optional) Define an upstream proxy (http, https, socks5) for a specific group.
*   **Variable:** `GEMINI_PROXY_GROUP_{NAME}_PROXY_URL`
*   **Value:** The full proxy URL (e.g., `"socks5://user:pass@host:port"`). Set to an empty string (`""`) or omit the variable entirely for no proxy.

### Target URL (Per Group)
*   **Purpose:** (Optional) Define a non-default base URL for the target API for a specific group.
*   **Variable:** `GEMINI_PROXY_GROUP_{NAME}_TARGET_URL`
*   **Value:** The base URL (e.g., `"https://alternative.api.endpoint.com"`). If omitted, uses the value from `config.yaml` (if defined for that group) or the default Google API endpoint.
*   **Override:** Replaces the `target_url` from `config.yaml` for the matching group.

### Server Port (Inside Container)
*   **Purpose:** Set the port the application listens on inside the container.
*   **Variable:** `SERVER_PORT_CONTAINER` (Used by `docker-compose.yml`)
*   **Value:** Port number (e.g., `8080`). Must match the container port in the `ports` mapping.

### Log Level
*   **Purpose:** Control the logging verbosity.
*   **Variable:** `RUST_LOG`
*   **Value:** Log level (e.g., `error`, `warn`, `info`, `debug`, `trace`).

### Group Name (`{NAME}`) in Variables
*   The `{NAME}` part in `GEMINI_PROXY_GROUP_{NAME}_API_KEYS`, `..._PROXY_URL`, and `..._TARGET_URL` **defines the canonical name** of the group.
*   Use clear, descriptive names consisting of **uppercase letters, numbers, and underscores** (e.g., `DEFAULT`, `TEAM_X`, `LOW_PRIORITY`, `GEMINI_ALT_8`). This name will be used internally and in logs.

**Naming Examples:**

| Group Name       | API Key Variable                         | Proxy URL Variable                         | Target URL Variable                        |
| :--------------- | :--------------------------------------- | :----------------------------------------- | :----------------------------------------- |
| `DEFAULT`        | `GEMINI_PROXY_GROUP_DEFAULT_API_KEYS`    | `GEMINI_PROXY_GROUP_DEFAULT_PROXY_URL`     | `GEMINI_PROXY_GROUP_DEFAULT_TARGET_URL`    |
| `TEAM_X`         | `GEMINI_PROXY_GROUP_TEAM_X_API_KEYS`     | `GEMINI_PROXY_GROUP_TEAM_X_PROXY_URL`      | `GEMINI_PROXY_GROUP_TEAM_X_TARGET_URL`     |
| `GEMINI_ALT_8`   | `GEMINI_PROXY_GROUP_GEMINI_ALT_8_API_KEYS`| `GEMINI_PROXY_GROUP_GEMINI_ALT_8_PROXY_URL`| `GEMINI_PROXY_GROUP_GEMINI_ALT_8_TARGET_URL` |

## Operation & Maintenance

(Sections on Logging, Health Check, Key State Persistence, Error Handling, Docker Commands remain largely the same but reviewed for clarity)

### Logging
*   Use `RUST_LOG` env var (e.g., `info`, `debug`, `trace`). Default: `info`.

### Health Check
*   `GET /health` returns `200 OK`. Use for basic monitoring.

### Key State Persistence (`key_states.json`)
*   **Purpose:** Remembers rate-limited keys to avoid checking them immediately after restarts.
*   **Location:** Saved as `key_states.json` in the current working directory of the application (or `/app/` inside the default Docker container). When using Docker Compose, the `docker-compose.yml` maps your local `./key_states.json` into the container for persistence. Create an empty file locally first if it doesn't exist (`touch key_states.json`).
*   **Reset Logic:** Daily midnight Pacific Time (America/Los_Angeles).
*   **Management:** Automatic. Deleting the file resets the state memory.
*   **.gitignore:** Included by default.

### Error Handling
*   **400 (from Target):** Invalid request from *your client*. Check OpenAI specs.
*   **401/403 (from Target):** Invalid/revoked Gemini key or permissions issue.
*   **429 (from Target):** Key rate-limited. Proxy handles retry with next key. Returns last 429 if all keys fail.
*   **503 (from Proxy):** All keys currently marked as rate-limited.
*   **502 (from Proxy):** Network error connecting to Google/upstream proxy.
*   **500 (from Proxy):** Internal proxy error. Check proxy logs.
*   **Config Errors:** Logged on startup, proxy exits.

### Common Docker Compose Commands
*   **Start/Run (background):** `docker compose up -d` (Builds if needed)
*   **View Logs:** `docker compose logs -f` (or `docker compose logs`)
*   **Stop:** `docker compose stop`
*   **Stop and Remove Containers/Networks:** `docker compose down`
*   **Stop and Remove Containers/Networks/Volumes:** `docker compose down -v` (Use cautiously!)
*   **Restart:** `docker compose restart`
*   **Rebuild Image:** `docker compose build` (or `docker compose up -d --build`)
*   **Check Status:** `docker compose ps`

## Security Considerations

*   **API Keys:** Use environment variables (`.env` file) for API keys. Avoid storing them directly in `config.yaml`, especially if the file is committed to version control.
*   **Files:** Do not commit `config.yaml` (if it contains secrets) or `key_states.json` to Git. (`.gitignore` includes these by default).
*   **Network:** Expose the proxy only to trusted networks. Consider a reverse proxy (Nginx/Caddy) for TLS and advanced access control if needed.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## License

MIT License - see the [LICENSE](LICENSE) file.