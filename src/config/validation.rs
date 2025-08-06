// src/config/validation.rs

use crate::config::AppConfig;
use crate::error::{AppError, Result};
use std::collections::HashSet;
use tracing::{debug, warn};
use url::Url;

pub struct ConfigValidator;

impl ConfigValidator {
    pub fn validate(config: &AppConfig) -> Result<()> {
        debug!("Starting configuration validation");
        
        if let Err(e) = Self::validate_groups(config) {
            warn!("Group validation failed: {}", e);
            return Err(e);
        }
        debug!("Group validation passed");
        
        if let Err(e) = Self::validate_redis_config(config) {
            warn!("Redis config validation failed: {}", e);
            return Err(e);
        }
        debug!("Redis config validation passed");
        
        if let Err(e) = Self::validate_server_config(config) {
            warn!("Server config validation failed: {}", e);
            return Err(e);
        }
        debug!("Server config validation passed");
        
        debug!("Configuration validation completed successfully");
        Ok(())
    }
    
    fn validate_groups(config: &AppConfig) -> Result<()> {
        debug!("Validating {} groups", config.groups.len());
        
        if config.groups.is_empty() {
            return Err(AppError::config_validation(
                "At least one key group must be configured",
                Some("groups"),
            ));
        }
        
        let mut group_names = HashSet::new();
        let mut all_keys = HashSet::new();
        
        for group in &config.groups {
            // Check for duplicate group names
            if !group_names.insert(&group.name) {
                return Err(AppError::config_validation(
                    format!("Duplicate group name: {}", group.name),
                    Some("group.name"),
                ));
            }
            
            // Validate group has keys
            if group.api_keys.is_empty() {
                warn!("Group '{}' has no API keys configured", group.name);
            }
            
            // Check for duplicate keys across groups
            for key in &group.api_keys {
                if !all_keys.insert(key) {
                    return Err(AppError::config_validation(
                        format!("Duplicate API key found across groups: {}",
                                Self::preview_key(key)),
                        Some("group.api_keys"),
                    ));
                }
            }
            
            // Validate target URL
            debug!("Validating target URL for group '{}': {}", group.name, group.target_url);
            Self::validate_url(&group.target_url, "target_url")?;
            
            // Validate proxy URL if present
            if let Some(proxy_url) = &group.proxy_url {
                Self::validate_proxy_url(&group.name, proxy_url)?;
            }
        }
        
        debug!("Validated {} groups with {} total keys", 
               config.groups.len(), all_keys.len());
        Ok(())
    }
    
    fn validate_redis_config(config: &AppConfig) -> Result<()> {
        if let Some(redis_url) = &config.redis_url {
            Self::validate_url(redis_url, "redis_url")?;
        }
        Ok(())
    }
    
    fn validate_server_config(config: &AppConfig) -> Result<()> {
        // Allow port 0 in test mode (system will assign a free port)
        if config.server.port == 0 && !config.server.test_mode {
            return Err(AppError::config_validation(
                "Server port cannot be 0 (except in test mode)",
                Some("server.port"),
            ));
        }
        
        if config.server.connect_timeout_secs == 0 {
            return Err(AppError::config_validation(
                "Connect timeout cannot be 0",
                Some("server.connect_timeout_secs"),
            ));
        }
        
        if config.server.request_timeout_secs == 0 {
            return Err(AppError::config_validation(
                "Request timeout cannot be 0",
                Some("server.request_timeout_secs"),
            ));
        }
        
        Ok(())
    }
    
    fn validate_url(url_str: &str, field_name: &str) -> Result<()> {
        Url::parse(url_str).map_err(|e| {
            AppError::config_validation(
                format!("Invalid URL in {}: {} - {}", field_name, url_str, e),
                Some(field_name),
            )
        })?;
        Ok(())
    }
    
    fn validate_proxy_url(group_name: &str, proxy_url: &str) -> Result<()> {
        let url = Url::parse(proxy_url).map_err(|e| {
            AppError::config_validation(
                format!("Invalid proxy URL in group '{}': {} - {}",
                        group_name, proxy_url, e),
                Some("proxy"),
            )
        })?;

        match url.scheme() {
            "http" | "https" | "socks5" => Ok(()),
            scheme => Err(AppError::config_validation(
                format!("Unsupported proxy scheme '{}' in group '{}'. Supported: http, https, socks5",
                        scheme, group_name),
                Some("proxy"),
            )),
        }
    }
    
    fn preview_key(key: &str) -> String {
        if key.len() > 8 {
            format!("{}...{}", &key[..4], &key[key.len() - 4..])
        } else {
            key.to_string()
        }
    }
}