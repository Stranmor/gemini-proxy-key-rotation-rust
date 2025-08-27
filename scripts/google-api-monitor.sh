#!/bin/bash

# Google API error monitoring
# Analyzes logs for 500 errors and patterns

set -euo pipefail

CONTAINER_NAME="gemini-proxy"
REPORT_FILE="/tmp/google-api-errors-$(date +%Y%m%d-%H%M%S).log"

echo "🔍 Google API Error Analysis - $(date)" | tee "$REPORT_FILE"
echo "=======================================" | tee -a "$REPORT_FILE"

# Check container availability
if ! docker ps --filter "name=$CONTAINER_NAME" --format "{{.Names}}" | grep -q "$CONTAINER_NAME"; then
    echo "❌ Container $CONTAINER_NAME not found or not running" | tee -a "$REPORT_FILE"
    exit 1
fi

# Analyze 500 errors in the last hour
echo "📊 500 Errors in the last hour:" | tee -a "$REPORT_FILE"
ERROR_COUNT=$(docker logs "$CONTAINER_NAME" --since 1h 2>&1 | grep -c "http.status_code=500" || echo "0")
echo "Total 500 errors: $ERROR_COUNT" | tee -a "$REPORT_FILE"

if [ "$ERROR_COUNT" -gt 0 ]; then
    echo -e "\n🔍 500 Error Details:" | tee -a "$REPORT_FILE"
    docker logs "$CONTAINER_NAME" --since 1h 2>&1 | grep "http.status_code=500" | tail -10 | tee -a "$REPORT_FILE"
    
    echo -e "\n📈 Statistics by Key Groups:" | tee -a "$REPORT_FILE"
    docker logs "$CONTAINER_NAME" --since 1h 2>&1 | grep "http.status_code=500" | grep -o 'key_group="[^"]*"' | sort | uniq -c | tee -a "$REPORT_FILE"
fi

# Analyze successful requests for comparison
echo -e "\n✅ Successful requests (200) in the last hour:" | tee -a "$REPORT_FILE"
SUCCESS_COUNT=$(docker logs "$CONTAINER_NAME" --since 1h 2>&1 | grep -c "http.status_code=200" || echo "0")
echo "Total successful: $SUCCESS_COUNT" | tee -a "$REPORT_FILE"

# Calculate error percentage
if [ "$SUCCESS_COUNT" -gt 0 ] || [ "$ERROR_COUNT" -gt 0 ]; then
    TOTAL=$((SUCCESS_COUNT + ERROR_COUNT))
    if [ "$TOTAL" -gt 0 ]; then
        ERROR_RATE=$(echo "scale=2; $ERROR_COUNT * 100 / $TOTAL" | bc -l 2>/dev/null || echo "0")
        echo "Error rate: ${ERROR_RATE}%" | tee -a "$REPORT_FILE"
        
        # Warning for high error rate
        if (( $(echo "$ERROR_RATE > 5" | bc -l 2>/dev/null || echo "0") )); then
            echo "⚠️  WARNING: High error rate (>${ERROR_RATE}%)!" | tee -a "$REPORT_FILE"
        fi
    else
        echo "Error rate: 0% (no data)" | tee -a "$REPORT_FILE"
    fi
fi

# Check error patterns
echo -e "\n🔍 Error Pattern Analysis:" | tee -a "$REPORT_FILE"
docker logs "$CONTAINER_NAME" --since 1h 2>&1 | grep "http.status_code=500" | grep -o '"[^"]*internal[^"]*"' | sort | uniq -c | head -5 | tee -a "$REPORT_FILE" || echo "No specific patterns found" | tee -a "$REPORT_FILE"

# Check response time
echo -e "\n⏱️  Response Time Analysis:" | tee -a "$REPORT_FILE"
AVG_RESPONSE_TIME=$(docker logs "$CONTAINER_NAME" --since 1h 2>&1 | grep -o 'duration_ms=[0-9]*' | cut -d= -f2 | awk '{sum+=$1; count++} END {if(count>0) print sum/count; else print 0}')
echo "Average response time: ${AVG_RESPONSE_TIME}ms" | tee -a "$REPORT_FILE"

echo -e "\n✅ Analysis completed. Report saved: $REPORT_FILE"

# Recommendations
echo -e "\n💡 Recommendations:" | tee -a "$REPORT_FILE"
if [ "$ERROR_COUNT" -gt 10 ]; then
    echo "- Consider adding delays between requests" | tee -a "$REPORT_FILE"
    echo "- Check API key quotas in Google AI Studio" | tee -a "$REPORT_FILE"
fi

if (( $(echo "$ERROR_RATE > 10" | bc -l 2>/dev/null || echo "0") )); then
    echo "- Critical error level! Immediate intervention required" | tee -a "$REPORT_FILE"
fi