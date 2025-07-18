// src/config.rs
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, io, path::Path};
use tracing::{debug, error, info, warn};
use url::Url;

use crate::error::{AppError, Result};

// --- Data Structures ---

#[derive(Debug, Deserialize, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeyGroup {
    pub name: String,
    #[serde(default)]
    pub api_keys: Vec<String>,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default = "default_target_url")]
    pub target_url: String,
    #[serde(default)]
    pub top_p: Option<f32>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RateLimitBehavior {
    BlockUntilMidnight,
    #[default]
    RetryNextKey,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Default, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub groups: Vec<KeyGroup>,
    #[serde(default)]
    pub rate_limit_behavior: RateLimitBehavior,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default = "default_server_port")]
    pub port: u16,
    #[serde(default = "default_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
    #[serde(default = "default_cache_max_size")]
    pub cache_max_size: usize,
    #[serde(default)]
    pub top_p: Option<f32>,
}

// --- Default Implementations ---

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_server_port(),
            cache_ttl_secs: default_cache_ttl_secs(),
            cache_max_size: default_cache_max_size(),
            top_p: None,
        }
    }
}
const fn default_server_port() -> u16 {
    8080
}
const fn default_cache_ttl_secs() -> u64 {
    300 // 5 minutes
}
const fn default_cache_max_size() -> usize {
    1000 // Max 1000 entries
}
fn default_target_url() -> String {
    "https://generativelanguage.googleapis.com/".to_string()
}

// --- Helper Functions ---

fn clean_target_url(url_str: &str) -> String {
    url_str.strip_suffix('/').unwrap_or(url_str).to_string()
}

// --- Configuration Validation Functions ---
fn validate_server_config(server: &ServerConfig) -> bool {
    let mut errors = 0;
    if server.port == 0 {
        error!(p = server.port, e = "bad server port");
        errors += 1;
    }
    if let Some(tp) = server.top_p {
        if !(0.0..=1.0).contains(&tp) {
            error!(err = "server top_p out of range", top_p = tp);
            errors += 1;
        }
    }
    errors == 0
}
fn validate_target_url(g: &str, url: &str) -> bool {
    match Url::parse(url) {
        Ok(p) => {
            if !p.has_host()
                || p.host_str().is_none_or(str::is_empty)
                || p.cannot_be_a_base()
            {
                error!(group = g, err = "invalid/base", url = %url);
                false
            } else {
                true
            }
        }
        Err(e) => {
            error!(group = g, err = %e, url = %url, "target parse err");
            false
        }
    }
}
fn validate_proxy_url(g: &str, url: &str) -> bool {
    match Url::parse(url) {
        Ok(p) => {
            if !p.has_host() || p.host_str().is_none_or(str::is_empty) {
                error!(group = g, err = "no_host", url = %url);
                return false;
            }
            let s = p.scheme().to_lowercase();
            if ["http", "https", "socks5"].contains(&s.as_str()) {
                true
            } else {
                error!(group = g, err = "bad_scheme", url = %url, scheme = %s);
                false
            }
        }
        Err(e) => {
            error!(group = g, err = %e, url = %url, "proxy parse err");
            false
        }
    }
}

#[tracing::instrument(level = "debug", skip(cfg, source), fields(cfg.source = %source))]
fn validate_config(cfg: &mut AppConfig, source: &str) -> bool {
    let mut errors = 0;
    if !validate_server_config(&cfg.server) {
        errors += 1;
    }

    if cfg.groups.is_empty() {
        error!(source = source, err = "no_groups");
        errors += 1;
    } else {
        let mut names = HashSet::new();
        let mut keys_total = 0;
        for group in &mut cfg.groups {
            let name = group.name.trim();
            if name.is_empty() {
                // Check for empty name first
                error!(err = "empty_name");
                errors += 1;
            } else {
                // Only check for duplicates if name is not empty
                let upper_name = group.name.to_uppercase(); // Use uppercase for HashSet check
                debug!(group.name = %group.name, group.upper = %upper_name, ?names, "Attempting to insert into HashSet"); // Added debug before insert
                let insert_result = names.insert(upper_name.clone());
                debug!(group.name = %group.name, group.upper = %upper_name, set.insert_result = insert_result, set.current_size = names.len(), "Checking group name for duplicates");
                if !insert_result {
                    // Check duplicate result
                    error!(group = %group.name, err = "duplicate");
                    errors += 1;
                }
            }
            // Check for empty keys independently
            if group.api_keys.is_empty() {
                warn!(group = %group.name, warn = "no_keys"); /* Group without keys is warned, but not instant error */
            }
            keys_total += group.api_keys.len();

            // Clean the target URL in-place
            group.target_url = clean_target_url(&group.target_url);

            if !validate_target_url(&group.name, &group.target_url) {
                errors += 1;
            }
            if let Some(p) = &group.proxy_url {
                if !validate_proxy_url(&group.name, p) {
                    errors += 1;
                }
            }
            if let Some(tp) = group.top_p {
                if !(0.0..=1.0).contains(&tp) {
                    error!(group = %group.name, err = "top_p_out_of_range", top_p = tp);
                    errors += 1;
                }
            }
        }
        // Error only if total keys across all groups is zero (ignoring groups that might have been defined but had no keys)
        if keys_total == 0 {
            error!(err = "no_usable_keys");
            errors += 1;
        }
    }
    if errors > 0 {
        error!(count = errors, "Validation finished: ERRORS.");
        false
    } else {
        debug!("Validation OK.");
        true
    }
}

