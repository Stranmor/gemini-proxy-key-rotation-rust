use serde::Deserialize;
use std::{fs::File, io::Read, path::Path};
use thiserror::Error;
// Keep Url for validation

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

/// Defines errors that can occur during configuration loading and validation.
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to open config file '{path}': {source}")]
    FileOpen {
        path: String,
        source: std::io::Error,
    },

    #[error("Failed to read config file '{path}': {source}")]
    FileRead {
        path: String,
        source: std::io::Error,
    },

    #[error("Failed to parse config file '{path}': {source}")]
    Parse {
        path: String,
        source: serde_yaml::Error,
    },

    #[error("Validation error in config file '{path}': {message}")]
    Validation { path: String, message: String },
}

/// Provides the default Gemini API URL.
fn default_target_url() -> String {
    "https://generativelanguage.googleapis.com".to_string()
}

/// Loads and validates the application configuration from the specified YAML file.
/// Validation logic will be updated in main.rs or a dedicated validation function later.
pub fn load_config(path: &Path) -> Result<AppConfig, ConfigError> {
    let path_str = path.display().to_string();

    let mut file = File::open(path).map_err(|e| ConfigError::FileOpen {
        path: path_str.clone(),
        source: e,
    })?;

    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|e| ConfigError::FileRead {
            path: path_str.clone(),
            source: e,
        })?;

    let config: AppConfig = serde_yaml::from_str(&contents).map_err(|e| ConfigError::Parse {
        path: path_str.clone(),
        source: e,
    })?;

    // Basic validation (more detailed validation will be in main.rs or called from there)
    if config.server.port == 0 {
        return Err(ConfigError::Validation {
            path: path_str,
            message: "Server port cannot be 0.".to_string(),
        });
    }
    if config.groups.is_empty() {
        return Err(ConfigError::Validation {
            path: path_str,
            message: "The 'groups' list cannot be empty.".to_string(),
        });
    }

    // Note: Further validation (unique group names, non-empty keys per group, valid URLs)
    // should be performed after loading, likely in main.rs.

    Ok(config)
}
