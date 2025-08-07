# Configuration Architecture

## 🎯 Single Source of Truth Principle

**config.yaml** - the single source of truth for all application settings.

## 📁 Configuration Structure

### config.yaml - Application Settings
```yaml
server:
  port: 4806                    # Application port (used everywhere)
  connect_timeout_secs: 10
  request_timeout_secs: 60
  top_p: 0.7
  max_tokens_per_request: 250000  # Optional: per-request token limit (reject if exceeded)

redis_url: "redis://redis:6379" # Redis connection
redis_key_prefix: "gemini_proxy"

groups:                         # API keys and groups
  - name: "default"
    api_keys: [...]

rate_limit:                     # Request limits
  requests_per_minute: 60
  burst_size: 10
```

### .env - Docker Environment Only
```bash
# External Redis port (if 6379 is occupied)
REDIS_PORT=6381

# Docker logging
RUST_LOG=info
RUST_BACKTRACE=0

# Redis UI
REDIS_UI_PORT=8082
REDIS_UI_USER=admin
REDIS_UI_PASSWORD=secure_password_here
```

## 🔄 How It Works

### Tokenizer Lifecycle (Fail-Fast vs Fallback)

- On production startup, tokenizer initialization is mandatory. Any failure results in an immediate startup error (fail-fast).
- In test/dev environments, if the tokenizer cannot be fetched/initialized, the application installs a minimal fallback (whitespace, WordLevel) to enable local testing.

### Token Limit Guardrails

- If `server.max_tokens_per_request` is set:
  - The proxy computes the request token count with the shared tokenizer before forwarding.
  - Requests exceeding the limit are rejected with a `RequestTooLarge` application error.
  - Metrics emitted:
    - `request_token_count` (histogram) — records calculated token counts per request
    - `token_limit_blocks_total` (counter) — increments on each limit-based rejection
- If unset, a safe default (e.g., 250,000) is used internally; adjust per deployment needs.

1. **Application reads config.yaml** and uses `server.port: 4806`
2. **Docker maps ports**: `localhost:4806 -> container:4806`
3. **No environment variables** for application settings
4. **One configuration file** = one source of truth

## ✅ Benefits

- **No duplication** of settings between files
- **Clear responsibility**: .env only for Docker, config.yaml for application
- **Easy to change port**: modify only in config.yaml
- **No cognitive dissonance** between different configurations

## 🚀 Usage

```bash
# Change application port
vim config.yaml  # server.port: 4807

# Restart
make docker-restart

# Application will be available on new port
curl http://localhost:4807/health
```

## 🔧 Migration from Old Architecture

**Before (bad):**
- Port in .env: `PROXY_PORT=8080`
- Port in config.yaml: `server.port: 4805`
- Port in docker-compose.yml: `${PROXY_PORT:-8080}:8080`
- Conflicts and confusion

**After (good):**
- Port only in config.yaml: `server.port: 4806`
- Docker uses the same port: `4806:4806`
- Single source of truth for all settings