// --- Main Loading Function ---
#[tracing::instrument(level = "info", skip(path), fields(config.path = %path.display()))]
/// Loads the application configuration from a YAML file.
///
/// # Arguments
/// * `path` - The path to the configuration YAML file.
///
/// # Errors
/// Returns an `AppError::Config` if:
/// - The YAML file cannot be read or parsed.
/// - Validation of the final configuration fails.
/// - Any I/O error occurs during file reading.
pub fn load_config(path: &Path) -> Result<AppConfig> {
    let path_str = path.display().to_string();
    let contents = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            error!(src = "yaml", path = %path_str, "Config file not found.");
            return Err(AppError::Config(format!(
                "Config file not found: {path_str}"
            )));
        }
        Err(e) => {
            error!(src = "yaml", e = %e, "Read error");
            return Err(AppError::Io(io::Error::new(
                e.kind(),
                format!("Read error {path_str}: {e}"),
            )));
        }
    };

    if contents.trim().is_empty() {
        warn!(src = "yaml", "Config file is empty.");
        return Err(AppError::Config(format!(
            "Config file is empty: {path_str}"
        )));
    }

    let mut config: AppConfig = match serde_yaml::from_str(&contents) {
        Ok(cfg) => cfg,
        Err(e) => {
            error!(src = "yaml", e = %e, "Parse error");
            return Err(AppError::Config(format!(
                "Failed to parse config file {path_str}: {e}"
            )));
        }
    };

    if !validate_config(&mut config, &path_str) {
        error!(config.source = %path_str, "Validation failed.");
        return Err(AppError::Config("Validation failed".to_string()));
    }

    info!(
        groups = config.groups.len(),
        keys_total = config
            .groups
            .iter()
            .map(|g| g.api_keys.len())
            .sum::<usize>(),
        "Config loaded OK."
    );
    Ok(config)
}

