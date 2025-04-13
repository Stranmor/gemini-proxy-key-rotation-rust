// src/config.rs
use serde::Deserialize;
use std::{env, fs, io, path::Path}; // Added `env`
use tracing::{info, warn}; // Added tracing for logging overrides
use crate::error::{AppError, Result}; // Import AppError and Result

/// Represents a group of API keys with optional proxy settings.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct KeyGroup {
    /// Unique name for the group, used for identification/logging.
    pub name: String,
    /// List of API keys for this group. Can be overridden by environment variables.
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

/// Provides the default Gemini API URL.
fn default_target_url() -> String {
    "https://generativelanguage.googleapis.com".to_string()
}

/// Helper function to sanitize group names for environment variable lookup.
/// Converts to uppercase and replaces non-alphanumeric characters with underscores.
fn sanitize_group_name_for_env(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}


/// Loads the application configuration from the specified YAML file
/// and overrides API keys from environment variables if present.
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
    let mut config: AppConfig = serde_yaml::from_str(&contents)?; // Use ? directly

    // --- Override API keys from environment variables ---
    for group in &mut config.groups {
        let sanitized_group_name = sanitize_group_name_for_env(&group.name);
        let env_var_name = format!("GEMINI_PROXY_GROUP_{}_API_KEYS", sanitized_group_name);

        match env::var(&env_var_name) {
            Ok(env_keys_str) => {
                if env_keys_str.trim().is_empty() {
                    // Env var exists but is empty - override with empty list and warn
                     warn!(
                        "Environment variable '{}' found but is empty. Overriding API keys for group '{}' with an empty list.",
                        env_var_name, group.name
                    );
                    group.api_keys = Vec::new();
                } else {
                    // Env var exists and is not empty - parse and override
                    let keys_from_env: Vec<String> = env_keys_str
                        .split(',')
                        .map(|k| k.trim().to_string())
                        .filter(|k| !k.is_empty()) // Filter out potential empty strings from bad formatting (e.g., "key1,,key2")
                        .collect();

                    if !keys_from_env.is_empty() {
                         info!(
                            "Overriding API keys for group '{}' from environment variable '{}' ({} keys found).",
                            group.name, env_var_name, keys_from_env.len()
                        );
                        group.api_keys = keys_from_env;
                    } else {
                        // Env var contained only commas/whitespace
                         warn!(
                            "Environment variable '{}' found but contained no valid keys after trimming/splitting. Overriding API keys for group '{}' with an empty list.",
                            env_var_name, group.name
                        );
                        group.api_keys = Vec::new();
                    }
                }
            }
            Err(env::VarError::NotPresent) => {
                // Variable not set, keep keys from config file (do nothing)
            }
            Err(e) => {
                // Other error reading environment variable (e.g., invalid UTF-8)
                 warn!(
                    "Error reading environment variable '{}': {}. Using API keys from config file for group '{}'.",
                    env_var_name, e, group.name
                );
            }
        }
    }
    // --- End of environment variable override ---


    Ok(config)
}
