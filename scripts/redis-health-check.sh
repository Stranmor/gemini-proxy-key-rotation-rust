#!/bin/bash

# Скрипт для диагностики здоровья Redis
# Использование: ./scripts/redis-health-check.sh

set -euo pipefail

REDIS_CONTAINER="gemini-proxy-redis"
LOG_FILE="/tmp/redis-health-$(date +%Y%m%d-%H%M%S).log"

echo "🔍 Диагностика Redis - $(date)" | tee "$LOG_FILE"
echo "=================================" | tee -a "$LOG_FILE"

# Проверка статуса контейнера
echo "📊 Статус контейнера:" | tee -a "$LOG_FILE"
docker ps --filter "name=$REDIS_CONTAINER" --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}" | tee -a "$LOG_FILE"

# Проверка использования ресурсов
echo -e "\n💾 Использование ресурсов:" | tee -a "$LOG_FILE"
docker stats "$REDIS_CONTAINER" --no-stream --format "table {{.Container}}\t{{.CPUPerc}}\t{{.MemUsage}}\t{{.MemPerc}}" | tee -a "$LOG_FILE"

# Информация о Redis
echo -e "\n📈 Информация Redis:" | tee -a "$LOG_FILE"
docker exec "$REDIS_CONTAINER" redis-cli INFO memory | grep -E "(used_memory|maxmemory|mem_fragmentation)" | tee -a "$LOG_FILE"

# Проверка логов на ошибки
echo -e "\n🚨 Последние ошибки в логах:" | tee -a "$LOG_FILE"
docker logs "$REDIS_CONTAINER" --tail 50 2>&1 | grep -i -E "(error|warning|oom|killed|restart)" | tail -10 | tee -a "$LOG_FILE" || echo "Ошибок не найдено" | tee -a "$LOG_FILE"

# Проверка системных логов на OOM kills
echo -e "\n⚠️  Проверка OOM kills:" | tee -a "$LOG_FILE"
if command -v dmesg >/dev/null 2>&1; then
    dmesg | grep -i "killed process" | grep -i redis | tail -5 | tee -a "$LOG_FILE" || echo "OOM kills не найдены" | tee -a "$LOG_FILE"
else
    echo "dmesg недоступен (возможно, в контейнере)" | tee -a "$LOG_FILE"
fi

# Проверка времени работы
echo -e "\n⏱️  Время работы Redis:" | tee -a "$LOG_FILE"
docker exec "$REDIS_CONTAINER" redis-cli INFO server | grep uptime_in_seconds | tee -a "$LOG_FILE"

# Проверка количества подключений
echo -e "\n🔗 Подключения:" | tee -a "$LOG_FILE"
docker exec "$REDIS_CONTAINER" redis-cli INFO clients | grep connected_clients | tee -a "$LOG_FILE"

# Проверка персистентности
echo -e "\n💾 Персистентность:" | tee -a "$LOG_FILE"
docker exec "$REDIS_CONTAINER" redis-cli LASTSAVE | tee -a "$LOG_FILE"

echo -e "\n✅ Диагностика завершена. Лог сохранен: $LOG_FILE"
echo "📋 Для постоянного мониторинга запустите: watch -n 30 ./scripts/redis-health-check.sh"