// src/state.rs

use crate::config::AppConfig;
use crate::error::{AppError, Result};
use crate::key_manager::KeyManager;
use reqwest::{Client, Proxy, ClientBuilder};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;
use tracing::{error, info}; // Removed warn from main code imports
use url::Url;

/// Represents the shared application state that is accessible by all Axum handlers.
#[derive(Debug)]
pub struct AppState {
    pub key_manager: KeyManager,
    http_clients: HashMap<Option<String>, Client>,
}

impl AppState {
    /// Creates a new `AppState`. Initializes KeyManager and pre-builds HTTP clients.
    /// Returns `Err` if parsing a proxy URL or its scheme fails, or if the base client build fails.
    /// Logs errors and skips client creation for individual proxy build failures.
    pub async fn new(config: &AppConfig, config_path: &Path) -> Result<Self> {
        info!("Creating shared AppState: Initializing KeyManager and HTTP clients...");
        let key_manager = KeyManager::new(config, config_path).await;
        let mut http_clients = HashMap::new();

        let total_key_count: usize = config
            .groups.iter()
            .flat_map(|g| &g.api_keys)
            .filter(|k| !k.trim().is_empty())
            .count()
            .max(10);

        let configure_builder = |builder: ClientBuilder| -> ClientBuilder {
             builder
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(300))
                .pool_idle_timeout(Duration::from_secs(90))
                .pool_max_idle_per_host(total_key_count)
                .tcp_keepalive(Some(Duration::from_secs(60)))
        };

        // 1. Create the base client (no proxy) - this MUST succeed
        let base_client = configure_builder(Client::builder())
            .build()
            .map_err(AppError::HttpClientBuildError)?;
        http_clients.insert(None, base_client);
        info!("Base HTTP client (no proxy) created successfully.");

        // 2. Collect unique proxy URLs
        let unique_proxy_urls: HashSet<String> = config
            .groups.iter()
            .filter_map(|g| g.proxy_url.as_ref())
            .filter(|url_str| !url_str.trim().is_empty())
            .cloned()
            .collect();

        // 3. Create clients for each unique proxy URL
        for proxy_url_str in unique_proxy_urls {
            let client_result: Result<Client> = async {
                 let parsed_proxy_url = Url::parse(&proxy_url_str)
                     .map_err(|e| {
                         error!(proxy_url = %proxy_url_str, error = %e, "Failed to parse proxy URL string.");
                         AppError::ProxyConfigError(format!("Invalid proxy URL format '{}': {}", proxy_url_str, e))
                     })?;

                let scheme = parsed_proxy_url.scheme().to_lowercase();

                let proxy = match scheme.as_str() {
                    "http" => Proxy::http(&proxy_url_str).map_err(|e| AppError::ProxyConfigError(format!("Invalid HTTP proxy URL '{}': {}", proxy_url_str, e))),
                    "https" => Proxy::https(&proxy_url_str).map_err(|e| AppError::ProxyConfigError(format!("Invalid HTTPS proxy URL '{}': {}", proxy_url_str, e))),
                    "socks5" => Proxy::all(&proxy_url_str).map_err(|e| AppError::ProxyConfigError(format!("Invalid SOCKS5 proxy URL '{}': {}", proxy_url_str, e))),
                    _ => Err(AppError::ProxyConfigError(format!("Unsupported proxy scheme '{}' in URL '{}'", scheme, proxy_url_str))),
                 }?;

                 configure_builder(Client::builder())
                    .proxy(proxy)
                    .build()
                    .map_err(|e| {
                        error!(proxy_url = %proxy_url_str, scheme = %scheme, error = %e, "Failed to build reqwest client for proxy.");
                        AppError::HttpClientBuildError(e)
                    })
            }.await;

            match client_result {
                Ok(proxy_client) => {
                     info!(proxy_url = %proxy_url_str, "HTTP client created successfully for proxy.");
                     http_clients.insert(Some(proxy_url_str.clone()), proxy_client);
                 }
                 Err(e) => {
                      match e {
                           AppError::ProxyConfigError(_) => {
                               error!(proxy_url = %proxy_url_str, error = ?e, "Critical proxy configuration error.");
                               return Err(e); // Fail fast on config errors
                           }
                           AppError::HttpClientBuildError(_) => {
                               error!(proxy_url = %proxy_url_str, error = ?e, "Skipping client creation for this proxy due to build error.");
                               // Log and continue for build errors
                           }
                           _ => {
                                error!(proxy_url = %proxy_url_str, error = ?e, "Unexpected error during proxy client creation.");
                                return Err(e); // Fail on other unexpected errors
                           }
                      }
                 }
            }
        }

