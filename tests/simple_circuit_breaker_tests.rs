// tests/simple_circuit_breaker_tests.rs

use gemini_proxy::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitState};
use std::time::Duration;

#[tokio::test]
async fn test_circuit_breaker_creation() {
    let config = CircuitBreakerConfig {
        failure_threshold: 5,
        recovery_timeout: Duration::from_secs(60),
        success_threshold: 3,
    };
    
    let circuit_breaker = CircuitBreaker::new("test".to_string(), config);
    assert_eq!(circuit_breaker.get_state().await, CircuitState::Closed);
}

#[tokio::test]
async fn test_circuit_breaker_successful_call() {
    let config = CircuitBreakerConfig::default();
    let circuit_breaker = CircuitBreaker::new("test".to_string(), config);
    
    let result = circuit_breaker.call(|| async { Ok::<i32, &str>(42) }).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 42);
}

#[tokio::test]
async fn test_circuit_breaker_failed_call() {
    let config = CircuitBreakerConfig::default();
    let circuit_breaker = CircuitBreaker::new("test".to_string(), config);
    
    let result = circuit_breaker.call(|| async { Err::<i32, &str>("error") }).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_circuit_breaker_stats() {
    let config = CircuitBreakerConfig::default();
    let circuit_breaker = CircuitBreaker::new("test".to_string(), config);
    
    let stats = circuit_breaker.get_stats();
    assert_eq!(stats.total_requests, 0);
    assert_eq!(stats.total_failures, 0);
    
    // Выполняем успешный вызов
    let _ = circuit_breaker.call(|| async { Ok::<i32, &str>(42) }).await;
    
    let stats = circuit_breaker.get_stats();
    assert_eq!(stats.total_requests, 1);
    assert_eq!(stats.total_failures, 0);
}

#[tokio::test]
async fn test_circuit_breaker_config_default() {
    let config = CircuitBreakerConfig::default();
    
    assert_eq!(config.failure_threshold, 5);
    assert_eq!(config.recovery_timeout, Duration::from_secs(60));
    assert_eq!(config.success_threshold, 3);
}

#[tokio::test]
async fn test_circuit_breaker_multiple_failures() {
    let config = CircuitBreakerConfig {
        failure_threshold: 2,
        recovery_timeout: Duration::from_secs(1),
        success_threshold: 1,
    };
    
    let circuit_breaker = CircuitBreaker::new("test".to_string(), config);
    
    // Первая неудача
    let _ = circuit_breaker.call(|| async { Err::<i32, &str>("error1") }).await;
    assert_eq!(circuit_breaker.get_state().await, CircuitState::Closed);
    
    // Вторая неудача должна открыть автомат
    let _ = circuit_breaker.call(|| async { Err::<i32, &str>("error2") }).await;
    assert_eq!(circuit_breaker.get_state().await, CircuitState::Open);
}

#[tokio::test]
async fn test_circuit_breaker_recovery() {
    let config = CircuitBreakerConfig {
        failure_threshold: 1,
        recovery_timeout: Duration::from_millis(100),
        success_threshold: 1,
    };
    
    let circuit_breaker = CircuitBreaker::new("test".to_string(), config);
    
    // Открываем автомат
    let _ = circuit_breaker.call(|| async { Err::<i32, &str>("error") }).await;
    assert_eq!(circuit_breaker.get_state().await, CircuitState::Open);
    
    // Ждем таймаут восстановления
    tokio::time::sleep(Duration::from_millis(150)).await;
    
    // Следующий вызов должен перевести в полуоткрытое состояние
    let result = circuit_breaker.call(|| async { Ok::<i32, &str>(42) }).await;
    assert!(result.is_ok());
    
    // После успешного вызова автомат должен закрыться
    assert_eq!(circuit_breaker.get_state().await, CircuitState::Closed);
}