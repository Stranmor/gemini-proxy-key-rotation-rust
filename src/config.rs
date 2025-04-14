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
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)] // Added PartialEq, Eq for easier testing
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
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)] // Added PartialEq, Eq for easier testing
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
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)] // Added PartialEq, Eq for easier testing
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default = "default_server_host")]
    pub host: String,
    #[serde(default = "default_server_port")]
    pub port: u16,
}

// --- Default Implementations ---

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

// --- Constants ---

const ENV_VAR_PREFIX: &str = "GEMINI_PROXY_GROUP_";
const API_KEYS_SUFFIX: &str = "_API_KEYS";
const PROXY_URL_SUFFIX: &str = "_PROXY_URL";
const TARGET_URL_SUFFIX: &str = "_TARGET_URL";

// --- Helper Functions ---

/// Helper function to sanitize group names for matching YAML keys if needed.
/// Note: Currently unused as env var name is the primary key. Kept for potential future use.
#[allow(dead_code)] // Keep potentially useful helper, suppress warning
fn sanitize_for_matching(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
        .collect()
}

/// Extracts the potential group name from an environment variable key based on a suffix.
fn extract_group_name_from_env<'a>(env_key: &'a str, suffix: &str) -> Option<String> {
    env_key.strip_prefix(ENV_VAR_PREFIX)?.strip_suffix(suffix).map(|s| s.to_string())
}

// --- Configuration Loading Logic ---

