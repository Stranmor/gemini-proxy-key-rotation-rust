//! Metrics collection and export module
//!
//! Provides comprehensive metrics collection using the `metrics` crate
//! with Prometheus export capabilities for production monitoring.

pub mod collectors;
pub mod exporters;
pub use exporters::metrics_handler;
pub mod middleware;

use metrics::{counter, gauge, histogram, Counter, Gauge, Histogram};
use once_cell::sync::Lazy;
use std::time::{Duration, Instant};

/// Global metrics registry
pub static METRICS: Lazy<MetricsRegistry> = Lazy::new(MetricsRegistry::new);

/// Centralized metrics registry for the application
pub struct MetricsRegistry {
    // Request metrics
    pub requests_total: Counter,
    pub requests_duration: Histogram,
    pub requests_in_flight: Gauge,

    // Key management metrics
    pub keys_total: Gauge,
    pub keys_healthy: Gauge,
    pub keys_unhealthy: Gauge,
    pub key_rotations_total: Counter,
    pub key_failures_total: Counter,

    // Circuit breaker metrics
    pub circuit_breaker_state: Gauge,
    pub circuit_breaker_trips_total: Counter,

    // Rate limiting metrics
    pub rate_limit_hits_total: Counter,
    pub rate_limit_blocks_total: Counter,

    // Token limit metrics
    pub token_limit_blocks_total: Counter,
    pub request_token_count: Histogram,

    // Redis metrics
    pub redis_operations_total: Counter,
    pub redis_errors_total: Counter,
    pub redis_connection_pool_size: Gauge,

    // System metrics
    pub memory_usage_bytes: Gauge,
    pub cpu_usage_percent: Gauge,
    pub uptime_seconds: Gauge,
}

