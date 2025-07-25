// src/lib.rs

// Declare modules that constitute the library's public API or internal structure
pub mod config;
pub mod error;
pub mod handler;
pub mod key_manager;
pub mod proxy;
pub mod state;
pub mod admin;

// Re-export key types for easier use by the binary or tests
pub use config::AppConfig;
pub use error::{AppError, Result};
pub use state::AppState;
use std::sync::Arc;
use axum::{
    routing::{any, get},
    Router,
};
use tower_cookies::CookieManagerLayer;


pub async fn run(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(handler::health_check))
        .merge(admin::admin_routes())
        .route("/*path", any(handler::proxy_handler))
        .layer(CookieManagerLayer::new())
        .with_state(state)
}
