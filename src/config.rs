use serde::Deserialize;
use std::{fs::File, io::Read, path::Path};
use thiserror::Error;

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub proxies: Vec<ProxyConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProxyConfig {
    pub name: String,
    pub listen_address: String,
    pub target_url: String,
    pub api_keys: Vec<String>,
    #[serde(default = "default_key_parameter_name")]
    pub key_parameter_name: String,
}

fn default_key_parameter_name() -> String {
    "key".to_string()
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to open config file: {0}")]
    FileOpen(#[from] std::io::Error),
    #[error("Failed to read config file: {0}")]
    FileRead(std::io::Error),
    #[error("Failed to parse config file: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error("Validation error: {0}")]
    Validation(String),
}

pub fn load_config(path: &Path) -> Result<AppConfig, ConfigError> {
    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(ConfigError::FileRead)?;

    let config: AppConfig = serde_yaml::from_str(&contents)?;

    for proxy in &config.proxies {
        if proxy.api_keys.is_empty() {
            return Err(ConfigError::Validation(format!(
                "Proxy '{}' must have at least one API key.",
                proxy.name
            )));
        }
        if url::Url::parse(&proxy.target_url).is_err() {
            return Err(ConfigError::Validation(format!(
                "Proxy '{}' has an invalid target_url: {}",
                proxy.name, proxy.target_url
            )));
        }
    }

    Ok(config)
}