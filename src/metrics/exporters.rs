
use axum::{
    http::StatusCode,
    response::IntoResponse,
};
use tracing::info;

pub async fn metrics_handler() -> impl IntoResponse {
    info!("Metrics handler called");
    StatusCode::OK
}
