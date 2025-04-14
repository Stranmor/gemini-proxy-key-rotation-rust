// src/config.rs
use serde::Deserialize;
use std::{env, fs, io, path::Path, collections::HashSet};
use tracing::{info, warn, error};
use crate::error::{AppError, Result};
use url::Url;

/// Represents a group of API keys with associated target URL and optional proxy settings.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct KeyGroup {
    /// A unique identifier for this group, used for logging and potentially future features.
    /// Also used to construct the environment variable name for overriding API keys.
    pub name: String,
    /// A list of API keys associated with this group.
    /// The proxy will rotate through these keys (along with keys from other groups).
    /// This list can be overridden by the `GEMINI_PROXY_GROUP_{GROUP_NAME}_API_KEYS` environment variable.
    pub api_keys: Vec<String>,
    /// An optional upstream proxy URL (supports http, https, socks5) for requests made using keys from this group.
    /// Example: "socks5://user:pass@host:port"
    #[serde(default)] // Makes proxy_url optional, defaults to None
    pub proxy_url: Option<String>,
    /// The base target API endpoint URL (scheme + host + port, e.g., "https://generativelanguage.googleapis.com").
    /// The path and query from the incoming request will be appended to this base.
    /// Defaults to the Google API endpoint host (`https://generativelanguage.googleapis.com`) if not specified.
    #[serde(default = "default_target_url")]
    pub target_url: String,
}

/// Represents the root of the application configuration, typically loaded from `config.yaml`.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    /// Configuration for the proxy server itself (host and port).
    pub server: ServerConfig,
    /// A list of key groups. The proxy rotates through keys from all groups combined.
    pub groups: Vec<KeyGroup>,
}

/// Configuration for the network address the proxy server listens on.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// The hostname or IP address to bind to (e.g., "0.0.0.0", "127.0.0.1").
    /// Use "0.0.0.0" when running inside Docker.
    pub host: String,
    /// The port number to listen on (e.g., 8080).
    pub port: u16,
}

