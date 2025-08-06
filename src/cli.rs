use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "gemini-proxy",
    version,
    about = "Production-ready HTTP proxy for Google Gemini API with intelligent key rotation",
    long_about = "A high-performance, production-ready HTTP proxy for Google Gemini API that provides intelligent key rotation, load balancing, circuit breaking, and comprehensive monitoring capabilities."
)]
pub struct Cli {
    /// Configuration file path
    #[arg(short, long, value_name = "FILE", env = "GEMINI_PROXY_CONFIG")]
    pub config: Option<PathBuf>,

    /// Server bind address
    #[arg(short, long, default_value = "0.0.0.0", env = "GEMINI_PROXY_HOST")]
    pub host: String,

    /// Server port
    #[arg(short, long, env = "GEMINI_PROXY_PORT")]
    pub port: Option<u16>,

    /// Log level
    #[arg(short, long, default_value = "info", env = "RUST_LOG")]
    pub log_level: String,

    /// Enable JSON logging
    #[arg(long, env = "GEMINI_PROXY_JSON_LOGS")]
    pub json_logs: bool,

    /// Disable colored output
    #[arg(long)]
    pub no_color: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start the proxy server
    Serve {
        /// Enable development mode with hot reload
        #[arg(long)]
        dev: bool,
        
        /// Number of worker threads
        #[arg(long, env = "GEMINI_PROXY_WORKERS")]
        workers: Option<usize>,
    },
    
    /// Validate configuration file
    Config {
        /// Configuration file to validate
        #[arg(value_name = "FILE")]
        file: Option<PathBuf>,
        
        /// Show detailed validation output
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// Health check commands
    Health {
        /// Proxy server URL to check
        #[arg(short, long, default_value = "http://localhost:8080")]
        url: String,
        
        /// Perform detailed health check
        #[arg(short, long)]
        detailed: bool,
        
        /// Timeout in seconds
        #[arg(short, long, default_value = "10")]
        timeout: u64,
    },
    
    /// Key management commands
    Keys {
        #[command(subcommand)]
        action: KeyCommands,
    },
    
    /// Generate configuration templates
    Generate {
        #[command(subcommand)]
        template: GenerateCommands,
    },
}

#[derive(Subcommand)]
pub enum KeyCommands {
    /// List all configured keys and their status
    List {
        /// Show detailed key information
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// Test key validity
    Test {
        /// Specific key to test (by index or name)
        key: Option<String>,
    },
    
    /// Rotate keys manually
    Rotate {
        /// Force rotation even if current key is healthy
        #[arg(short, long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub enum GenerateCommands {
    /// Generate example configuration file
    Config {
        /// Output file path
        #[arg(short, long, default_value = "config.yaml")]
        output: PathBuf,
        
        /// Include advanced configuration options
        #[arg(short, long)]
        advanced: bool,
    },
    
    /// Generate systemd service file
    Systemd {
        /// Output file path
        #[arg(short, long, default_value = "gemini-proxy.service")]
        output: PathBuf,
        
        /// Binary path
        #[arg(short, long)]
        binary_path: Option<PathBuf>,
        
        /// User to run service as
        #[arg(short, long, default_value = "gemini-proxy")]
        user: String,
    },
    
    /// Generate Docker Compose file
    Docker {
        /// Output file path
        #[arg(short, long, default_value = "docker-compose.yml")]
        output: PathBuf,
        
        /// Include monitoring stack
        #[arg(short, long)]
        monitoring: bool,
    },
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}