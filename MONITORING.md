# 📊 Monitoring Guide

Comprehensive monitoring and observability for Gemini Proxy Key Rotation.

## 🎯 Overview

The proxy provides multiple layers of monitoring:
- **Health Scoring**: Real-time key performance metrics (0.0-1.0)
- **Proactive Alerts**: Automated notifications for degraded performance
- **Admin Dashboard**: Web-based monitoring interface
- **Metrics Export**: Prometheus-compatible metrics
- **Structured Logging**: JSON logs with correlation IDs

## 📈 Key Health Scoring

### Health Score Calculation

Each API key receives a health score from 0.0 (unhealthy) to 1.0 (perfect):

```
Health Score = Success Rate - Failure Penalty
```

Where:
- **Success Rate**: `successful_requests / total_requests`
- **Failure Penalty**: `consecutive_failures * 0.1` (capped at 0.5)

### Health Score Ranges

| Score | Status | Description |
|-------|--------|-------------|
| 0.9-1.0 | 🟢 Excellent | Perfect or near-perfect performance |
| 0.7-0.8 | 🟡 Good | Good performance with occasional failures |
| 0.5-0.6 | 🟠 Degraded | Frequent failures, needs attention |
| 0.0-0.4 | 🔴 Poor | Mostly failing, likely blocked |
| Blocked | ⛔ Disabled | Temporarily disabled due to failures |

### Monitoring Key Health

```bash
# Check key health via API
curl http://localhost:8081/admin/api/keys/health

# View in admin dashboard
open http://localhost:8081/admin/
```

## 🚨 Automated Alerts

### Alert Conditions

The system automatically generates alerts when:

1. **Unhealthy Keys Alert**
   - **Trigger**: >3 keys with health score <0.5
   - **Severity**: Warning
   - **Action**: Check API quotas, rotate keys if needed

2. **High Error Rate Alert**
   - **Trigger**: Overall error rate >10%
   - **Severity**: Critical
   - **Action**: Investigate upstream issues

3. **Slow Response Alert**
   - **Trigger**: Average response time >5 seconds
   - **Severity**: Warning
   - **Action**: Check network connectivity

4. **Circuit Breaker Open**
   - **Trigger**: Circuit breaker opens for upstream service
   - **Severity**: Critical
   - **Action**: Upstream service is down

### Alert Configuration

```yaml
# config.yaml
monitoring:
  alert_thresholds:
    unhealthy_keys: 3
    error_rate: 0.1  # 10%
    response_time_secs: 5
    
  # Alert destinations (future feature)
  notifications:
    webhook_url: "https://your-webhook.com/alerts"
    email: "admin@yourcompany.com"
```

## 🎛️ Admin Dashboard

### Accessing the Dashboard

1. **Configure admin token**:
   ```yaml
   # config.yaml
   server:
     admin_token: "your-secure-token"  # Generate with: openssl rand -hex 32
   ```

2. **Access dashboard**:
   ```
   http://localhost:8081/admin/
   ```

### Dashboard Features

#### **Overview Page**
- System uptime and version
- Total requests and success rate
- Active keys and health distribution
- Recent alerts and incidents

#### **Key Management**
- Real-time health scores for all keys
- Request counts and success rates
- Manual key enable/disable
- Key rotation history

#### **Performance Metrics**
- Response time trends
- Request volume over time
- Error rate by endpoint
- Geographic distribution (if available)

#### **System Health**
- Circuit breaker status
- Redis connectivity
- Memory and CPU usage
- Network connectivity tests

#### **Configuration**
- View current configuration
- Hot-reload settings (future feature)
- Backup and restore config

## 📊 Metrics Export

### Prometheus Metrics

The proxy exports Prometheus-compatible metrics at `/metrics`:

```bash
curl http://localhost:8081/metrics
```

#### Key Metrics

```prometheus
# Request metrics
gemini_proxy_requests_total{method="POST",status="200",group="primary"} 1234
gemini_proxy_request_duration_seconds{method="POST",group="primary"} 0.123

# Key health metrics
gemini_proxy_key_health_score{key_id="key_1",group="primary"} 0.95
gemini_proxy_key_requests_total{key_id="key_1",group="primary"} 567
gemini_proxy_key_failures_total{key_id="key_1",group="primary"} 12

# System metrics
gemini_proxy_uptime_seconds 86400
gemini_proxy_active_keys{group="primary"} 3
gemini_proxy_blocked_keys{group="primary"} 0

# Circuit breaker metrics
gemini_proxy_circuit_breaker_state{target="upstream"} 0  # 0=closed, 1=open, 2=half-open
gemini_proxy_circuit_breaker_failures{target="upstream"} 2
```

### Grafana Dashboard

Import the provided Grafana dashboard:

```bash
# Download dashboard JSON
curl -O https://raw.githubusercontent.com/stranmor/gemini-proxy-key-rotation-rust/main/monitoring/grafana-dashboard.json

# Import in Grafana UI or via API
curl -X POST \
  http://grafana:3000/api/dashboards/db \
  -H "Authorization: Bearer $GRAFANA_API_KEY" \
  -H "Content-Type: application/json" \
  -d @grafana-dashboard.json
```

