use axum::{extract::Request, middleware::Next, response::Response};
use std::time::Instant;
use tracing::info;

// Simple counter incrementors
use crate::metrics::exporters::{inc_total_errors, inc_total_requests};

/// Simple middleware for metrics: measures request processing duration
/// and writes it to logs, as well as increments basic counters.
pub async fn metrics_middleware(req: Request, next: Next) -> Response {
    let start = Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    // increment general incoming requests counter
    inc_total_requests();

    let response = next.run(req).await;
    let status = response.status();

    // if response is error (4xx/5xx), increment error counter
    if status.is_client_error() || status.is_server_error() {
        inc_total_errors();
    }

    let elapsed = start.elapsed();
    info!(
        http.method = %method,
        url.path = %path,
        http.status_code = status.as_u16(),
        http.response.duration = ?elapsed,
        "metrics_middleware: request handled"
    );

    response
}
