// src/state.rs

use crate::config::AppConfig;
use crate::error::{AppError, ProxyConfigErrorData, ProxyConfigErrorKind, Result}; // Ensured Proxy types are imported
use crate::key_manager::KeyManager;
use reqwest::{Client, ClientBuilder, Proxy};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;
use tracing::Instrument;
use tracing::{debug, error, info, instrument, warn}; // Base tracing macros // Explicitly import the Instrument trait

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
    #[instrument(level = "info", skip(config, config_path), fields(config.path = %config_path.display()))]
    pub async fn new(config: &AppConfig, config_path: &Path) -> Result<Self> {
        info!("Creating shared AppState: Initializing KeyManager and HTTP clients...");
        let key_manager = KeyManager::new(config, config_path).await; // KeyManager init logs its progress
        let mut http_clients = HashMap::new();

        // Determine connection pool size based on key count, with a minimum floor
        let total_key_count: usize = config
            .groups
            .iter()
            .flat_map(|g| &g.api_keys)
            .filter(|k| !k.trim().is_empty())
            .count()
            .max(10); // Ensure at least 10 connections possible even with few keys
        debug!(
            pool.max_idle_per_host = total_key_count,
            "Calculated max idle connections per host"
        );

        // Centralized client configuration function
        let configure_builder = |builder: ClientBuilder| -> ClientBuilder {
            builder
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(300)) // Overall request timeout
                .pool_idle_timeout(Duration::from_secs(90)) // Keep idle connections open for 90s
                .pool_max_idle_per_host(total_key_count) // Adjust pool size based on keys
                .tcp_keepalive(Some(Duration::from_secs(60))) // Enable TCP keepalive
                                                              // Add user-agent?
                                                              // .user_agent(format!("gemini-proxy/{}", env!("CARGO_PKG_VERSION")))
        };

        // 1. Create the base client (no proxy) - this MUST succeed
        let base_client_result = configure_builder(Client::builder()).build();
        let base_client = match base_client_result {
            Ok(client) => client,
            Err(e) => {
                // Structured error for base client failure - this is fatal
                error!(error = ?e, "Failed to build base HTTP client (no proxy). This is required. Exiting.");
                // Map error explicitly, including None proxy context
                return Err(AppError::HttpClientBuildError {
                    source: e,
                    proxy_url: None,
                });
            }
        };
        http_clients.insert(None, base_client);
        info!(client.type = "base", "Base HTTP client (no proxy) created successfully.");

        // 2. Collect unique proxy URLs from the configuration
        let unique_proxy_urls: HashSet<String> = config
            .groups
            .iter()
            .filter_map(|g| g.proxy_url.as_ref()) // Get Option<&String>
            .filter(|url_str| !url_str.trim().is_empty()) // Filter out empty strings
            .cloned() // Clone the String
            .collect();
        debug!(
            proxy.count = unique_proxy_urls.len(),
            ?unique_proxy_urls,
            "Found unique proxy URLs for client creation"
        );

        // 3. Create clients for each unique proxy URL
        for proxy_url_str in unique_proxy_urls {
            // Create a span for each proxy client creation attempt
            let proxy_span = tracing::info_span!("create_proxy_client", proxy.url = %proxy_url_str);
            let client_result: Result<Client> = async {
                 // Parse URL first, map error to specific ProxyConfigErrorKind
                 let parsed_proxy_url = Url::parse(&proxy_url_str).map_err(|e| {
                     // Structured error log for URL parsing
                     error!(error = %e, "Failed to parse proxy URL string.");
                     AppError::ProxyConfigError(ProxyConfigErrorData {
                         url: proxy_url_str.clone(),
                         kind: ProxyConfigErrorKind::UrlParse(e),
                     })
                 })?;

                let scheme = parsed_proxy_url.scheme().to_lowercase();
                debug!(proxy.scheme = %scheme, "Parsed proxy scheme");

                 // Create proxy object, map errors to specific ProxyConfigErrorKind
                 let proxy = match scheme.as_str() {
                     "http" => Proxy::http(&proxy_url_str).map_err(|e| {
                         // Log error detail
                         error!(error = %e, proxy.scheme = %scheme, "Invalid HTTP proxy definition");
                         AppError::ProxyConfigError(ProxyConfigErrorData {
                             url: proxy_url_str.clone(),
                             kind: ProxyConfigErrorKind::InvalidDefinition(e.to_string()),
                         })
                     }),
                     "https" => Proxy::https(&proxy_url_str).map_err(|e| {
                          // Log error detail
                          error!(error = %e, proxy.scheme = %scheme, "Invalid HTTPS proxy definition");
                          AppError::ProxyConfigError(ProxyConfigErrorData {
                             url: proxy_url_str.clone(),
                             kind: ProxyConfigErrorKind::InvalidDefinition(e.to_string()),
                         })
                     }),
                     "socks5" => Proxy::all(&proxy_url_str).map_err(|e| {
                          // Log error detail
                          error!(error = %e, proxy.scheme = %scheme, "Invalid SOCKS5 proxy definition");
                          AppError::ProxyConfigError(ProxyConfigErrorData {
                             url: proxy_url_str.clone(),
                             kind: ProxyConfigErrorKind::InvalidDefinition(e.to_string()),
                         })
                     }),
                     _ => {
                          // Log unsupported scheme error
                          error!(proxy.scheme = %scheme, "Unsupported proxy scheme");
                          Err(AppError::ProxyConfigError(ProxyConfigErrorData {
                             url: proxy_url_str.clone(),
                             kind: ProxyConfigErrorKind::UnsupportedScheme(scheme.to_string()),
                         }))
                     },
                 }?;
                 debug!("Proxy object created successfully");

                 // Build the client with the proxy
                 configure_builder(Client::builder())
                    .proxy(proxy)
                    .build()
                     // Map build error with proxy context
                     .map_err(|e| {
                         // Structured error log for client build failure
                         error!(proxy.scheme = %scheme, error = ?e, "Failed to build reqwest client for proxy.");
                         AppError::HttpClientBuildError { source: e, proxy_url: Some(proxy_url_str.clone()) }
                     })
            }.instrument(proxy_span).await; // Instrument the async block

            // Handle the result of client creation
            match client_result {
                Ok(proxy_client) => {
                    // Structured success log
                    info!(proxy.url = %proxy_url_str, "HTTP client created successfully for proxy.");
                    http_clients.insert(Some(proxy_url_str.clone()), proxy_client);
                }
                Err(e) => {
                    // Log errors based on their type, ensuring proxy_url is included
                    match e {
                        AppError::ProxyConfigError(_) => {
                            // Already logged within the async block, re-log as critical error for AppState
                            error!(proxy.url = %proxy_url_str, error = ?e, "Critical proxy configuration error. Aborting AppState creation.");
                            return Err(e); // Fail fast on config errors
                        }
                        AppError::HttpClientBuildError {
                            ref source,
                            proxy_url: Some(ref url),
                        } => {
                            // Match specific error
                            // Already logged within the async block, re-log as warning for skipping
                            warn!(proxy.url = %url, error = ?source, "Skipping client creation for this proxy due to build error. Groups using this proxy might fail.");
                            // Log and continue for build errors
                        }
                        _ => {
                            // Catch-all for unexpected errors during creation for this proxy
                            error!(proxy.url = %proxy_url_str, error = ?e, "Unexpected error during proxy client creation. Aborting AppState creation.");
                            return Err(e); // Fail on other unexpected errors
                        }
                    }
                }
            }
        }

        // Log final client count
        info!(
            client.count = http_clients.len(),
            "Finished initializing HTTP clients."
        );
        Ok(Self {
            key_manager,
            http_clients,
        })
    }

    /// Returns a reference to the appropriate HTTP client.
    /// Logs an error if the requested client (identified by proxy_url Option) is not found.
    #[instrument(level = "debug", skip(self), fields(proxy.url = ?proxy_url))]
    #[inline]
    pub fn get_client(&self, proxy_url: Option<&str>) -> Result<&Client> {
        let key = proxy_url.map(String::from); // Create Option<String> key for HashMap lookup
        match self.http_clients.get(&key) {
            Some(client) => {
                debug!("Retrieved HTTP client");
                Ok(client)
            }
            None => {
                let msg = if let Some(p_url) = proxy_url {
                    format!("Requested HTTP client for proxy '{}' was not found/initialized in AppState.", p_url)
                } else {
                    "Requested base HTTP client (None proxy) was unexpectedly missing.".to_string()
                };
                // Structured error log
                error!(proxy.url = ?proxy_url, error.message = %msg, "HTTP client lookup failed");
                Err(AppError::Internal(msg)) // Return internal error
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{KeyGroup, ServerConfig};
    use crate::error::ProxyConfigErrorKind; // Import the kind enum for tests
    use std::fs::File;
    use tempfile::tempdir;
    use tracing::warn; // Import warn for logging in tests specifically

    const DEFAULT_TARGET_URL_STR: &str = "https://generativelanguage.googleapis.com";

    fn create_test_state_config(groups: Vec<KeyGroup>) -> AppConfig {
        AppConfig {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 8080,
            },
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
        assert!(state.http_clients.contains_key(&None)); // Base client only
        assert!(state.get_client(None).is_ok());
        assert!(state.get_client(Some("http://nonexistent.proxy")).is_err());
    }

    #[tokio::test]
    async fn test_appstate_new_with_valid_proxies() {
        let dir = tempdir().unwrap();
        let dummy_path = create_dummy_config_path(&dir);

        // Mock server or use potentially invalid ports to test resilience
        let http_proxy_url = "http://127.0.0.1:34567"; // Use a likely free port
        let socks_proxy_url = "socks5://127.0.0.1:34568"; // Use a likely free port

        let groups = vec![
            KeyGroup {
                name: "g_http".to_string(),
                api_keys: vec!["key_http".to_string()],
                proxy_url: Some(http_proxy_url.to_string()),
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
            },
            KeyGroup {
                name: "g_socks".to_string(),
                api_keys: vec!["key_socks".to_string()],
                proxy_url: Some(socks_proxy_url.to_string()),
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
            },
            KeyGroup {
                // Same HTTP proxy, should reuse client map entry
                name: "g_http_dup".to_string(),
                api_keys: vec!["key_http2".to_string()],
                proxy_url: Some(http_proxy_url.to_string()),
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
            },
            KeyGroup {
                name: "g_no_proxy".to_string(),
                api_keys: vec!["key_none".to_string()],
                proxy_url: None,
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
            },
        ];
        let config = create_test_state_config(groups);
        let state_result = AppState::new(&config, &dummy_path).await;

        // AppState::new should succeed even if proxy servers aren't actually running
        assert!(
            state_result.is_ok(),
            "AppState::new failed unexpectedly: {:?}",
            state_result.err()
        );
        let state = state_result.unwrap();

        assert!(state.http_clients.contains_key(&None)); // Base client must exist

        let http_key = Some(http_proxy_url.to_string());
        let socks_key = Some(socks_proxy_url.to_string());

        let http_created = state.http_clients.contains_key(&http_key);
        let socks_created = state.http_clients.contains_key(&socks_key);

        // We expect all clients to be created successfully if URLs are valid syntactically
        assert!(http_created, "HTTP proxy client was not created");
        assert!(socks_created, "SOCKS5 proxy client was not created");
        assert_eq!(
            state.http_clients.len(),
            3,
            "Expected Base + HTTP + SOCKS clients"
        ); // 1 base + 2 unique proxies

        // Verify get_client behavior
        assert!(
            state.get_client(http_key.as_deref()).is_ok(),
            "get_client failed for created HTTP proxy"
        );
        assert!(
            state.get_client(socks_key.as_deref()).is_ok(),
            "get_client failed for created SOCKS5 proxy"
        );
        assert!(state.get_client(None).is_ok()); // Check base client retrieval
        assert!(state.get_client(Some("http://other.proxy")).is_err()); // Check non-existent proxy
    }

    #[tokio::test]
    async fn test_appstate_new_returns_err_on_invalid_url_syntax() {
        let dir = tempdir().unwrap();
        let dummy_path = create_dummy_config_path(&dir);

        let groups = vec![KeyGroup {
            name: "g_invalid_url".to_string(),
            api_keys: vec!["key_invalid".to_string()],
            proxy_url: Some("::not a proxy url::".to_string()), // Invalid syntax
            target_url: DEFAULT_TARGET_URL_STR.to_string(),
        }];
        let config = create_test_state_config(groups);
        let state_result = AppState::new(&config, &dummy_path).await;

        assert!(
            state_result.is_err(),
            "AppState::new should return Err for invalid proxy URL syntax"
        );
        // Check for the correct error variant and kind
        assert!(
            matches!(state_result.as_ref().err().unwrap(), AppError::ProxyConfigError(data) if matches!(data.kind, ProxyConfigErrorKind::UrlParse(_))),
            "Expected ProxyConfigError with UrlParse kind"
        );
    }

    #[tokio::test]
    async fn test_appstate_new_returns_err_on_unsupported_scheme() {
        let dir = tempdir().unwrap();
        let dummy_path = create_dummy_config_path(&dir);

        let groups = vec![KeyGroup {
            name: "g_unsupported".to_string(),
            api_keys: vec!["key_unsupported".to_string()],
            proxy_url: Some("ftp://unsupported.proxy".to_string()), // Unsupported scheme
            target_url: DEFAULT_TARGET_URL_STR.to_string(),
        }];
        let config = create_test_state_config(groups);
        let state_result = AppState::new(&config, &dummy_path).await;

        assert!(
            state_result.is_err(),
            "AppState::new should return Err for unsupported proxy scheme"
        );
        // Check for the correct error variant and kind
        assert!(
            matches!(state_result.as_ref().err().unwrap(), AppError::ProxyConfigError(data) if matches!(data.kind, ProxyConfigErrorKind::UnsupportedScheme(_))),
            "Expected ProxyConfigError with UnsupportedScheme kind"
        );
    }

    // Test where Client::build() itself might fail (less common, requires specific setup or invalid proxy def)
    // This test might be flaky depending on environment/reqwest version behavior
    #[tokio::test]
    async fn test_appstate_new_skips_client_on_build_error() {
        // This test simulates a reqwest build failure for one proxy,
        // but AppState creation should still succeed with other valid clients.
        // We use a syntactically valid but likely non-functional SOCKS5 URL.
        let dir = tempdir().unwrap();
        let dummy_path = create_dummy_config_path(&dir);

        let groups = vec![
            KeyGroup {
                // Valid HTTP
                name: "g_http_ok".to_string(),
                api_keys: vec!["k1".to_string()],
                proxy_url: Some("http://127.0.0.1:34569".to_string()), // Likely free port
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
            },
            // Use a socks URL that might cause build issues or is hard to resolve
            KeyGroup {
                name: "g_socks_fail_build".to_string(),
                api_keys: vec!["k2".to_string()],
                // Provide a URL that might fail build if socks feature isn't compiled correctly or has issues
                proxy_url: Some("socks5://invalid-host-that-causes-build-error:1080".to_string()),
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
            },
        ];
        let config = create_test_state_config(groups);
        let state_result = AppState::new(&config, &dummy_path).await;

        // AppState::new should succeed by skipping the failing client build
        assert!(
            state_result.is_ok(),
            "AppState::new should return Ok even if a proxy client build fails"
        );
        let state = state_result.unwrap();

        assert!(state.http_clients.contains_key(&None)); // Base client

        let http_key = Some("http://127.0.0.1:34569".to_string());
        let socks_key = Some("socks5://invalid-host-that-causes-build-error:1080".to_string());

        let http_created = state.http_clients.contains_key(&http_key);
        let socks_created = state.http_clients.contains_key(&socks_key);

        // We expect HTTP client to be created, but SOCKS client might fail build
        assert!(http_created, "Valid HTTP client should have been created");
        assert!(state.get_client(http_key.as_deref()).is_ok());

        if socks_created {
            // If it was created, get_client should succeed
            assert!(state.get_client(socks_key.as_deref()).is_ok());
            warn!("SOCKS client build succeeded unexpectedly in test - test might not cover build failure path");
        } else {
            // If it wasn't created, get_client should fail
            assert!(state.get_client(socks_key.as_deref()).is_err());
            // Info log happens inside AppState::new if build fails
        }

        let expected_clients =
            1 + (if http_created { 1 } else { 0 }) + (if socks_created { 1 } else { 0 });
        assert_eq!(
            state.http_clients.len(),
            expected_clients,
            "Unexpected number of clients created"
        );
    }
} // end tests module
