// src/metrics.rs

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::RwLock;
use tracing::debug;

/// Metrics collector for the proxy service
#[derive(Debug, Clone)]
pub struct MetricsCollector {
    // Request counters
    pub total_requests: Arc<AtomicU64>,
    pub successful_requests: Arc<AtomicU64>,
    pub failed_requests: Arc<AtomicU64>,
    pub rate_limited_requests: Arc<AtomicU64>,
    
    // Key usage metrics
    pub key_usage_count: Arc<RwLock<HashMap<String, u64>>>,
    pub key_failure_count: Arc<RwLock<HashMap<String, u64>>>,
    
    // Response time tracking
    pub response_times: Arc<RwLock<Vec<Duration>>>,
    
    // Error tracking
    pub error_counts: Arc<RwLock<HashMap<String, u64>>>,
    
    // Service start time
    pub start_time: Instant,
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self {
            total_requests: Arc::new(AtomicU64::new(0)),
            successful_requests: Arc::new(AtomicU64::new(0)),
            failed_requests: Arc::new(AtomicU64::new(0)),
            rate_limited_requests: Arc::new(AtomicU64::new(0)),
            key_usage_count: Arc::new(RwLock::new(HashMap::new())),
            key_failure_count: Arc::new(RwLock::new(HashMap::new())),
            response_times: Arc::new(RwLock::new(Vec::new())),
            error_counts: Arc::new(RwLock::new(HashMap::new())),
            start_time: Instant::now(),
        }
    }

    /// Record a request start
    pub fn record_request_start(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        debug!("Recorded request start");
    }

    /// Record a successful request
    pub async fn record_request_success(&self, duration: Duration, api_key_preview: &str) {
        self.successful_requests.fetch_add(1, Ordering::Relaxed);
        
        // Record response time (keep only last 1000 for memory efficiency)
        {
            let mut times = self.response_times.write().await;
            times.push(duration);
            if times.len() > 1000 {
                times.remove(0);
            }
        }
        
        // Record key usage
        {
            let mut usage = self.key_usage_count.write().await;
            *usage.entry(api_key_preview.to_string()).or_insert(0) += 1;
        }
        
        debug!(
            duration_ms = duration.as_millis(),
            api_key_preview = api_key_preview,
            "Recorded successful request"
        );
    }

    /// Record a failed request
    pub async fn record_request_failure(&self, error_type: &str, api_key_preview: Option<&str>) {
        self.failed_requests.fetch_add(1, Ordering::Relaxed);
        
        // Record error type
        {
            let mut errors = self.error_counts.write().await;
            *errors.entry(error_type.to_string()).or_insert(0) += 1;
        }
        
        // Record key failure if key was involved
        if let Some(key_preview) = api_key_preview {
            let mut failures = self.key_failure_count.write().await;
            *failures.entry(key_preview.to_string()).or_insert(0) += 1;
        }
        
        debug!(
            error_type = error_type,
            api_key_preview = api_key_preview,
            "Recorded failed request"
        );
    }

    /// Record a rate limited request
    pub fn record_rate_limit(&self) {
        self.rate_limited_requests.fetch_add(1, Ordering::Relaxed);
        debug!("Recorded rate limited request");
    }

    /// Get current metrics snapshot
    pub async fn get_metrics_snapshot(&self) -> MetricsSnapshot {
        let response_times_guard = self.response_times.read().await;
        let avg_response_time = if response_times_guard.is_empty() {
            Duration::from_millis(0)
        } else {
            let total: Duration = response_times_guard.iter().sum();
            total / response_times_guard.len() as u32
        };

        let p95_response_time = if response_times_guard.is_empty() {
            Duration::from_millis(0)
        } else {
            let mut sorted_times = response_times_guard.clone();
            sorted_times.sort();
            let p95_index = (sorted_times.len() as f64 * 0.95) as usize;
            sorted_times.get(p95_index).copied().unwrap_or(Duration::from_millis(0))
        };

        MetricsSnapshot {
            total_requests: self.total_requests.load(Ordering::Relaxed),
            successful_requests: self.successful_requests.load(Ordering::Relaxed),
            failed_requests: self.failed_requests.load(Ordering::Relaxed),
            rate_limited_requests: self.rate_limited_requests.load(Ordering::Relaxed),
            avg_response_time_ms: avg_response_time.as_millis() as u64,
            p95_response_time_ms: p95_response_time.as_millis() as u64,
            uptime_seconds: self.start_time.elapsed().as_secs(),
            key_usage_count: self.key_usage_count.read().await.clone(),
            key_failure_count: self.key_failure_count.read().await.clone(),
            error_counts: self.error_counts.read().await.clone(),
        }
    }

    /// Export metrics in Prometheus format
    pub async fn export_prometheus_metrics(&self) -> String {
        let snapshot = self.get_metrics_snapshot().await;
        
        let mut output = String::new();
        
        // Basic counters
        output.push_str(&format!(
            "# HELP gemini_proxy_requests_total Total number of requests\n\
             # TYPE gemini_proxy_requests_total counter\n\
             gemini_proxy_requests_total {}\n\n",
            snapshot.total_requests
        ));
        
        output.push_str(&format!(
            "# HELP gemini_proxy_requests_successful_total Total number of successful requests\n\
             # TYPE gemini_proxy_requests_successful_total counter\n\
             gemini_proxy_requests_successful_total {}\n\n",
            snapshot.successful_requests
        ));
        
        output.push_str(&format!(
            "# HELP gemini_proxy_requests_failed_total Total number of failed requests\n\
             # TYPE gemini_proxy_requests_failed_total counter\n\
             gemini_proxy_requests_failed_total {}\n\n",
            snapshot.failed_requests
        ));
        
        output.push_str(&format!(
            "# HELP gemini_proxy_requests_rate_limited_total Total number of rate limited requests\n\
             # TYPE gemini_proxy_requests_rate_limited_total counter\n\
             gemini_proxy_requests_rate_limited_total {}\n\n",
            snapshot.rate_limited_requests
        ));
        
        // Response time metrics
        output.push_str(&format!(
            "# HELP gemini_proxy_response_time_avg_ms Average response time in milliseconds\n\
             # TYPE gemini_proxy_response_time_avg_ms gauge\n\
             gemini_proxy_response_time_avg_ms {}\n\n",
            snapshot.avg_response_time_ms
        ));
        
        output.push_str(&format!(
            "# HELP gemini_proxy_response_time_p95_ms 95th percentile response time in milliseconds\n\
             # TYPE gemini_proxy_response_time_p95_ms gauge\n\
             gemini_proxy_response_time_p95_ms {}\n\n",
            snapshot.p95_response_time_ms
        ));
        
        // Uptime
        output.push_str(&format!(
            "# HELP gemini_proxy_uptime_seconds Service uptime in seconds\n\
             # TYPE gemini_proxy_uptime_seconds gauge\n\
             gemini_proxy_uptime_seconds {}\n\n",
            snapshot.uptime_seconds
        ));
        
        // Key usage metrics
        output.push_str("# HELP gemini_proxy_key_usage_total Total usage count per API key\n");
        output.push_str("# TYPE gemini_proxy_key_usage_total counter\n");
        for (key_preview, count) in &snapshot.key_usage_count {
            output.push_str(&format!(
                "gemini_proxy_key_usage_total{{key_preview=\"{}\"}} {}\n",
                key_preview, count
            ));
        }
        output.push('\n');
        
        // Key failure metrics
        output.push_str("# HELP gemini_proxy_key_failures_total Total failure count per API key\n");
        output.push_str("# TYPE gemini_proxy_key_failures_total counter\n");
        for (key_preview, count) in &snapshot.key_failure_count {
            output.push_str(&format!(
                "gemini_proxy_key_failures_total{{key_preview=\"{}\"}} {}\n",
                key_preview, count
            ));
        }
        output.push('\n');
        
        // Error type metrics
        output.push_str("# HELP gemini_proxy_errors_total Total error count by type\n");
        output.push_str("# TYPE gemini_proxy_errors_total counter\n");
        for (error_type, count) in &snapshot.error_counts {
            output.push_str(&format!(
                "gemini_proxy_errors_total{{error_type=\"{}\"}} {}\n",
                error_type, count
            ));
        }
        
        output
    }
}

