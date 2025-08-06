use axum::{extract::Request, middleware::Next, response::Response};
use std::time::Instant;
use tracing::info;

// Инкрементаторы простых счетчиков
use crate::metrics::exporters::{inc_total_errors, inc_total_requests};

/// Простейший middleware для метрик: измеряет длительность обработки запроса
/// и пишет её в логи, а также инкрементирует базовые счетчики.
pub async fn metrics_middleware(req: Request, next: Next) -> Response {
    let start = Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    // инкрементируем общий счетчик входящих запросов
    inc_total_requests();

    let response = next.run(req).await;
    let status = response.status();

    // если ответ ошибочный (4xx/5xx), инкрементируем счетчик ошибок
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
