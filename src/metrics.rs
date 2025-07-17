// src/metrics.rs

use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::time::Instant;

pub fn initialize_metrics() -> PrometheusHandle {
    let builder = PrometheusBuilder::new();
    builder
        .install_recorder()
        .expect("failed to install Prometheus recorder")
}

pub fn record_request_start() {
    gauge!("proxy_requests_in_flight").increment(1.0);
}

pub fn record_request_end(start_time: Instant, status_code: u16, group_name: &str) {
    let duration = start_time.elapsed().as_secs_f64();
    gauge!("proxy_requests_in_flight").decrement(1.0);
    counter!("proxy_requests_total", "status" => status_code.to_string(), "group" => group_name.to_string()).increment(1);
    histogram!("proxy_request_duration_seconds", "status" => status_code.to_string(), "group" => group_name.to_string()).record(duration);
}