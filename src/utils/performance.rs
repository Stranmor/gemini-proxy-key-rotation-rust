// src/utils/performance.rs

use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Performance monitoring utility for tracking operation durations
pub struct PerformanceMonitor {
    operation_name: String,
    start_time: Instant,
    warn_threshold: Option<Duration>,
}

impl PerformanceMonitor {
    /// Create a new performance monitor for an operation
    pub fn new(operation_name: impl Into<String>) -> Self {
        Self {
            operation_name: operation_name.into(),
            start_time: Instant::now(),
            warn_threshold: None,
        }
    }
    
    /// Set a warning threshold - operations taking longer will be logged as warnings
    pub fn with_warn_threshold(mut self, threshold: Duration) -> Self {
        self.warn_threshold = Some(threshold);
        self
    }
    
    /// Finish monitoring and log the duration
    pub fn finish(self) {
        let duration = self.start_time.elapsed();
        
        match self.warn_threshold {
            Some(threshold) if duration > threshold => {
                warn!(
                    operation = %self.operation_name,
                    duration_ms = duration.as_millis(),
                    threshold_ms = threshold.as_millis(),
                    "Operation exceeded warning threshold"
                );
            }
            _ => {
                info!(
                    operation = %self.operation_name,
                    duration_ms = duration.as_millis(),
                    "Operation completed"
                );
            }
        }
    }
    
    /// Get the elapsed time without finishing the monitor
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }
}

impl Drop for PerformanceMonitor {
    fn drop(&mut self) {
        // Auto-log if not explicitly finished
        let duration = self.start_time.elapsed();
        info!(
            operation = %self.operation_name,
            duration_ms = duration.as_millis(),
            "Operation completed (auto-logged)"
        );
    }
}