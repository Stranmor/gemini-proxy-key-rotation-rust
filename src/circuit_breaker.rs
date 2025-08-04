use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, PartialEq)]
pub enum CircuitState {
    Closed,   // Normal operation
    Open,     // Circuit is open, failing fast
    HalfOpen, // Testing if service recovered
}

#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    pub failure_threshold: usize,
    pub recovery_timeout: Duration,
    pub success_threshold: usize, // Successes needed in half-open to close
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            recovery_timeout: Duration::from_secs(60),
            success_threshold: 3,
        }
    }
}

#[derive(Debug)]
struct CircuitBreakerState {
    state: CircuitState,
    failure_count: usize,
    success_count: usize,
    last_failure_time: Option<Instant>,
    next_attempt: Option<Instant>,
}

#[derive(Debug)]
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: Arc<RwLock<CircuitBreakerState>>,
    total_requests: AtomicU64,
    total_failures: AtomicU64,
    name: String,
}

impl CircuitBreaker {
    pub fn new(name: String, config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(CircuitBreakerState {
                state: CircuitState::Closed,
                failure_count: 0,
                success_count: 0,
                last_failure_time: None,
                next_attempt: None,
            })),
            total_requests: AtomicU64::new(0),
            total_failures: AtomicU64::new(0),
            name,
        }
    }

    pub async fn call<F, Fut, T, E>(&self, operation: F) -> Result<T, CircuitBreakerError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
    {
        self.total_requests.fetch_add(1, Ordering::Relaxed);

        // Check if we should allow the request
        if !self.should_allow_request().await {
            debug!(circuit_breaker = %self.name, "Circuit breaker is open, failing fast");
            return Err(CircuitBreakerError::CircuitOpen);
        }

        // Execute the operation
        match operation().await {
            Ok(result) => {
                self.on_success().await;
                Ok(result)
            }
            Err(error) => {
                self.on_failure().await;
                Err(CircuitBreakerError::OperationFailed(error))
            }
        }
    }

    async fn should_allow_request(&self) -> bool {
        let mut state = self.state.write().await;
        let now = Instant::now();

        match state.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                if let Some(next_attempt) = state.next_attempt {
                    if now >= next_attempt {
                        info!(circuit_breaker = %self.name, "Circuit breaker transitioning to half-open");
                        state.state = CircuitState::HalfOpen;
                        state.success_count = 0;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => true,
        }
    }

    async fn on_success(&self) {
        let mut state = self.state.write().await;

        match state.state {
            CircuitState::Closed => {
                // Reset failure count on success
                state.failure_count = 0;
            }
            CircuitState::HalfOpen => {
                state.success_count += 1;
                if state.success_count >= self.config.success_threshold {
                    info!(circuit_breaker = %self.name, "Circuit breaker closing after successful recovery");
                    state.state = CircuitState::Closed;
                    state.failure_count = 0;
                    state.success_count = 0;
                    state.last_failure_time = None;
                    state.next_attempt = None;
                }
            }
            CircuitState::Open => {
                // This shouldn't happen, but reset if it does
                warn!(circuit_breaker = %self.name, "Unexpected success in open state");
            }
        }
    }

    async fn on_failure(&self) {
        let mut state = self.state.write().await;
        let now = Instant::now();

        self.total_failures.fetch_add(1, Ordering::Relaxed);
        state.failure_count += 1;
        state.last_failure_time = Some(now);

        match state.state {
            CircuitState::Closed => {
                if state.failure_count >= self.config.failure_threshold {
                    warn!(
                        circuit_breaker = %self.name,
                        failure_count = state.failure_count,
                        threshold = self.config.failure_threshold,
                        "Circuit breaker opening due to failures"
                    );
                    state.state = CircuitState::Open;
                    state.next_attempt = Some(now + self.config.recovery_timeout);
                }
            }
            CircuitState::HalfOpen => {
                warn!(circuit_breaker = %self.name, "Circuit breaker reopening after failed recovery attempt");
                state.state = CircuitState::Open;
                state.next_attempt = Some(now + self.config.recovery_timeout);
                state.success_count = 0;
            }
            CircuitState::Open => {
                // Update next attempt time
                state.next_attempt = Some(now + self.config.recovery_timeout);
            }
        }
    }

    pub async fn get_state(&self) -> CircuitState {
        self.state.read().await.state.clone()
    }

    pub fn get_stats(&self) -> CircuitBreakerStats {
        CircuitBreakerStats {
            total_requests: self.total_requests.load(Ordering::Relaxed),
            total_failures: self.total_failures.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CircuitBreakerStats {
    pub total_requests: u64,
    pub total_failures: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum CircuitBreakerError<E> {
    #[error("Circuit breaker is open")]
    CircuitOpen,
    #[error("Operation failed: {0}")]
    OperationFailed(E),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn test_circuit_breaker_opens_on_failures() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            recovery_timeout: Duration::from_millis(100),
            success_threshold: 2,
        };
        let cb = CircuitBreaker::new("test".to_string(), config);

        // Simulate failures
        for _ in 0..3 {
            let result = cb.call(|| async { Err::<(), &str>("error") }).await;
            assert!(matches!(result, Err(CircuitBreakerError::OperationFailed(_))));
        }

        // Circuit should be open now
        assert_eq!(cb.get_state().await, CircuitState::Open);

        // Next call should fail fast
        let result = cb.call(|| async { Ok::<(), &str>(()) }).await;
        assert!(matches!(result, Err(CircuitBreakerError::CircuitOpen)));
    }

    #[tokio::test]
    async fn test_circuit_breaker_recovers() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            recovery_timeout: Duration::from_millis(50),
            success_threshold: 1,
        };
        let cb = CircuitBreaker::new("test".to_string(), config);

        // Open the circuit
        for _ in 0..2 {
            let _ = cb.call(|| async { Err::<(), &str>("error") }).await;
        }
        assert_eq!(cb.get_state().await, CircuitState::Open);

        // Wait for recovery timeout
        sleep(Duration::from_millis(60)).await;

        // Should transition to half-open and allow request
        let result = cb.call(|| async { Ok::<(), &str>(()) }).await;
        assert!(result.is_ok());

        // Should be closed now
        assert_eq!(cb.get_state().await, CircuitState::Closed);
    }
}