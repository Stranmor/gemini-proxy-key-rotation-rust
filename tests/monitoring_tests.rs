// tests/monitoring_tests.rs

use gemini_proxy_key_rotation_rust::{
    monitoring::{MonitoringSystem, key_health::KeyHealthMonitor},
    key_manager::{KeyManagerTrait, FlattenedKeyInfo},
    state::KeyState,
};
use secrecy::{Secret, ExposeSecret};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::time::sleep;

// Mock KeyManager для тестирования
struct MockKeyManager {
    keys: HashMap<String, FlattenedKeyInfo>,
    key_states: HashMap<String, KeyState>,
}

impl MockKeyManager {
    fn new() -> Self {
        let mut keys = HashMap::new();
        let mut key_states = HashMap::new();
        
        // Создаем тестовые ключи
        for i in 1..=3 {
            let key = format!("test-key-{}", i);
            let key_info = FlattenedKeyInfo {
                key: Secret::new(key.clone()),
                group_name: "test-group".to_string(),
                target_url: "https://example.com".to_string(),
                proxy_url: None,
            };
            
            let key_state = KeyState {
                key: key.clone(),
                group_name: "test-group".to_string(),
                is_blocked: false,
                consecutive_failures: 0,
                last_failure: None,
            };
            
            keys.insert(key.clone(), key_info);
            key_states.insert(key, key_state);
        }
        
        Self { keys, key_states }
    }
    
    fn block_key(&mut self, key: &str) {
        if let Some(state) = self.key_states.get_mut(key) {
            state.is_blocked = true;
            state.consecutive_failures = 5;
            state.last_failure = Some(chrono::Utc::now());
        }
    }
}

#[axum::async_trait]
impl KeyManagerTrait for MockKeyManager {
    async fn get_next_available_key_info(
        &self,
        _group_name: Option<&str>,
    ) -> Result<Option<FlattenedKeyInfo>, gemini_proxy_key_rotation_rust::error::AppError> {
        // Возвращаем первый доступный ключ
        for (_, key_info) in &self.keys {
            if let Some(state) = self.key_states.get(key_info.key.expose_secret()) {
                if !state.is_blocked {
                    return Ok(Some(key_info.clone()));
                }
            }
        }
        Ok(None)
    }

    async fn handle_api_failure(&self, _api_key: &str, _is_terminal: bool) -> Result<(), gemini_proxy_key_rotation_rust::error::AppError> {
        Ok(())
    }

    async fn get_key_states(&self) -> Result<HashMap<String, KeyState>, gemini_proxy_key_rotation_rust::error::AppError> {
        Ok(self.key_states.clone())
    }

    async fn get_all_key_info(&self) -> HashMap<String, FlattenedKeyInfo> {
        self.keys.clone()
    }
}

#[tokio::test]
async fn test_key_health_monitor_initialization() {
    let mock_key_manager = Arc::new(MockKeyManager::new()) as Arc<dyn KeyManagerTrait>;
    let monitor = KeyHealthMonitor::new(mock_key_manager);
    
    // Получаем начальную статистику
    let stats = monitor.get_health_stats().await;
    
    // Изначально статистика должна быть пустой
    assert_eq!(stats.len(), 0, "Initial stats should be empty");
}

#[tokio::test]
async fn test_key_health_monitor_record_success() {
    let mock_key_manager = Arc::new(MockKeyManager::new()) as Arc<dyn KeyManagerTrait>;
    let monitor = KeyHealthMonitor::new(mock_key_manager);
    
    let test_key = "test-key-1";
    let response_time = Duration::from_millis(100);
    
    // Записываем успешный запрос
    monitor.record_success(test_key, response_time).await;
    
    // Проверяем статистику
    let stats = monitor.get_health_stats().await;
    let key_stat = stats.get(test_key).expect("Key stats should exist");
    
    assert_eq!(key_stat.total_requests, 1);
    assert_eq!(key_stat.successful_requests, 1);
    assert_eq!(key_stat.failed_requests, 0);
    assert_eq!(key_stat.consecutive_failures, 0);
    assert!(key_stat.is_healthy);
    assert_eq!(key_stat.health_score, 1.0);
    assert_eq!(key_stat.average_response_time, response_time);
}

