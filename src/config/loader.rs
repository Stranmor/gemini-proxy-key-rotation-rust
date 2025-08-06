// src/config/loader.rs

use crate::config::{AppConfig, ConfigValidator};
use crate::error::{AppError, Result};
use std::path::Path;
use tracing::{debug, info, warn};

/// Load configuration from file or environment variables
pub fn load_config(config_path: &Path) -> Result<AppConfig> {
    let mut config = if config_path.exists() {
        info!("Loading configuration from file: {}", config_path.display());
        load_from_file(config_path)?
    } else {
        info!("Configuration file not found, using defaults");
        AppConfig::default()
    };
    
    // Override with environment variables
    override_with_env(&mut config);
    
    // Validate the final configuration
    ConfigValidator::validate(&config)?;
    
    debug!("Configuration loaded and validated successfully");
    Ok(config)
}

fn load_from_file(config_path: &Path) -> Result<AppConfig> {
    let content = std::fs::read_to_string(config_path)
        .map_err(|e| AppError::ConfigNotFound { path: config_path.display().to_string() })?;
    
    serde_yaml::from_str(&content)
        .map_err(|e| AppError::ConfigParse { message: format!("Failed to parse config file: {}", e), line: e.location().map(|loc| loc.line()) })
}

fn override_with_env(config: &mut AppConfig) {
    // Override Redis URL from environment
    if let Ok(redis_url) = std::env::var("REDIS_URL") {
        info!("Overriding Redis URL from environment variable");
        config.redis_url = Some(redis_url);
    }
    
    // Override server port from environment
    if let Ok(port_str) = std::env::var("PORT") {
        if let Ok(port) = port_str.parse::<u16>() {
            info!("Overriding server port from environment variable: {}", port);
            config.server.port = port;
        } else {
            warn!("Invalid PORT environment variable: {}", port_str);
        }
    }
    
    // Override max failures threshold
    if let Ok(threshold_str) = std::env::var("MAX_FAILURES_THRESHOLD") {
        if let Ok(threshold) = threshold_str.parse::<u32>() {
            info!("Overriding max failures threshold from environment: {}", threshold);
            config.max_failures_threshold = Some(threshold);
        } else {
            warn!("Invalid MAX_FAILURES_THRESHOLD environment variable: {}", threshold_str);
        }
    }
}

/// Save configuration to file (for admin interface)
pub async fn save_config(config: &AppConfig, config_path: &Path) -> Result<()> {
    let yaml_content = serde_yaml::to_string(config)
        .map_err(|e| AppError::Serialization { message: format!("Failed to serialize config: {}", e) })?;
    
    tokio::fs::write(config_path, yaml_content)
        .await
        .map_err(|e| AppError::Io { operation: "write_config".to_string(), message: format!("Failed to write config file: {}", e) })?;
    
    info!("Configuration saved to: {}", config_path.display());
    Ok(())
}

/// Validate configuration (for admin interface)
pub fn validate_config(config: &mut AppConfig, source: &str) -> bool {
    match ConfigValidator::validate(config) {
        Ok(()) => {
            debug!("Configuration validation passed for source: {}", source);
            true
        }
        Err(e) => {
            warn!("Configuration validation failed for source '{}': {}", source, e);
            // Log more details about the config
            debug!("Failed config groups: {:?}", config.groups.iter().map(|g| (&g.name, g.api_keys.len())).collect::<Vec<_>>());
            false
        }
    }
}