// src/config/app.rs

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Clone, PartialEq, Serialize)]
pub struct KeyGroup {
    pub name: String,
    #[serde(default)]
    pub api_keys: Vec<String>,
    #[serde(default)]
    pub model_aliases: Vec<String>,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default = "default_target_url")]
    pub target_url: String,
    #[serde(default)]
    pub top_p: Option<f32>,
}

impl Default for KeyGroup {
    fn default() -> Self {
        Self {
            name: String::new(),
            api_keys: Vec::new(),
            model_aliases: Vec::new(),
            proxy_url: None,
            target_url: default_target_url(),
            top_p: None,
        }
    }
}

#[derive(Debug, Deserialize, Clone, PartialEq, Serialize)]
pub struct ServerConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: u64,
    #[serde(default = "default_request_timeout")]
    pub request_timeout_secs: u64,
    #[serde(default)]
    pub test_mode: bool,
    #[serde(default)]
    pub admin_token: Option<String>,
    #[serde(default)]
    pub top_p: Option<f32>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            connect_timeout_secs: default_connect_timeout(),
            request_timeout_secs: default_request_timeout(),
            test_mode: false,
            admin_token: None,
            top_p: None,
        }
    }
}

#[derive(Debug, Deserialize, Clone, PartialEq, Default, Serialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub groups: Vec<KeyGroup>,
    #[serde(default)]
    pub redis_url: Option<String>,
    #[serde(default)]
    pub redis_key_prefix: Option<String>,
    #[serde(default)]
    pub max_failures_threshold: Option<u32>,
    #[serde(default)]
    pub rate_limit: Option<RateLimitConfig>,
    #[serde(default)]
    pub circuit_breaker: Option<CircuitBreakerConfig>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub internal_retries: Option<u32>,
    #[serde(default)]
    pub temporary_block_minutes: Option<u32>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Serialize)]
pub struct RateLimitConfig {
    pub requests_per_minute: u32,
    pub burst_size: u32,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Serialize)]
pub struct CircuitBreakerConfig {
    pub failure_threshold: u32,
    pub recovery_timeout_secs: u64,
    pub half_open_max_calls: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            recovery_timeout_secs: 60,
            half_open_max_calls: 3,
        }
    }
}

// Default value functions
fn default_target_url() -> String {
    "https://generativelanguage.googleapis.com".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_connect_timeout() -> u64 {
    10
}

fn default_request_timeout() -> u64 {
    60
}

impl AppConfig {
    /// Get the group name for a given model
    pub fn get_group_for_model(&self, model: &str) -> Option<&str> {
        self.groups
            .iter()
            .find(|group| group.model_aliases.contains(&model.to_string()))
            .map(|group| group.name.as_str())
    }
}
