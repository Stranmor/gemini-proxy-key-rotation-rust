// src/core/health_check.rs

use std::time::{Duration, Instant};

/// Simple health checker for monitoring system health
pub struct HealthChecker {
    start_time: Instant,
}

impl HealthChecker {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
        }
    }

    pub fn uptime(&self) -> Duration {
        self.start_time.elapsed()
    }

    pub fn is_healthy(&self) -> bool {
        // Basic health check - can be extended with more sophisticated checks
        true
    }
}

impl Default for HealthChecker {
    fn default() -> Self {
        Self::new()
    }
}
