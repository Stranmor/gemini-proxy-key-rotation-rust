// src/config.rs
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    env, fs, io,
    path::Path,
};
use tracing::{error, info, warn};
use url::Url;

use crate::error::{AppError, Result};

/// Represents a group of API keys with associated target URL and optional proxy settings.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct KeyGroup {
    /// Group name, derived from environment variables (e.g., DEFAULT, GROUP1).
    pub name: String,
    /// API keys, populated from `GEMINI_PROXY_GROUP_{NAME}_API_KEYS` env var.
    #[serde(default)]
    pub api_keys: Vec<String>,
    /// Optional upstream proxy URL, populated from `GEMINI_PROXY_GROUP_{NAME}_PROXY_URL` env var.
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// Target API endpoint URL, populated from `GEMINI_PROXY_GROUP_{NAME}_TARGET_URL` env var,
    /// or from `config.yaml` as a fallback, or the hardcoded default.
    #[serde(default = "default_target_url")]
    pub target_url: String,
}

/// Represents the root of the application configuration.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    /// Server configuration (host/port). Defaults can be set in config.yaml.
    #[serde(default)]
    pub server: ServerConfig,
    /// List of key groups, constructed primarily from environment variables.
    #[serde(default)]
    pub groups: Vec<KeyGroup>,
}

/// Configuration for the network address the proxy server listens on.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default = "default_server_host")]
    pub host: String,
    #[serde(default = "default_server_port")]
    pub port: u16,
}

// Default implementations
impl Default for AppConfig {
    fn default() -> Self {
        AppConfig { server: ServerConfig::default(), groups: Vec::new() }
    }
}
impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig { host: default_server_host(), port: default_server_port() }
    }
}
fn default_server_host() -> String { "0.0.0.0".to_string() }
fn default_server_port() -> u16 { 8080 }
fn default_target_url() -> String { "https://generativelanguage.googleapis.com".to_string() }

/// Helper function to sanitize group names for matching YAML keys if needed.
fn sanitize_for_matching(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
        .collect()
}

// Environment variable constants
const ENV_VAR_PREFIX: &str = "GEMINI_PROXY_GROUP_";
const API_KEYS_SUFFIX: &str = "_API_KEYS";
const PROXY_URL_SUFFIX: &str = "_PROXY_URL";
const TARGET_URL_SUFFIX: &str = "_TARGET_URL";

/// Extracts the potential group name from an environment variable key based on a suffix.
fn extract_group_name_from_env<'a>(env_key: &'a str, suffix: &str) -> Option<String> {
    env_key.strip_prefix(ENV_VAR_PREFIX)?.strip_suffix(suffix).map(|s| s.to_string())
}

