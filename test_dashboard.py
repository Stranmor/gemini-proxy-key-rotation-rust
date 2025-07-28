#!/usr/bin/env python3
"""
Скрипт для тестирования функциональности дашборда
"""

import requests
import json
import time

BASE_URL = "http://localhost:8081"

def test_endpoint(endpoint, description):
    """Тестирует эндпоинт и выводит результат"""
    print(f"\n=== {description} ===")
    try:
        response = requests.get(f"{BASE_URL}{endpoint}")
        print(f"Status: {response.status_code}")
        
        if response.status_code == 200:
            if 'application/json' in response.headers.get('content-type', ''):
                data = response.json()
                print(f"Response: {json.dumps(data, indent=2)}")
            else:
                print(f"HTML Response length: {len(response.text)} chars")
        else:
            print(f"Error: {response.text}")
    except Exception as e:
        print(f"Exception: {e}")

def main():
    print("🚀 Тестирование дашборда Gemini Proxy")
    
    # Тестируем основные эндпоинты
    test_endpoint("/admin/health", "Health Check")
    test_endpoint("/admin/keys", "API Keys List")
    test_endpoint("/admin/model-stats", "Model Statistics")
    test_endpoint("/admin", "Dashboard HTML")
    
    print(f"\n✅ Тестирование завершено!")
    print(f"🌐 Откройте дашборд: {BASE_URL}/admin")

if __name__ == "__main__":
    main()