# Quick Start Guide

Get the Gemini Proxy up and running in 5 minutes.

## Prerequisites

- Docker or Podman installed
- Google Gemini API keys ([Get them here](https://aistudio.google.com/app/apikey))

## 1. Clone and Setup

```bash
git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git
cd gemini-proxy-key-rotation-rust
```

## 2. Configure

```bash
# Copy example config
cp config.yaml.example config.yaml

# Edit config.yaml and add your API keys
nano config.yaml  # or use your preferred editor
```

**Minimum required changes in `config.yaml`:**
- Replace `YOUR_API_KEY_1_HERE` with your actual Gemini API keys
- Set a strong `admin_token` if you want to use the admin panel

## 3. Run

```bash
# Using the provided script (recommended)
./run.sh

# Or using Docker directly
docker build -t gemini-proxy .
docker run -d --name gemini-proxy -p 8080:8080 -v $(pwd)/config.yaml:/app/config.yaml:ro gemini-proxy
```

## 4. Test

```bash
# Health check
curl http://localhost:8080/health

# Test with OpenAI-compatible request
curl http://localhost:8080/v1/models \
  -H "Authorization: Bearer dummy-key"
```

## 5. Use with OpenAI Clients

Configure your OpenAI client:
- **Base URL**: `http://localhost:8080`
- **API Key**: Any non-empty string (e.g., "dummy")

## Admin Panel (Optional)

If you set `admin_token` in config.yaml:
1. Visit `http://localhost:8080/admin/`
2. Login with your admin token
3. Manage keys, view status, monitor health

## Next Steps

- Read the full [README.md](README.md) for advanced configuration
- Check [SECURITY.md](SECURITY.md) for security best practices
- See [CONTRIBUTING.md](CONTRIBUTING.md) if you want to contribute

## Troubleshooting

- **Port already in use**: Change `port` in `config.yaml`
- **Connection refused**: Check if Docker container is running with `docker ps`
- **API errors**: Verify your Gemini API keys are valid
- **Logs**: View with `docker logs gemini-proxy-container`