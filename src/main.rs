mod config;
mod handler;
mod state;

use actix_web::{web, App, HttpServer, middleware::Logger};
use clap::Parser;
use state::ProxyState;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the configuration file
    #[arg(short, long, default_value = "config.yaml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    info!("Loading configuration from: {:?}", args.config);

    let app_config = match config::load_config(&args.config) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

    let mut server_handles = Vec::new();

    for proxy_config in app_config.proxies {
        let proxy_name = proxy_config.name.clone();
        let listen_address = proxy_config.listen_address.clone();
        let proxy_state = web::Data::new(ProxyState::new(proxy_config));

        info!(
            "Starting proxy '{}' on {} -> {}",
            proxy_name,
            listen_address,
            proxy_state.config.target_url
        );

        let server = HttpServer::new(move || {
            App::new()
                .app_data(proxy_state.clone())
                .wrap(Logger::default())
                .default_service(web::route().to(handler::proxy_handler))
        })
        .bind(&listen_address)?
        .run();

        server_handles.push(server);
        info!("Proxy '{}' listening on {}", proxy_name, listen_address);
    }

    if server_handles.is_empty() {
        eprintln!("No proxies defined in the configuration file. Exiting.");
        std::process::exit(1);
    }

    info!("All proxy servers started. Press Ctrl+C to stop.");

    tokio::signal::ctrl_c().await?;
    info!("Ctrl+C received, shutting down servers.");

    Ok(())
}