/// Provides the default Google API base URL.
fn default_target_url() -> String {
    // Should only contain the scheme and host. Path is appended from the request.
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


/// Loads the application configuration from the specified YAML file path.
///
/// This function reads the file, parses the YAML content into an `AppConfig` struct,
/// and then checks for environment variables (`GEMINI_PROXY_GROUP_{GROUP_NAME}_API_KEYS`)
/// to potentially override the `api_keys` listed in the file for each group.
///
/// # Arguments
///
/// * `path` - A `Path` reference to the configuration file.
///
/// # Errors
///
/// Returns `AppError::Io` if the file cannot be read.
/// Returns `AppError::YamlParsing` if the YAML content is invalid.
pub fn load_config(path: &Path) -> Result<AppConfig> {
    let path_str = path.display().to_string();

    // Reading the file content using AppError::Io
    let contents = fs::read_to_string(path).map_err(|e| {
        AppError::Io(io::Error::new(
            e.kind(),
            format!("Failed to read config file '{}': {}", path_str, e),
        ))
    })?;

    // Parsing YAML using AppError::YamlParsing
    let mut config: AppConfig = serde_yaml::from_str(&contents).map_err(AppError::from)?;


    // --- Override API keys from environment variables ---
    for group in &mut config.groups {
        let sanitized_group_name = sanitize_group_name_for_env(&group.name);
        let env_var_name = format!("GEMINI_PROXY_GROUP_{}_API_KEYS", sanitized_group_name);

        match env::var(&env_var_name) {
            Ok(env_keys_str) => {
                let trimmed_env_keys = env_keys_str.trim();
                if trimmed_env_keys.is_empty() {
                     warn!(
                        "Environment variable '{}' found but is empty after trimming. Using keys from config file for group '{}'.",
                        env_var_name, group.name
                    );
                } else {
                    let keys_from_env: Vec<String> = trimmed_env_keys
                        .split(',')
                        .map(|k| k.trim().to_string())
                        .filter(|k| !k.is_empty())
                        .collect();

                    if !keys_from_env.is_empty() {
                         info!(
                            "Overriding API keys for group '{}' from environment variable '{}' ({} keys found).",
                            group.name, env_var_name, keys_from_env.len()
                        );
                        group.api_keys = keys_from_env;
                    } else {
                         warn!(
                            "Environment variable '{}' found but contained no valid keys after trimming/splitting. Using keys from config file for group '{}'.",
                            env_var_name, group.name
                        );
                    }
                }
            }
            Err(env::VarError::NotPresent) => {} // Do nothing, use file keys
            Err(e) => {
                 warn!(
                    "Error reading environment variable '{}': {}. Using keys from config file for group '{}'.",
                    env_var_name, e, group.name
                );
            }
        }
    }
    // --- End of environment variable override ---

    // Remove trailing slash from target_url if present, as url::join handles it.
    for group in &mut config.groups {
        if group.target_url.ends_with('/') {
            group.target_url.pop();
        }
    }

    Ok(config)
}


/// Performs validation checks on the loaded and potentially modified `AppConfig`.
///
/// This should be called *after* `load_config` to ensure validation runs on the final
/// configuration, including overrides from environment variables.
///
/// Logs errors and warnings for configuration issues.
///
/// # Arguments
///
/// * `cfg` - A mutable reference to the `AppConfig` to validate.
/// * `config_path_str` - The string representation of the config file path (for logging).
///
/// # Returns
///
/// Returns `true` if the configuration is valid, `false` otherwise.
pub fn validate_config(cfg: &mut AppConfig, config_path_str: &str) -> bool {
    let mut has_errors = false;

    if cfg.server.host.trim().is_empty() {
        error!("Configuration error in {}: Server host cannot be empty.", config_path_str);
        has_errors = true;
    }
    if cfg.server.port == 0 {
        error!("Configuration error in {}: Server port cannot be 0.", config_path_str);
        has_errors = true;
    }

    if cfg.groups.is_empty() {
        error!("Configuration error in {}: The 'groups' list cannot be empty.", config_path_str);
        return false;
    }

    let mut group_names = HashSet::new();
    let mut total_keys_after_override = 0;

    for group in &mut cfg.groups {
        group.name = group.name.trim().to_string();
        if group.name.is_empty() {
            error!("Configuration error in {}: Group name cannot be empty.", config_path_str);
            has_errors = true;
        } else if !group_names.insert(group.name.clone()) {
            error!("Configuration error in {}: Duplicate group name found: '{}'.", config_path_str, group.name);
            has_errors = true;
        }
        if group.name.contains('/') || group.name.contains(':') || group.name.contains(' ') {
            warn!("Configuration warning in {}: Group name '{}' contains potentially problematic characters (/, :, space).", config_path_str, group.name);
        }

        if group.api_keys.iter().any(|key| key.trim().is_empty()) {
             error!("Configuration error in {}: Group '{}' contains one or more empty API key strings AFTER potential environment override.", config_path_str, group.name);
             has_errors = true;
        } else {
             total_keys_after_override += group.api_keys.len();
        }

        // Validate target_url is a valid base URL (scheme + host)
        match Url::parse(&group.target_url) {
            Ok(parsed_url) => {
                if parsed_url.path() != "/" && parsed_url.path() != "" {
                     error!("Configuration error in {}: Group '{}' target_url ('{}') should be a base URL (e.g., 'https://host.com') without a path component.", config_path_str, group.name, group.target_url);
                     has_errors = true;
                }
                 if parsed_url.query().is_some() {
                     error!("Configuration error in {}: Group '{}' target_url ('{}') should not contain a query string.", config_path_str, group.name, group.target_url);
                     has_errors = true;
                }
            }
            Err(_) => {
                error!("Configuration error in {}: Group '{}' has an invalid target_url: '{}'.", config_path_str, group.name, group.target_url);
                has_errors = true;
            }
        }


        if let Some(proxy_url) = &group.proxy_url {
            match Url::parse(proxy_url) {
                Ok(parsed_url) => {
                    let scheme = parsed_url.scheme().to_lowercase();
                    if scheme != "http" && scheme != "https" && scheme != "socks5" {
                         error!("Configuration error in {}: Group '{}' has an unsupported proxy scheme: '{}' in proxy_url: '{}'. Only http, https, socks5 are supported.", config_path_str, group.name, scheme, proxy_url);
                         has_errors = true;
                    }
                }
                Err(_) => {
                     error!("Configuration error in {}: Group '{}' has an invalid proxy_url: '{}'.", config_path_str, group.name, proxy_url);
                    has_errors = true;
                }
            }
        }
    }

    if total_keys_after_override == 0 {
        error!("Configuration error in {}: No usable API keys found after processing config file and environment variables. At least one valid key is required.", config_path_str);
        has_errors = true;
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

    fn create_temp_config(dir: &tempfile::TempDir, content: &str) -> PathBuf {
        let file_path = dir.path().join("test_config.yaml");
        let mut file = File::create(&file_path).expect("Failed to create temp config file");
        writeln!(file, "{}", content).expect("Failed to write to temp config file");
        file_path
    }

    const TEST_CONFIG_CONTENT_VALID_BASE: &str = r#"
server:
  host: "0.0.0.0"
  port: 8080
groups:
  - name: "default"
    api_keys: ["file_key_1", "file_key_2"]
    # target_url uses default: https://generativelanguage.googleapis.com
  - name: "group-two"
    target_url: "http://example.com" # Specific base URL
    api_keys: ["file_key_3"]
  - name: "special_chars!"
    api_keys: ["file_key_4"]
    # target_url uses default
"#;

     const TEST_CONFIG_CONTENT_INVALID_GROUP: &str = r#"
server:
  host: "127.0.0.1"
  port: 8081
groups:
  - name: " " # Invalid empty name after trim
    api_keys: ["key1"]
  - name: "dup"
    api_keys: ["key2"]
  - name: "dup" # Duplicate name
    api_keys: ["key3"]
  - name: "bad_base_url"
    target_url: "::not a url::"
    api_keys: ["key4"]
  - name: "bad_proxy"
    proxy_url: "ftp://invalid.proxy" # Unsupported scheme
    api_keys: ["key5"]
  - name: "empty_keys"
    api_keys: ["key6", " "] # Contains empty key string
  - name: "base_with_path"
    target_url: "https://host.com/some/path/" # Invalid: target_url should be base only
    api_keys: ["key7"]
"#;

    #[test]
    fn test_load_basic_config_success() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let dir = tempdir().unwrap();
        let config_path = create_temp_config(&dir, TEST_CONFIG_CONTENT_VALID_BASE);

        std::env::remove_var("GEMINI_PROXY_GROUP_DEFAULT_API_KEYS");
        std::env::remove_var("GEMINI_PROXY_GROUP_GROUP_TWO_API_KEYS");
        std::env::remove_var("GEMINI_PROXY_GROUP_SPECIAL_CHARS__API_KEYS");

        let config = load_config(&config_path).expect("Failed to load valid config");

        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.groups.len(), 3);
        assert_eq!(config.groups[0].name, "default");
        assert_eq!(config.groups[0].api_keys, vec!["file_key_1", "file_key_2"]);
        // Check default base URL
        assert_eq!(config.groups[0].target_url, default_target_url());
        assert!(config.groups[0].proxy_url.is_none());
        assert_eq!(config.groups[1].name, "group-two");
        assert_eq!(config.groups[1].api_keys, vec!["file_key_3"]);
        assert_eq!(config.groups[1].target_url, "http://example.com"); // Check specific base URL
        assert!(config.groups[1].proxy_url.is_none());
        assert_eq!(config.groups[2].name, "special_chars!");
        assert_eq!(config.groups[2].api_keys, vec!["file_key_4"]);
        assert_eq!(config.groups[2].target_url, default_target_url());
        assert!(config.groups[2].proxy_url.is_none());
    }

    #[test]
    fn test_override_keys_with_env_var() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let dir = tempdir().unwrap();
        let config_path = create_temp_config(&dir, TEST_CONFIG_CONTENT_VALID_BASE);
        let env_var_name_default = "GEMINI_PROXY_GROUP_DEFAULT_API_KEYS";
        let env_keys_default = "env_key_a, env_key_b ";
        let env_var_name_special = "GEMINI_PROXY_GROUP_SPECIAL_CHARS__API_KEYS";
        let env_keys_special = "env_key_c";
        std::env::remove_var("GEMINI_PROXY_GROUP_GROUP_TWO_API_KEYS");

        std::env::set_var(env_var_name_default, env_keys_default);
        std::env::set_var(env_var_name_special, env_keys_special);

        let config = load_config(&config_path).expect("Failed to load config");

        std::env::remove_var(env_var_name_default);
        std::env::remove_var(env_var_name_special);

        assert_eq!(config.groups.len(), 3);
        assert_eq!(config.groups[0].api_keys, vec!["env_key_a".to_string(), "env_key_b".to_string()]);
        assert_eq!(config.groups[1].api_keys, vec!["file_key_3".to_string()]);
        assert_eq!(config.groups[2].api_keys, vec!["env_key_c".to_string()]);
    }

     #[test]
    fn test_override_with_empty_env_var() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let dir = tempdir().unwrap();
        let config_path = create_temp_config(&dir, TEST_CONFIG_CONTENT_VALID_BASE);
        let env_var_name = "GEMINI_PROXY_GROUP_DEFAULT_API_KEYS";
        std::env::remove_var("GEMINI_PROXY_GROUP_GROUP_TWO_API_KEYS");
        std::env::remove_var("GEMINI_PROXY_GROUP_SPECIAL_CHARS__API_KEYS");

        std::env::set_var(env_var_name, "  ");

        let config = load_config(&config_path).expect("Failed to load config");
        std::env::remove_var(env_var_name);

        assert_eq!(config.groups[0].api_keys, vec!["file_key_1", "file_key_2"], "Keys should be from file when env var is empty");
    }

     #[test]
    fn test_override_with_comma_only_env_var() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let dir = tempdir().unwrap();
        let config_path = create_temp_config(&dir, TEST_CONFIG_CONTENT_VALID_BASE);
        let env_var_name = "GEMINI_PROXY_GROUP_DEFAULT_API_KEYS";
        std::env::remove_var("GEMINI_PROXY_GROUP_GROUP_TWO_API_KEYS");
        std::env::remove_var("GEMINI_PROXY_GROUP_SPECIAL_CHARS__API_KEYS");

        std::env::set_var(env_var_name, ",,,");

        let config = load_config(&config_path).expect("Failed to load config");
        std::env::remove_var(env_var_name);

        assert_eq!(config.groups[0].api_keys, vec!["file_key_1", "file_key_2"], "Keys should be from file when env var contains only commas");
    }


    #[test]
    fn test_env_vars_not_present() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let dir = tempdir().unwrap();
        let config_path = create_temp_config(&dir, TEST_CONFIG_CONTENT_VALID_BASE);

        std::env::remove_var("GEMINI_PROXY_GROUP_DEFAULT_API_KEYS");
        std::env::remove_var("GEMINI_PROXY_GROUP_GROUP_TWO_API_KEYS");
        std::env::remove_var("GEMINI_PROXY_GROUP_SPECIAL_CHARS__API_KEYS");

        let config = load_config(&config_path).expect("Failed to load config");

        assert_eq!(config.groups[0].api_keys, vec!["file_key_1", "file_key_2"]);
        assert_eq!(config.groups[1].api_keys, vec!["file_key_3"]);
        assert_eq!(config.groups[2].api_keys, vec!["file_key_4"]);
    }

    #[test]
    fn test_sanitize_group_name_for_env_logic() {
        assert_eq!(sanitize_group_name_for_env("default"), "DEFAULT");
        assert_eq!(sanitize_group_name_for_env("group-two"), "GROUP_TWO");
        assert_eq!(sanitize_group_name_for_env("special_chars!"), "SPECIAL_CHARS_");
        assert_eq!(sanitize_group_name_for_env("UPPER"), "UPPER");
        assert_eq!(sanitize_group_name_for_env("with space"), "WITH_SPACE");
        assert_eq!(sanitize_group_name_for_env("a.b/c:d"), "A_B_C_D");
    }

     #[test]
    fn test_load_config_file_not_found_error() {
        let non_existent_path = PathBuf::from("this_file_truly_does_not_exist_123.yaml");
        let result = load_config(&non_existent_path);
        assert!(result.is_err());
        matches!(result, Err(AppError::Io(_)));
    }

    #[test]
    fn test_load_config_invalid_yaml_error() {
         let dir = tempdir().unwrap();
         let invalid_content = "server: { host: \"0.0.0.0\", port: 8080 }\ngroups: [ name: \"bad\" ";
         let config_path = create_temp_config(&dir, invalid_content);
         let result = load_config(&config_path);
         assert!(result.is_err());
         assert!(matches!(result, Err(AppError::YamlParsing(_))));
    }

     // --- Tests for validate_config ---

     #[test]
    fn test_validate_config_valid_base() {
        let dir = tempdir().unwrap();
        let config_path = create_temp_config(&dir, TEST_CONFIG_CONTENT_VALID_BASE);
        let mut config = load_config(&config_path).expect("Load should succeed");
        let is_valid = validate_config(&mut config, &config_path.display().to_string());
        assert!(is_valid, "Valid config with base URLs should pass validation");
    }

    #[test]
    fn test_validate_config_invalid_groups() {
        let dir = tempdir().unwrap();
        let config_path = create_temp_config(&dir, TEST_CONFIG_CONTENT_INVALID_GROUP);
        // load_config might succeed if YAML is parseable, validation should catch issues
        let mut config = load_config(&config_path).expect("Load should succeed even with invalid content for validation");
        let is_valid = validate_config(&mut config, &config_path.display().to_string());
        assert!(!is_valid, "Config with multiple group errors should fail validation");
    }

    #[test]
    fn test_validate_config_no_groups() {
        let dir = tempdir().unwrap();
        let no_groups_content = r#"
server:
  host: "127.0.0.1"
  port: 8080
groups: [] # Empty groups list
"#;
        let config_path = create_temp_config(&dir, no_groups_content);
        let mut config = load_config(&config_path).expect("Load should succeed");
        let is_valid = validate_config(&mut config, &config_path.display().to_string());
        assert!(!is_valid, "Config with empty groups list should fail validation");
    }

     #[test]
    fn test_validate_config_no_usable_keys_after_override() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let dir = tempdir().unwrap();
        let config_content = r#"
