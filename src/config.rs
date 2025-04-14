// src/config.rs
use serde::Deserialize;
use std::{env, fs, io, path::Path};
use tracing::{info, warn};
use crate::error::{AppError, Result}; // Import AppError and Result

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
    /// The target API endpoint URL for this group.
    /// Defaults to the Google API endpoint for OpenAI compatibility (`https://generativelanguage.googleapis.com/v1beta/openai/`) if not specified.
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

/// Provides the default Google API URL for OpenAI compatibility.
fn default_target_url() -> String {
    "https://generativelanguage.googleapis.com/v1beta/openai/".to_string()
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
    let mut config: AppConfig = serde_yaml::from_str(&contents).map_err(AppError::from)?; // Explicit conversion


    // --- Override API keys from environment variables ---
    for group in &mut config.groups {
        let sanitized_group_name = sanitize_group_name_for_env(&group.name);
        let env_var_name = format!("GEMINI_PROXY_GROUP_{}_API_KEYS", sanitized_group_name);

        // Attempt to read the environment variable
        match env::var(&env_var_name) {
            Ok(env_keys_str) => {
                // Variable found, attempt to parse it
                let trimmed_env_keys = env_keys_str.trim();
                if trimmed_env_keys.is_empty() {
                    // Found but empty after trimming - Use keys from file (do nothing)
                     warn!(
                        "Environment variable '{}' found but is empty after trimming. Using keys from config file for group '{}'.",
                        env_var_name, group.name
                    );
                } else {
                    // Parse keys, splitting by comma and trimming each part
                    let keys_from_env: Vec<String> = trimmed_env_keys
                        .split(',')
                        .map(|k| k.trim().to_string())
                        .filter(|k| !k.is_empty()) // Filter out empty strings resulting from split (e.g., "key1,,key2")
                        .collect();

                    // Check if *after* splitting and filtering, we still have keys
                    if !keys_from_env.is_empty() {
                        // Keys successfully parsed from env - OVERRIDE
                         info!(
                            "Overriding API keys for group '{}' from environment variable '{}' ({} keys found).",
                            group.name, env_var_name, keys_from_env.len()
                        );
                        group.api_keys = keys_from_env;
                    } else {
                        // Contained only commas/whitespace - Use keys from file (do nothing)
                         warn!(
                            "Environment variable '{}' found but contained no valid keys after trimming/splitting. Using keys from config file for group '{}'.",
                            env_var_name, group.name
                        );
                    }
                }
            }
            Err(env::VarError::NotPresent) => {
                // Variable not found - Use keys from file (do nothing)
            }
            Err(e) => {
                 // Other error reading env var - Use keys from file (do nothing)
                 warn!(
                    "Error reading environment variable '{}': {}. Using keys from config file for group '{}'.",
                    env_var_name, e, group.name
                );
            }
        }
    }
    // --- End of environment variable override ---


    Ok(config)
}


#[cfg(test)]
mod tests {
    use super::*; // Import items from parent module (config.rs)
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::tempdir; // Using tempfile crate for temporary files/dirs
    // --- Added for mutex ---
    use std::sync::Mutex;
    use lazy_static::lazy_static;
    // ---

    // --- Added static mutex ---
    // Mutex to synchronize tests modifying environment variables.
    lazy_static! {
        static ref ENV_MUTEX: Mutex<()> = Mutex::new(());
    }
    // ---

    // Helper to create a temporary config file for testing
    fn create_temp_config(dir: &tempfile::TempDir, content: &str) -> PathBuf {
        let file_path = dir.path().join("test_config.yaml");
        let mut file = File::create(&file_path).expect("Failed to create temp config file");
        writeln!(file, "{}", content).expect("Failed to write to temp config file");
        file_path
    }

