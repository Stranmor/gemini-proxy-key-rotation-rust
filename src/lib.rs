// src/lib.rs

// Declare modules that constitute the library's public API or internal structure
pub mod admin;
pub mod config;
pub mod error;
pub mod handler;
pub mod key_manager;
pub mod proxy;
pub mod state;

// Re-export key types for easier use by the binary or tests
use axum::{
    Router,
    body::Body,
    http::Request as AxumRequest,
    middleware::{self, Next},
    response::Response as AxumResponse,
    routing::{any, get},
};
use std::{path::PathBuf, sync::Arc, time::Instant};
use tower_cookies::CookieManagerLayer;
use tracing::{Instrument, Level, error, info, span};
use uuid::Uuid;

pub use config::AppConfig;
pub use error::{AppError, Result};
pub use state::AppState;

/// Creates the main Axum router for the application.
pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(handler::health_check))
        .merge(admin::admin_routes())
        .route("/v1/*path", any(handler::proxy_handler))
        .route("/v1beta/*path", any(handler::proxy_handler))
        .route("/chat/*path", any(handler::proxy_handler))
        .route("/embeddings", any(handler::proxy_handler))
        .route("/models", any(handler::proxy_handler))
        .layer(CookieManagerLayer::new())
        .with_state(state)
}

/// Middleware to add Request ID and trace requests.
async fn trace_requests(req: AxumRequest<Body>, next: Next) -> AxumResponse {
    let request_id = Uuid::new_v4();
    let start_time = Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    let span = span!(
        Level::INFO,
        "request",
        request_id = %request_id,
        http.method = %method,
        url.path = %path,
    );

    let response = next.run(req).instrument(span).await;
    let elapsed = start_time.elapsed();

    info!(
        http.response.duration = ?elapsed,
        http.status_code = response.status().as_u16(),
        "Finished processing request"
    );

    response
}

/// The main application setup function, responsible for configuration, state initialization,
/// and router creation.
///
/// # Errors
///
/// This function will return an error if:
/// - Configuration loading or validation fails.
/// - The application state (e.g., HTTP clients) cannot be initialized.
pub async fn run(
    config_path_override: Option<PathBuf>,
) -> std::result::Result<(Router, AppConfig), AppError> {
    // --- Configuration Path ---
    let config_path = config_path_override.unwrap_or_else(|| {
        std::env::var("CONFIG_PATH").map_or_else(|_| PathBuf::from("config.yaml"), PathBuf::from)
    });

    info!("Starting Gemini API Key Rotation Proxy...");

    let config_path_display = config_path.display().to_string();
    if config_path.exists() {
        info!(config.path = %config_path_display, "Using configuration file");
    } else {
        info!(config.path = %config_path_display, "Optional configuration file not found. Using defaults and environment variables.");
    }

    // --- Configuration Loading & Validation ---
    let app_config = config::load_config(&config_path).map_err(|e| {
        error!(
            config.path = %config_path_display,
            error = ?e,
            "Failed to load or validate configuration. Exiting."
        );
        e
    })?;

    let total_keys: usize = app_config.groups.iter().map(|g| g.api_keys.len()).sum();
    let group_names: Vec<String> = app_config.groups.iter().map(|g| g.name.clone()).collect();
    info!(
         config.groups.count = app_config.groups.len(),
         config.groups.names = ?group_names,
         config.total_keys = total_keys,
         server.port = app_config.server.port,
         "Configuration loaded and validated successfully."
    );

    // --- Application State Initialization ---
    let app_state = AppState::new(&app_config, &config_path)
        .await
        .map_err(|e| {
            error!(
                error = ?e,
                "Failed to initialize application state. Exiting."
            );
            e
        })?;
    let app_state = Arc::new(app_state);
    // --- Router Setup ---
    let app = create_router(app_state.clone()).layer(middleware::from_fn(trace_requests));

    Ok((app, app_config))
}
