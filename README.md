# Gemini Proxy Key Rotation (Rust) - OpenAI Compatibility

[![CI](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml/badge.svg)](https://github.com/stranmor/gemini-proxy-key-rotation-rust/actions/workflows/rust.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A lightweight, high-performance asynchronous HTTP proxy for Google Gemini models, designed to be compatible with the OpenAI API. This proxy rotates Google Gemini API keys, distributes load, and manages rate limits, allowing you to use Gemini models with your existing OpenAI-compatible applications.

## Key Features

*   **Key Rotation:** Automatically rotates through multiple Gemini API keys to avoid rate limits.
*   **Load Balancing:** Distributes requests across your keys, increasing availability.
*   **Easy Configuration:** Manage all settings in a single `config.yaml` file.
*   **OpenAI Compatibility:** Works with any client that supports the OpenAI API.
*   **Group-Specific Routing:** Use different upstream proxies for different sets of keys.
*   **State Persistence:** Remembers rate-limited keys between restarts.

## Getting Started (Docker - Recommended)

1.  **Clone the repository:**
    ```bash
    git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git
    cd gemini-proxy-key-rotation-rust
    ```

2.  **Build the Docker image:**
    ```bash
    docker build -t gemini-proxy-key-rotation:latest .
    ```

3.  **Configure the proxy:**
    *   Copy the example configuration file:
        ```bash
        cp config.example.yaml config.yaml
        ```
    *   Edit `config.yaml` to add your Gemini API keys.

4.  **Run the proxy:**
    ```bash
    ./run.sh
    ```

5.  **Verify:**
    *   Check the logs: `docker logs -f gemini-proxy-openai-compose`
    *   Test the health check: `curl http://localhost:PORT/health` (replace `PORT` with the port from your `config.yaml`).

## Usage

Configure your OpenAI client to use the proxy by setting the base URL to the proxy's address (e.g., `http://localhost:8080`). The API key can be any non-empty string, as the proxy will use the keys from your `config.yaml` file.

**Example `curl` request:**
```bash
curl http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer dummy-key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gemini-1.5-flash-latest",
    "messages": [{"role": "user", "content": "Explain Rust."}],
    "temperature": 0.7
  }'
```

## Configuration

The `config.yaml` file is used to configure the proxy. Here's a basic example:

```yaml
# config.yaml
server:
  port: 8080

groups:
  - name: "Default"
    api_keys:
      - "your-gemini-api-key-1"
      - "your-gemini-api-key-2"
```

## Advanced Usage

### Building and Running Locally

If you prefer to build and run the proxy locally, you can use the following commands:

```bash
# Build the project
cargo build --release

# Run the proxy
./target/release/gemini-proxy-key-rotation-rust
```

### API Reference

*   `GET /health`: Returns a `200 OK` status if the proxy is running.
*   `/v1/*`: Proxies all requests to the Google Gemini API.

### Error Handling

The proxy is designed to handle errors from the Gemini API gracefully:

*   **429 (Too Many Requests):** The proxy will automatically retry the request with the next available key.
*   **403 (Forbidden):** The key is considered invalid and will be removed from the rotation.
*   **5xx (Server Errors):** The proxy will retry the request with the same key a few times before moving to the next key.

## Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for more information.

## License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.
