// tests/circuit_breaker_tests.rs

use crate::circuit_breaker::{
    CircuitBreaker, CircuitBreakerConfig, CircuitState
};
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_circuit_breaker_initial_state() {
    let config = CircuitBreakerConfig::default();
    let cb = CircuitBreaker::new("test".to_string(), config);
    
    assert_eq!(cb.get_state().await, CircuitState::Closed);
}

#[tokio::test]
async fn test_circuit_breaker_failure_threshold() {
    let config = CircuitBreakerConfig {
        failure_threshold: 3,
        recovery_timeout: Duration::from_millis(100),
        success_threshold: 2,
    };
    let cb = CircuitBreaker::new("test".to_string(), config);
    
    // Simulate failures by calling with failing operations
    for _ in 0..3 {
        let _ = cb.call(|| async { Err::<(), &str>("test error") }).await;
    }
    
    assert_eq!(cb.get_state().await, CircuitState::Open);
}

#[tokio::test]
async fn test_circuit_breaker_recovery() {
    let config = CircuitBreakerConfig {
        failure_threshold: 2,
        recovery_timeout: Duration::from_millis(50),
        success_threshold: 1,
    };
    let cb = CircuitBreaker::new("test".to_string(), config);
    
    // Open the circuit
    let _ = cb.call(|| async { Err::<(), &str>("error") }).await;
    let _ = cb.call(|| async { Err::<(), &str>("error") }).await;
    assert_eq!(cb.get_state().await, CircuitState::Open);
    
    // Wait for recovery timeout
    sleep(Duration::from_millis(60)).await;
    
    // Try a successful operation to close the circuit
    let _ = cb.call(|| async { Ok::<(), &str>(()) }).await;
    assert_eq!(cb.get_state().await, CircuitState::Closed);
}

#[tokio::test]
async fn test_circuit_breaker_half_open_failure() {
    let config = CircuitBreakerConfig {
        failure_threshold: 1,
        recovery_timeout: Duration::from_millis(50),
        success_threshold: 2,
    };
    let cb = CircuitBreaker::new("test".to_string(), config);
    
    // Open the circuit
    let _ = cb.call(|| async { Err::<(), &str>("error") }).await;
    assert_eq!(cb.get_state().await, CircuitState::Open);
    
    // Wait for recovery
    sleep(Duration::from_millis(60)).await;
    
    // Fail in half-open state
    let _ = cb.call(|| async { Err::<(), &str>("error") }).await;
    assert_eq!(cb.get_state().await, CircuitState::Open);
}