// --- Tests ---
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::tempdir;
    fn create_temp_config_file(d: &tempfile::TempDir, c: &str) -> PathBuf {
        let p = d.path().join("t.yaml");
        File::create(&p)
            .unwrap()
            .write_all(c.as_bytes())
            .unwrap();
        p
    }
    #[test]
    fn test_clean_url() {
        assert_eq!(clean_target_url("h://a/p/"), "h://a/p");
        assert_eq!(clean_target_url("h://a/p"), "h://a/p");
    }
    #[test]
    fn test_val_srv() {
        assert!(validate_server_config(&ServerConfig::default()));
        assert!(!validate_server_config(&ServerConfig {
            port: 0,
            cache_ttl_secs: 300,
            cache_max_size: 100,
            top_p: None,
        }));
    }
    #[test]
    fn test_val_target_ok() {
        assert!(validate_target_url("g", "https://e.com"));
    }
    #[test]
    fn test_val_target_bad() {
        let _ = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::WARN)
                .finish(),
        );
        assert!(!validate_target_url("g", ":b"));
        assert!(!validate_target_url("g", "e.com"));
        assert!(!validate_target_url("g", "https://"));
        assert!(!validate_target_url("g", ""));
        assert!(!validate_target_url("g", "http://"));
    }
    #[test]
    fn test_val_proxy_ok() {
        assert!(validate_proxy_url("g", "http://p"));
        assert!(validate_proxy_url("g", "socks5://p"));
    }
    #[test]
    fn test_val_proxy_bad() {
        let _ = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::WARN)
                .finish(),
        );
        assert!(!validate_proxy_url("g", ":b"));
        assert!(!validate_proxy_url("g", "ftp://p"));
        assert!(!validate_proxy_url("g", "http://"));
        assert!(!validate_proxy_url("g", "socks5://"));
        assert!(!validate_proxy_url("g", ""));
    }
    #[test]
    fn test_val_cfg_ok() {
        let mut cfg = AppConfig {
            groups: vec![KeyGroup {
                name: "G".into(),
                api_keys: vec!["k".into()],
                proxy_url: None,
                target_url: default_target_url(),
                top_p: None,
            }],
            ..Default::default()
        };
        assert!(validate_config(&mut cfg, ""));
    }
    #[test]
    fn test_val_cfg_bad_name() {
        let _ = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::WARN)
                .finish(),
        );
        let mut cfg = AppConfig {
            groups: vec![KeyGroup {
                name: "".into(),
                api_keys: vec!["k".into()],
                proxy_url: None,
                target_url: default_target_url(),
                top_p: None,
            }],
            ..Default::default()
        };
        assert!(!validate_config(&mut cfg, ""));
    }
    #[test]
    fn test_val_cfg_dupe_name() {
        let _ = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::WARN)
                .finish(),
        );
        let mut cfg = AppConfig {
            groups: vec![
                KeyGroup {
                    name: "N".into(),
                    api_keys: vec!["k".into()],
                    proxy_url: None,
                    target_url: default_target_url(),
                    top_p: None,
                },
                KeyGroup {
                    name: "n".into(),
                    api_keys: vec!["k".into()],
                    proxy_url: None,
                    target_url: default_target_url(),
                    top_p: None,
                },
            ],
            ..Default::default()
        };
        assert!(!validate_config(&mut cfg, ""));
    }
    #[test]
    fn test_val_cfg_empty_keys_ok() {
        let mut cfg = AppConfig {
            groups: vec![KeyGroup {
                name: "G".into(),
                api_keys: vec!["k1".to_string(), "k3".to_string()],
                proxy_url: None,
                target_url: default_target_url(),
                top_p: None,
            }],
            ..Default::default()
        };
        assert!(validate_config(&mut cfg, ""));
    }
    #[test]
    fn test_val_cfg_no_keys() {
        let _ = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::WARN)
                .finish(),
        );
        let mut cfg = AppConfig {
            groups: vec![KeyGroup {
                name: "G".into(),
                api_keys: vec![],
                proxy_url: None,
                target_url: default_target_url(),
                top_p: None,
            }],
            ..Default::default()
        };
        assert!(!validate_config(&mut cfg, ""));
    } // Should fail validation as total keys = 0
    #[test]
    fn test_val_cfg_no_groups() {
        let _ = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::WARN)
                .finish(),
        );
        let mut cfg = AppConfig {
            groups: vec![],
            ..Default::default()
        };
        assert!(!validate_config(&mut cfg, ""));
    }

    #[test]
    fn test_load_no_groups_from_file() {
        let d = tempdir().unwrap();
        let p = create_temp_config_file(&d, "server:\n  port: 1\n");
        let r = load_config(&p);
        assert!(
            r.is_err() && matches!(r.err(), Some(AppError::Config(m)) if m.contains("Validation failed"))
        );
    }

    #[test]
    fn test_load_config_from_file_ok() {
        let yaml_content = r#"
server:
  port: 8081
groups:
  - name: "Group1"
    api_keys: ["key1", "key2"]
    target_url: "https://api.example.com/"
    proxy_url: "http://proxy.example.com"
    top_p: 0.9
"#;
        let d = tempdir().unwrap();
        let p = create_temp_config_file(&d, yaml_content);
        let cfg = load_config(&p).expect("Should load config successfully");

        assert_eq!(cfg.server.port, 8081);
        assert_eq!(cfg.groups.len(), 1);
        let group = &cfg.groups[0];
        assert_eq!(group.name, "Group1");
        assert_eq!(group.api_keys, vec!["key1", "key2"]);
        assert_eq!(group.target_url, "https://api.example.com"); // Note trailing slash is removed
        assert_eq!(group.proxy_url, Some("http://proxy.example.com".to_string()));
        assert_eq!(group.top_p, Some(0.9));
    }

    #[test]
    fn test_load_config_file_not_found() {
        let d = tempdir().unwrap();
        let p = d.path().join("non_existent_config.yaml");
        let r = load_config(&p);
        assert!(
            r.is_err() && matches!(r.err(), Some(AppError::Config(m)) if m.contains("Config file not found"))
        );
    }

    #[test]
    fn test_load_empty_config_file() {
        let d = tempdir().unwrap();
        let p = create_temp_config_file(&d, "");
        let r = load_config(&p);
        assert!(
            r.is_err() && matches!(r.err(), Some(AppError::Config(m)) if m.contains("Config file is empty"))
        );
    }

    #[test]
    fn test_load_bad_yaml() {
        let d = tempdir().unwrap();
        let p = create_temp_config_file(&d, "server: { port: 123,");
        let r = load_config(&p);
        assert!(
            r.is_err() && matches!(r.err(), Some(AppError::Config(m)) if m.contains("Failed to parse config file"))
        );
    }
} // end tests module
