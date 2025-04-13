// src/main.rs
mod config;
mod error;
mod handler;
mod key_manager;
mod proxy;
mod state;

use axum::{routing::any, serve, Router};
use clap::Parser;
use config::AppConfig; // Keep explicit import for clarity
use state::AppState;
use std::{collections::HashSet, net::SocketAddr, path::PathBuf, process, sync::Arc};
use tokio::net::TcpListener;
use tokio::signal;
use tracing::{error, info, warn};
use tracing_subscriber::{EnvFilter, FmtSubscriber}; // Added EnvFilter
use url::Url; // Keep for validation

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
    // Removed call to non-existent init_tracing()
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| {
            // If RUST_LOG is not set, default to 'info' for the current crate
            // and 'warn' for others. Adjust as needed for desired default verbosity.
            // Example: "warn,gemini_proxy_key_rotation_rust=info"
            // Using "info" for simplicity now.
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
    info!("Using configuration file: {}", config_path.display());

    // --- Configuration Loading & Validation ---
    // Removed call to non-existent get_config()
    let mut app_config = match config::load_config(config_path) {
        Ok(cfg) => cfg,
        Err(e) => {
            error!(
                path = %config_path.display(),
                error = ?e,
                "Failed to load configuration"
            );
            process::exit(1);
        }
    };

    let config_path_str = config_path.display().to_string();
    if !validate_config(&mut app_config, &config_path_str) {
        error!(
            "Configuration validation failed. Please check the errors above in {}.",
            config_path_str
        );
        process::exit(1);
    } else {
        let total_keys: usize = app_config
            .groups
            .iter()
            .map(|g| g.api_keys.len())
            .sum();
        info!(
            "Configuration loaded and validated successfully. Found {} group(s) with a total of {} API key(s). Server configured for {}:{}",
            app_config.groups.len(),
            total_keys,
            app_config.server.host,
            app_config.server.port
        );
    }

    // --- Application State Initialization ---
    // AppState::new handles HttpClient and KeyManager initialization internally
    let app_state = match AppState::new(&app_config) {
        Ok(state) => Arc::new(state),
        Err(e) => {
            error!(error = ?e, "Failed to initialize application state");
            process::exit(1);
        }
    };

    // --- Server Setup ---
    // Removed call to non-existent create_app()
    let app = Router::new()
        .route("/*path", any(handler::proxy_handler)) // Ensure handler::proxy_handler exists and is pub
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

/// Performs validation checks on the loaded `AppConfig`.
///
/// Checks include:
/// - Non-empty server host and non-zero port.
/// - Presence of at least one group.
/// - Unique and non-empty group names.
/// - Presence of non-empty `api_keys` within each group.
/// - Validity of `target_url` and optional `proxy_url` formats.
/// - Presence of at least one valid API key across all groups.
///
/// Logs errors or warnings using `tracing` and returns `true` if valid, `false` otherwise.
// (Keep validate_config function as is - it seems correct)
fn validate_config(cfg: &mut AppConfig, config_path_str: &str) -> bool {
    let mut has_errors = false;

    // Basic server config validation
    if cfg.server.host.trim().is_empty() {
        error!(
            "Configuration error in {}: Server host cannot be empty.",
            config_path_str
        );
        has_errors = true;
    }
    if cfg.server.port == 0 {
        error!(
            "Configuration error in {}: Server port cannot be 0.",
            config_path_str
        );
        has_errors = true;
    }

    // Groups validation
    if cfg.groups.is_empty() {
        error!(
            "Configuration error in {}: The 'groups' list cannot be empty.",
            config_path_str
        );
        // Return early if no groups, as further checks depend on them
        return false; // Indicate failure directly
    }

    let mut group_names = HashSet::new();
    let mut total_keys = 0;

    for group in &mut cfg.groups {
        // Validate group name
        group.name = group.name.trim().to_string(); // Trim whitespace
        if group.name.is_empty() {
            error!(
                "Configuration error in {}: Group name cannot be empty.",
                config_path_str
            );
            has_errors = true;
        } else if !group_names.insert(group.name.clone()) {
            error!(
                "Configuration error in {}: Duplicate group name found: '{}'.",
                config_path_str, group.name
            );
            has_errors = true;
        }
        // Basic check for invalid characters in group name
        if group.name.contains('/') || group.name.contains(':') || group.name.contains(' ') {
            warn!("Configuration warning in {}: Group name '{}' contains potentially problematic characters (/, :, space).", config_path_str, group.name);
        }

        // Validate API keys within the group
        if group.api_keys.is_empty() {
            error!(
                "Configuration error in {}: Group '{}' has an empty 'api_keys' list.",
                config_path_str, group.name
            );
            has_errors = true;
        }
        if group.api_keys.iter().any(|key| key.trim().is_empty()) {
             error!("Configuration error in {}: Group '{}' contains one or more empty API key strings.", config_path_str, group.name);
             has_errors = true;
        }
        // Count only non-empty keys for the total_keys check
        total_keys += group.api_keys.iter().filter(|k| !k.trim().is_empty()).count();


        // Validate target_url
        if Url::parse(&group.target_url).is_err() {
            error!(
                "Configuration error in {}: Group '{}' has an invalid target_url: '{}'.",
                config_path_str, group.name, group.target_url
            );
            has_errors = true;
        }

        // Validate proxy_url if present
        if let Some(proxy_url) = &group.proxy_url {
            match Url::parse(proxy_url) {
                Ok(parsed_url) => {
                    // Check scheme
                    let scheme = parsed_url.scheme().to_lowercase();
                    if scheme != "http" && scheme != "https" && scheme != "socks5" {
                         error!("Configuration error in {}: Group '{}' has an unsupported proxy scheme: '{}' in proxy_url: '{}'. Only http, https, socks5 are supported.", config_path_str, group.name, scheme, proxy_url);
                         has_errors = true;
                    }
                }
                Err(_) => {
                     error!(
                        "Configuration error in {}: Group '{}' has an invalid proxy_url: '{}'.",
                        config_path_str, group.name, proxy_url
                    );
                    has_errors = true;
                }
            }
        }
    } // End loop through groups

    // Final check for total non-empty keys
    if total_keys == 0 && !cfg.groups.is_empty() { // Check only if groups exist
        error!(
            "Configuration error in {}: No valid (non-empty) API keys found across all groups.",
            config_path_str
        );
        has_errors = true;
    }


    !has_errors // Return true if no errors were found
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