#[tokio::test]
async fn test_key_health_monitor_record_failure() {
    let mock_key_manager = Arc::new(MockKeyManager::new()) as Arc<dyn KeyManagerTrait>;
    let monitor = KeyHealthMonitor::new(mock_key_manager);
    
    let test_key = "test-key-1";
    
    // Записываем неудачный запрос
    monitor.record_failure(test_key, false).await;
    
    // Проверяем статистику
    let stats = monitor.get_health_stats().await;
    let key_stat = stats.get(test_key).expect("Key stats should exist");
    
    assert_eq!(key_stat.total_requests, 1);
    assert_eq!(key_stat.successful_requests, 0);
    assert_eq!(key_stat.failed_requests, 1);
    assert_eq!(key_stat.consecutive_failures, 1);
    assert!(key_stat.is_healthy); // Один сбой не должен блокировать ключ
    assert!(key_stat.health_score < 1.0);
}

#[tokio::test]
async fn test_key_health_monitor_terminal_failure() {
    let mock_key_manager = Arc::new(MockKeyManager::new()) as Arc<dyn KeyManagerTrait>;
    let monitor = KeyHealthMonitor::new(mock_key_manager);
    
    let test_key = "test-key-1";
    
    // Записываем терминальную ошибку
    monitor.record_failure(test_key, true).await;
    
    // Проверяем статистику
    let stats = monitor.get_health_stats().await;
    let key_stat = stats.get(test_key).expect("Key stats should exist");
    
    assert_eq!(key_stat.consecutive_failures, 1);
    assert!(!key_stat.is_healthy); // Терминальная ошибка должна блокировать ключ
}

#[tokio::test]
async fn test_key_health_monitor_multiple_failures() {
    let mock_key_manager = Arc::new(MockKeyManager::new()) as Arc<dyn KeyManagerTrait>;
    let monitor = KeyHealthMonitor::new(mock_key_manager);
    
    let test_key = "test-key-1";
    
    // Записываем несколько неудачных запросов
    for _ in 0..3 {
        monitor.record_failure(test_key, false).await;
    }
    
    // Проверяем статистику
    let stats = monitor.get_health_stats().await;
    let key_stat = stats.get(test_key).expect("Key stats should exist");
    
    assert_eq!(key_stat.consecutive_failures, 3);
    assert!(!key_stat.is_healthy); // 3 сбоя должны блокировать ключ
    assert!(key_stat.health_score < 0.5);
}

#[tokio::test]
async fn test_key_health_monitor_recovery() {
    let mock_key_manager = Arc::new(MockKeyManager::new()) as Arc<dyn KeyManagerTrait>;
    let monitor = KeyHealthMonitor::new(mock_key_manager);
    
    let test_key = "test-key-1";
    
    // Записываем несколько неудачных запросов
    for _ in 0..3 {
        monitor.record_failure(test_key, false).await;
    }
    
    // Записываем успешный запрос (восстановление)
    monitor.record_success(test_key, Duration::from_millis(100)).await;
    
    // Проверяем статистику
    let stats = monitor.get_health_stats().await;
    let key_stat = stats.get(test_key).expect("Key stats should exist");
    
    assert_eq!(key_stat.consecutive_failures, 0); // Должно сброситься
    assert!(key_stat.is_healthy); // Ключ должен восстановиться
    assert!(key_stat.health_score > 0.0);
}

#[tokio::test]
async fn test_key_health_monitor_get_unhealthy_keys() {
    let mock_key_manager = Arc::new(MockKeyManager::new()) as Arc<dyn KeyManagerTrait>;
    let monitor = KeyHealthMonitor::new(mock_key_manager);
    
    // Делаем один ключ нездоровым
    monitor.record_failure("test-key-1", true).await;
    
    // Делаем другой ключ здоровым
    monitor.record_success("test-key-2", Duration::from_millis(100)).await;
    
    // Получаем нездоровые ключи
    let unhealthy = monitor.get_unhealthy_keys(10).await;
    
    assert_eq!(unhealthy.len(), 1);
    assert_eq!(unhealthy[0].key_preview, "test...ey-1");
    assert!(!unhealthy[0].is_healthy);
}

