# 🚀 Quick Start Guide

Get Gemini Proxy Key Rotation running in under 5 minutes!

## 📋 Prerequisites

- **Google Gemini API Keys**: Get them from [Google AI Studio](https://aistudio.google.com/app/apikey)
- **System**: Linux, macOS, or Windows with WSL2
- **Internet Connection**: For downloading dependencies

## ⚡ 1-Minute Setup

### Option A: Automated Installer (Recommended)

```bash
# Download and run the installer
curl -fsSL https://raw.githubusercontent.com/stranmor/gemini-proxy-key-rotation-rust/main/install.sh | bash

# Follow the prompts - the installer will:
# ✅ Install Rust and Docker (if needed)
# ✅ Clone the repository
# ✅ Build the application
# ✅ Set up configuration files
# ✅ Run tests to verify installation
```

### Option B: Manual Setup

```bash
# Clone the repository
git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git
cd gemini-proxy-key-rotation-rust

# Quick setup
make quick-start
```

## 🔑 Configure Your API Keys

1. **Edit the configuration file**:
   ```bash
   nano config.yaml
   ```

2. **Add your Gemini API keys**:
   ```yaml
   groups:
     - name: "Primary"
       api_keys:
         - "your-gemini-api-key-1"
         - "your-gemini-api-key-2"
         - "your-gemini-api-key-3"
       target_url: "https://generativelanguage.googleapis.com/v1beta/openai/"
   ```

3. **Set up admin access** (optional):
   ```yaml
   server:
     admin_token: "your-secure-admin-token"  # Generate with: openssl rand -hex 32
   ```

## 🚀 Start the Proxy

Choose your preferred method:

### Docker (Recommended for Production)
```bash
make docker-run

# Services will start:
# 🔗 Proxy: http://localhost:4806
# 🗄️ Redis: localhost:6379
# 📊 Admin: http://localhost:4806/admin/
```

### Direct Binary (Development)
```bash
make run

# Proxy starts at: http://localhost:4806
```

### Systemd Service (Linux Production)
```bash
sudo systemctl start gemini-proxy
sudo systemctl status gemini-proxy
```

## ✅ Verify Installation

1. **Check health**:
   ```bash
   curl http://localhost:4806/health
   # Expected: HTTP 200 OK
   ```

2. **Test with detailed health check**:
   ```bash
   curl http://localhost:4806/health/detailed
   # Expected: JSON response with key validation
   ```

3. **View admin dashboard** (if configured):
   ```bash
   open http://localhost:4806/admin/
   ```

## 🔌 Connect Your Application

### Python (OpenAI Library)
```python
import openai

client = openai.OpenAI(
    base_url="http://localhost:4806",
    api_key="dummy-key"  # Ignored, real keys managed by proxy
)

response = client.chat.completions.create(
    model="gemini-1.5-flash-latest",
    messages=[{"role": "user", "content": "Hello!"}]
)
print(response.choices[0].message.content)
```

### Node.js
```javascript
import OpenAI from 'openai';

const openai = new OpenAI({
  baseURL: 'http://localhost:4806',
  apiKey: 'dummy-key', // Ignored, real keys managed by proxy
});

const response = await openai.chat.completions.create({
  model: 'gemini-1.5-flash-latest',
  messages: [{ role: 'user', content: 'Hello!' }],
});

console.log(response.choices[0].message.content);
```

### cURL
```bash
curl http://localhost:4806/v1/chat/completions \
  -H "Authorization: Bearer dummy-key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gemini-1.5-flash-latest",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

## 📊 Monitor Your Proxy

### Command Line
```bash
# View status
make status

# View logs
make logs

# Check health
make health
```

### Web Dashboard
Visit `http://localhost:4806/admin/` to see:
- 📈 Real-time key health scores
- 📊 Request success rates and response times
- 🔧 Key management and configuration
- 🚨 Alert history and system status

## 🔧 Common Operations

### Update Configuration
```bash
# Edit config
nano config.yaml

# Apply changes (Docker)
make docker-restart

# Apply changes (Direct)
# Restart the process (Ctrl+C and make run)
```

### View Logs
```bash
# Docker logs
make docker-logs

# Systemd logs (Linux)
sudo journalctl -u gemini-proxy -f

# Direct binary logs
# Logs appear in terminal where you ran 'make run'
```

### Stop Services
```bash
# Docker
make docker-stop

# Systemd
sudo systemctl stop gemini-proxy

# Direct binary
# Press Ctrl+C in the terminal
```

## 🆘 Troubleshooting

### UAT

To run a full non-interactive verification:

```bash
make uat
```

Expected:
- Build completes
- Services up
- Healthcheck OK at http://localhost:4806/health

### Troubleshooting healthcheck

1) Check /app/busybox exists in container:
```bash
docker compose exec gemini-proxy ls -l /app/busybox || echo "busybox not present"
```

2) Verify docker-compose healthcheck path/port:
- http://localhost:4806/health

3) Port conflict
- Do not kill processes on occupied port.
- Change port via env `PORT` or set `server.port` in config.yaml and re-run docker compose.

### Proxy Won't Start
1. **Check configuration**:
   ```bash
   # Validate YAML syntax
   python -c "import yaml; yaml.safe_load(open('config.yaml'))"
   ```

2. **Check port availability**:
   ```bash
   # See if port 4806 is in use
   lsof -i :4806
   ```

3. **Check logs**:
   ```bash
   make logs
   ```

### API Keys Not Working
1. **Verify keys in Google AI Studio**
2. **Check key format** (should start with `AIza...`)
3. **Test key directly**:
   ```bash
   curl "https://generativelanguage.googleapis.com/v1beta/models?key=YOUR_API_KEY"
   ```

### High Error Rates
1. **Check admin dashboard** for key health scores
2. **Verify quota limits** in Google AI Studio
3. **Check upstream connectivity**:
   ```bash
   curl https://generativelanguage.googleapis.com/v1beta/models
   ```

## 🎯 Next Steps

- **Production Deployment**: See [README.md](README.md#-security--production-deployment)
- **Security Hardening**: Read [SECURITY.md](SECURITY.md)
- **Advanced Configuration**: Check [README.md](README.md#️-configuration)
- **Monitoring Setup**: See [MONITORING.md](MONITORING.md)

## 💡 Tips

- **Start with 2-3 API keys** for basic redundancy
- **Monitor key health scores** in the admin dashboard
- **Set up Redis** for production deployments
- **Use HTTPS** in production environments
- **Backup your configuration** regularly

---

**🎉 You're all set! Your Gemini Proxy is now running and ready to handle requests.**

Need help? Check the [main documentation](README.md) or [open an issue](https://github.com/stranmor/gemini-proxy-key-rotation-rust/issues).