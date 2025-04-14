// src/main.rs

// Use the library crate
use gemini_proxy_key_rotation_rust::*;

use axum::{routing::{any, get}, serve, Router};
use clap::Parser;
// AppConfig, AppState etc. are now brought into scope via the library use statement
use std::{net::SocketAddr, path::PathBuf, process, sync::Arc};
use tokio::net::TcpListener;
use tokio::signal;
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

/// Defines command-line arguments for the application using `clap`.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct CliArgs {
    /// Specifies the path to the YAML configuration file.
    #[arg(short, long, value_name = "FILE", default_value = "config.yaml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() {
    // --- Initialize Tracing ---
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| {
            // Default to `info` level if RUST_LOG is not set
            EnvFilter::new("info")
        });

    let subscriber = FmtSubscriber::builder()
        .with_env_filter(env_filter) // Use the determined filter
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("Setting default tracing subscriber failed");

    // --- Parse Command Line Arguments ---
    let args = CliArgs::parse();
    let config_path = &args.config;
    info!("Starting Gemini API Key Rotation Proxy...");
    // Log path only if it exists or if explicitly provided, to avoid confusion when it's optional
    if config_path.exists() || args.config != PathBuf::from("config.yaml") {
        info!("Using configuration file: {}", config_path.display());
    } else {
         info!("Optional configuration file '{}' not found. Using defaults and environment variables.", config_path.display());
    }


    // --- Configuration Loading & Validation ---
    // load_config now handles defaults, optional file, env vars, and validation internally
    // Correctly handle the Result from load_config which performs validation internally
    let app_config = match config::load_config(config_path) {
        Ok(cfg) => cfg,
        Err(e) => {
            error!(
                config_path = %config_path.display(),
                error = ?e,
                "Failed to load or validate configuration"
            );
            process::exit(1);
        }
    };

     // Log successful load details
     let total_keys: usize = app_config
         .groups
         .iter()
         .map(|g| g.api_keys.len())
         .sum();
     info!(
         "Configuration loaded successfully. Found {} group(s) with a total of {} API key(s). Server configured for {}:{}",
         app_config.groups.len(),
         total_keys,
         app_config.server.host,
         app_config.server.port
     );


    // --- Application State Initialization ---
    // AppState::new now returns a Result, handle it properly
    let app_state = match AppState::new(&app_config, config_path).await {
        Ok(state) => Arc::new(state),
        Err(e) => {
            // Log the specific error from AppState initialization
            error!(error = ?e, "Failed to initialize application state (e.g., HTTP client creation failed)");
            process::exit(1);
        }
    };

    // --- Server Setup ---
    let app = Router::new()
        .route("/health", get(handler::health_check)) // Add health check route
        .route("/*path", any(handler::proxy_handler)) // Catch-all proxy route
        .with_state(app_state);

    let addr: SocketAddr =
        match format!("{}:{}", app_config.server.host, app_config.server.port).parse() {
            Ok(addr) => addr,
            Err(e) => {
                error!(
                    "Invalid server address derived from config '{}:{}': {}",
                    app_config.server.host, app_config.server.port, e
                );
                process::exit(1);
            }
        };

    let listener = match TcpListener::bind(addr).await {
        Ok(listener) => {
            info!("Server listening on {}", addr);
            listener
        }
        Err(e) => {
            error!("Failed to bind to address {}: {}", addr, e);
            process::exit(1);
        }
    };

    // --- Run with Graceful Shutdown ---
    info!("Starting server...");
    if let Err(e) = serve(listener, app.into_make_service())
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        error!("Server error: {}", e);
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
        () = ctrl_c => { info!("Received Ctrl+C signal. Shutting down...") },
        () = terminate => { info!("Received terminate signal. Shutting down...") },
    }
}
