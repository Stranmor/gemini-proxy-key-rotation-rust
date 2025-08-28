#!/bin/bash

echo "🔍 Проверка размера build context..."

echo "📊 Текущий .dockerignore:"
echo "Размер context: $(tar --exclude-from=.dockerignore -cf - . 2>/dev/null | wc -c | numfmt --to=iec)"

echo ""
echo "📊 Оптимизированный .dockerignore:"
echo "Размер context: $(tar --exclude-from=.dockerignore.optimized -cf - . 2>/dev/null | wc -c | numfmt --to=iec)"

echo ""
echo "📁 Топ-10 самых больших файлов/папок:"
du -sh * 2>/dev/null | sort -hr | head -10