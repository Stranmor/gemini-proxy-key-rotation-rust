mod config;
mod handler;
mod state;

use axum::{routing::any, serve, Router};
use clap::Parser;
use state::AppState;
use std::{collections::HashSet, net::SocketAddr, path::PathBuf, process, sync::Arc}; // Added HashSet for duplicate check
use tokio::net::TcpListener;
use tokio::signal;
use tracing::{error, info, warn, Level}; // Added warn
use tracing_subscriber::FmtSubscriber;
use url::Url; // Needed for URL validation

/// Command line arguments
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct CliArgs {
    /// Path to the configuration file
    #[arg(short, long, value_name = "FILE", default_value = "config.yaml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() {
    // Initialize tracing subscriber
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO) // Default level
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env()) // Allow overriding with RUST_LOG
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .expect("Setting default tracing subscriber failed");

    // --- Parse Command Line Arguments ---
    let args = CliArgs::parse();
    let config_path = &args.config;

    info!("Starting Gemini API Key Rotation Proxy...");
    info!("Using configuration file: {}", config_path.display());

    // --- Configuration Loading and Validation ---
    let app_config = match config::load_config(config_path) {
        Ok(mut cfg) => {
            // --- Perform Detailed Validation ---
            let config_path_str = config_path.display().to_string(); // For error messages

            // Basic server config validation (already partially done in load_config)
            if cfg.server.host.trim().is_empty() {
                error!(
                    "Configuration error in {}: Server host cannot be empty.",
                    config_path_str
                );
                process::exit(1);
            }
            if cfg.server.port == 0 {
                error!(
                    "Configuration error in {}: Server port cannot be 0.",
                    config_path_str
                );
                process::exit(1);
            }

            // Groups validation
            if cfg.groups.is_empty() {
                error!(
                    "Configuration error in {}: The 'groups' list cannot be empty.",
                    config_path_str
                );
                process::exit(1);
            }

            let mut group_names = HashSet::new();
            let mut total_keys = 0;
            let mut has_errors = false;

            for group in &mut cfg.groups {
                // Iterate mutably to potentially trim names
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
                // Basic check for invalid characters in group name (optional, adjust as needed)
                if group.name.contains('/') || group.name.contains(':') || group.name.contains(' ')
                {
                    warn!("Configuration warning in {}: Group name '{}' contains potentially problematic characters (/, :, space).", config_path_str, group.name);
                    // Decide if this should be a hard error or just a warning
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
                total_keys += group.api_keys.len();

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
                    if Url::parse(proxy_url).is_err() {
                        error!(
                            "Configuration error in {}: Group '{}' has an invalid proxy_url: '{}'.",
                            config_path_str, group.name, proxy_url
                        );
                        has_errors = true;
                    }
                    // Check scheme (optional but recommended)
                    match Url::parse(proxy_url).map(|u| u.scheme().to_lowercase()) {
                        Ok(scheme)
                            if scheme == "http" || scheme == "https" || scheme == "socks5" =>
                        {
                            ()
                        } // OK
                        Ok(scheme) => {
                            error!("Configuration error in {}: Group '{}' has an unsupported proxy scheme: '{}' in proxy_url: '{}'. Only http, https, socks5 are supported by reqwest.", config_path_str, group.name, scheme, proxy_url);
                            has_errors = true;
                        }
                        Err(_) => {} // Already handled by the parse check above
                    }
                }
            }

            if has_errors {
                error!(
                    "Configuration validation failed. Please check the errors above in {}.",
                    config_path_str
                );
                process::exit(1);
            }

            if total_keys == 0 {
                error!(
                    "Configuration error in {}: No valid API keys found across all groups.",
                    config_path_str
                );
                process::exit(1);
            }

            // Log success with updated info
            info!(
                "Configuration loaded and validated successfully. Found {} group(s) with a total of {} API key(s). Server configured for {}:{}",
                cfg.groups.len(),
                total_keys,
                cfg.server.host,
                cfg.server.port
            );
            cfg // Return validated config
        }
        Err(e) => {
            error!(
                "Failed to load configuration from {}: {}",
                config_path.display(),
                e
            );
            process::exit(1);
        }
    };

    // --- Application State ---
    // Pass the validated and potentially modified config to AppState::new
    let app_state = match AppState::new(&app_config) {
        // Pass by reference
        Ok(state) => Arc::new(state),
        Err(e) => {
            // AppState::new now handles HTTP client creation errors
            error!("Failed to initialize application state: {}", e);
            process::exit(1);
        }
    };

    // --- Server Setup ---
    let app = Router::new()
        // Route all paths; the handler will manage key selection
        .route("/*path", any(handler::proxy_handler))
        .with_state(app_state);

    // Use the server config from the loaded app_config
    let addr: SocketAddr =
        match format!("{}:{}", app_config.server.host, app_config.server.port).parse() {
            Ok(addr) => addr,
            Err(e) => {
                // This should ideally be caught by validation now, but keep as a safeguard
                error!(
                    "Invalid server address derived from config '{}:{}': {}",
                    app_config.server.host, app_config.server.port, e
                );
                process::exit(1);
            }
        };

    // Create TcpListener
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
