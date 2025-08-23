#!/bin/bash

# Мониторинг ошибок Google API
# Анализирует логи на предмет ошибок 500 и паттернов

set -euo pipefail

CONTAINER_NAME="gemini-proxy"
REPORT_FILE="/tmp/google-api-errors-$(date +%Y%m%d-%H%M%S).log"

echo "🔍 Анализ ошибок Google API - $(date)" | tee "$REPORT_FILE"
echo "=======================================" | tee -a "$REPORT_FILE"

# Проверка доступности контейнера
if ! docker ps --filter "name=$CONTAINER_NAME" --format "{{.Names}}" | grep -q "$CONTAINER_NAME"; then
    echo "❌ Контейнер $CONTAINER_NAME не найден или не запущен" | tee -a "$REPORT_FILE"
    exit 1
fi

# Анализ ошибок 500 за последний час
echo "📊 Ошибки 500 за последний час:" | tee -a "$REPORT_FILE"
ERROR_COUNT=$(docker logs "$CONTAINER_NAME" --since 1h 2>&1 | grep -c "http.status_code=500" || echo "0")
echo "Всего ошибок 500: $ERROR_COUNT" | tee -a "$REPORT_FILE"

if [ "$ERROR_COUNT" -gt 0 ]; then
    echo -e "\n🔍 Детали ошибок 500:" | tee -a "$REPORT_FILE"
    docker logs "$CONTAINER_NAME" --since 1h 2>&1 | grep "http.status_code=500" | tail -10 | tee -a "$REPORT_FILE"
    
    echo -e "\n📈 Статистика по группам ключей:" | tee -a "$REPORT_FILE"
    docker logs "$CONTAINER_NAME" --since 1h 2>&1 | grep "http.status_code=500" | grep -o 'key_group="[^"]*"' | sort | uniq -c | tee -a "$REPORT_FILE"
fi

# Анализ успешных запросов для сравнения
echo -e "\n✅ Успешные запросы (200) за последний час:" | tee -a "$REPORT_FILE"
SUCCESS_COUNT=$(docker logs "$CONTAINER_NAME" --since 1h 2>&1 | grep -c "http.status_code=200" || echo "0")
echo "Всего успешных: $SUCCESS_COUNT" | tee -a "$REPORT_FILE"

# Расчет процента ошибок
if [ "$SUCCESS_COUNT" -gt 0 ] || [ "$ERROR_COUNT" -gt 0 ]; then
    TOTAL=$((SUCCESS_COUNT + ERROR_COUNT))
    if [ "$TOTAL" -gt 0 ]; then
        ERROR_RATE=$(echo "scale=2; $ERROR_COUNT * 100 / $TOTAL" | bc -l 2>/dev/null || echo "0")
        echo "Процент ошибок: ${ERROR_RATE}%" | tee -a "$REPORT_FILE"
        
        # Предупреждение при высоком проценте ошибок
        if (( $(echo "$ERROR_RATE > 5" | bc -l 2>/dev/null || echo "0") )); then
            echo "⚠️  ВНИМАНИЕ: Высокий процент ошибок (>${ERROR_RATE}%)!" | tee -a "$REPORT_FILE"
        fi
    else
        echo "Процент ошибок: 0% (нет данных)" | tee -a "$REPORT_FILE"
    fi
fi

# Проверка паттернов ошибок
echo -e "\n🔍 Анализ паттернов ошибок:" | tee -a "$REPORT_FILE"
docker logs "$CONTAINER_NAME" --since 1h 2>&1 | grep "http.status_code=500" | grep -o '"[^"]*internal[^"]*"' | sort | uniq -c | head -5 | tee -a "$REPORT_FILE" || echo "Специфичных паттернов не найдено" | tee -a "$REPORT_FILE"

# Проверка времени ответа
echo -e "\n⏱️  Анализ времени ответа:" | tee -a "$REPORT_FILE"
AVG_RESPONSE_TIME=$(docker logs "$CONTAINER_NAME" --since 1h 2>&1 | grep -o 'duration_ms=[0-9]*' | cut -d= -f2 | awk '{sum+=$1; count++} END {if(count>0) print sum/count; else print 0}')
echo "Среднее время ответа: ${AVG_RESPONSE_TIME}ms" | tee -a "$REPORT_FILE"

echo -e "\n✅ Анализ завершен. Отчет сохранен: $REPORT_FILE"

# Рекомендации
echo -e "\n💡 Рекомендации:" | tee -a "$REPORT_FILE"
if [ "$ERROR_COUNT" -gt 10 ]; then
    echo "- Рассмотрите возможность добавления задержки между запросами" | tee -a "$REPORT_FILE"
    echo "- Проверьте квоты API ключей в Google AI Studio" | tee -a "$REPORT_FILE"
fi

if (( $(echo "$ERROR_RATE > 10" | bc -l 2>/dev/null || echo "0") )); then
    echo "- Критический уровень ошибок! Требуется немедленное вмешательство" | tee -a "$REPORT_FILE"
fi