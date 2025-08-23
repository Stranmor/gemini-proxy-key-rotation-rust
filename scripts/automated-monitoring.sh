#!/bin/bash

# Автоматизированный мониторинг системы
# Запускает все проверки и генерирует сводный отчет

set -euo pipefail

TIMESTAMP=$(date +%Y%m%d-%H%M%S)
REPORT_DIR="/tmp/monitoring-reports"
MAIN_REPORT="$REPORT_DIR/system-health-$TIMESTAMP.log"

# Создаем директорию для отчетов
mkdir -p "$REPORT_DIR"

echo "🚀 Автоматизированный мониторинг системы - $(date)" | tee "$MAIN_REPORT"
echo "=================================================" | tee -a "$MAIN_REPORT"

# Функция для выполнения проверки с обработкой ошибок
run_check() {
    local check_name="$1"
    local check_script="$2"
    
    echo -e "\n🔍 Выполняется: $check_name" | tee -a "$MAIN_REPORT"
    echo "-----------------------------------" | tee -a "$MAIN_REPORT"
    
    if [ -x "$check_script" ]; then
        if timeout 60 "$check_script" >> "$MAIN_REPORT" 2>&1; then
            echo "✅ $check_name: УСПЕШНО" | tee -a "$MAIN_REPORT"
        else
            echo "❌ $check_name: ОШИБКА" | tee -a "$MAIN_REPORT"
        fi
    else
        echo "⚠️  $check_name: Скрипт не найден или не исполняемый" | tee -a "$MAIN_REPORT"
    fi
}

# Проверка доступности основных компонентов
echo -e "\n📊 Статус сервисов:" | tee -a "$MAIN_REPORT"
docker-compose ps | tee -a "$MAIN_REPORT"

# Запуск всех проверок
run_check "Диагностика Redis" "./scripts/redis-health-check.sh"
run_check "Мониторинг Google API" "./scripts/google-api-monitor.sh"

# Проверка доступности API
echo -e "\n🌐 Проверка доступности API:" | tee -a "$MAIN_REPORT"
if curl -s -f http://localhost:4806/health >/dev/null; then
    echo "✅ API доступен" | tee -a "$MAIN_REPORT"
    
    # Проверка детального здоровья
    if curl -s http://localhost:4806/health/detailed | jq -r '.healthy' | grep -q "true"; then
        echo "✅ Детальная проверка здоровья: OK" | tee -a "$MAIN_REPORT"
    else
        echo "⚠️  Детальная проверка здоровья: ПРОБЛЕМЫ" | tee -a "$MAIN_REPORT"
    fi
else
    echo "❌ API недоступен" | tee -a "$MAIN_REPORT"
fi

# Проверка использования дискового пространства
echo -e "\n💾 Использование диска:" | tee -a "$MAIN_REPORT"
df -h | grep -E "(Filesystem|/dev/)" | tee -a "$MAIN_REPORT"

# Проверка Docker volumes
echo -e "\n📦 Docker volumes:" | tee -a "$MAIN_REPORT"
docker volume ls | grep gemini | tee -a "$MAIN_REPORT"

# Сводка и рекомендации
echo -e "\n📋 СВОДКА И РЕКОМЕНДАЦИИ:" | tee -a "$MAIN_REPORT"
echo "=========================" | tee -a "$MAIN_REPORT"

# Подсчет ошибок в отчете
ERROR_COUNT=$(grep -c "❌\|ERROR\|ОШИБКА" "$MAIN_REPORT" || echo "0")
WARNING_COUNT=$(grep -c "⚠️\|WARNING\|ВНИМАНИЕ" "$MAIN_REPORT" || echo "0")

if [ "$ERROR_COUNT" -eq 0 ] && [ "$WARNING_COUNT" -eq 0 ]; then
    echo "🟢 Система работает нормально" | tee -a "$MAIN_REPORT"
elif [ "$ERROR_COUNT" -eq 0 ] && [ "$WARNING_COUNT" -gt 0 ]; then
    echo "🟡 Система работает с предупреждениями ($WARNING_COUNT)" | tee -a "$MAIN_REPORT"
    echo "   Рекомендуется проверить предупреждения" | tee -a "$MAIN_REPORT"
else
    echo "🔴 Обнаружены критические проблемы ($ERROR_COUNT ошибок, $WARNING_COUNT предупреждений)" | tee -a "$MAIN_REPORT"
    echo "   Требуется немедленное вмешательство!" | tee -a "$MAIN_REPORT"
fi

# Следующие шаги
echo -e "\n🎯 Следующие шаги:" | tee -a "$MAIN_REPORT"
echo "1. Просмотрите полный отчет: cat $MAIN_REPORT" | tee -a "$MAIN_REPORT"
echo "2. Для постоянного мониторинга: watch -n 300 $0" | tee -a "$MAIN_REPORT"
echo "3. Настройте cron для автоматических проверок:" | tee -a "$MAIN_REPORT"
echo "   */15 * * * * $PWD/$0 >/dev/null 2>&1" | tee -a "$MAIN_REPORT"

echo -e "\n✅ Мониторинг завершен. Основной отчет: $MAIN_REPORT"
echo "📁 Все отчеты сохранены в: $REPORT_DIR"

# Очистка старых отчетов (старше 7 дней)
find "$REPORT_DIR" -name "*.log" -mtime +7 -delete 2>/dev/null || true