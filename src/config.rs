// src/config.rs
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    env, fs, io,
    path::Path,
};
use tracing::{debug, error, info, warn};
use url::Url;

use crate::error::{AppError, Result};

// --- Constants ---

const ENV_VAR_PREFIX: &str = "GEMINI_PROXY_GROUP_";
const API_KEYS_SUFFIX: &str = "_API_KEYS";
const PROXY_URL_SUFFIX: &str = "_PROXY_URL";
const TARGET_URL_SUFFIX: &str = "_TARGET_URL";

// --- Data Structures ---

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeyGroup {
    pub name: String,
    #[serde(default)]
    pub api_keys: Vec<String>,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default = "default_target_url")]
    pub target_url: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RateLimitBehavior {
    BlockUntilMidnight,
    #[default]
    RetryNextKey,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub groups: Vec<KeyGroup>,
    #[serde(default)]
    pub rate_limit_behavior: RateLimitBehavior,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default = "default_server_host")]
    pub host: String,
    #[serde(default = "default_server_port")]
    pub port: u16,
    #[serde(default = "default_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
    #[serde(default = "default_cache_max_size")]
    pub cache_max_size: usize,
}

#[derive(Default, Debug)]
struct EnvGroupData {
    // Store the first original casing encountered for this group name
    original_name: Option<String>, // Keep this to ensure final KeyGroup uses the *actual* name from env
    api_keys: Option<Vec<String>>,
    proxy_url: Option<String>, // Simplified: Some("url") or None
    target_url: Option<String>,
}

// --- Default Implementations ---

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_server_host(),
            port: default_server_port(),
            cache_ttl_secs: default_cache_ttl_secs(),
            cache_max_size: default_cache_max_size(),
        }
    }
}
fn default_server_host() -> String {
    "0.0.0.0".to_string()
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
    "https://generativelanguage.googleapis.com".to_string()
}

// --- Helper Functions ---

