#!/bin/bash

# Script for Redis health diagnostics
# Usage: ./scripts/redis-health-check.sh

set -euo pipefail

REDIS_CONTAINER="gemini-proxy-redis"
LOG_FILE="/tmp/redis-health-$(date +%Y%m%d-%H%M%S).log"

echo "🔍 Redis Diagnostics - $(date)" | tee "$LOG_FILE"
echo "=================================" | tee -a "$LOG_FILE"

# Check container status
echo "📊 Container Status:" | tee -a "$LOG_FILE"
docker ps --filter "name=$REDIS_CONTAINER" --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}" | tee -a "$LOG_FILE"

# Check resource usage
echo -e "\n💾 Resource Usage:" | tee -a "$LOG_FILE"
docker stats "$REDIS_CONTAINER" --no-stream --format "table {{.Container}}\t{{.CPUPerc}}\t{{.MemUsage}}\t{{.MemPerc}}" | tee -a "$LOG_FILE"

# Redis information
echo -e "\n📈 Redis Information:" | tee -a "$LOG_FILE"
docker exec "$REDIS_CONTAINER" redis-cli INFO memory | grep -E "(used_memory|maxmemory|mem_fragmentation)" | tee -a "$LOG_FILE"

# Check logs for errors
echo -e "\n🚨 Recent Errors in Logs:" | tee -a "$LOG_FILE"
docker logs "$REDIS_CONTAINER" --tail 50 2>&1 | grep -i -E "(error|warning|oom|killed|restart)" | tail -10 | tee -a "$LOG_FILE" || echo "No errors found" | tee -a "$LOG_FILE"

# Check system logs for OOM kills
echo -e "\n⚠️  Check OOM kills:" | tee -a "$LOG_FILE"
if command -v dmesg >/dev/null 2>&1; then
    dmesg | grep -i "killed process" | grep -i redis | tail -5 | tee -a "$LOG_FILE" || echo "No OOM kills found" | tee -a "$LOG_FILE"
else
    echo "dmesg unavailable (possibly in container)" | tee -a "$LOG_FILE"
fi

# Check uptime
echo -e "\n⏱️  Redis Uptime:" | tee -a "$LOG_FILE"
docker exec "$REDIS_CONTAINER" redis-cli INFO server | grep uptime_in_seconds | tee -a "$LOG_FILE"

# Check connections
echo -e "\n🔗 Connections:" | tee -a "$LOG_FILE"
docker exec "$REDIS_CONTAINER" redis-cli INFO clients | grep connected_clients | tee -a "$LOG_FILE"

# Check persistence
echo -e "\n💾 Persistence:" | tee -a "$LOG_FILE"
docker exec "$REDIS_CONTAINER" redis-cli LASTSAVE | tee -a "$LOG_FILE"

echo -e "\n✅ Diagnostics completed. Log saved: $LOG_FILE"
echo "📋 For continuous monitoring run: watch -n 30 ./scripts/redis-health-check.sh"