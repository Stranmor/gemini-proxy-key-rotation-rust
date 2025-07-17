// src/main.rs

// Use the library crate
use gemini_proxy_key_rotation_rust::*;

use axum::{
    body::Body,                         // Import Body
    http::Request as AxumRequest,       // Import Request explicitly
    middleware::{self, Next},           // Import Axum middleware utilities
    response::Response as AxumResponse, // Import Response
    routing::{any, get},
    serve,
    Router,
};
use clap::Parser;
// AppConfig, AppState etc. are now brought into scope via the library use statement
use std::{net::SocketAddr, path::PathBuf, process, sync::Arc, time::Instant}; // Added Instant for middleware timing
use tokio::net::TcpListener;
use tokio::signal;
use tower::ServiceBuilder; // Import ServiceBuilder for layering middleware
use tracing::{error, info, span, Instrument, Level}; // Correct tracing imports
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter}; // Correct subscriber imports
use uuid::Uuid; // Import Uuid

/// Defines command-line arguments for the application using `clap`.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct CliArgs {
    /// Specifies the path to the YAML configuration file.
    #[arg(short, long, value_name = "FILE", default_value = "config.yaml")]
    config: PathBuf,
}

// Middleware to add Request ID and trace requests
// Note: Changed signature to accept Request<Body> and return AxumResponse directly
// This simplifies the middleware implementation and error handling.
/// # Panics
/// This function does not panic.
async fn trace_requests(
    req: AxumRequest<Body>, // Accepts Request<Body> directly
    next: Next,             // Next no longer needs <B>
) -> AxumResponse {
    let request_id = Uuid::new_v4();
    let start_time = Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_string(); // Clone path for span

    // Create a span for the request
    let span = span!(
        Level::INFO,
        "request", // Span name
        request_id = %request_id,
        http.method = %method, // Use cloned method
        url.path = %path, // Use cloned path
        // Example of adding another header if needed (be careful with sensitive data)
        // http.user_agent = req.headers().get("user-agent").and_then(|v| v.to_str().ok()).unwrap_or("unknown"),
    );

    // Enter the span and execute the rest of the request chain
    // Instrument the future returned by next.run()
    let response = next.run(req).instrument(span).await;

    // Log completion after the response is generated
    let elapsed = start_time.elapsed();
    // Use the already created span for the final log
    // Note: We use info! directly here because the span is automatically attached
    info!(
        http.response.duration = ?elapsed,
        http.status_code = response.status().as_u16(),
        "Finished processing request"
    );

    response // Return the response directly
}

/// # Panics
///
/// This function will panic if:
/// - It fails to install the Ctrl+C or terminate signal handlers.
/// - Configuration loading or validation fails.
/// - The application state (e.g., HTTP clients) cannot be initialized.
/// - The server fails to bind to the specified address.
/// - The server run loop encounters a fatal error.
#[tokio::main]
async fn main() {
    // --- Initialize Tracing (JSON format) ---
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // Configure JSON logging layer
    let json_layer = fmt::layer()
        .json() // Output logs in JSON format
        .with_current_span(true) // Include current span info in logs
        .with_span_list(true); // Include parent spans info in logs

    // Combine filter and JSON layer, and set as global default
    tracing_subscriber::registry()
        .with(env_filter)
        .with(json_layer)
        .init();

    // --- Parse Command Line Arguments ---
    let args = CliArgs::parse();
    let config_path = &args.config;

    // Use structured logging from the start
    info!("Starting Gemini API Key Rotation Proxy...");

    // Log config path details
    let config_path_display = config_path.display().to_string(); // Capture display string
    if config_path.exists() || args.config != PathBuf::from("config.yaml") {
        info!(config.path = %config_path_display, "Using configuration file");
    } else {
        info!(config.path = %config_path_display, "Optional configuration file not found. Using defaults and environment variables.");
    }

    // --- Configuration Loading & Validation ---
    let app_config = match config::load_config(config_path) {
        Ok(cfg) => cfg,
        Err(e) => {
            // Structured error logging
            error!(
                config.path = %config_path_display,
                error = ?e, // Use debug formatting for the error
                "Failed to load or validate configuration. Exiting."
            );
            process::exit(1);
        }
    };

    // Log successful load details with structured fields
    let total_keys: usize = app_config.groups.iter().map(|g| g.api_keys.len()).sum();
    let group_names: Vec<String> = app_config.groups.iter().map(|g| g.name.clone()).collect();
    info!(
         config.groups.count = app_config.groups.len(),
         config.groups.names = ?group_names, // Log group names
         config.total_keys = total_keys,
         server.host = %app_config.server.host,
         server.port = app_config.server.port,
         "Configuration loaded and validated successfully."
    );

    // --- Application State Initialization ---
    let app_state = match AppState::new(&app_config, config_path).await {
        Ok(state) => Arc::new(state),
        Err(e) => {
            // Structured error logging
            error!(
                error = ?e,
                "Failed to initialize application state (e.g., HTTP client creation or state loading failed). Exiting."
            );
            process::exit(1);
        }
    };

    // --- Server Setup ---
    // Apply the tracing middleware
    let app = Router::new()
        .route("/health", get(handler::health_check)) // Add health check route
        .route("/*path", any(handler::proxy_handler)) // Catch-all proxy route
        .layer(
            // Apply middleware using ServiceBuilder
            ServiceBuilder::new().layer(middleware::from_fn(trace_requests)),
        )
        .with_state(app_state);

    let addr_str = format!("{}:{}", app_config.server.host, app_config.server.port);
    let addr: SocketAddr = match addr_str.parse() {
        Ok(addr) => addr,
        Err(e) => {
            error!(
                server.address = %addr_str,
                error = ?e,
                "Invalid server address derived from configuration. Exiting."
            );
            process::exit(1);
        }
    };

    let listener = match TcpListener::bind(addr).await {
        Ok(listener) => {
            // Log with structured address field
            info!(server.address = %addr, "Server listening");
            listener
        }
        Err(e) => {
            error!(server.address = %addr, error = ?e, "Failed to bind to address. Exiting.");
            process::exit(1);
        }
    };

    // --- Run with Graceful Shutdown ---
    info!("Starting server run loop...");
    if let Err(e) = serve(listener, app.into_make_service())
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        error!(error = ?e, "Server run loop encountered an error. Exiting.");
        process::exit(1);
    }

    info!("Server shut down gracefully.");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        // Log the specific signal received
        () = ctrl_c => { info!(signal = "Ctrl+C", "Received signal. Initiating graceful shutdown...") },
        () = terminate => { info!(signal = "Terminate", "Received signal. Initiating graceful shutdown...") },
    }
}
