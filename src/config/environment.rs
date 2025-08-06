//! Environment-based configuration management

use serde::{Deserialize, Serialize};
use std::env;

/// Environment configuration that can override file-based config
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentConfig {
    pub server_host: Option<String>,
    pub server_port: Option<u16>,
    pub redis_url: Option<String>,
    pub log_level: Option<String>,
    pub admin_token: Option<String>,
    pub max_request_size: Option<usize>,
    pub request_timeout: Option<u64>,
}

impl EnvironmentConfig {
    /// Load configuration from environment variables
    pub fn from_env() -> Self {
        Self {
            server_host: env::var("GEMINI_PROXY_HOST").ok(),
            server_port: env::var("GEMINI_PROXY_PORT")
                .ok()
                .and_then(|s| s.parse().ok()),
            redis_url: env::var("REDIS_URL").ok(),
            log_level: env::var("RUST_LOG").ok(),
            admin_token: env::var("GEMINI_PROXY_ADMIN_TOKEN").ok(),
            max_request_size: env::var("GEMINI_PROXY_MAX_REQUEST_SIZE")
                .ok()
                .and_then(|s| s.parse().ok()),
            request_timeout: env::var("GEMINI_PROXY_REQUEST_TIMEOUT")
                .ok()
                .and_then(|s| s.parse().ok()),
        }
    }

    /// Check if any environment overrides are present
    pub fn has_overrides(&self) -> bool {
        self.server_host.is_some()
            || self.server_port.is_some()
            || self.redis_url.is_some()
            || self.log_level.is_some()
            || self.admin_token.is_some()
            || self.max_request_size.is_some()
            || self.request_timeout.is_some()
    }

    /// Get a summary of active environment overrides
    pub fn override_summary(&self) -> Vec<String> {
        let mut overrides = Vec::new();
        
        if self.server_host.is_some() {
            overrides.push("GEMINI_PROXY_HOST".to_string());
        }
        if self.server_port.is_some() {
            overrides.push("GEMINI_PROXY_PORT".to_string());
        }
        if self.redis_url.is_some() {
            overrides.push("REDIS_URL".to_string());
        }
        if self.log_level.is_some() {
            overrides.push("RUST_LOG".to_string());
        }
        if self.admin_token.is_some() {
            overrides.push("GEMINI_PROXY_ADMIN_TOKEN".to_string());
        }
        if self.max_request_size.is_some() {
            overrides.push("GEMINI_PROXY_MAX_REQUEST_SIZE".to_string());
        }
        if self.request_timeout.is_some() {
            overrides.push("GEMINI_PROXY_REQUEST_TIMEOUT".to_string());
        }
        
        overrides
    }
}

/// Load API keys from environment variables
/// Supports patterns like GEMINI_API_KEY_1, GEMINI_API_KEY_2, etc.
pub fn load_api_keys_from_env() -> Vec<String> {
    let mut keys = Vec::new();
    let mut index = 1;

    // Try numbered keys first
    loop {
        let key_name = format!("GEMINI_API_KEY_{}", index);
        if let Ok(key) = env::var(&key_name) {
            if !key.trim().is_empty() {
                keys.push(key.trim().to_string());
            }
            index += 1;
        } else {
            break;
        }
    }

    // Also check for a single GEMINI_API_KEY
    if keys.is_empty() {
        if let Ok(key) = env::var("GEMINI_API_KEY") {
            if !key.trim().is_empty() {
                keys.push(key.trim().to_string());
            }
        }
    }

    // Check for comma-separated keys in GEMINI_API_KEYS
    if keys.is_empty() {
        if let Ok(keys_str) = env::var("GEMINI_API_KEYS") {
            keys.extend(
                keys_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            );
        }
    }

    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_environment_config_empty() {
        // Clear relevant env vars
        let vars_to_clear = [
            "GEMINI_PROXY_HOST",
            "GEMINI_PROXY_PORT", 
            "REDIS_URL",
            "RUST_LOG",
            "GEMINI_PROXY_ADMIN_TOKEN",
        ];
        
        for var in &vars_to_clear {
            env::remove_var(var);
        }

        let config = EnvironmentConfig::from_env();
        assert!(!config.has_overrides());
        assert!(config.override_summary().is_empty());
    }

    #[test]
    fn test_environment_config_with_values() {
        env::set_var("GEMINI_PROXY_HOST", "127.0.0.1");
        env::set_var("GEMINI_PROXY_PORT", "8080");
        env::set_var("REDIS_URL", "redis://localhost:6379");

        let config = EnvironmentConfig::from_env();
        assert!(config.has_overrides());
        assert_eq!(config.server_host, Some("127.0.0.1".to_string()));
        assert_eq!(config.server_port, Some(8080));
        assert_eq!(config.redis_url, Some("redis://localhost:6379".to_string()));

        let overrides = config.override_summary();
        assert!(overrides.contains(&"GEMINI_PROXY_HOST".to_string()));
        assert!(overrides.contains(&"GEMINI_PROXY_PORT".to_string()));
        assert!(overrides.contains(&"REDIS_URL".to_string()));

        // Cleanup
        env::remove_var("GEMINI_PROXY_HOST");
        env::remove_var("GEMINI_PROXY_PORT");
        env::remove_var("REDIS_URL");
    }

    #[test]
    fn test_load_api_keys_numbered() {
        env::set_var("GEMINI_API_KEY_1", "key1");
        env::set_var("GEMINI_API_KEY_2", "key2");
        env::set_var("GEMINI_API_KEY_3", "key3");

        let keys = load_api_keys_from_env();
        assert_eq!(keys, vec!["key1", "key2", "key3"]);

        // Cleanup
        env::remove_var("GEMINI_API_KEY_1");
        env::remove_var("GEMINI_API_KEY_2");
        env::remove_var("GEMINI_API_KEY_3");
    }

    #[test]
    fn test_load_api_keys_single() {
        env::remove_var("GEMINI_API_KEY_1");
        env::set_var("GEMINI_API_KEY", "single_key");

        let keys = load_api_keys_from_env();
        assert_eq!(keys, vec!["single_key"]);

        env::remove_var("GEMINI_API_KEY");
    }

    #[test]
    fn test_load_api_keys_comma_separated() {
        env::remove_var("GEMINI_API_KEY_1");
        env::remove_var("GEMINI_API_KEY");
        env::set_var("GEMINI_API_KEYS", "key1,key2,key3");

        let keys = load_api_keys_from_env();
        assert_eq!(keys, vec!["key1", "key2", "key3"]);

        env::remove_var("GEMINI_API_KEYS");
    }
}