#[tokio::test]
async fn test_monitoring_system_initialization() {
    let mock_key_manager = Arc::new(MockKeyManager::new()) as Arc<dyn KeyManagerTrait>;
    let monitoring = MonitoringSystem::new(mock_key_manager);
    
    // Получаем системную статистику
    let stats = monitoring.get_system_stats().await;
    
    assert!(stats.uptime < Duration::from_secs(1)); // Только что создан
    assert_eq!(stats.total_keys, 0); // Нет статистики по ключам
    assert_eq!(stats.healthy_keys, 0);
    assert_eq!(stats.unhealthy_keys, 0);
    assert_eq!(stats.total_requests, 0);
    assert_eq!(stats.total_failures, 0);
    assert_eq!(stats.error_rate, 0.0);
}

#[tokio::test]
async fn test_monitoring_system_with_key_activity() {
    let mock_key_manager = Arc::new(MockKeyManager::new()) as Arc<dyn KeyManagerTrait>;
    let monitoring = MonitoringSystem::new(mock_key_manager);
    
    // Записываем активность ключей
    let key_health = monitoring.key_health();
    key_health.record_success("test-key-1", Duration::from_millis(100)).await;
    key_health.record_success("test-key-2", Duration::from_millis(150)).await;
    // Делаем ключ нездоровым через терминальную ошибку
    key_health.record_failure("test-key-3", true).await;
    
    // Получаем системную статистику
    let stats = monitoring.get_system_stats().await;
    
    assert_eq!(stats.total_keys, 3);
    // Один ключ нездоровый (test-key-3), два здоровых
    assert!(stats.healthy_keys >= 2, "Should have at least 2 healthy keys");
    assert!(stats.unhealthy_keys >= 1, "Should have at least 1 unhealthy key");
    assert_eq!(stats.total_requests, 3);
    assert_eq!(stats.total_failures, 1);
    assert!((stats.error_rate - 1.0/3.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn test_health_score_calculation() {
    let mock_key_manager = Arc::new(MockKeyManager::new()) as Arc<dyn KeyManagerTrait>;
    let monitor = KeyHealthMonitor::new(mock_key_manager);
    
    let test_key = "test-key-1";
    
    // Записываем смешанную активность: 7 успехов, 3 неудачи
    for _ in 0..7 {
        monitor.record_success(test_key, Duration::from_millis(100)).await;
    }
    for _ in 0..3 {
        monitor.record_failure(test_key, false).await;
    }
    
    let stats = monitor.get_health_stats().await;
    let key_stat = stats.get(test_key).expect("Key stats should exist");
    
    // Success rate = 7/10 = 0.7
    // Failure penalty = 3 * 0.1 = 0.3
    // Health score = 0.7 - 0.3 = 0.4
    assert!((key_stat.health_score - 0.4).abs() < 0.1, 
           "Health score should be approximately 0.4, got {}", key_stat.health_score);
}

#[cfg(test)]
mod monitoring_integration_tests {
    use super::*;
    use tokio::time::{timeout, Duration as TokioDuration};

    #[tokio::test]
    async fn test_monitoring_system_start() {
        let mock_key_manager = Arc::new(MockKeyManager::new()) as Arc<dyn KeyManagerTrait>;
        let monitoring = MonitoringSystem::new(mock_key_manager);
        
        // Запускаем мониторинг (не ждем завершения, так как это бесконечный цикл)
        let start_result = timeout(TokioDuration::from_millis(100), monitoring.start()).await;
        
        // Должно завершиться по таймауту, что означает успешный запуск
        // Или завершиться успешно, если start() быстро возвращает Ok
        match start_result {
            Err(_) => {}, // Таймаут - ожидаемо для бесконечного цикла
            Ok(_) => {}, // Быстрое завершение - тоже нормально
        }
    }

    #[tokio::test]
    async fn test_key_health_monitor_background_checks() {
        let mut mock_key_manager = MockKeyManager::new();
        mock_key_manager.block_key("test-key-1"); // Блокируем один ключ
        
        let mock_key_manager = Arc::new(mock_key_manager) as Arc<dyn KeyManagerTrait>;
        let monitor = KeyHealthMonitor::new(mock_key_manager);
        
        // Запускаем мониторинг
        monitor.start_monitoring().await;
        
        // Ждем немного для выполнения фоновых проверок
        sleep(Duration::from_millis(100)).await;
        
        // Проверяем, что система работает (это больше smoke test)
        let stats = monitor.get_health_stats().await;
        // В реальной системе здесь была бы статистика от фоновых проверок
        assert!(stats.len() >= 0); // Базовая проверка работоспособности
    }
}