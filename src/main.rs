// src/main.rs

use axum::serve;
use gemini_proxy_key_rotation_rust::{AppError, run};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::signal;
use tracing::{error, info};

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
        () = ctrl_c => { info!(signal = "Ctrl+C", "Received signal. Initiating graceful shutdown...") },
        () = terminate => { info!(signal = "Terminate", "Received signal. Initiating graceful shutdown...") },
    }
}

use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), AppError> {
    // --- Initialize Tracing (JSON format) ---
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let json_layer = fmt::layer()
        .json()
        .with_current_span(true)
        .with_span_list(true);
    tracing_subscriber::registry()
        .with(env_filter)
        .with(json_layer)
        .init();

    // The `run` function now configures the app and returns both the router and the config.
    let (app, config) = run(None).await.map_err(|e| {
        eprintln!("Application setup error: {e:?}");
        e
    })?;

    let addr = SocketAddr::from(([0, 0, 0, 0], config.server.port));
    let listener = TcpListener::bind(addr).await.map_err(|e| {
        error!(server.address = %addr, error = ?e, "Failed to bind to address. Exiting.");
        AppError::from(e)
    })?;
    info!(server.address = %addr, "Server listening");

    // --- Run with Graceful Shutdown ---
    info!("Starting server run loop...");
    serve(listener, app.into_make_service())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| {
            error!(error = ?e, "Server run loop encountered an error. Exiting.");
            AppError::from(e)
        })?;

    info!("Server shut down gracefully.");
    Ok(())
}