    // Basic valid config content for tests
    const TEST_CONFIG_CONTENT_VALID: &str = r#"
server:
  host: "0.0.0.0"
  port: 8080
groups:
  - name: "default"
    api_keys:
      - "file_key_1"
      - "file_key_2"
  - name: "group-two"
    target_url: "http://example.com" # Non-default target
    api_keys:
      - "file_key_3"
  - name: "special_chars!" # Group name with special chars
    api_keys:
      - "file_key_4"

"#;

    #[test]
    fn test_load_basic_config_success() {
        // --- Lock mutex ---
        let _guard = ENV_MUTEX.lock().unwrap();
        // ---
        let dir = tempdir().unwrap();
        let config_path = create_temp_config(&dir, TEST_CONFIG_CONTENT_VALID);

        // Ensure env vars are not set for this test
        std::env::remove_var("GEMINI_PROXY_GROUP_DEFAULT_API_KEYS");
        std::env::remove_var("GEMINI_PROXY_GROUP_GROUP_TWO_API_KEYS");
        std::env::remove_var("GEMINI_PROXY_GROUP_SPECIAL_CHARS__API_KEYS");


        let config = load_config(&config_path).expect("Failed to load valid config");

        // Assertions (remain unchanged)
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.groups.len(), 3);
        assert_eq!(config.groups[0].name, "default");
        assert_eq!(config.groups[0].api_keys, vec!["file_key_1", "file_key_2"]); // Should now match file
        assert_eq!(config.groups[0].target_url, default_target_url());
        assert!(config.groups[0].proxy_url.is_none());
        assert_eq!(config.groups[1].name, "group-two");
        assert_eq!(config.groups[1].api_keys, vec!["file_key_3"]);
        assert_eq!(config.groups[1].target_url, "http://example.com");
        assert!(config.groups[1].proxy_url.is_none());
        assert_eq!(config.groups[2].name, "special_chars!");
        assert_eq!(config.groups[2].api_keys, vec!["file_key_4"]);
        assert_eq!(config.groups[2].target_url, default_target_url());
        assert!(config.groups[2].proxy_url.is_none());
        // --- Mutex unlocked automatically ---
    }

    #[test]
    fn test_override_keys_with_env_var() {
         // --- Lock mutex ---
        let _guard = ENV_MUTEX.lock().unwrap();
        // ---
        let dir = tempdir().unwrap();
        let config_path = create_temp_config(&dir, TEST_CONFIG_CONTENT_VALID);
        let env_var_name_default = "GEMINI_PROXY_GROUP_DEFAULT_API_KEYS";
        let env_keys_default = "env_key_a, env_key_b "; // Include spaces for trim testing
        let env_var_name_special = "GEMINI_PROXY_GROUP_SPECIAL_CHARS__API_KEYS";
        let env_keys_special = "env_key_c";
        // Ensure group-two var is not set
        std::env::remove_var("GEMINI_PROXY_GROUP_GROUP_TWO_API_KEYS");

        std::env::set_var(env_var_name_default, env_keys_default);
        std::env::set_var(env_var_name_special, env_keys_special);


        let config = load_config(&config_path).expect("Failed to load config");

        // Clean up env vars immediately after loading config
        std::env::remove_var(env_var_name_default);
        std::env::remove_var(env_var_name_special);


        // Assertions (remain unchanged)
        assert_eq!(config.groups.len(), 3);
        assert_eq!(config.groups[0].name, "default");
        assert_eq!(config.groups[0].api_keys, vec!["env_key_a".to_string(), "env_key_b".to_string()]);
        assert_eq!(config.groups[1].name, "group-two");
        assert_eq!(config.groups[1].api_keys, vec!["file_key_3".to_string()]);
        assert_eq!(config.groups[2].name, "special_chars!");
        assert_eq!(config.groups[2].api_keys, vec!["env_key_c".to_string()]);
         // --- Mutex unlocked automatically ---
    }

     #[test]
    fn test_override_with_empty_env_var() {
         // --- Lock mutex ---
        let _guard = ENV_MUTEX.lock().unwrap();
        // ---
        let dir = tempdir().unwrap();
        let config_path = create_temp_config(&dir, TEST_CONFIG_CONTENT_VALID);
        let env_var_name = "GEMINI_PROXY_GROUP_DEFAULT_API_KEYS";
         // Ensure other vars are not set
        std::env::remove_var("GEMINI_PROXY_GROUP_GROUP_TWO_API_KEYS");
        std::env::remove_var("GEMINI_PROXY_GROUP_SPECIAL_CHARS__API_KEYS");


        std::env::set_var(env_var_name, "  "); // Spaces only

        let config = load_config(&config_path).expect("Failed to load config");

        std::env::remove_var(env_var_name);

        // Assertions (remain unchanged)
        assert_eq!(config.groups[0].name, "default");
        assert_eq!(config.groups[0].api_keys, vec!["file_key_1", "file_key_2"], "Keys should be from file when env var is empty");
        assert_eq!(config.groups[1].name, "group-two");
        assert_eq!(config.groups[1].api_keys, vec!["file_key_3"]);
        // --- Mutex unlocked automatically ---
    }

     #[test]
    fn test_override_with_comma_only_env_var() {
         // --- Lock mutex ---
        let _guard = ENV_MUTEX.lock().unwrap();
        // ---
        let dir = tempdir().unwrap();
        let config_path = create_temp_config(&dir, TEST_CONFIG_CONTENT_VALID);
        let env_var_name = "GEMINI_PROXY_GROUP_DEFAULT_API_KEYS";
         // Ensure other vars are not set
        std::env::remove_var("GEMINI_PROXY_GROUP_GROUP_TWO_API_KEYS");
        std::env::remove_var("GEMINI_PROXY_GROUP_SPECIAL_CHARS__API_KEYS");

        std::env::set_var(env_var_name, ",,,");

        let config = load_config(&config_path).expect("Failed to load config");

        std::env::remove_var(env_var_name);

        // Assertions (remain unchanged)
        assert_eq!(config.groups[0].name, "default");
        assert_eq!(config.groups[0].api_keys, vec!["file_key_1", "file_key_2"], "Keys should be from file when env var contains only commas");
        // --- Mutex unlocked automatically ---
    }


    #[test]
    fn test_env_vars_not_present() {
         // --- Lock mutex ---
        let _guard = ENV_MUTEX.lock().unwrap();
        // ---
        let dir = tempdir().unwrap();
        let config_path = create_temp_config(&dir, TEST_CONFIG_CONTENT_VALID);

        std::env::remove_var("GEMINI_PROXY_GROUP_DEFAULT_API_KEYS");
        std::env::remove_var("GEMINI_PROXY_GROUP_GROUP_TWO_API_KEYS");
        std::env::remove_var("GEMINI_PROXY_GROUP_SPECIAL_CHARS__API_KEYS");

        let config = load_config(&config_path).expect("Failed to load config");

        // Assertions (remain unchanged)
        assert_eq!(config.groups[0].api_keys, vec!["file_key_1", "file_key_2"]);
        assert_eq!(config.groups[1].api_keys, vec!["file_key_3"]);
        assert_eq!(config.groups[2].api_keys, vec!["file_key_4"]);
         // --- Mutex unlocked automatically ---
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
        if let Err(AppError::Io(e)) = result {
             assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
        } else {
            panic!("Expected Io error for non-existent file, got {:?}", result);
        }
    }

    #[test]
    fn test_load_config_invalid_yaml_error() {
         let dir = tempdir().unwrap();
         let invalid_content = "server: { host: \"0.0.0.0\", port: 8080 }\ngroups: [ name: \"bad\" ";
         let config_path = create_temp_config(&dir, invalid_content);
         let result = load_config(&config_path);
         assert!(result.is_err());
         assert!(matches!(result, Err(AppError::YamlParsing(_))), "Expected YamlParsing error, got {:?}", result);
    }
}
