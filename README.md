# Gemini Proxy Key Rotation (Rust) - OpenAI Compatibility

 [![CI](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml/badge.svg)](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml)
 [![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
 <!-- Add Docker Hub badge if applicable: [![Docker Hub](https://img.shields.io/docker/pulls/your_dockerhub_user/your_repo)](https://hub.docker.com/r/your_dockerhub_user/your_repo) -->

 **A lightweight, high-performance asynchronous HTTP proxy specifically designed to use Google Gemini models via their OpenAI-compatible API layer.** This proxy rotates Google Gemini API keys, distributes load, and manages rate limits effectively when interacting with the `generativelanguage.googleapis.com/v1beta/openai/` endpoint. Built with Rust, Axum, and Tokio.

 **Note:** This proxy is intended *only* for use with Google's OpenAI compatibility layer. It does not support native Gemini API endpoints like `:generateContent`.

 ## Overview

 This proxy acts as a middleman between your OpenAI-compatible application (like clients using OpenAI libraries or tools like Roo Code/Cline) and the Google Gemini API's OpenAI compatibility endpoint. You provide it with multiple Gemini API keys **(ideally via environment variables)**, and it automatically rotates through them for outgoing requests, handling authentication and rate limits.

 **Key Benefits:**

 *   **Avoid Rate Limits:** Distributes requests across many Gemini keys.
 *   **Increased Availability:** If one key hits its limit, the proxy automatically switches to another.
 *   **Secure Key Management:** **Recommended method:** Provide keys via environment variables, avoiding storage in configuration files.
 *   **Simplified Client Configuration:** Point your OpenAI client's base URL to this proxy; no need to manage Gemini keys in the client.
 *   **Group-Specific Routing:** Use different upstream proxies (e.g., SOCKS5) for different sets of keys if needed.
 *   **State Persistence:** Remembers rate-limited keys between restarts, avoiding checks on known limited keys until their reset time (daily midnight Pacific Time by default).

 ## Features

 *   Proxies requests specifically to Google's OpenAI compatibility endpoint (`https://generativelanguage.googleapis.com/v1beta/openai/` by default).
 *   Supports multiple **groups** of Gemini API keys with optional upstream proxies (`http`, `https`, `socks5`) per group.
 *   Automatic round-robin key rotation across **all** configured keys (from all groups combined).
 *   Handles `429 Too Many Requests` responses from the target API by temporarily disabling the rate-limited key.
 *   **Rate Limit Reset:** Limited keys are automatically considered available again after the next **daily midnight in the Pacific Time zone (America/Los_Angeles)** by default.
 *   **Persists Rate Limit State:** Saves the limited status and UTC reset time of keys to `key_states.json` (located next to `config.yaml`), allowing the proxy to skip known limited keys on startup.
 *   Configurable via a single YAML file (`config.yaml`).
 *   **API keys should ideally be provided using environment variables** for better security (overrides `config.yaml`).
 *   Correctly adds the required `x-goog-api-key` and `Authorization: Bearer <key>` headers, replacing any client-sent `Authorization` headers.
 *   High performance asynchronous request handling using Axum and Tokio.
 *   Graceful shutdown handling (`SIGINT`, `SIGTERM`).
 *   Configurable logging using `tracing` and the `RUST_LOG` environment variable.
 *   Basic health check endpoint (`/health`).

 ## Requirements

 *   **Docker:** The easiest and **most secure** way to run the proxy, especially when handling API keys. ([Install Docker](https://docs.docker.com/engine/install/))
 *   **Google Gemini API Keys:** Obtain these from [Google AI Studio](https://aistudio.google.com/app/apikey).
 *   **(Optional) Rust & Cargo:** Only needed if you want to build or develop locally without Docker. ([Install Rust](https://rustup.rs/)) (Uses Rust 2021 Edition or later).

 ## Getting Started

 ### Option 1: Running with Docker (Recommended & Most Secure)

 This method uses environment variables to pass API keys securely, which is the preferred approach.

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
         *   Define your `groups`. Add group names (e.g., `default`, `team-a`).
         *   **IMPORTANT:** For groups where you will provide keys via environment variables, leave the `api_keys` list **empty (`api_keys: []`)** or **omit it entirely** within that group's definition.
         *   Configure `proxy_url` per group only if needed.
     *   *(The `key_states.json` file will be automatically created/updated in the same directory where `config.yaml` is mounted inside the container).*

 3.  **Build the Docker Image:**
     ```bash
     docker build -t gemini-proxy-openai .
     ```

 4.  **Run the Container (Providing Keys via Environment Variables):**

     Replace `<YOUR_COMMA_SEPARATED_GEMINI_KEYS>` with your actual keys. Adjust ports and add more `-e` variables for other groups as needed. **Mount the directory containing `config.yaml`.**

     ```bash
     # Example: Using the 'default' group defined in config.yaml
     docker run -d --name gemini-proxy \
       -p 8081:8080 \
       -v "$(pwd):/app" \
       -e RUST_LOG="info" \
       -e GEMINI_PROXY_GROUP_DEFAULT_API_KEYS="<YOUR_COMMA_SEPARATED_GEMINI_KEYS>" \
       gemini-proxy-openai --config /app/config.yaml
     ```
     *   **Why environment variables?** This prevents storing your secret API keys directly in configuration files, which is much safer.
     *   **Explanation:**
         *   `-p 8081:8080`: Maps host port 8081 to container port 8080.
         *   `-v "$(pwd):/app"`: Mounts the host directory (containing `config.yaml`) to `/app` in the container. Needed for reading config and writing `key_states.json`.
         *   `-e RUST_LOG="info"`: Sets log level.
         *   `-e GEMINI_PROXY_GROUP_DEFAULT_API_KEYS="..."`: Securely provides keys for the group named `default`. See [API Key Environment Variables](#api-key-environment-variables) for naming rules for other groups.
         *   `--config /app/config.yaml`: Tells the proxy where to find the config *inside* the container.

 5.  **Verify:**
     *   Check logs: `docker logs gemini-proxy`.
     *   Check health: `curl http://localhost:8081/health`
     *   Test with an OpenAI client pointed to `http://localhost:8081`.
     *   Check if `key_states.json` appeared in your local directory.

 ### Option 2: Building and Running Locally (Less Secure for Keys)

 Use this primarily for development or if you understand the security implications of potentially storing keys in `config.yaml`. Using environment variables (Step 4 below) is still recommended even for local runs.

 1.  **Clone Repository:** (If needed)
     ```bash
     git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git
     cd gemini-proxy-key-rotation-rust
     ```
 2.  **Prepare `config.yaml`:**
     *   Copy `config.example.yaml` to `config.yaml`.
     *   Edit `server.host` (`127.0.0.1` or `0.0.0.0`) and `server.port`.
     *   Define `groups`. **Preferably, leave `api_keys: []` empty and use environment variables (Step 4).** If you must add keys here, be aware of the security risk.
 3.  **Build:**
     ```bash
     cargo build --release
     ```
 4.  **Run (Recommended: Using Environment Variables):**
     ```bash
     export RUST_LOG="info" # Optional
     export GEMINI_PROXY_GROUP_DEFAULT_API_KEYS="key1,key2"
     # export GEMINI_PROXY_GROUP_ANOTHER_GROUP_API_KEYS="key3"

     ./target/release/gemini-proxy-key-rotation-rust --config config.yaml
     ```
     *   *(The `key_states.json` file will be created/updated in the same directory as `config.yaml`)*

 5.  **Verify:**
     *   Check terminal logs.
     *   Check health: `curl http://<HOST>:<PORT>/health`
     *   Test with an OpenAI client pointed to `http://<HOST>:<PORT>`.

 ## Usage with OpenAI Clients

 (This section remains largely the same - the client doesn't need to know how the keys are managed by the proxy)

 Once the proxy is running, configure your OpenAI client (e.g., Python/JS libraries, Roo Code/Cline, etc.) as follows:

 1.  **Set the Base URL / API Host:** Point the client to the proxy's address (protocol, host, port only).
     *   Example (Docker): `http://localhost:8081`
     *   Example (Local): `http://127.0.0.1:8080` (or your configured address)
     *   **Do NOT include `/v1` or other paths in the Base URL.**

 2.  **Set the API Key:** Enter **any non-empty placeholder** (e.g., "dummy-key", "ignored"). The proxy manages the *real* Gemini keys internally and **ignores the key sent by the client**, but the field usually requires input.

 3.  **Send Requests:** Make requests as you normally would using the OpenAI client library or tool (e.g., to `/v1/chat/completions`, `/v1/models`, etc.). The proxy will intercept these, add the correct Google authentication for the OpenAI compatibility layer using a rotated key, and forward them.

 ### Example (`curl` to proxy)

 ```bash
 # Example request to list models via the proxy
 curl http://localhost:8081/v1/models \
   -H "Authorization: Bearer dummy-ignored-key" # This header is ignored/replaced

 # Example request for chat completion via the proxy
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
 2.  Set **Base URL** to the proxy address (e.g., `http://localhost:8081`).
 3.  Set **API Key** to any non-empty placeholder (e.g., "dummy").

 **Example Configuration Screenshot:**
 ![Roo Code Configuration Example](2025-04-13_14-02.png)

 ## Configuration (`config.yaml`)

 This file defines the server settings and groups of keys.

 ```yaml
 # config.yaml
 server:
   host: "0.0.0.0" # Use "0.0.0.0" for Docker, "127.0.0.1" for local-only
   port: 8080     # Port the proxy listens on

 groups:
   # --- Example Group 1: Keys via Environment Variable (Recommended) ---
   - name: "default"
     # api_keys is empty here; keys provided by GEMINI_PROXY_GROUP_DEFAULT_API_KEYS env var
     api_keys: []

   # --- Example Group 2: Another group using Env Vars & SOCKS5 Proxy ---
   - name: "team-x-proxy"
     proxy_url: "socks5://user:pass@your-proxy.com:1080"
     # Keys provided by GEMINI_PROXY_GROUP_TEAM_X_PROXY_API_KEYS env var
     api_keys: []

   # --- Example Group 3: Keys directly in config (NOT RECOMMENDED for secrets) ---
   # - name: "direct-keys-example"
   #   api_keys:
   #     - "AIzaSy..............." # Less secure
   #     - "AIzaSy..............."
 ```
 *   **Key Management Priority:** If an [environment variable](#api-key-environment-variables) exists for a group's keys, it **always overrides** the `api_keys` list in this file for that group.

 ## API Key Environment Variables

 **This is the recommended and most secure method for providing API keys.**

 *   **Purpose:** Allows passing sensitive API keys during container startup without storing them in configuration files.
 *   **Naming Convention:** `GEMINI_PROXY_GROUP_{SANITIZED_GROUP_NAME}_API_KEYS`
     *   The `name` field from `config.yaml` is converted to `UPPERCASE`.
     *   All non-alphanumeric characters in the name are replaced with underscores (`_`).
 *   **Value:** A comma-separated string of your Gemini API keys (e.g., `"key1,key2,key3"`). Whitespace around keys/commas is automatically trimmed.

 **How it Works (Override):**
 When the proxy starts, it first reads the groups from `config.yaml`. Then, for each group, it checks if a corresponding environment variable exists.
 *   **If the environment variable IS SET:** The keys from the environment variable are used for that group, completely **replacing** any keys listed under `api_keys:` in `config.yaml` for that specific group.
 *   **If the environment variable IS NOT SET:** The keys listed under `api_keys:` in `config.yaml` are used for that group. If `api_keys` is empty or missing in the config file AND the environment variable is not set, that group will have no keys.

 **Examples:**

 | Group Name (`config.yaml`) | Environment Variable Variable Name              | Example Value        |
 | :------------------------- | :---------------------------------------------- | :------------------- |
 | `default`                  | `GEMINI_PROXY_GROUP_DEFAULT_API_KEYS`           | `"keyA,keyB"`        |
 | `team-x-proxy`             | `GEMINI_PROXY_GROUP_TEAM_X_PROXY_API_KEYS`      | `"keyC, keyD"`       |
 | `Group 1!`                 | `GEMINI_PROXY_GROUP_GROUP_1__API_KEYS`          | `"keyE"`             |

 ## Operation & Maintenance

 (Sections on Logging, Health Check, Key State Persistence, Error Handling, Docker Commands remain largely the same but reviewed for clarity)

 ### Logging
 *   Use `RUST_LOG` env var (e.g., `info`, `debug`, `trace`). Default: `info`.

 ### Health Check
 *   `GET /health` returns `200 OK`. Use for basic monitoring.

 ### Key State Persistence (`key_states.json`)
 *   **Purpose:** Remembers rate-limited keys to avoid checking them immediately after restarts.
 *   **Location:** Same directory as the active `config.yaml`.
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

 ### Common Docker Commands
 *   Logs: `docker logs gemini-proxy`
 *   Stop: `docker stop gemini-proxy`
 *   Start: `docker start gemini-proxy`
 *   Remove: `docker rm gemini-proxy`
 *   Rebuild: `docker build -t gemini-proxy-openai .`

 ## Security Considerations

 *   **API Keys:** **Use environment variables.** Avoid storing keys in `config.yaml`.
 *   **Files:** Do not commit `config.yaml` (if it contains secrets) or `key_states.json` to Git.
 *   **Network:** Expose the proxy only to trusted networks. Consider a reverse proxy (Nginx/Caddy) for TLS and advanced access control if needed.

 ## Contributing

 See [CONTRIBUTING.md](CONTRIBUTING.md) and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

 ## License

 MIT License - see the [LICENSE](LICENSE) file.