/// Snapshot of current metrics
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub rate_limited_requests: u64,
    pub avg_response_time_ms: u64,
    pub p95_response_time_ms: u64,
    pub uptime_seconds: u64,
    pub key_usage_count: HashMap<String, u64>,
    pub key_failure_count: HashMap<String, u64>,
    pub error_counts: HashMap<String, u64>,
}

/// Handler for Prometheus metrics endpoint
pub async fn metrics_handler(
    State(state): State<Arc<crate::state::AppState>>,
) -> impl IntoResponse {
    let prometheus_output = state.metrics.export_prometheus_metrics().await;
    
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/plain; version=0.0.4; charset=utf-8")
        .body(prometheus_output)
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body("Failed to generate metrics".to_string())
                .unwrap()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_metrics_collector_basic_operations() {
        let collector = MetricsCollector::new();
        
        // Record some metrics
        collector.record_request_start();
        collector.record_request_start();
        
        collector.record_request_success(Duration::from_millis(100), "key1").await;
        collector.record_request_failure("timeout", Some("key2")).await;
        collector.record_rate_limit();
        
        let snapshot = collector.get_metrics_snapshot().await;
        
        assert_eq!(snapshot.total_requests, 2);
        assert_eq!(snapshot.successful_requests, 1);
        assert_eq!(snapshot.failed_requests, 1);
        assert_eq!(snapshot.rate_limited_requests, 1);
        assert!(snapshot.avg_response_time_ms > 0);
        assert_eq!(snapshot.key_usage_count.get("key1"), Some(&1));
        assert_eq!(snapshot.key_failure_count.get("key2"), Some(&1));
        assert_eq!(snapshot.error_counts.get("timeout"), Some(&1));
    }

    #[tokio::test]
    async fn test_prometheus_export() {
        let collector = MetricsCollector::new();
        
        collector.record_request_start();
        collector.record_request_success(Duration::from_millis(50), "test_key").await;
        
        let prometheus_output = collector.export_prometheus_metrics().await;
        
        assert!(prometheus_output.contains("gemini_proxy_requests_total 1"));
        assert!(prometheus_output.contains("gemini_proxy_requests_successful_total 1"));
        assert!(prometheus_output.contains("gemini_proxy_key_usage_total{key_preview=\"test_key\"} 1"));
        assert!(prometheus_output.contains("# HELP"));
        assert!(prometheus_output.contains("# TYPE"));
    }

    #[test]
    fn test_metrics_collector_thread_safety() {
        let collector = Arc::new(MetricsCollector::new());
        let mut handles = vec![];
        
        // Spawn multiple threads to test thread safety
        for _i in 0..10 {
            let collector_clone = Arc::clone(&collector);
            let handle = std::thread::spawn(move || {
                for _ in 0..100 {
                    collector_clone.record_request_start();
                }
            });
            handles.push(handle);
        }
        
        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }
        
        // Should have recorded 1000 requests total
        assert_eq!(collector.total_requests.load(Ordering::Relaxed), 1000);
    }
}