fn extract_original_group_name_segment<'a>(env_key: &'a str, suffix: &'a str) -> Option<&'a str> {
    env_key
        .strip_prefix(ENV_VAR_PREFIX)?
        .strip_suffix(suffix)
}
#[tracing::instrument(level = "trace", fields(value.len = value.len()))]
fn parse_api_keys(value: &str) -> Vec<String> {
    value
        .trim()
        .split(',')
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect()
}
#[tracing::instrument(level = "trace", fields(value.len = value.len()))]
fn parse_proxy_url(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
#[tracing::instrument(level = "trace", fields(value.len = value.len()))]
fn parse_target_url(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
fn clean_target_url(url_str: &str) -> String {
    url_str.strip_suffix('/').unwrap_or(url_str).to_string()
}

// --- Configuration Loading Logic ---

#[tracing::instrument(level = "debug", skip(path), fields(config.path = %path.display()))]
fn load_yaml_defaults(path: &Path) -> Result<ServerConfig> {
    #[derive(Deserialize)]
    struct Cfg {
        #[serde(default)]
        server: ServerConfig,
    }

    let path_str = path.display().to_string();
    match fs::read_to_string(path) {
        Ok(contents) => {
            if contents.trim().is_empty() {
                warn!(src = "yaml", "Empty file");
                return Ok(ServerConfig::default());
            }

            match serde_yaml::from_str::<Cfg>(&contents) {
                Ok(cfg) => Ok(cfg.server),
                Err(e) => {
                    warn!(src = "yaml", e = %e, "Parse error");
                    Ok(ServerConfig::default())
                }
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            warn!(src = "yaml", "Not found");
            Ok(ServerConfig::default())
        }
        Err(e) => {
            error!(src = "yaml", e = %e, "Read error");
            Err(AppError::Io(io::Error::new(
                e.kind(),
                format!("Read error {path_str}: {e}"),
            )))
        }
    }
}

/// Discovers group settings from environment variables. Keys are case-sensitive original group names.
#[tracing::instrument(level = "debug")]
fn discover_env_groups() -> HashMap<String, EnvGroupData> { // Returns map keyed by ORIGINAL name
    let mut env_groups: HashMap<String, EnvGroupData> = HashMap::new();
    debug!("Discovering ENV groups (case-sensitive keys)...");

    for (env_key, value) in env::vars() {
        if !env_key.starts_with(ENV_VAR_PREFIX) {
            continue; // Skip unrelated env vars quickly
        }
        let process_suffix = |suffix: &str| -> Option<String> { // Return original name as String
            extract_original_group_name_segment(&env_key, suffix).map(ToString::to_string)
        };

        if let Some(original_name) = process_suffix(API_KEYS_SUFFIX) {
            if original_name.is_empty() { warn!(var=%env_key, "Skip group with empty name derived from ENV var"); continue; }
            let keys = parse_api_keys(&value);
            debug!(var=%env_key, group.original=%original_name, keys=keys.len(), "Discovered API keys");
            // Use original_name (case-sensitive) as the key
            let entry = env_groups.entry(original_name.clone()).or_default();
            // Store the original_name within the data as well
            if entry.original_name.is_none() { entry.original_name = Some(original_name); }
            entry.api_keys = Some(keys);
        } else if let Some(original_name) = process_suffix(PROXY_URL_SUFFIX) {
             if original_name.is_empty() { warn!(var=%env_key, "Skip group with empty name derived from ENV var"); continue; }
            let proxy = parse_proxy_url(&value);
            debug!(var=%env_key, group.original=%original_name, proxy=?proxy, "Discovered Proxy URL");
            let entry = env_groups.entry(original_name.clone()).or_default();
            if entry.original_name.is_none() { entry.original_name = Some(original_name); }
            entry.proxy_url = proxy;
        } else if let Some(original_name) = process_suffix(TARGET_URL_SUFFIX) {
             if original_name.is_empty() { warn!(var=%env_key, "Skip group with empty name derived from ENV var"); continue; }
            if let Some(target) = parse_target_url(&value) {
                debug!(var=%env_key, group.original=%original_name, target=%target, "Discovered Target URL");
                let entry = env_groups.entry(original_name.clone()).or_default();
                if entry.original_name.is_none() { entry.original_name = Some(original_name); }
                entry.target_url = Some(target);
            } else {
                warn!(var=%env_key, group.original=%original_name, "Empty target URL ignored");
            }
        }
    }

    debug!(discovered_keys=?env_groups.keys().collect::<Vec<_>>(), "Finished ENV discovery (keys are original case)");
    env_groups
}

/// Builds the final list of KeyGroups using data discovered from environment variables.
/// Expects ORIGINAL names as keys in env_groups map.
#[tracing::instrument(level = "debug", skip(env_groups))]
fn build_final_groups(env_groups: HashMap<String, EnvGroupData>) -> Vec<KeyGroup> {
    let mut final_groups: Vec<KeyGroup> = Vec::new();
    debug!("Building final groups from case-sensitive map...");

    for (original_group_name_key, data) in env_groups {
        // Use the map key (which is the original_name) directly as the group name
        let final_group_name = original_group_name_key; // No need for data.original_name here

        match data.api_keys {
            Some(api_keys) => {
                if api_keys.is_empty() {
                    warn!(group = %final_group_name, "Skipping group defined via ENV but with empty API_KEYS list.");
                    continue;
                }
                info!(group = %final_group_name, keys = api_keys.len(), "Building group details");
                let target =
                    data.target_url
                        .map_or_else(default_target_url, |u| clean_target_url(&u));

                final_groups.push(KeyGroup {
                    name: final_group_name, // Use the original case name from the key
                    api_keys,
                    proxy_url: data.proxy_url,
                    target_url: target,
                });
            }
            None => {
                // Only warn if other settings were provided for this group name
                if data.proxy_url.is_some() || data.target_url.is_some() {
                    warn!(group = %final_group_name, "Orphaned proxy/target URL settings found via ENV for group without API_KEYS defined.");
                }
                // Otherwise, just skip silently
            }
        }
    }

    debug!(count = final_groups.len(), "Finished building groups");
    final_groups
}

// --- Configuration Validation Functions ---
fn validate_server_config(server: &ServerConfig) -> bool {
    if server.host.trim().is_empty() || server.port == 0 {
        error!(h = %server.host, p = server.port, e = "bad server");
        false
    } else {
        true
    }
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
fn validate_config(cfg: &AppConfig, source: &str) -> bool {
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
        for group in &cfg.groups {
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
            if !validate_target_url(&group.name, &group.target_url) {
                errors += 1;
            }
            if let Some(p) = &group.proxy_url {
                if !validate_proxy_url(&group.name, p) {
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
/// Loads the application configuration from a YAML file and environment variables.
///
/// Environment variables take precedence over YAML file settings.
///
/// # Arguments
/// * `path` - The path to the configuration YAML file.
///
/// # Errors
/// Returns an `AppError::Config` if:
/// - The YAML file cannot be read or parsed.
/// - Environment variables are malformed or conflict.
/// - Validation of the final configuration fails.
/// - Any I/O error occurs during file reading.
pub fn load_config(path: &Path) -> Result<AppConfig> {
    let path_str = path.display().to_string();
    let server_config = load_yaml_defaults(path)?;
    let env_groups = discover_env_groups(); // Now uses ORIGINAL keys
    let discovered_count = env_groups.len(); // Get count before move
    debug!(
        discovered_env_group_count = discovered_count,
        "Groups discovered from environment"
    );
    let final_groups = build_final_groups(env_groups); // Expects ORIGINAL keys
    let built_count = final_groups.len(); // Get count before move
    debug!(
        built_group_count = built_count,
        "Groups built before validation"
    );
    let final_config = AppConfig {
        server: server_config,
        groups: final_groups,
        rate_limit_behavior: RateLimitBehavior::default(),
    };
    if !validate_config(&final_config, &path_str) {
        error!(config.source = %path_str, "Validation failed.");
        return Err(AppError::Config("Validation failed".to_string()));
    }
    // No need for redundant total_keys check here, validate_config handles it
    info!(
        groups = final_config.groups.len(),
        keys_total = final_config
            .groups
            .iter()
            .map(|g| g.api_keys.len())
            .sum::<usize>(),
        "Config loaded OK."
    );
    Ok(final_config)
}

// --- Tests ---
#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
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
    fn delete_file(p: &PathBuf) {
        let _ = std::fs::remove_file(p);
    }
    fn set_env(k: &str, v: &str) {
        std::env::set_var(k, v);
    }
    fn remove_env(k: &str) {
        std::env::remove_var(k);
    }
    fn clean_env() {
        debug!(
            "Cleaning environment variables starting with '{}'",
            ENV_VAR_PREFIX
        );
        let vars_to_remove: Vec<String> = std::env::vars()
            .filter(|(k, _)| k.starts_with(ENV_VAR_PREFIX))
            .map(|(k, _)| k)
            .collect();
        for k in vars_to_remove {
            debug!(env_var = %k, "Removing environment variable");
            std::env::remove_var(&k);
        }
        // Also remove the specific ones mentioned before, just in case they don't match the prefix exactly
        // (though they should match based on current const definition)
        remove_env("gemini_proxy_group_duplicate_api_keys"); // Example from previous code
        remove_env("gemini_proxy_group_mixedcase_api_keys"); // Example from previous code
    }
    #[test]
    fn test_extract_orig() {
        assert_eq!(
            extract_original_group_name_segment("GEMINI_PROXY_GROUP_MY_API_KEYS", "_API_KEYS"),
            Some("MY")
        );
    }
    #[test]
    fn test_extract_fail() {
        assert_eq!(extract_original_group_name_segment("X", "_"), None);
    }
    #[test]
    fn test_parse_keys() {
        assert_eq!(
            parse_api_keys("a, b"),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(parse_api_keys(" ,, "), Vec::<String>::new());
    } // Added .to_string()
    #[test]
    fn test_parse_proxy() {
        assert_eq!(parse_proxy_url(" h://p "), Some("h://p".to_string()));
        assert_eq!(parse_proxy_url(""), None);
    }
    #[test]
    fn test_parse_target() {
        assert_eq!(parse_target_url(" h://t "), Some("h://t".to_string()));
        assert_eq!(parse_target_url(""), None);
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
            host: " ".into(),
            port: 1,
            cache_ttl_secs: 300,
            cache_max_size: 100,
        }));
        assert!(!validate_server_config(&ServerConfig {
            host: "h".into(),
            port: 0,
            cache_ttl_secs: 300,
            cache_max_size: 100,
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
        let cfg = AppConfig {
            groups: vec![KeyGroup {
                name: "G".into(),
                api_keys: vec!["k".into()],
                proxy_url: None,
                target_url: default_target_url(),
            }],
            ..Default::default()
        };
        assert!(validate_config(&cfg, ""));
    }
    #[test]
    fn test_val_cfg_bad_name() {
        let _ = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::WARN)
                .finish(),
        );
        let cfg = AppConfig {
            groups: vec![KeyGroup {
                name: "".into(),
                api_keys: vec!["k".into()],
                proxy_url: None,
                target_url: default_target_url(),
            }],
            ..Default::default()
        };
        assert!(!validate_config(&cfg, ""));
    }
    #[test]
    fn test_val_cfg_dupe_name() {
        let _ = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::WARN)
                .finish(),
        );
        let cfg = AppConfig {
            groups: vec![
                KeyGroup {
                    name: "N".into(),
                    api_keys: vec!["k".into()],
                    proxy_url: None,
                    target_url: default_target_url(),
                },
                KeyGroup {
                    name: "n".into(),
                    api_keys: vec!["k".into()],
                    proxy_url: None,
                    target_url: default_target_url(),
                },
            ],
            ..Default::default()
        };
        assert!(!validate_config(&cfg, ""));
    }
    #[test]
    fn test_val_cfg_empty_keys_ok() {
        let cfg = AppConfig {
            groups: vec![KeyGroup {
                name: "G".into(),
                api_keys: vec!["k1".to_string(), "k3".to_string()],
                proxy_url: None,
                target_url: default_target_url(),
            }],
            ..Default::default()
        };
        assert!(validate_config(&cfg, ""));
    }
    #[test]
    fn test_val_cfg_no_keys() {
        let _ = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::WARN)
                .finish(),
        );
        let cfg = AppConfig {
            groups: vec![KeyGroup {
                name: "G".into(),
                api_keys: vec![],
                proxy_url: None,
                target_url: default_target_url(),
            }],
            ..Default::default()
        };
        assert!(!validate_config(&cfg, ""));
    } // Should fail validation as total keys = 0
    #[test]
    fn test_val_cfg_no_groups() {
        let _ = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::WARN)
                .finish(),
        );
        let cfg = AppConfig {
            groups: vec![],
            ..Default::default()
        };
        assert!(!validate_config(&cfg, ""));
    }
    #[test]
    #[serial]
    fn test_load_env_ok() {
        clean_env();
        let d = tempdir().unwrap();
        let p = d.path().join("_.yaml");
        delete_file(&p);
        set_env("GEMINI_PROXY_GROUP_DEFAULT_API_KEYS", "k1");
        set_env("GEMINI_PROXY_GROUP_G1_API_KEYS", "k2");
        set_env("GEMINI_PROXY_GROUP_G1_PROXY_URL", "http://p");
        let cfg = load_config(&p).expect("!");
        assert_eq!(cfg.groups.len(), 2);
        clean_env();
    } // Uses load_config now
    #[test]
    #[serial]
    fn test_load_yaml_env() {
        clean_env();
        let d = tempdir().unwrap();
        let p = create_temp_config_file(&d, "server:\n  port: 1\n");
        set_env("GEMINI_PROXY_GROUP_D_API_KEYS", " k ");
        let cfg = load_config(&p).expect("!");
        assert_eq!(cfg.server.port, 1);
        assert_eq!(cfg.groups.len(), 1);
        assert_eq!(cfg.groups[0].api_keys, vec!["k".to_string()]);
        clean_env();
    } // Added .to_string()
      // Removed test_env_var_case_insensitivity_for_discovery
    #[test]
    #[serial]
    fn test_load_no_groups() {
        clean_env();
        let d = tempdir().unwrap();
        let p = create_temp_config_file(&d, "");
        let r = load_config(&p);
        assert!(
            r.is_err() && matches!(r.err(), Some(AppError::Config(m)) if m == "Validation failed")
        );
        clean_env();
    }
    #[test]
    #[serial]
    fn test_load_no_keys() {
        clean_env();
        let d = tempdir().unwrap();
        let p = d.path().join("_.yaml");
        delete_file(&p);
        set_env("GEMINI_PROXY_GROUP_A_API_KEYS", "");
        let r = load_config(&p);
        assert!(
            r.is_err() && matches!(r.err(), Some(AppError::Config(m)) if m == "Validation failed")
        );
        clean_env();
    }
    #[test]
    #[serial]
    fn test_val_bad_proxy_env() {
        clean_env();
        let d = tempdir().unwrap();
        let p = d.path().join("_.yaml");
        delete_file(&p);
        set_env("GEMINI_PROXY_GROUP_A_API_KEYS", "k");
        set_env("GEMINI_PROXY_GROUP_A_PROXY_URL", ":b");
        let r = load_config(&p);
        assert!(
            r.is_err() && matches!(r.err(), Some(AppError::Config(m)) if m == "Validation failed")
        );
        clean_env();
    }
    #[test]
    #[serial]
    fn test_val_bad_scheme_env() {
        clean_env();
        let d = tempdir().unwrap();
        let p = d.path().join("_.yaml");
        delete_file(&p);
        set_env("GEMINI_PROXY_GROUP_A_API_KEYS", "k");
        set_env("GEMINI_PROXY_GROUP_A_PROXY_URL", "ftp://p");
        let r = load_config(&p);
        assert!(
            r.is_err() && matches!(r.err(), Some(AppError::Config(m)) if m == "Validation failed")
        );
        clean_env();
    }
    #[test]
    #[serial]
    fn test_val_bad_target_env() {
        clean_env();
        let d = tempdir().unwrap();
        let p = d.path().join("_.yaml");
        delete_file(&p);
        set_env("GEMINI_PROXY_GROUP_A_API_KEYS", "k");
        set_env("GEMINI_PROXY_GROUP_A_TARGET_URL", ":b");
        let r = load_config(&p);
        assert!(
            r.is_err() && matches!(r.err(), Some(AppError::Config(m)) if m == "Validation failed")
        );
        clean_env();
    }
    #[test]
    #[serial]
    fn test_val_empty_name_env() {
        clean_env();
        let d = tempdir().unwrap();
        let p = d.path().join("_.yaml");
        delete_file(&p);
        set_env("GEMINI_PROXY_GROUP__API_KEYS", "k");
        let r = load_config(&p);
        assert!(
            r.is_err() && matches!(r.err(), Some(AppError::Config(m)) if m == "Validation failed")
        );
        clean_env();
    }
    #[test]
    #[serial]
    fn test_validation_fails_on_duplicate_group_name() {
        clean_env();
        let d = tempdir().unwrap();
        let p = d.path().join("_.yaml");
        delete_file(&p);
        set_env("GEMINI_PROXY_GROUP_A_API_KEYS", "k");
        set_env("GEMINI_PROXY_GROUP_a_API_KEYS", "k");
        let r = load_config(&p);
        assert!(
            r.is_err() && matches!(r.err(), Some(AppError::Config(m)) if m == "Validation failed")
        );
        clean_env();
    }
    #[test]
    #[serial]
    fn test_load_empty_key_ok() {
        clean_env();
        let d = tempdir().unwrap();
        let p = d.path().join("_.yaml");
        delete_file(&p);
        set_env("GEMINI_PROXY_GROUP_DEF_API_KEYS", "k1,,k3");
        let result = load_config(&p);
        assert!(result.is_ok(), "Got: {:?}", result.err());
        let cfg = result.unwrap();
        assert_eq!(
            cfg.groups[0].api_keys,
            vec!["k1".to_string(), "k3".to_string()]
        );
        clean_env();
    } // Added .to_string()
    #[test]
    #[serial]
    fn test_target_path_ok() {
        clean_env();
        let d = tempdir().unwrap();
        let p = d.path().join("_");
        delete_file(&p);
        set_env("GEMINI_PROXY_GROUP_A_API_KEYS", "k");
        set_env("GEMINI_PROXY_GROUP_A_TARGET_URL", "https://a.com/p");
        let result = load_config(&p);
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().groups[0].target_url,
            "https://a.com/p"
        );
        clean_env();
    }
    #[test]
    #[serial]
    fn test_warn_orphan() {
        clean_env();
        let d = tempdir().unwrap();
        let p = d.path().join("_");
        delete_file(&p);
        set_env("GEMINI_PROXY_GROUP_O_PURL", "http://p");
        set_env("GEMINI_PROXY_GROUP_O_TURL", "http://t");
        let result = load_config(&p);
        assert!(
            result.is_err()
                && matches!(result.err(), Some(AppError::Config(m)) if m == "Validation failed")
        );
        clean_env();
    }
    #[test]
    #[serial]
    fn test_empty_proxy_none() {
        clean_env();
        let d = tempdir().unwrap();
        let p = d.path().join("_");
        delete_file(&p);
        set_env("GEMINI_PROXY_GROUP_A_API_KEYS", "k");
        set_env("GEMINI_PROXY_GROUP_A_PROXY_URL", " ");
        let cfg = load_config(&p).expect("!");
        assert!(cfg.groups[0].proxy_url.is_none());
        clean_env();
    }
    #[test]
    #[serial]
    fn test_empty_target_default() {
        clean_env();
        let d = tempdir().unwrap();
        let p = d.path().join("_");
        delete_file(&p);
        set_env("GEMINI_PROXY_GROUP_A_API_KEYS", "k");
        set_env("GEMINI_PROXY_GROUP_A_TARGET_URL", " ");
        let cfg = load_config(&p).expect("!");
        assert_eq!(cfg.groups[0].target_url, default_target_url());
        clean_env();
    }
} // end tests module
