use anyhow::Result;
use gemini_proxy::{
    cli::{Cli, Commands, GenerateCommands, KeyCommands},
    error::{context::ErrorContext, AppError},
    run,
};
use std::{io::ErrorKind, net::SocketAddr, path::PathBuf, process};
use tokio::{net::TcpListener, signal};
use tracing::{error, info};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Registry};

/// Initialize structured logging with configurable output format
fn init_logging(json_logs: bool, log_level: &str, no_color: bool) -> Result<()> {
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(log_level))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let registry = Registry::default().with(env_filter);

    if json_logs {
        let json_layer = fmt::layer()
            .json()
            .with_current_span(true)
            .with_span_list(true)
            .with_target(true)
            .with_thread_ids(true)
            .with_file(true)
            .with_line_number(true);

        registry.with(json_layer).init();
    } else {
        let fmt_layer = fmt::layer()
            .with_target(true)
            .with_thread_ids(true)
            .with_file(true)
            .with_line_number(true)
            .with_ansi(!no_color);

        registry.with(fmt_layer).init();
    }

    Ok(())
}

/// Graceful shutdown signal handler
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
        () = ctrl_c => {
            info!(signal = "SIGINT", "Received shutdown signal. Initiating graceful shutdown...");
        },
        () = terminate => {
            info!(signal = "SIGTERM", "Received shutdown signal. Initiating graceful shutdown...");
        },
    }
}

/// Start the proxy server
async fn serve_command(
    config_path: Option<PathBuf>,
    host: String,
    port: Option<u16>,
    dev: bool,
    workers: Option<usize>,
) -> Result<()> {
    let context = ErrorContext::new("server_startup")
        .with_metadata("dev_mode", dev.to_string())
        .with_metadata("host", host.clone());

    gemini_proxy::error::context::set_error_context(context);
    let result = {
        info!(
            dev_mode = dev,
            workers = workers,
            "Starting Gemini Proxy server"
        );

        // Configure tokio runtime if workers specified
        if let Some(worker_count) = workers {
            info!(
                workers = worker_count,
                "Configuring custom worker thread count"
            );
        }

        // Load configuration and create app
        let (app, config) = run(config_path).await?;

        // Override port if specified via CLI
        // Override port if specified via CLI
        let mut server_port = port.unwrap_or(config.server.port);
        let listener = {
            let mut listener: Option<TcpListener> = None;
            for i in 0..10 {
                let current_port = server_port + i;
                let addr = SocketAddr::from(([0, 0, 0, 0], current_port));
                match TcpListener::bind(addr).await {
                    Ok(l) => {
                        server_port = current_port;
                        listener = Some(l);
                        break;
                    }
                    Err(e) if e.kind() == ErrorKind::AddrInUse => {
                        tracing::warn!(
                            port = current_port,
                            "Port already in use, trying next port"
                        );
                        continue;
                    }
                    Err(e) => {
                        error!(
                            address = %addr,
                            error = %e,
                            "Failed to bind to address"
                        );
                        return Err(AppError::Internal {
                            message: format!("Failed to bind to {addr}: {e}"),
                        }
                        .into());
                    }
                }
            }
            listener.ok_or_else(|| {
                let final_error_msg =
                    format!("Failed to bind to any port after 10 attempts. Last port tried: {server_port}");
                error!(error = final_error_msg);
                AppError::Internal {
                    message: final_error_msg,
                }
            })?
        };

        info!(
            address = %listener.local_addr().unwrap(),
            version = env!("CARGO_PKG_VERSION"),
            "Gemini Proxy server started successfully"
        );
        // Start server with graceful shutdown
        axum::serve(listener, app.into_make_service())
            .with_graceful_shutdown(shutdown_signal())
            .await
            .map_err(|e| {
                error!(error = %e, "Server error occurred");
                AppError::Internal {
                    message: format!("Server error: {e}"),
                }
            })?;

        info!("Server shut down gracefully");
        Ok(())
    };
    gemini_proxy::error::context::clear_error_context();
    result
}