/// Loads server config defaults and target URLs from the YAML file.
/// Returns the ServerConfig and a map of group names to their target URLs from YAML.
fn load_yaml_defaults(path: &Path) -> Result<(ServerConfig, HashMap<String, String>)> {
    let path_str = path.display().to_string();
    let mut server_config = ServerConfig::default();
    let mut yaml_target_urls: HashMap<String, String> = HashMap::new();

    match fs::read_to_string(path) {
        Ok(contents) => {
            if !contents.trim().is_empty() {
                match serde_yaml::from_str::<AppConfig>(&contents) {
                    Ok(yaml_config) => {
                        info!("Loaded base server config and group target_urls from '{}'.", path_str);
                        server_config = yaml_config.server;
                        for group in yaml_config.groups {
                            // Use uppercase name from env var as the canonical key
                            let group_name_upper = group.name.to_uppercase();
                            if !group.target_url.is_empty() {
                                yaml_target_urls.insert(group_name_upper, group.target_url);
                            } else {
                                warn!("Ignoring empty target_url for group '{}' in YAML file '{}'.", group.name, path_str);
                            }
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
    Ok((server_config, yaml_target_urls))
}

/// Discovers group settings (API keys, proxy URL, target URL) from environment variables.
/// Returns a map where keys are group names and values are tuples containing options for keys, proxy, and target URL.
fn discover_env_groups() -> HashMap<String, (Option<Vec<String>>, Option<Option<String>>, Option<String>)> {
    // Map: Group Name (from Env Var, uppercase) -> (Option<Keys>, Option<Option<ProxyURL>>, Option<TargetURL>)
    let mut env_group_data: HashMap<String, (Option<Vec<String>>, Option<Option<String>>, Option<String>)> = HashMap::new();

    for (key, value) in env::vars() {
         let key_upper = key.to_uppercase(); // Use uppercase for matching
        if let Some(group_name) = extract_group_name_from_env(&key_upper, API_KEYS_SUFFIX) {
            let keys = value.trim().split(',')
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty())
                .collect::<Vec<String>>();
            env_group_data.entry(group_name).or_default().0 = Some(keys);
        } else if let Some(group_name) = extract_group_name_from_env(&key_upper, PROXY_URL_SUFFIX) {
            let proxy_url = value.trim();
            env_group_data.entry(group_name).or_default().1 = Some(if proxy_url.is_empty() { None } else { Some(proxy_url.to_string()) });
        } else if let Some(group_name) = extract_group_name_from_env(&key_upper, TARGET_URL_SUFFIX) {
            let target_url = value.trim();
             if !target_url.is_empty() {
                env_group_data.entry(group_name).or_default().2 = Some(target_url.to_string());
             } else {
                 warn!("Environment variable '{}' for group '{}' defining target_url is empty. It will be ignored.", key, group_name);
             }
        }
    }
    env_group_data
}

/// Builds the final list of KeyGroups using data from environment variables and YAML defaults.
fn build_final_groups(
    env_group_data: HashMap<String, (Option<Vec<String>>, Option<Option<String>>, Option<String>)>,
    yaml_target_urls: &HashMap<String, String>
) -> Vec<KeyGroup> {
    let mut final_groups: Vec<KeyGroup> = Vec::new();

    for (group_name, (keys_opt, proxy_opt_opt, target_opt)) in env_group_data {
        if let Some(api_keys) = keys_opt {
             if api_keys.is_empty() {
                 warn!("Group '{}' defined via env vars has no valid API keys. Skipping group.", group_name);
                 continue;
             }
             info!("Processing group '{}' from environment variables.", group_name);

            // Determine Target URL: Env Var > YAML Fallback > Default
            // Note: YAML lookup uses uppercase group_name
            let target_url_from_env = target_opt;
            let target_url_from_yaml = yaml_target_urls.get(&group_name).cloned();

            let mut final_target_url = target_url_from_env.or(target_url_from_yaml)
                                      .unwrap_or_else(|| {
                                          info!("Using default target URL for group '{}'.", group_name);
                                          default_target_url()
                                      });

            // Clean up trailing slash
            if final_target_url.ends_with('/') { final_target_url.pop(); }


            let proxy_url = match proxy_opt_opt {
                Some(p_opt) => p_opt,
                None => None, // Default is no proxy if env var wasn't set
            };

            final_groups.push(KeyGroup {
                name: group_name.clone(), // Keep original case from env var derivation
                api_keys,
                proxy_url,
                target_url: final_target_url,
            });
        } else if proxy_opt_opt.is_some() || target_opt.is_some() {
             warn!("Proxy or Target URL variables found for group '{}', but no corresponding API key variable ('{}{}{}'). Group not created.", group_name, ENV_VAR_PREFIX, group_name, API_KEYS_SUFFIX);
        }
    }
    final_groups
}


/// Loads application configuration primarily from environment variables,
/// optionally using config.yaml only for default server settings or default target_urls.
pub fn load_config(path: &Path) -> Result<AppConfig> {
    let path_str = path.display().to_string();

    // --- 1. Load defaults from YAML (Server config, target URLs) ---
    let (server_config, yaml_target_urls) = load_yaml_defaults(path)?;

    // --- 2. Discover groups and settings from Environment Variables ---
    let env_group_data = discover_env_groups();

    // --- 3. Construct Final Groups ---
    let final_groups = build_final_groups(env_group_data, &yaml_target_urls);

    // --- 4. Assemble Final Config ---
    let final_config = AppConfig {
        server: server_config,
        groups: final_groups,
    };

    // --- 5. Final Check & Validation ---
    // Check if any usable keys exist across all groups
    let total_usable_keys: usize = final_config.groups.iter().map(|g| g.api_keys.len()).sum();
    if total_usable_keys == 0 {
         error!("Configuration error: No groups with usable API keys found. Define at least one group via environment variables (e.g., GEMINI_PROXY_GROUP_DEFAULT_API_KEYS=...).");
         return Err(AppError::Config("No groups with usable keys found".to_string()));
    }

    // Perform detailed validation
    if !validate_config(&final_config, &path_str) {
         return Err(AppError::Config("Validation failed".to_string()));
    }

    info!("Configuration loaded and validated successfully ({} groups total).", final_config.groups.len());
    Ok(final_config)
}

// --- Configuration Validation ---

/// Performs validation checks on the fully assembled AppConfig.
pub fn validate_config(cfg: &AppConfig, config_source: &str) -> bool {
    let mut has_errors = false;

    // Validate Server Config
    if cfg.server.host.trim().is_empty() || cfg.server.port == 0 {
        error!("Invalid server configuration: host='{}', port={}", cfg.server.host, cfg.server.port);
        has_errors = true;
    }

    // Validate Groups (Presence checked in load_config)
    if cfg.groups.is_empty() {
         error!("Internal Error: validate_config called with empty groups list (source: {}). This should have been caught earlier.", config_source);
         return false;
     }

    let mut group_names = HashSet::new();
    let mut total_keys = 0;

    for group in &cfg.groups {
        let group_name_trimmed = group.name.trim();

        // Check for empty or duplicate group names (using original case for error message, uppercase for check)
        if group_name_trimmed.is_empty() || !group_names.insert(group.name.to_uppercase()) {
            error!("Invalid or duplicate group name found: '{}'", group.name);
            has_errors = true;
        }

        // Check API Keys
        if group.api_keys.is_empty() {
             // This is only a warning now, the main check is total_keys > 0 in load_config
             warn!("Group '{}' has no API keys defined.", group.name);
        } else if group.api_keys.iter().any(|key| key.trim().is_empty()) {
            error!("Group '{}' contains empty API key strings.", group.name);
            has_errors = true;
        }
        total_keys += group.api_keys.len();

        // Validate Target URL
        match Url::parse(&group.target_url) {
            Ok(parsed_url) => {
                if parsed_url.cannot_be_a_base() {
                     error!("Group '{}' target_url ('{}') cannot be a base URL.", group.name, group.target_url);
                     has_errors = true;
                }
                 // Allow query strings in target URLs now, as the proxy logic handles appending
                 // else if parsed_url.query().is_some() {
                 //    error!("Group '{}' target_url ('{}') must not contain a query string.", group.name, group.target_url);
                 //    has_errors = true;
                 // }
            }
            Err(e) => {
                error!("Group '{}' has an invalid target_url ('{}'): {}", group.name, group.target_url, e);
                has_errors = true;
            }
        }

        // Validate Proxy URL
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

    // Check for total keys again (redundant with load_config check, but safe)
    if total_keys == 0 {
        error!("Internal Error: validate_config found no usable API keys across all groups.");
        has_errors = true;
    }

    !has_errors
}


// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::tempdir;
    use std::sync::Mutex;
    use lazy_static::lazy_static;

    // Mutex to prevent environment variable tests from interfering with each other
    lazy_static! {
        static ref ENV_MUTEX: Mutex<()> = Mutex::new(());
    }

    // --- Test Helpers ---

    fn create_temp_config_file(dir: &tempfile::TempDir, content: &str) -> PathBuf {
        let file_path = dir.path().join("test_config.yaml");
        let mut file = File::create(&file_path).expect("Failed to create temp config file");
        writeln!(file, "{}", content).expect("Failed to write to temp config file");
        file_path
    }

     fn delete_file(path: &PathBuf) { let _ = std::fs::remove_file(path); }
     fn set_env_var(key: &str, value: &str) { std::env::set_var(key, value); }
     fn remove_env_var(key: &str) { std::env::remove_var(key); }

     // Cleans up *all* known potential test env vars used in this module
     fn cleanup_test_env_vars() {
         const VARS_TO_CLEAN: &[&str] = &[
             "GEMINI_PROXY_GROUP_DEFAULT_API_KEYS",
             "GEMINI_PROXY_GROUP_DEFAULT_PROXY_URL",
             "GEMINI_PROXY_GROUP_DEFAULT_TARGET_URL",
             "GEMINI_PROXY_GROUP_GROUP1_API_KEYS",
             "GEMINI_PROXY_GROUP_GROUP1_PROXY_URL",
             "GEMINI_PROXY_GROUP_GROUP1_TARGET_URL",
             "GEMINI_PROXY_GROUP_NO_ENV_GROUP_API_KEYS", // From override test
             "GEMINI_PROXY_GROUP_EMPTYKEYS_API_KEYS",   // From validation tests
             "GEMINI_PROXY_GROUP_BADPROXY_API_KEYS",
             "GEMINI_PROXY_GROUP_BADPROXY_PROXY_URL",
             "GEMINI_PROXY_GROUP_FTPPROXY_API_KEYS",
             "GEMINI_PROXY_GROUP_FTPPROXY_PROXY_URL",
             "GEMINI_PROXY_GROUP_BADTARGET_API_KEYS",
             "GEMINI_PROXY_GROUP_BADTARGET_TARGET_URL",
             "GEMINI_PROXY_GROUP_TARGETWITHPATH_API_KEYS",
             "GEMINI_PROXY_GROUP_TARGETWITHPATH_TARGET_URL",
             "GEMINI_PROXY_GROUP_UPPERCASE_API_KEYS", // For case test
             "GEMINI_PROXY_GROUP_UPPERCASE_TARGET_URL",
             "gemini_proxy_group_lowercase_api_keys", // For case test
             "gemini_proxy_group_lowercase_target_url",
         ];
         for var in VARS_TO_CLEAN {
             remove_env_var(var);
         }
     }

    // --- Test Cases ---

    #[test]
    fn test_load_from_env_only_success() {
        let _lock = ENV_MUTEX.lock().unwrap();
        cleanup_test_env_vars();
        let dir = tempdir().unwrap();
        let non_existent_path = dir.path().join("ne.yaml");
        delete_file(&non_existent_path); // Ensure it doesn't exist

        set_env_var("GEMINI_PROXY_GROUP_DEFAULT_API_KEYS", "keyA, keyB");
        set_env_var("GEMINI_PROXY_GROUP_GROUP1_API_KEYS", "keyC");
        set_env_var("GEMINI_PROXY_GROUP_GROUP1_PROXY_URL", "socks5://proxy.com:1080");
        set_env_var("GEMINI_PROXY_GROUP_GROUP1_TARGET_URL", "http://env.target.g1"); // No trailing slash

        let config = load_config(&non_existent_path).expect("Load from env only failed");

        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.groups.len(), 2);

        // Use find for robustness against order changes
        let g_default = config.groups.iter().find(|g| g.name == "DEFAULT").expect("DEFAULT group not found");
        assert_eq!(g_default.api_keys, vec!["keyA", "keyB"]);
        assert!(g_default.proxy_url.is_none());
        assert_eq!(g_default.target_url, default_target_url()); // Should get default target

        let g1 = config.groups.iter().find(|g| g.name == "GROUP1").expect("GROUP1 group not found");
        assert_eq!(g1.api_keys, vec!["keyC"]);
        assert_eq!(g1.proxy_url, Some("socks5://proxy.com:1080".to_string()));
        assert_eq!(g1.target_url, "http://env.target.g1"); // Target from env

        cleanup_test_env_vars();
    }

    #[test]
    fn test_load_from_yaml_and_env_override() {
         let _lock = ENV_MUTEX.lock().unwrap();
         cleanup_test_env_vars();
         let dir = tempdir().unwrap();
         let yaml_content = r#"
 server: { host: "192.168.1.1", port: 9999 }
 groups:
   - name: default # YAML name case doesn't matter for target lookup, matches DEFAULT env var
     target_url: "http://yaml.target.default" # No trailing slash
   - name: group1
     target_url: "http://yaml.target.g1" # Will be overridden by env
   - name: no_env_group # No matching env var for keys, target_url ignored
     target_url: "http://yaml.target.no_env"
 "#;
         let config_path = create_temp_config_file(&dir, yaml_content);

         set_env_var("GEMINI_PROXY_GROUP_DEFAULT_API_KEYS", "env_keyA");
         set_env_var("GEMINI_PROXY_GROUP_DEFAULT_PROXY_URL", ""); // Explicitly no proxy
         set_env_var("GEMINI_PROXY_GROUP_GROUP1_API_KEYS", "env_keyC");
         set_env_var("GEMINI_PROXY_GROUP_GROUP1_PROXY_URL", "socks5://env.proxy.g1:1080");
         set_env_var("GEMINI_PROXY_GROUP_GROUP1_TARGET_URL", "http://env.target.override.g1"); // Override target

         let config = load_config(&config_path).expect("Load with overrides failed");

         assert_eq!(config.server.host, "192.168.1.1");
         assert_eq!(config.server.port, 9999);
         assert_eq!(config.groups.len(), 2); // Only groups with keys from env exist

         let g_default = config.groups.iter().find(|g| g.name == "DEFAULT").expect("DEFAULT group not found");
         assert_eq!(g_default.api_keys, vec!["env_keyA"]);
         assert!(g_default.proxy_url.is_none());
         assert_eq!(g_default.target_url, "http://yaml.target.default"); // Target from YAML

         let g1 = config.groups.iter().find(|g| g.name == "GROUP1").expect("GROUP1 group not found");
         assert_eq!(g1.api_keys, vec!["env_keyC"]);
         assert_eq!(g1.proxy_url, Some("socks5://env.proxy.g1:1080".to_string()));
         assert_eq!(g1.target_url, "http://env.target.override.g1"); // Target from ENV override

         assert!(config.groups.iter().find(|g| g.name == "NO_ENV_GROUP").is_none());
         cleanup_test_env_vars();
    }

     #[test]
     fn test_env_var_case_insensitivity_for_discovery() {
         let _lock = ENV_MUTEX.lock().unwrap();
         cleanup_test_env_vars();
         let dir = tempdir().unwrap();
         let non_existent_path = dir.path().join("ne.yaml"); delete_file(&non_existent_path);

         // Set env vars with mixed casing
         set_env_var("GEMINI_PROXY_GROUP_UPPERCASE_API_KEYS", "keyUpper");
         set_env_var("gemini_proxy_group_lowercase_api_keys", "keyLower");
         set_env_var("GEMINI_PROXY_GROUP_UPPERCASE_TARGET_URL", "http://target.upper");
         set_env_var("gemini_proxy_group_lowercase_target_url", "http://target.lower");


         let config = load_config(&non_existent_path).expect("Load with mixed case env vars failed");

         assert_eq!(config.groups.len(), 2);

         let g_upper = config.groups.iter().find(|g| g.name == "UPPERCASE").expect("UPPERCASE group not found");
         assert_eq!(g_upper.api_keys, vec!["keyUpper"]);
         assert_eq!(g_upper.target_url, "http://target.upper");

         let g_lower = config.groups.iter().find(|g| g.name == "LOWERCASE").expect("LOWERCASE group not found");
         assert_eq!(g_lower.api_keys, vec!["keyLower"]);
         assert_eq!(g_lower.target_url, "http://target.lower");


         cleanup_test_env_vars();
     }


    #[test]
    fn test_validation_fails_if_no_groups_defined_via_env() {
         let _lock = ENV_MUTEX.lock().unwrap();
         cleanup_test_env_vars(); // Ensure clean state
         let dir = tempdir().unwrap();
         let empty_config_path = create_temp_config_file(&dir, ""); // YAML doesn't define groups
         let non_existent_path = dir.path().join("no_such_config.yaml"); delete_file(&non_existent_path);

         // Test with empty YAML (no env vars set)
         let result_empty = load_config(&empty_config_path);
         assert!(result_empty.is_err(), "Expected Err for empty config with no env vars, got Ok");
         assert!(matches!(result_empty.as_ref().err().unwrap(), AppError::Config(msg) if msg == "No groups with usable keys found"));

         // Test with non-existent YAML (no env vars set)
         let result_nonexist = load_config(&non_existent_path);
         assert!(result_nonexist.is_err(), "Expected Err for non-existent config with no env vars, got Ok");
         assert!(matches!(result_nonexist.as_ref().err().unwrap(), AppError::Config(msg) if msg == "No groups with usable keys found"));
         cleanup_test_env_vars();
    }

     #[test]
    fn test_validation_fails_if_no_usable_keys_across_all_groups_via_env() {
         let _lock = ENV_MUTEX.lock().unwrap();
         cleanup_test_env_vars();
         let dir = tempdir().unwrap(); let non_existent_path = dir.path().join("no_such_config.yaml"); delete_file(&non_existent_path);

         set_env_var("GEMINI_PROXY_GROUP_DEFAULT_API_KEYS", ", ,"); // Invalid keys only
         set_env_var("GEMINI_PROXY_GROUP_GROUP1_API_KEYS", "");    // Empty keys

         let result = load_config(&non_existent_path);
         assert!(result.is_err(), "Expected Err for groups with no usable keys, got Ok");
         // The error comes from the check within load_config before validation now
         assert!(matches!(result.as_ref().err().unwrap(), AppError::Config(msg) if msg == "No groups with usable keys found"));
         cleanup_test_env_vars();
    }

      #[test]
     fn test_validation_fails_on_invalid_proxy_url_from_env() {
         let _lock = ENV_MUTEX.lock().unwrap();
         cleanup_test_env_vars();
         let dir = tempdir().unwrap(); let non_existent_path = dir.path().join("no_such_config.yaml"); delete_file(&non_existent_path);

         set_env_var("GEMINI_PROXY_GROUP_BADPROXY_API_KEYS", "key1");
         set_env_var("GEMINI_PROXY_GROUP_BADPROXY_PROXY_URL", "::not a valid url::"); // Invalid URL

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
           set_env_var("GEMINI_PROXY_GROUP_FTPPROXY_PROXY_URL", "ftp://myproxy.com"); // Unsupported scheme

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
          set_env_var("GEMINI_PROXY_GROUP_BADTARGET_TARGET_URL", ":::not a url"); // Invalid URL

          let result = load_config(&non_existent_path);
          assert!(result.is_err(), "Expected Err for invalid target URL, got Ok");
          assert!(matches!(result.as_ref().err().unwrap(), AppError::Config(msg) if msg == "Validation failed"));
          cleanup_test_env_vars();
     }

    #[test]
    fn test_target_url_trailing_slash_cleanup() {
        let _lock = ENV_MUTEX.lock().unwrap();
        cleanup_test_env_vars();
        let dir = tempdir().unwrap();
        let non_existent_path = dir.path().join("ne.yaml"); delete_file(&non_existent_path);

        // Target URL with trailing slash from env
        set_env_var("GEMINI_PROXY_GROUP_DEFAULT_API_KEYS", "keyA");
        set_env_var("GEMINI_PROXY_GROUP_DEFAULT_TARGET_URL", "http://example.com/api/");

        // Target URL with trailing slash from yaml (requires matching env keys)
        let yaml_content = r#"
groups:
  - name: group1
    target_url: "http://another.com/v1/"
"#;
        let config_path = create_temp_config_file(&dir, yaml_content);
        set_env_var("GEMINI_PROXY_GROUP_GROUP1_API_KEYS", "keyB");


        let config_env = load_config(&non_existent_path).expect("Load failed for env slash test");
        let g_default_env = config_env.groups.iter().find(|g| g.name == "DEFAULT").unwrap();
        assert_eq!(g_default_env.target_url, "http://example.com/api"); // Slash removed

        let config_yaml = load_config(&config_path).expect("Load failed for yaml slash test");
        let g1_yaml = config_yaml.groups.iter().find(|g| g.name == "GROUP1").unwrap();
        assert_eq!(g1_yaml.target_url, "http://another.com/v1"); // Slash removed


        cleanup_test_env_vars();
    }

    #[test]
    fn test_target_url_with_path_is_valid() {
         let _lock = ENV_MUTEX.lock().unwrap();
         cleanup_test_env_vars();
         let dir = tempdir().unwrap(); let non_existent_path = dir.path().join("ne.yaml"); delete_file(&non_existent_path);

         set_env_var("GEMINI_PROXY_GROUP_TARGETWITHPATH_API_KEYS", "key1");
         set_env_var("GEMINI_PROXY_GROUP_TARGETWITHPATH_TARGET_URL", "https://api.example.com/some/path"); // Valid URL with path

         let result = load_config(&non_existent_path);
         assert!(result.is_ok(), "Expected Ok for target URL with path, got Err: {:?}", result.err());
         let config = result.unwrap();
         let group = config.groups.iter().find(|g| g.name == "TARGETWITHPATH").unwrap();
         assert_eq!(group.target_url, "https://api.example.com/some/path");
         cleanup_test_env_vars();
    }

    #[test]
    fn test_warning_for_orphaned_env_vars() {
         // This test doesn't assert directly on logs, assumes manual inspection or future log capture setup
         let _lock = ENV_MUTEX.lock().unwrap();
         cleanup_test_env_vars();
         let dir = tempdir().unwrap(); let non_existent_path = dir.path().join("ne.yaml"); delete_file(&non_existent_path);

         // Set only proxy/target, no keys
         set_env_var("GEMINI_PROXY_GROUP_ORPHAN_PROXY_URL", "http://proxy.orphan");
         set_env_var("GEMINI_PROXY_GROUP_ORPHAN_TARGET_URL", "http://target.orphan");

         let result = load_config(&non_existent_path);
         assert!(result.is_err()); // Should fail because no keys means no groups
         assert!(matches!(result.err().unwrap(), AppError::Config(msg) if msg == "No groups with usable keys found"));
         // Expect warnings in the log output about group "ORPHAN" not being created.
         remove_env_var("GEMINI_PROXY_GROUP_ORPHAN_PROXY_URL");
         remove_env_var("GEMINI_PROXY_GROUP_ORPHAN_TARGET_URL");
         cleanup_test_env_vars(); // Ensure full cleanup
    }


} // end tests module