## 📋 Structured Logging

### Log Format

All logs are structured JSON with correlation IDs:

```json
{
  "timestamp": "2024-01-15T10:30:00.123Z",
  "level": "INFO",
  "target": "gemini_proxy::handlers",
  "message": "Request completed successfully",
  "request_id": "req_abc123",
  "method": "POST",
  "path": "/v1/chat/completions",
  "status": 200,
  "duration_ms": 234,
  "key_group": "primary",
  "key_preview": "AIza...xyz"
}
```

### Log Levels

```bash
# Debug: Detailed request/response info
RUST_LOG=debug make run

# Info: Standard operational logs (default)
RUST_LOG=info make run

# Warn: Warnings and recoverable errors
RUST_LOG=warn make run

# Error: Errors and failures only
RUST_LOG=error make run
```

### Log Aggregation

#### ELK Stack Integration

```yaml
# docker-compose.yml
version: '3.8'
services:
  gemini-proxy:
    # ... existing config
    logging:
      driver: "json-file"
      options:
        max-size: "10m"
        max-file: "3"
        
  filebeat:
    image: docker.elastic.co/beats/filebeat:8.11.0
    volumes:
      - /var/lib/docker/containers:/var/lib/docker/containers:ro
      - ./filebeat.yml:/usr/share/filebeat/filebeat.yml:ro
```

#### Fluentd Integration

```yaml
# fluent.conf
<source>
  @type tail
  path /var/log/gemini-proxy/*.log
  pos_file /var/log/fluentd/gemini-proxy.log.pos
  tag gemini.proxy
  format json
</source>

<match gemini.proxy>
  @type elasticsearch
  host elasticsearch
  port 9200
  index_name gemini-proxy
</match>
```

## 🔍 Health Checks

### Endpoint Types

1. **Basic Health Check** (`/health`)
   - **Purpose**: Liveness probe
   - **Response**: HTTP 200 if service is running
   - **Use**: Load balancer health checks

2. **Detailed Health Check** (`/health/detailed`)
   - **Purpose**: Readiness probe
   - **Response**: JSON with key validation results
   - **Use**: Kubernetes readiness probes

3. **Metrics Endpoint** (`/metrics`)
   - **Purpose**: Prometheus scraping
   - **Response**: Prometheus format metrics
   - **Use**: Monitoring system integration

### Kubernetes Health Checks

```yaml
# k8s-deployment.yaml
spec:
  containers:
  - name: gemini-proxy
    livenessProbe:
      httpGet:
        path: /health
        port: 8081
      initialDelaySeconds: 30
      periodSeconds: 10
      
    readinessProbe:
      httpGet:
        path: /health/detailed
        port: 8081
      initialDelaySeconds: 5
      periodSeconds: 5
```

## 📱 Monitoring Best Practices

### 1. Set Up Alerts

```bash
# Generate secure admin token
make generate-admin-token

# Configure in config.yaml
server:
  admin_token: "generated-token-here"
```

### 2. Monitor Key Health Trends

- **Daily**: Check key health scores in admin dashboard
- **Weekly**: Review key rotation patterns and usage
- **Monthly**: Analyze quota usage and plan capacity

### 3. Set Up Automated Monitoring

```bash
# Health check script
#!/bin/bash
HEALTH=$(curl -s http://localhost:8081/health/detailed | jq -r '.healthy')
if [ "$HEALTH" != "true" ]; then
  echo "ALERT: Gemini Proxy unhealthy" | mail -s "Proxy Alert" admin@company.com
fi
```

### 4. Log Analysis

```bash
# Find high error rates
grep '"level":"ERROR"' /var/log/gemini-proxy/app.log | wc -l

# Analyze response times
grep '"duration_ms"' /var/log/gemini-proxy/app.log | jq '.duration_ms' | sort -n

# Check key usage distribution
grep '"key_preview"' /var/log/gemini-proxy/app.log | jq -r '.key_preview' | sort | uniq -c
```

### 5. Performance Monitoring

```bash
# Monitor resource usage
docker stats gemini-proxy

# Check connection counts
ss -tuln | grep :8081

# Monitor Redis if used
redis-cli info stats
```

## 🚨 Troubleshooting

### High Error Rates

1. **Check key health scores** in admin dashboard
2. **Verify API quotas** in Google AI Studio
3. **Test keys individually**:
   ```bash
   curl "https://generativelanguage.googleapis.com/v1beta/models?key=YOUR_KEY"
   ```

### Slow Response Times

1. **Check network connectivity** to Google APIs
2. **Monitor upstream response times** in logs
3. **Verify circuit breaker status** in admin dashboard

### Memory Issues

1. **Check Redis memory usage** if configured
2. **Monitor request queue sizes** in metrics
3. **Review log retention settings**

### Key Rotation Issues

1. **Check key health trends** over time
2. **Verify key quotas** haven't been exceeded
3. **Review rotation algorithm** in logs

---

**📊 With comprehensive monitoring in place, you'll have full visibility into your proxy's performance and health.**

Need help setting up monitoring? [Open an issue](https://github.com/stranmor/gemini-proxy-key-rotation-rust/issues) or check the [main documentation](README.md).