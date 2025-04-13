// src/config.rs
use serde::Deserialize;
use std::{fs, io, path::Path}; // Use fs::read_to_string
// Removed use thiserror::Error;
// Keep Url for validation
use crate::error::{AppError, Result}; // Import AppError and Result

/// Represents a group of API keys with optional proxy settings.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct KeyGroup {
    /// Unique name for the group, used for identification/logging.
    pub name: String,
    /// List of API keys for this group.
    pub api_keys: Vec<String>,
    /// Optional outgoing proxy URL (e.g., "http://user:pass@host:port", "socks5://host:port")
    /// If set, requests using keys from this group will be routed through this proxy.
    #[serde(default)] // Makes proxy_url optional, defaults to None
    pub proxy_url: Option<String>,
    /// Target API URL for this group. Defaults to the global Gemini URL.
    #[serde(default = "default_target_url")]
    pub target_url: String,
}

/// Represents the overall application configuration.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    /// Server listener configuration.
    pub server: ServerConfig,
    /// List of key groups. The order matters for sequential key selection.
    pub groups: Vec<KeyGroup>,
}

/// Represents the server binding configuration.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

// ConfigError enum removed, using AppError now.

/// Provides the default Gemini API URL.
fn default_target_url() -> String {
    "https://generativelanguage.googleapis.com".to_string()
}

/// Loads the application configuration from the specified YAML file.
/// Performs basic parsing only; detailed validation should occur elsewhere.
pub fn load_config(path: &Path) -> Result<AppConfig> { // Changed return type
    let path_str = path.display().to_string();

    // Reading the file content using AppError::Io
    let contents = fs::read_to_string(path).map_err(|e| {
        AppError::Io(io::Error::new( // Provide context within the error
            e.kind(),
            format!("Failed to read config file '{}': {}", path_str, e),
        ))
    })?; // Keep using ? with the new Result type

    // Parsing YAML using AppError::YamlParsing (#[from] handles this)
    let config: AppConfig = serde_yaml::from_str(&contents)?; // Use ? directly

    // Basic validation removed from here. It's handled in main.rs.

    Ok(config)
}