server:
  host: "127.0.0.1"
  port: 8080
groups:
  - name: "group1"
    api_keys: ["file_key1"] # Key in file
    target_url: "https://valid.base"
"#;
        let config_path = create_temp_config(&dir, config_content);
        let env_var_name = "GEMINI_PROXY_GROUP_GROUP1_API_KEYS";

        // Override with an empty string
        std::env::set_var(env_var_name, "   ");

        let mut config = load_config(&config_path).expect("Load should succeed");
        std::env::remove_var(env_var_name);

        // Validation should FAIL because the override resulted in zero keys for the group,
        // and the fallback logic in load_config was removed.
        let is_valid = validate_config(&mut config, &config_path.display().to_string());
        assert!(!is_valid, "Config with no usable keys after override should fail validation");
     }

     #[test]
    fn test_validate_config_invalid_server() {
        let dir = tempdir().unwrap();
        let invalid_server_content = r#"
server:
  host: " " # Empty host
  port: 0 # Invalid port
groups:
  - name: "default"
    api_keys: ["key1"]
    target_url: "https://valid.base"
"#;
        let config_path = create_temp_config(&dir, invalid_server_content);
        let mut config = load_config(&config_path).expect("Load should succeed");
        let is_valid = validate_config(&mut config, &config_path.display().to_string());
        assert!(!is_valid, "Config with invalid server settings should fail validation");
    }

     #[test]
     fn test_load_config_removes_trailing_slash_from_target_url() {
        let dir = tempdir().unwrap();
        let content = r#"
server:
  host: "127.0.0.1"
  port: 8080
groups:
  - name: "trailing"
    api_keys: ["key1"]
    target_url: "https://example.com/trailing/" # Base URL with trailing slash
  - name: "no-trailing"
    api_keys: ["key2"]
    target_url: "http://anotherexample.com" # Base URL without trailing slash
"#;
        let config_path = create_temp_config(&dir, content);
        let config = load_config(&config_path).expect("Load should succeed");

        assert_eq!(config.groups[0].target_url, "https://example.com/trailing", "Trailing slash should be removed by load_config");
        assert_eq!(config.groups[1].target_url, "http://anotherexample.com", "URL without trailing slash should remain unchanged");

     }
}