/// Loads application configuration primarily from environment variables,
/// optionally using config.yaml only for default server settings or default target_urls.
pub fn load_config(path: &Path) -> Result<AppConfig> {
    let path_str = path.display().to_string();
    let mut final_config = AppConfig::default();
    let mut yaml_target_urls: HashMap<String, String> = HashMap::new();

    // --- 1. Try loading base config from YAML (optional) ---
    match fs::read_to_string(path) {
        Ok(contents) => {
            if !contents.trim().is_empty() {
                match serde_yaml::from_str::<AppConfig>(&contents) {
                    Ok(yaml_config) => {
                        info!("Loaded base server config and group target_urls from '{}'.", path_str);
                        final_config.server = yaml_config.server;
                        for group in yaml_config.groups {
                            // Store target URLs keyed by the name defined in YAML (sanitized for potential matching later if needed, though primary key is now from ENV)
                            yaml_target_urls.insert(sanitize_for_matching(&group.name), group.target_url);
                        }
                    }
                    Err(e) => warn!("Failed to parse YAML config file '{}': {}. Using defaults.", path_str, e),
                }
            } else { warn!("Config file '{}' is empty. Using defaults.", path_str); }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
             warn!("Config file '{}' not found. Using defaults.", path_str);
        }
        Err(e) => return Err(AppError::Io(io::Error::new(e.kind(), format!("Failed to read config file '{}': {}", path_str, e)))),
    }

    // --- 2. Discover groups and settings from Environment Variables ---
    // Map: Group Name (from Env Var) -> (Option<Keys>, Option<Option<ProxyURL>>, Option<TargetURL>)
    let mut env_group_data: HashMap<String, (Option<Vec<String>>, Option<Option<String>>, Option<String>)> = HashMap::new();

    for (key, value) in env::vars() {
        if let Some(group_name) = extract_group_name_from_env(&key, API_KEYS_SUFFIX) {
            let keys = value.trim().split(',').map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect::<Vec<String>>();
            env_group_data.entry(group_name).or_default().0 = Some(keys);
        } else if let Some(group_name) = extract_group_name_from_env(&key, PROXY_URL_SUFFIX) {
            let proxy_url = value.trim();
            env_group_data.entry(group_name).or_default().1 = Some(if proxy_url.is_empty() { None } else { Some(proxy_url.to_string()) });
        } else if let Some(group_name) = extract_group_name_from_env(&key, TARGET_URL_SUFFIX) {
            let target_url = value.trim();
             if !target_url.is_empty() {
                env_group_data.entry(group_name).or_default().2 = Some(target_url.to_string());
             } else {
                 warn!("Environment variable '{}' for group '{}' defining target_url is empty. It will be ignored.", key, group_name);
             }
        }
    }

    // --- 3. Construct Final Groups ---
    for (group_name, (keys_opt, proxy_opt_opt, target_opt)) in env_group_data {
        if let Some(api_keys) = keys_opt {
             if api_keys.is_empty() {
                 warn!("Group '{}' defined via env vars has no valid API keys. Skipping group.", group_name);
                 continue;
             }
             info!("Processing group '{}' from environment variables.", group_name);

            // Determine Target URL: Env Var > YAML Fallback > Default
            let target_url = target_opt.or_else(|| yaml_target_urls.get(&group_name).cloned())
                                      .unwrap_or_else(|| {
                                          info!("Using default target URL for group '{}'.", group_name);
                                          default_target_url()
                                      });

            let proxy_url = match proxy_opt_opt {
                Some(p_opt) => p_opt,
                None => None, // Default is no proxy if env var wasn't set
            };

            final_config.groups.push(KeyGroup {
                name: group_name.clone(),
                api_keys,
                proxy_url,
                target_url,
            });
        } else if proxy_opt_opt.is_some() || target_opt.is_some() {
             warn!("Proxy or Target URL variables found for group '{}', but no corresponding API key variable ('{}{}{}'). Group not created.", group_name, ENV_VAR_PREFIX, group_name, API_KEYS_SUFFIX);
        }
    }

    // Clean up trailing slashes
    for group in &mut final_config.groups {
        if group.target_url.ends_with('/') { group.target_url.pop(); }
    }

    // --- 4. Final Check & Validation ---
    if final_config.groups.is_empty() || final_config.groups.iter().all(|g| g.api_keys.is_empty()) {
        error!("Configuration error: No groups with usable API keys found. Define at least one group via environment variables (e.g., GEMINI_PROXY_GROUP_DEFAULT_API_KEYS=...).");
        return Err(AppError::Config("No groups with usable keys found".to_string()));
    }
    if !validate_config(&final_config, &path_str) {
         return Err(AppError::Config("Validation failed".to_string()));
    }

    info!("Configuration loaded and validated successfully ({} groups total).", final_config.groups.len());
    Ok(final_config)
}

