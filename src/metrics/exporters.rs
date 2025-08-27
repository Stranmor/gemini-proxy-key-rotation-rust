use axum::{http::StatusCode, response::IntoResponse};
use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::info;

/// Simple counters (Prometheus stub):
static TOTAL_REQUESTS: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));
static TOTAL_ERRORS: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));

/// Helper functions for increment (can be called from middleware in the future)
pub fn inc_total_requests() {
    TOTAL_REQUESTS.fetch_add(1, Ordering::Relaxed);
}
pub fn inc_total_errors() {
    TOTAL_ERRORS.fetch_add(1, Ordering::Relaxed);
}

/// Metrics exporter: returns text in format compatible with simple parser.
/// Will be replaced with prometheus_client later.
pub async fn metrics_handler() -> impl IntoResponse {
    info!("Metrics handler called");
    let total = TOTAL_REQUESTS.load(Ordering::Relaxed);
    let errors = TOTAL_ERRORS.load(Ordering::Relaxed);
    let body = format!(
        "# HELP app_requests_total Total number of requests\n# TYPE app_requests_total counter\napp_requests_total {total}\n# HELP app_errors_total Total number of error responses\n# TYPE app_errors_total counter\napp_errors_total {errors}\n"
    );
    (StatusCode::OK, body)
}