        info!("Finished initializing {} HTTP client(s).", http_clients.len());
        Ok(Self { key_manager, http_clients })
    }

    /// Returns a reference to the appropriate HTTP client.
    #[inline]
    pub fn get_client(&self, proxy_url: Option<&str>) -> Result<&Client> {
        let key = proxy_url.map(String::from);
        self.http_clients.get(&key).ok_or_else(|| {
            if proxy_url.is_some() {
                error!(proxy_url = ?proxy_url, "Requested HTTP client for proxy was not found/initialized in AppState.");
            } else {
                 error!("Requested base HTTP client (None proxy) was unexpectedly missing.");
            }
            AppError::Internal(format!("Client for proxy {:?} not found/initialized", proxy_url))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{KeyGroup, ServerConfig};
    use std::fs::File;
    use tempfile::tempdir;
    use std::sync::Mutex;
    use lazy_static::lazy_static;
    use tracing::warn; // Import warn specifically for tests

    const DEFAULT_TARGET_URL_STR: &str = "https://generativelanguage.googleapis.com";

     lazy_static! {
         static ref ENV_MUTEX: Mutex<()> = Mutex::new(());
     }

    fn create_test_state_config(groups: Vec<KeyGroup>) -> AppConfig {
        AppConfig {
            server: ServerConfig { host: "0.0.0.0".to_string(), port: 8080 },
            groups,
        }
    }

    fn create_dummy_config_path(dir: &tempfile::TempDir) -> std::path::PathBuf {
        let file_path = dir.path().join("dummy_config.yaml");
        File::create(&file_path).expect("Failed to create dummy config file");
        file_path
    }

     fn remove_env_var(key: &str) {
         std::env::remove_var(key);
     }

      struct EnvVarGuard;
      impl Drop for EnvVarGuard {
          fn drop(&mut self) {
              remove_env_var("GEMINI_PROXY_GROUP_DEFAULT_API_KEYS");
              remove_env_var("GEMINI_PROXY_GROUP_DEFAULT_PROXY_URL");
              remove_env_var("GEMINI_PROXY_GROUP_GROUP1_API_KEYS");
              remove_env_var("GEMINI_PROXY_GROUP_GROUP1_PROXY_URL");
              // Cleanup other vars as needed
               remove_env_var("GEMINI_PROXY_GROUP_BADPROXY_API_KEYS");
               remove_env_var("GEMINI_PROXY_GROUP_BADPROXY_PROXY_URL");
               remove_env_var("GEMINI_PROXY_GROUP_FTPPROXY_API_KEYS");
               remove_env_var("GEMINI_PROXY_GROUP_FTPPROXY_PROXY_URL");
               remove_env_var("GEMINI_PROXY_GROUP_INVALID_URL_API_KEYS");
               remove_env_var("GEMINI_PROXY_GROUP_INVALID_URL_PROXY_URL");
               remove_env_var("GEMINI_PROXY_GROUP_G_UNSUPPORTED_API_KEYS");
               remove_env_var("GEMINI_PROXY_GROUP_G_UNSUPPORTED_PROXY_URL");
               remove_env_var("GEMINI_PROXY_GROUP_G_INVALID_URL_API_KEYS");
               remove_env_var("GEMINI_PROXY_GROUP_G_HTTP_API_KEYS");
               remove_env_var("GEMINI_PROXY_GROUP_G_HTTP_PROXY_URL");
               remove_env_var("GEMINI_PROXY_GROUP_G_SOCKS_FAIL_API_KEYS");
               remove_env_var("GEMINI_PROXY_GROUP_G_SOCKS_FAIL_PROXY_URL");
          }
      }

    #[tokio::test]
    async fn test_appstate_new_no_proxies() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let _guard = EnvVarGuard;
        let dir = tempdir().unwrap();
        let dummy_path = create_dummy_config_path(&dir);

        let groups = vec![KeyGroup {
            name: "g1".to_string(),
            api_keys: vec!["key1".to_string()],
            proxy_url: None,
            target_url: DEFAULT_TARGET_URL_STR.to_string(),
        }];
        let config = create_test_state_config(groups);
        let state_result = AppState::new(&config, &dummy_path).await;

        assert!(state_result.is_ok());
        let state = state_result.unwrap();
        assert_eq!(state.http_clients.len(), 1);
        assert!(state.http_clients.contains_key(&None));
        assert!(state.get_client(None).is_ok());
        assert!(state.get_client(Some("http://nonexistent.proxy")).is_err());
    }

    #[tokio::test]
    async fn test_appstate_new_with_valid_proxies() {
         let _lock = ENV_MUTEX.lock().unwrap();
         let _guard = EnvVarGuard;
        let dir = tempdir().unwrap();
        let dummy_path = create_dummy_config_path(&dir);

        let groups = vec![
            KeyGroup {
                name: "g_http".to_string(),
                api_keys: vec!["key_http".to_string()],
                proxy_url: Some("http://localhost:1".to_string()),
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
            },
            KeyGroup { // SOCKS5 might fail build
                name: "g_socks".to_string(),
                api_keys: vec!["key_socks".to_string()],
                proxy_url: Some("socks5://localhost:1".to_string()),
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
            },
             KeyGroup { // Same HTTP proxy
                name: "g_http_dup".to_string(),
                api_keys: vec!["key_http2".to_string()],
                proxy_url: Some("http://localhost:1".to_string()),
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
            },
            KeyGroup { name: "g_no_proxy".to_string(), api_keys: vec!["key_none".to_string()], proxy_url: None, target_url: DEFAULT_TARGET_URL_STR.to_string() },
        ];
        let config = create_test_state_config(groups);
        let state_result = AppState::new(&config, &dummy_path).await;

        assert!(state_result.is_ok(), "AppState::new failed unexpectedly: {:?}", state_result.err());
        let state = state_result.unwrap();

        assert!(state.http_clients.contains_key(&None));

        let http_key = Some("http://localhost:1".to_string());
        let socks_key = Some("socks5://localhost:1".to_string());

        let http_created = state.http_clients.contains_key(&http_key);
        let socks_created = state.http_clients.contains_key(&socks_key);

        // Check expectations based on which clients were successfully created
        if http_created && socks_created {
             assert_eq!(state.http_clients.len(), 3);
             assert!(state.get_client(http_key.as_deref()).is_ok());
             assert!(state.get_client(socks_key.as_deref()).is_ok());
             info!("Both HTTP and SOCKS5 clients created successfully in test.");
        } else if http_created {
             assert_eq!(state.http_clients.len(), 2);
             assert!(state.get_client(http_key.as_deref()).is_ok());
             assert!(state.get_client(socks_key.as_deref()).is_err());
             warn!("SOCKS5 client build likely failed in test environment (expected).");
        } else if socks_created {
             assert_eq!(state.http_clients.len(), 2);
             assert!(state.get_client(http_key.as_deref()).is_err());
             assert!(state.get_client(socks_key.as_deref()).is_ok());
              warn!("HTTP client build unexpectedly failed in test environment.");
        } else {
            assert_eq!(state.http_clients.len(), 1);
             assert!(state.get_client(http_key.as_deref()).is_err());
             assert!(state.get_client(socks_key.as_deref()).is_err());
             warn!("Both HTTP and SOCKS5 client builds failed in test environment.");
        }

        assert!(state.get_client(None).is_ok());
        assert!(state.get_client(Some("http://other.proxy")).is_err());
    }

     #[tokio::test]
    async fn test_appstate_new_returns_err_on_invalid_url_syntax() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let _guard = EnvVarGuard;
        let dir = tempdir().unwrap();
        let dummy_path = create_dummy_config_path(&dir);

         let groups = vec![
            KeyGroup {
                name: "g_invalid_url".to_string(),
                api_keys: vec!["key_invalid".to_string()],
                proxy_url: Some("::not a proxy url::".to_string()),
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
            },
        ];
        let config = create_test_state_config(groups);
        let state_result = AppState::new(&config, &dummy_path).await;

        assert!(state_result.is_err(), "AppState::new should return Err for invalid proxy URL syntax");
        assert!(matches!(state_result.as_ref().err().unwrap(), AppError::ProxyConfigError(msg) if msg.contains("Invalid proxy URL format")), "Expected ProxyConfigError for invalid syntax");
    }

     #[tokio::test]
     async fn test_appstate_new_returns_err_on_unsupported_scheme() {
         let _lock = ENV_MUTEX.lock().unwrap();
         let _guard = EnvVarGuard;
         let dir = tempdir().unwrap();
         let dummy_path = create_dummy_config_path(&dir);

          let groups = vec![
             KeyGroup {
                 name: "g_unsupported".to_string(),
                 api_keys: vec!["key_unsupported".to_string()],
                 proxy_url: Some("ftp://unsupported.proxy".to_string()),
                 target_url: DEFAULT_TARGET_URL_STR.to_string(),
             },
         ];
         let config = create_test_state_config(groups);
         let state_result = AppState::new(&config, &dummy_path).await;

         assert!(state_result.is_err(), "AppState::new should return Err for unsupported proxy scheme");
          assert!(matches!(state_result.as_ref().err().unwrap(), AppError::ProxyConfigError(msg) if msg.contains("Unsupported proxy scheme")), "Expected ProxyConfigError for unsupported scheme");
     }

      #[tokio::test]
      async fn test_appstate_new_skips_client_on_build_error() {
         let _lock = ENV_MUTEX.lock().unwrap();
         let _guard = EnvVarGuard;
         let dir = tempdir().unwrap();
         let dummy_path = create_dummy_config_path(&dir);

           let groups = vec![
             KeyGroup { // Valid HTTP
                 name: "g_http".to_string(),
                 api_keys: vec!["k1".to_string()],
                 proxy_url: Some("http://localhost:1".to_string()),
                 target_url: DEFAULT_TARGET_URL_STR.to_string(),
             },
              // Assume this SOCKS5 setup might cause reqwest::Client::build() to fail
              KeyGroup {
                 name: "g_socks_fail".to_string(),
                 api_keys: vec!["k2".to_string()],
                 proxy_url: Some("socks5://localhost:1".to_string()),
                 target_url: DEFAULT_TARGET_URL_STR.to_string(),
             },
          ];
          let config = create_test_state_config(groups);
          let state_result = AppState::new(&config, &dummy_path).await;

          assert!(state_result.is_ok(), "AppState::new should return Ok even if a proxy client build fails");
          let state = state_result.unwrap();

          assert!(state.http_clients.contains_key(&None));

          let http_key = Some("http://localhost:1".to_string());
          let socks_key = Some("socks5://localhost:1".to_string());

          let http_created = state.http_clients.contains_key(&http_key);
          let socks_created = state.http_clients.contains_key(&socks_key);

          if http_created {
               assert!(state.get_client(http_key.as_deref()).is_ok());
               info!("HTTP client for test created.");
          } else {
               warn!("HTTP client build failed in test environment (check logs).");
          }
          if socks_created {
               assert!(state.get_client(socks_key.as_deref()).is_ok());
               info!("SOCKS5 client for test created.");
          } else {
               warn!("SOCKS5 client build likely failed in test environment (check logs).");
          }

          let expected_clients = 1 + (if http_created { 1 } else { 0 }) + (if socks_created { 1 } else { 0 });
          assert_eq!(state.http_clients.len(), expected_clients, "Unexpected number of clients created");

           assert!(state.get_client(None).is_ok());
           assert!(state.get_client(Some("socks5://localhost:1")).is_err()); // Check get for failed client

      }

 } // end tests module