/// Performs validation checks on the AppConfig.
pub fn validate_config(cfg: &AppConfig, config_source: &str) -> bool {
    let mut has_errors = false;

    if cfg.server.host.trim().is_empty() || cfg.server.port == 0 {
        error!("Invalid server configuration: host={}, port={}", cfg.server.host, cfg.server.port);
        has_errors = true;
    }

    if cfg.groups.is_empty() {
        error!("Configuration error: No groups loaded (source: {}).", config_source);
        return false; // Should be caught earlier in load_config, but check again
    }

    let mut group_names = HashSet::new();
    let mut total_keys = 0;

    for group in &cfg.groups {
        let group_name_trimmed = group.name.trim();
        if group_name_trimmed.is_empty() || !group_names.insert(group_name_trimmed.to_string()) {
            error!("Invalid or duplicate group name found: '{}'", group.name);
            has_errors = true;
        }

        if group.api_keys.is_empty() {
             // This is only a warning now, the main check is total_keys
             warn!("Group '{}' has no API keys defined.", group.name);
        } else if group.api_keys.iter().any(|key| key.trim().is_empty()) {
            error!("Group '{}' contains empty API key strings.", group.name);
            has_errors = true;
        }
        total_keys += group.api_keys.len();

        match Url::parse(&group.target_url) {
            Ok(parsed_url) if parsed_url.query().is_none() => {}
            Ok(_) => {
                 error!("Group '{}' target_url ('{}') must not contain a query string.", group.name, group.target_url);
                 has_errors = true;
            }
            Err(e) => {
                error!("Group '{}' has an invalid target_url ('{}'): {}", group.name, group.target_url, e);
                has_errors = true;
            }
        }

        if let Some(proxy_url) = &group.proxy_url {
            match Url::parse(proxy_url) {
                Ok(parsed_url) => {
                    let scheme = parsed_url.scheme().to_lowercase();
                    if !["http", "https", "socks5"].contains(&scheme.as_str()) {
                        error!("Group '{}' has unsupported proxy scheme '{}' in url '{}'", group.name, scheme, proxy_url);
                        has_errors = true;
                    }
                }
                Err(e) => {
                    error!("Group '{}' has an invalid proxy_url ('{}'): {}", group.name, proxy_url, e);
                    has_errors = true;
                }
            }
        }
    }

    if total_keys == 0 {
        error!("Configuration error: No usable API keys found across all defined groups.");
        has_errors = true; // This should have been caught earlier but validate anyway
    }

    !has_errors
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::tempdir;
    use std::sync::Mutex;
    use lazy_static::lazy_static;

    lazy_static! {
        static ref ENV_MUTEX: Mutex<()> = Mutex::new(());
    }

    fn create_temp_config_file(dir: &tempfile::TempDir, content: &str) -> PathBuf {
        let file_path = dir.path().join("test_config.yaml");
        let mut file = File::create(&file_path).expect("Failed to create temp config file");
        writeln!(file, "{}", content).expect("Failed to write to temp config file");
        file_path
    }

     fn delete_file(path: &PathBuf) { let _ = std::fs::remove_file(path); }
     fn set_env_var(key: &str, value: &str) { std::env::set_var(key, value); }
     fn remove_env_var(key: &str) { std::env::remove_var(key); }

     // Cleans up *all* potential test env vars
     fn cleanup_test_env_vars() {
         remove_env_var("GEMINI_PROXY_GROUP_DEFAULT_API_KEYS");
         remove_env_var("GEMINI_PROXY_GROUP_DEFAULT_PROXY_URL");
         remove_env_var("GEMINI_PROXY_GROUP_DEFAULT_TARGET_URL");
         remove_env_var("GEMINI_PROXY_GROUP_GROUP1_API_KEYS");
         remove_env_var("GEMINI_PROXY_GROUP_GROUP1_PROXY_URL");
         remove_env_var("GEMINI_PROXY_GROUP_GROUP1_TARGET_URL");
         remove_env_var("GEMINI_PROXY_GROUP_NO_ENV_GROUP_API_KEYS"); // From override test
         remove_env_var("GEMINI_PROXY_GROUP_BADPROXY_API_KEYS");
         remove_env_var("GEMINI_PROXY_GROUP_BADPROXY_PROXY_URL");
         remove_env_var("GEMINI_PROXY_GROUP_FTPPROXY_API_KEYS");
         remove_env_var("GEMINI_PROXY_GROUP_FTPPROXY_PROXY_URL");
         remove_env_var("GEMINI_PROXY_GROUP_BADTARGET_API_KEYS");
         remove_env_var("GEMINI_PROXY_GROUP_BADTARGET_TARGET_URL");
     }


    #[test]
    fn test_load_from_env_only_success() {
        let _lock = ENV_MUTEX.lock().unwrap();
        cleanup_test_env_vars(); // Explicit cleanup at start
        let dir = tempdir().unwrap(); let non_existent_path = dir.path().join("ne.yaml"); delete_file(&non_existent_path);

        set_env_var("GEMINI_PROXY_GROUP_DEFAULT_API_KEYS", "keyA, keyB");
        set_env_var("GEMINI_PROXY_GROUP_GROUP1_API_KEYS", "keyC");
        set_env_var("GEMINI_PROXY_GROUP_GROUP1_PROXY_URL", "socks5://proxy.com:1080");
        set_env_var("GEMINI_PROXY_GROUP_GROUP1_TARGET_URL", "http://env.target.g1");

        let config = load_config(&non_existent_path).expect("Load from env only failed");

        assert_eq!(config.server.host, "0.0.0.0"); assert_eq!(config.server.port, 8080);
        assert_eq!(config.groups.len(), 2);

        let g_default = config.groups.iter().find(|g| g.name == "DEFAULT").unwrap();
        assert_eq!(g_default.api_keys, vec!["keyA", "keyB"]); assert!(g_default.proxy_url.is_none());
        assert_eq!(g_default.target_url, default_target_url());

        let g1 = config.groups.iter().find(|g| g.name == "GROUP1").unwrap();
        assert_eq!(g1.api_keys, vec!["keyC"]); assert_eq!(g1.proxy_url, Some("socks5://proxy.com:1080".to_string()));
        assert_eq!(g1.target_url, "http://env.target.g1");
        cleanup_test_env_vars(); // Explicit cleanup at end
    }

    #[test]
    fn test_load_from_yaml_and_env_override() {
         let _lock = ENV_MUTEX.lock().unwrap();
         cleanup_test_env_vars();
         let dir = tempdir().unwrap();
         let yaml_content = r#"
 server: { host: "192.168.1.1", port: 9999 }
 groups:
   - name: default # Will be matched by sanitized name DEFAULT
     target_url: "http://yaml.target.default"
   - name: group1
     target_url: "http://yaml.target.g1" # Will be overridden
   - name: no_env_group # No matching env var for keys, won't be created
     target_url: "http://yaml.target.no_env"
 "#;
         let config_path = create_temp_config_file(&dir, yaml_content);

         set_env_var("GEMINI_PROXY_GROUP_DEFAULT_API_KEYS", "env_keyA");
         set_env_var("GEMINI_PROXY_GROUP_DEFAULT_PROXY_URL", ""); // Explicitly no proxy
         set_env_var("GEMINI_PROXY_GROUP_GROUP1_API_KEYS", "env_keyC");
         set_env_var("GEMINI_PROXY_GROUP_GROUP1_PROXY_URL", "socks5://env.proxy.g1:1080");
         set_env_var("GEMINI_PROXY_GROUP_GROUP1_TARGET_URL", "http://env.target.override.g1"); // Override target

         let config = load_config(&config_path).expect("Load with overrides failed");

         assert_eq!(config.server.host, "192.168.1.1"); assert_eq!(config.server.port, 9999);
         assert_eq!(config.groups.len(), 2); // Only groups with keys from env exist

         let g_default = config.groups.iter().find(|g| g.name == "DEFAULT").unwrap();
         assert_eq!(g_default.api_keys, vec!["env_keyA"]); assert!(g_default.proxy_url.is_none());
         assert_eq!(g_default.target_url, "http://yaml.target.default"); // Target from YAML

         let g1 = config.groups.iter().find(|g| g.name == "GROUP1").unwrap();
         assert_eq!(g1.api_keys, vec!["env_keyC"]); assert_eq!(g1.proxy_url, Some("socks5://env.proxy.g1:1080".to_string()));
         assert_eq!(g1.target_url, "http://env.target.override.g1"); // Target from ENV override

         assert!(config.groups.iter().find(|g| g.name == "NO_ENV_GROUP").is_none());
         cleanup_test_env_vars();
    }

    #[test]
    fn test_validation_fails_if_no_groups_defined() {
         let _lock = ENV_MUTEX.lock().unwrap();
         cleanup_test_env_vars(); // Ensure clean state
         let dir = tempdir().unwrap();
         let empty_config_path = create_temp_config_file(&dir, "");
         let non_existent_path = dir.path().join("no_such_config.yaml"); delete_file(&non_existent_path);

         let result_empty = load_config(&empty_config_path);
         assert!(result_empty.is_err(), "Expected Err for empty config, got Ok");
         assert!(matches!(result_empty.as_ref().err().unwrap(), AppError::Config(msg) if msg == "No groups with usable keys found"));

         let result_nonexist = load_config(&non_existent_path);
         assert!(result_nonexist.is_err(), "Expected Err for non-existent config, got Ok");
         assert!(matches!(result_nonexist.as_ref().err().unwrap(), AppError::Config(msg) if msg == "No groups with usable keys found"));
         cleanup_test_env_vars();
    }

     #[test]
    fn test_validation_fails_if_no_usable_keys_across_all_groups() {
         let _lock = ENV_MUTEX.lock().unwrap();
         cleanup_test_env_vars();
         let dir = tempdir().unwrap(); let non_existent_path = dir.path().join("no_such_config.yaml"); delete_file(&non_existent_path);

         set_env_var("GEMINI_PROXY_GROUP_DEFAULT_API_KEYS", ", ,"); // Invalid keys
         set_env_var("GEMINI_PROXY_GROUP_GROUP1_API_KEYS", "");    // Invalid keys

         let result = load_config(&non_existent_path);
         assert!(result.is_err(), "Expected Err for groups with no usable keys, got Ok");
         assert!(matches!(result.as_ref().err().unwrap(), AppError::Config(msg) if msg == "No groups with usable keys found"));
         cleanup_test_env_vars();
    }

      #[test]
     fn test_validation_fails_on_invalid_proxy_url_from_env() {
         let _lock = ENV_MUTEX.lock().unwrap();
         cleanup_test_env_vars();
         let dir = tempdir().unwrap(); let non_existent_path = dir.path().join("no_such_config.yaml"); delete_file(&non_existent_path);

         set_env_var("GEMINI_PROXY_GROUP_BADPROXY_API_KEYS", "key1");
         set_env_var("GEMINI_PROXY_GROUP_BADPROXY_PROXY_URL", "::not a valid url::");

         let result = load_config(&non_existent_path);
         assert!(result.is_err(), "Expected Err for invalid proxy URL, got Ok");
         assert!(matches!(result.as_ref().err().unwrap(), AppError::Config(msg) if msg == "Validation failed"));
         cleanup_test_env_vars();
     }

      #[test]
     fn test_validation_fails_on_unsupported_proxy_scheme_from_env() {
           let _lock = ENV_MUTEX.lock().unwrap();
           cleanup_test_env_vars();
           let dir = tempdir().unwrap(); let non_existent_path = dir.path().join("no_such_config.yaml"); delete_file(&non_existent_path);

           set_env_var("GEMINI_PROXY_GROUP_FTPPROXY_API_KEYS", "key1");
           set_env_var("GEMINI_PROXY_GROUP_FTPPROXY_PROXY_URL", "ftp://myproxy.com");

           let result = load_config(&non_existent_path);
           assert!(result.is_err(), "Expected Err for unsupported proxy scheme, got Ok");
           assert!(matches!(result.as_ref().err().unwrap(), AppError::Config(msg) if msg == "Validation failed"));
           cleanup_test_env_vars();
     }

     #[test]
     fn test_validation_fails_on_invalid_target_url_from_env() {
          let _lock = ENV_MUTEX.lock().unwrap();
          cleanup_test_env_vars();
          let dir = tempdir().unwrap(); let non_existent_path = dir.path().join("no_such_config.yaml"); delete_file(&non_existent_path);

          set_env_var("GEMINI_PROXY_GROUP_BADTARGET_API_KEYS", "key1");
          set_env_var("GEMINI_PROXY_GROUP_BADTARGET_TARGET_URL", "http://invalid?query=bad"); // Target with query

          let result = load_config(&non_existent_path);
          assert!(result.is_err(), "Expected Err for invalid target URL, got Ok");
          assert!(matches!(result.as_ref().err().unwrap(), AppError::Config(msg) if msg == "Validation failed"));
          cleanup_test_env_vars();
     }

} // end tests module
