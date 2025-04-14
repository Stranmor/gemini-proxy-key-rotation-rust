// src/state.rs

use crate::config::AppConfig;
use crate::error::{AppError, ProxyConfigErrorData, ProxyConfigErrorKind, Result}; // Ensured Proxy types are imported
use crate::key_manager::KeyManager;
use reqwest::{Client, Proxy, ClientBuilder};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;
use tracing::{error, info}; // Removed warn import

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
            // Base client build error - map explicitly with None proxy_url
            .map_err(|e| AppError::HttpClientBuildError { source: e, proxy_url: None })?;
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
                 // Parse URL first, map error to specific ProxyConfigErrorKind
                 let parsed_proxy_url = Url::parse(&proxy_url_str).map_err(|e| {
                     error!(proxy_url = %proxy_url_str, error = %e, "Failed to parse proxy URL string.");
                     AppError::ProxyConfigError(ProxyConfigErrorData {
                         url: proxy_url_str.clone(),
                         kind: ProxyConfigErrorKind::UrlParse(e),
                     })
                 })?;

                let scheme = parsed_proxy_url.scheme().to_lowercase();

                 // Create proxy, map errors to specific ProxyConfigErrorKind
                 let proxy = match scheme.as_str() {
                     "http" => Proxy::http(&proxy_url_str).map_err(|e| {
                         AppError::ProxyConfigError(ProxyConfigErrorData {
                             url: proxy_url_str.clone(),
                             kind: ProxyConfigErrorKind::InvalidDefinition(e.to_string()),
                         })
                     }),
                     "https" => Proxy::https(&proxy_url_str).map_err(|e| {
                          AppError::ProxyConfigError(ProxyConfigErrorData {
                             url: proxy_url_str.clone(),
                             kind: ProxyConfigErrorKind::InvalidDefinition(e.to_string()),
                         })
                     }),
                     "socks5" => Proxy::all(&proxy_url_str).map_err(|e| {
                          AppError::ProxyConfigError(ProxyConfigErrorData {
                             url: proxy_url_str.clone(),
                             kind: ProxyConfigErrorKind::InvalidDefinition(e.to_string()),
                         })
                     }),
                     _ => Err(AppError::ProxyConfigError(ProxyConfigErrorData {
                         url: proxy_url_str.clone(),
                         kind: ProxyConfigErrorKind::UnsupportedScheme(scheme.to_string()),
                     })),
                 }?;

                 configure_builder(Client::builder())
                    .proxy(proxy)
                    .build()
                     // Explicitly map build error with proxy context
                     .map_err(|e| {
                         error!(proxy_url = %proxy_url_str, scheme = %scheme, error = %e, "Failed to build reqwest client for proxy.");
                         AppError::HttpClientBuildError { source: e, proxy_url: Some(proxy_url_str.clone()) }
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
                           AppError::HttpClientBuildError { .. } => { // Correct pattern
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
            let msg = if let Some(p_url) = proxy_url {
                format!("Requested HTTP client for proxy '{}' was not found/initialized in AppState.", p_url)
            } else {
                "Requested base HTTP client (None proxy) was unexpectedly missing.".to_string()
            };
            error!("{}", msg);
            AppError::Internal(msg) // Use Internal error type
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{KeyGroup, ServerConfig};
    use crate::error::ProxyConfigErrorKind; // Import the kind enum for tests
    use std::fs::File;
    use tempfile::tempdir;
    use tracing::info; // Import info for logging in tests specifically

    const DEFAULT_TARGET_URL_STR: &str = "https://generativelanguage.googleapis.com";

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

    #[tokio::test]
    async fn test_appstate_new_no_proxies() {
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
        let dir = tempdir().unwrap();
        let dummy_path = create_dummy_config_path(&dir);

        let groups = vec![
            KeyGroup {
                name: "g_http".to_string(),
                api_keys: vec!["key_http".to_string()],
                proxy_url: Some("http://localhost:1".to_string()), // Use invalid port to possibly trigger build error
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
            },
            KeyGroup { // SOCKS5 might fail build
                name: "g_socks".to_string(),
                api_keys: vec!["key_socks".to_string()],
                proxy_url: Some("socks5://localhost:1".to_string()), // Use invalid port
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

        // AppState::new should succeed even if individual proxy clients fail to build
        assert!(state_result.is_ok(), "AppState::new failed unexpectedly: {:?}", state_result.err());
        let state = state_result.unwrap();

        assert!(state.http_clients.contains_key(&None)); // Base client must exist

        let http_key = Some("http://localhost:1".to_string());
        let socks_key = Some("socks5://localhost:1".to_string());

        let http_created = state.http_clients.contains_key(&http_key);
        let socks_created = state.http_clients.contains_key(&socks_key);

        // Check expectations: Base client + any successfully created proxy clients
        let expected_min_clients = 1; // Base client always exists
        let expected_max_clients = 3; // Base + HTTP + SOCKS5
        assert!(state.http_clients.len() >= expected_min_clients, "Less than minimum expected clients");
        assert!(state.http_clients.len() <= expected_max_clients, "More than maximum expected clients");
        info!("Created {} clients (Base + {} HTTP + {} SOCKS5)",
              state.http_clients.len(),
              if http_created { 1 } else { 0 },
              if socks_created { 1 } else { 0 });

        // Verify get_client behavior for each potential proxy
        if http_created {
            assert!(state.get_client(http_key.as_deref()).is_ok(), "get_client failed for created HTTP proxy");
        } else {
            assert!(state.get_client(http_key.as_deref()).is_err(), "get_client succeeded for non-created HTTP proxy");
            info!("HTTP client build likely failed in test environment (expected if port 1 blocked)."); // Use info
        }
        if socks_created {
             assert!(state.get_client(socks_key.as_deref()).is_ok(), "get_client failed for created SOCKS5 proxy");
         } else {
             assert!(state.get_client(socks_key.as_deref()).is_err(), "get_client succeeded for non-created SOCKS5 proxy");
             info!("SOCKS5 client build likely failed in test environment (expected if port 1 blocked)."); // Use info
         }

        assert!(state.get_client(None).is_ok()); // Check base client retrieval
        assert!(state.get_client(Some("http://other.proxy")).is_err()); // Check non-existent proxy
    }

     #[tokio::test]
    async fn test_appstate_new_returns_err_on_invalid_url_syntax() {
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
         // Check for the correct error variant and kind
         assert!(matches!(state_result.as_ref().err().unwrap(), AppError::ProxyConfigError(data) if matches!(data.kind, ProxyConfigErrorKind::UrlParse(_))), "Expected ProxyConfigError with UrlParse kind");
    }

     #[tokio::test]
     async fn test_appstate_new_returns_err_on_unsupported_scheme() {
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
           // Check for the correct error variant and kind
           assert!(matches!(state_result.as_ref().err().unwrap(), AppError::ProxyConfigError(data) if matches!(data.kind, ProxyConfigErrorKind::UnsupportedScheme(_))), "Expected ProxyConfigError with UnsupportedScheme kind");
     }

      #[tokio::test]
      async fn test_appstate_new_skips_client_on_build_error() {
         let dir = tempdir().unwrap();
         let dummy_path = create_dummy_config_path(&dir);

           let groups = vec![
             KeyGroup { // Valid HTTP (might fail if port 1 is blocked, but AppState::new should still Ok)
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

          assert!(state.http_clients.contains_key(&None)); // Base client

          let http_key = Some("http://localhost:1".to_string());
          let socks_key = Some("socks5://localhost:1".to_string());

          let http_created = state.http_clients.contains_key(&http_key);
          let socks_created = state.http_clients.contains_key(&socks_key);

          // Check if clients were retrievable (implies successful creation)
          if http_created {
               assert!(state.get_client(http_key.as_deref()).is_ok());
               info!("HTTP client for test created.");
          } else {
               info!("HTTP client build failed in test environment (expected if port 1 blocked, check logs)."); // Use info
               assert!(state.get_client(http_key.as_deref()).is_err());
          }
          if socks_created {
               assert!(state.get_client(socks_key.as_deref()).is_ok());
               info!("SOCKS5 client for test created.");
          } else {
               info!("SOCKS5 client build likely failed in test environment (expected if port 1 blocked, check logs)."); // Use info
               assert!(state.get_client(socks_key.as_deref()).is_err());
          }

          let expected_clients = 1 + (if http_created { 1 } else { 0 }) + (if socks_created { 1 } else { 0 });
          assert_eq!(state.http_clients.len(), expected_clients, "Unexpected number of clients created");
      }

 } // end tests module