impl MetricsRegistry {
    pub fn new() -> Self {
        Self {
            // Request metrics
            requests_total: counter!("gemini_proxy_requests_total"),
            requests_duration: histogram!("gemini_proxy_request_duration_seconds"),
            requests_in_flight: gauge!("gemini_proxy_requests_in_flight"),

            // Key management metrics
            keys_total: gauge!("gemini_proxy_keys_total"),
            keys_healthy: gauge!("gemini_proxy_keys_healthy"),
            keys_unhealthy: gauge!("gemini_proxy_keys_unhealthy"),
            key_rotations_total: counter!("gemini_proxy_key_rotations_total"),
            key_failures_total: counter!("gemini_proxy_key_failures_total"),

            // Circuit breaker metrics
            circuit_breaker_state: gauge!("gemini_proxy_circuit_breaker_state"),
            circuit_breaker_trips_total: counter!("gemini_proxy_circuit_breaker_trips_total"),

            // Rate limiting metrics
            rate_limit_hits_total: counter!("gemini_proxy_rate_limit_hits_total"),
            rate_limit_blocks_total: counter!("gemini_proxy_rate_limit_blocks_total"),

            // Token limit metrics
            token_limit_blocks_total: counter!("gemini_proxy_token_limit_blocks_total"),
            // Histogram for token count distribution in requests
            request_token_count: histogram!("gemini_proxy_request_token_count"),

            // Redis metrics
            redis_operations_total: counter!("gemini_proxy_redis_operations_total"),
            redis_errors_total: counter!("gemini_proxy_redis_errors_total"),
            redis_connection_pool_size: gauge!("gemini_proxy_redis_connection_pool_size"),

            // System metrics
            memory_usage_bytes: gauge!("gemini_proxy_memory_usage_bytes"),
            cpu_usage_percent: gauge!("gemini_proxy_cpu_usage_percent"),
            uptime_seconds: gauge!("gemini_proxy_uptime_seconds"),
        }
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsRegistry {
    /// Record actual token count per request (for histogram)
    pub fn record_request_tokens(&self, count: u64) {
        self.request_token_count.record(count as f64);
        histogram!("gemini_proxy_request_token_count").record(count as f64);
    }

    /// Increment token limit blocks
    pub fn record_token_limit_block(&self, model: Option<String>) {
        self.token_limit_blocks_total.increment(1);
        match model {
            Some(m) => counter!("gemini_proxy_token_limit_blocks_total", "model" => m).increment(1),
            None => counter!("gemini_proxy_token_limit_blocks_total").increment(1),
        };
    }
    /// Record a request with labels
    pub fn record_request(&self, method: String, path: String, status: u16, duration: Duration) {
        self.requests_total.increment(1);
        self.requests_duration.record(duration.as_secs_f64());

        // Record with labels using the metrics macros
        counter!("gemini_proxy_requests_total", "method" => method.clone(), "path" => path.clone(), "status" => status.to_string()).increment(1);
        histogram!("gemini_proxy_request_duration_seconds", "method" => method, "path" => path)
            .record(duration.as_secs_f64());
    }

    /// Record key health status
    pub fn record_key_health(&self, total: usize, healthy: usize, unhealthy: usize) {
        self.keys_total.set(total as f64);
        self.keys_healthy.set(healthy as f64);
        self.keys_unhealthy.set(unhealthy as f64);
    }

    /// Record key rotation
    pub fn record_key_rotation(&self, group: String, success: bool) {
        self.key_rotations_total.increment(1);

        if success {
            counter!("gemini_proxy_key_rotations_total", "group" => group, "result" => "success")
                .increment(1);
        } else {
            counter!("gemini_proxy_key_rotations_total", "group" => group, "result" => "failure")
                .increment(1);
            self.key_failures_total.increment(1);
        }
    }

    /// Record circuit breaker state
    pub fn record_circuit_breaker_state(&self, service: String, state: CircuitBreakerState) {
        let state_value = match state {
            CircuitBreakerState::Closed => 0.0,
            CircuitBreakerState::Open => 1.0,
            CircuitBreakerState::HalfOpen => 0.5,
        };

        gauge!("gemini_proxy_circuit_breaker_state", "service" => service).set(state_value);
    }

    /// Record circuit breaker trip
    pub fn record_circuit_breaker_trip(&self, service: String) {
        self.circuit_breaker_trips_total.increment(1);
        counter!("gemini_proxy_circuit_breaker_trips_total", "service" => service).increment(1);
    }

    /// Record rate limit hit
    pub fn record_rate_limit(&self, resource: String, blocked: bool) {
        self.rate_limit_hits_total.increment(1);
        counter!("gemini_proxy_rate_limit_hits_total", "resource" => resource.clone()).increment(1);

        if blocked {
            self.rate_limit_blocks_total.increment(1);
            counter!("gemini_proxy_rate_limit_blocks_total", "resource" => resource).increment(1);
        }
    }

    /// Record Redis operation
    pub fn record_redis_operation(&self, operation: String, success: bool) {
        self.redis_operations_total.increment(1);

        let result = if success { "success" } else { "error" };
        counter!("gemini_proxy_redis_operations_total", "operation" => operation, "result" => result).increment(1);

        if !success {
            self.redis_errors_total.increment(1);
        }
    }

    /// Update system metrics
    pub fn update_system_metrics(&self, memory_bytes: u64, cpu_percent: f64, uptime: Duration) {
        self.memory_usage_bytes.set(memory_bytes as f64);
        self.cpu_usage_percent.set(cpu_percent);
        self.uptime_seconds.set(uptime.as_secs() as f64);
    }

    /// Set requests in flight
    pub fn set_requests_in_flight(&self, count: usize) {
        self.requests_in_flight.set(count as f64);
    }

    /// Set Redis connection pool size
    pub fn set_redis_pool_size(&self, size: usize) {
        self.redis_connection_pool_size.set(size as f64);
    }
}

/// Circuit breaker states for metrics
#[derive(Debug, Clone, Copy)]
pub enum CircuitBreakerState {
    Closed,
    Open,
    HalfOpen,
}

/// Timer utility for measuring operation duration
pub struct Timer {
    start: Instant,
}

impl Timer {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    pub fn record_and_finish<F>(self, record_fn: F)
    where
        F: FnOnce(Duration),
    {
        let duration = self.elapsed();
        record_fn(duration);
    }
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience macros for common metric operations
#[macro_export]
macro_rules! time_operation {
    ($operation:expr) => {{
        let timer = $crate::metrics::Timer::new();
        let result = $operation;
        let duration = timer.elapsed();
        (result, duration)
    }};
}

#[macro_export]
macro_rules! record_request_metrics {
    ($method:expr, $path:expr, $status:expr, $duration:expr) => {
        $crate::metrics::METRICS.record_request($method, $path, $status, $duration);
    };
}

/// Initialize metrics system
pub fn init_metrics() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize the metrics registry
    Lazy::force(&METRICS);

    tracing::info!("Metrics system initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_metrics_registry_creation() {
        let registry = MetricsRegistry::new();

        // Test that we can record metrics without panicking
        registry.record_request(
            "GET".to_string(),
            "/health".to_string(),
            200,
            Duration::from_millis(100),
        );
        registry.record_key_health(5, 4, 1);
        registry.record_key_rotation("primary".to_string(), true);
        registry.record_circuit_breaker_state("upstream".to_string(), CircuitBreakerState::Closed);
        registry.record_rate_limit("api".to_string(), false);
        registry.record_redis_operation("get".to_string(), true);
    }

    #[test]
    fn test_timer() {
        let timer = Timer::new();
        std::thread::sleep(Duration::from_millis(10));
        let elapsed = timer.elapsed();

        assert!(elapsed >= Duration::from_millis(10));
        assert!(elapsed < Duration::from_millis(100)); // Should be much less
    }

    #[test]
    fn test_time_operation_macro() {
        let (result, duration) = time_operation!({
            std::thread::sleep(Duration::from_millis(10));
            42
        });

        assert_eq!(result, 42);
        assert!(duration >= Duration::from_millis(10));
    }
}