/// Validate configuration file
async fn config_command(file: Option<PathBuf>, verbose: bool) -> Result<()> {
    let config_path = file.unwrap_or_else(|| PathBuf::from("config.yaml"));

    info!(path = %config_path.display(), "Validating configuration file");

    match gemini_proxy::config::load_config(&config_path) {
        Ok(config) => {
            info!("✅ Configuration is valid");

            if verbose {
                println!("Configuration details:");
                println!("  Server port: {}", config.server.port);
                println!("  Groups: {}", config.groups.len());

                let total_keys: usize = config.groups.iter().map(|g| g.api_keys.len()).sum();
                println!("  Total API keys: {total_keys}");

                if let Some(redis_url) = &config.redis_url {
                    println!("  Redis URL: {redis_url}");
                }
            }
        }
        Err(e) => {
            error!(error = %e, "Configuration validation failed");
            return Err(e.into());
        }
    }

    Ok(())
}

/// Perform health check
async fn health_command(url: String, detailed: bool, timeout: u64) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout))
        .build()?;

    let endpoint = if detailed {
        format!("{url}/health/detailed")
    } else {
        format!("{url}/health")
    };

    info!(endpoint = %endpoint, "Performing health check");

    match client.get(&endpoint).send().await {
        Ok(response) => {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();

            if status.is_success() {
                info!(status = %status, "✅ Health check passed");
                if detailed && !body.is_empty() {
                    println!("Response: {body}");
                }
            } else {
                error!(status = %status, body = %body, "❌ Health check failed");
                process::exit(1);
            }
        }
        Err(e) => {
            error!(error = %e, "❌ Health check request failed");
            process::exit(1);
        }
    }

    Ok(())
}

/// Handle key management commands
async fn keys_command(action: KeyCommands) -> Result<()> {
    match action {
        KeyCommands::List { verbose: _verbose } => {
            info!("Listing configured API keys");
            // TODO: Implement key listing
            println!("Key listing not yet implemented");
        }
        KeyCommands::Test { key } => {
            info!(key = ?key, "Testing API key validity");
            // TODO: Implement key testing
            println!("Key testing not yet implemented");
        }
        KeyCommands::Rotate { force } => {
            info!(force = force, "Rotating API keys");
            // TODO: Implement key rotation
            println!("Key rotation not yet implemented");
        }
    }
    Ok(())
}

/// Generate configuration templates
async fn generate_command(template: GenerateCommands) -> Result<()> {
    match template {
        GenerateCommands::Config { output, advanced } => {
            info!(path = %output.display(), advanced = advanced, "Generating configuration template");
            // TODO: Implement config generation
            println!("Config generation not yet implemented");
        }
        GenerateCommands::Systemd {
            output,
            binary_path,
            user,
        } => {
            info!(
                path = %output.display(),
                binary_path = ?binary_path,
                user = %user,
                "Generating systemd service file"
            );
            // TODO: Implement systemd service generation
            println!("Systemd service generation not yet implemented");
        }
        GenerateCommands::Docker { output, monitoring } => {
            info!(
                path = %output.display(),
                monitoring = monitoring,
                "Generating Docker Compose file"
            );
            // TODO: Implement Docker Compose generation
            println!("Docker Compose generation not yet implemented");
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse_args();

    // Initialize logging first
    if let Err(e) = init_logging(cli.json_logs, &cli.log_level, cli.no_color) {
        eprintln!("Failed to initialize logging: {e}");
        process::exit(1);
    }

    // Set global error context
    let context = ErrorContext::new("main")
        .with_metadata("version", env!("CARGO_PKG_VERSION"))
        .with_metadata(
            "args",
            format!("{:?}", std::env::args().collect::<Vec<_>>()),
        );
    gemini_proxy::error::context::set_error_context(context);

    info!(version = env!("CARGO_PKG_VERSION"), "Starting Gemini Proxy");

    // Handle commands
    let result = match cli.command {
        Some(Commands::Serve { dev, workers }) => {
            serve_command(cli.config, cli.host, cli.port, dev, workers).await
        }
        Some(Commands::Config { file, verbose }) => config_command(file, verbose).await,
        Some(Commands::Health {
            url,
            detailed,
            timeout,
        }) => health_command(url, detailed, timeout).await,
        Some(Commands::Keys { action }) => keys_command(action).await,
        Some(Commands::Generate { template }) => generate_command(template).await,
        None => {
            // Default to serve command
            serve_command(cli.config, cli.host, cli.port, false, None).await
        }
    };

    if let Err(e) = result {
        error!(error = %e, "Command failed");
        process::exit(1);
    }

    Ok(())
}
