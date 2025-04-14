// src/state.rs

use crate::config::AppConfig;
use crate::error::{AppError, Result};
use crate::key_manager::KeyManager;
use reqwest::{Client, Proxy, ClientBuilder}; // Added ClientBuilder explicitly
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tracing::{error, info, warn};
use url::Url;

/// Represents the shared application state that is accessible by all Axum handlers.
///
/// This struct holds instances of shared resources like the key manager and a pool
/// of pre-configured HTTP clients (one for direct connections, one for each unique proxy).
/// It is typically wrapped in an `Arc` for thread-safe sharing across asynchronous tasks.
#[derive(Debug)]
pub struct AppState {
    /// Manages the API keys and their states. Made public for handler access.
    pub key_manager: KeyManager,
    /// A map of HTTP clients. The key is `None` for the direct client
    /// or `Some(proxy_url)` for clients configured with a specific proxy.
    http_clients: HashMap<Option<String>, Client>,
}

impl AppState {
    /// Creates a new `AppState`.
    ///
    /// Initializes the KeyManager and pre-builds HTTP clients for direct connections
    /// and for each unique proxy URL found in the configuration.
    pub fn new(config: &AppConfig) -> Result<Self> {
        info!("Creating shared AppState: Initializing KeyManager and HTTP clients...");

        // --- Key Manager Initialization ---
        let key_manager = KeyManager::new(config);

        // --- HTTP Client Initialization ---
        let mut http_clients = HashMap::new();

        // Calculate total keys for pool size estimation (can be shared across clients)
        let total_key_count: usize = config
            .groups
            .iter()
            .flat_map(|g| &g.api_keys)
            .filter(|k| !k.trim().is_empty())
            .count()
            .max(10); // Use max(10) as a reasonable minimum pool size

        // Helper function to configure common builder settings
        let configure_builder = |builder: ClientBuilder| -> ClientBuilder {
             builder
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(300)) // 5 minutes total timeout
                .pool_idle_timeout(Duration::from_secs(90))
                .pool_max_idle_per_host(total_key_count) // Pool size based on keys
                .tcp_keepalive(Some(Duration::from_secs(60)))
        };

        // 1. Create the base client (no proxy)
        let base_client = configure_builder(Client::builder()) // Apply common settings
            .build()
            .map_err(AppError::from)?;
        http_clients.insert(None, base_client);
        info!("Base HTTP client (no proxy) created successfully.");

        // 2. Collect unique, valid proxy URLs from the config
        let unique_proxy_urls: HashSet<String> = config
            .groups
            .iter()
            .filter_map(|g| g.proxy_url.as_ref()) // Get only Some(proxy_url)
            .filter(|url_str| !url_str.trim().is_empty()) // Filter out empty strings
            .cloned() // Clone the String
            .collect();

        // 3. Create clients for each unique proxy URL
        for proxy_url_str in unique_proxy_urls {
            match Url::parse(&proxy_url_str) {
                Ok(parsed_proxy_url) => {
                    let scheme = parsed_proxy_url.scheme().to_lowercase();
                    let proxy_obj_result = match scheme.as_str() {
                        "http" => Proxy::http(&proxy_url_str),
                        "https" => Proxy::https(&proxy_url_str),
                        "socks5" => Proxy::all(&proxy_url_str),
                        _ => {
                            warn!(proxy_url = %proxy_url_str, scheme = %scheme, "Unsupported proxy scheme found during AppState initialization. Skipping client creation for this proxy.");
                            continue; // Skip this proxy URL
                        }
                    };

                    match proxy_obj_result {
                        Ok(proxy) => {
                            // Create a new builder instance inside the loop
                            match configure_builder(Client::builder()) // Apply common settings to new builder
                                .proxy(proxy)
                                .build() {
                                    Ok(proxy_client) => {
                                        info!(proxy_url = %proxy_url_str, "HTTP client created successfully for proxy.");
                                        http_clients.insert(Some(proxy_url_str.clone()), proxy_client);
                                    }
                                    Err(e) => {
                                         error!(proxy_url = %proxy_url_str, error = %e, "Failed to build reqwest client for proxy during AppState initialization. Requests needing this proxy might fail.");
                                        // Don't insert a failed client
                                    }
                            }
                        }
                        Err(e) => {
                             warn!(proxy_url = %proxy_url_str, scheme=%scheme, error = %e, "Failed to create proxy object from URL during AppState initialization. Skipping client creation for this proxy.");
                             // Don't insert a failed client
                        }
                    }
                }
                Err(e) => {
                    warn!(proxy_url = %proxy_url_str, error = %e, "Failed to parse proxy URL string during AppState initialization. Skipping client creation for this proxy.");
                     // Don't insert a failed client
                }
            }
        }

        info!("Finished initializing {} HTTP client(s).", http_clients.len());

        Ok(Self {
            key_manager,
            http_clients,
        })
    }

    /// Returns a reference to the appropriate pre-configured HTTP client based on the proxy URL.
    ///
    /// Returns the base client if `proxy_url` is `None`.
    /// Returns the specific client configured for the given proxy URL if `proxy_url` is `Some`.
    /// Returns an `AppError::Internal` if a client for the requested proxy was not successfully initialized or found.
    #[inline]
    pub fn get_client(&self, proxy_url: Option<&str>) -> Result<&Client> {
        let key = proxy_url.map(String::from); // Convert Option<&str> to Option<String> for lookup
        self.http_clients.get(&key).ok_or_else(|| {
            error!(proxy_url = ?proxy_url, "Requested HTTP client for proxy was not found in AppState. This might indicate an initialization issue or an unexpected proxy URL requested at runtime.");
            AppError::Internal(format!(
                "Client for proxy {:?} not found/initialized",
                proxy_url
            ))
        })
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{KeyGroup, ServerConfig};

    // Helper to create a basic AppConfig for testing AppState
    fn create_test_state_config(groups: Vec<KeyGroup>) -> AppConfig {
        AppConfig {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 8080,
            },
            groups,
        }
    }

    #[tokio::test]
    async fn test_appstate_new_no_proxies() {
        let groups = vec![KeyGroup {
            name: "g1".to_string(),
            api_keys: vec!["key1".to_string()],
            proxy_url: None, // No proxy
            target_url: "target1".to_string(),
        }];
        let config = create_test_state_config(groups);
        let state_result = AppState::new(&config);

        assert!(state_result.is_ok());
        let state = state_result.unwrap();

        // Should have 1 client: the base client (key = None)
        assert_eq!(state.http_clients.len(), 1);
        assert!(state.http_clients.contains_key(&None));

        // Test get_client for None
        let client = state.get_client(None);
        assert!(client.is_ok());

        // Test get_client for a non-existent proxy
        let client_err = state.get_client(Some("http://nonexistent.proxy"));
        assert!(client_err.is_err());
        assert!(matches!(client_err, Err(AppError::Internal(_))));
    }

    #[tokio::test]
    async fn test_appstate_new_with_valid_proxies() {
        let groups = vec![
            KeyGroup { // Group with http proxy
                name: "g_http".to_string(),
                api_keys: vec!["key_http".to_string()],
                proxy_url: Some("http://proxy.example.com:8080".to_string()),
                target_url: "target_http".to_string(),
            },
            KeyGroup { // Group with socks5 proxy
                name: "g_socks".to_string(),
                api_keys: vec!["key_socks".to_string()],
                proxy_url: Some("socks5://user:pass@another.proxy:1080".to_string()),
                target_url: "target_socks".to_string(),
            },
             KeyGroup { // Group with the SAME http proxy (should only create one client)
                name: "g_http_dup".to_string(),
                api_keys: vec!["key_http2".to_string()],
                proxy_url: Some("http://proxy.example.com:8080".to_string()),
                target_url: "target_http_dup".to_string(),
            },
            KeyGroup { // Group without proxy
                name: "g_no_proxy".to_string(),
                api_keys: vec!["key_none".to_string()],
                proxy_url: None,
                target_url: "target_none".to_string(),
            },
        ];
        let config = create_test_state_config(groups);
        let state_result = AppState::new(&config);

        assert!(state_result.is_ok());
        let state = state_result.unwrap();

        // Should have 3 clients: base (None), http proxy, socks5 proxy
        // Expected 3 clients: base (None), http proxy, socks5 proxy
        // NOTE: This test might fail if the environment lacks necessary dependencies or configuration
        // for reqwest's SOCKS5 support, causing the SOCKS5 client build to fail silently in AppState::new.
        assert_eq!(state.http_clients.len(), 3, "Expected 3 clients (base, http, socks5)");
        assert!(state.http_clients.contains_key(&None));
        assert!(state.http_clients.contains_key(&Some("http://proxy.example.com:8080".to_string())));
        assert!(state.http_clients.contains_key(&Some("socks5://user:pass@another.proxy:1080".to_string())), "SOCKS5 client key missing");
        assert!(state.http_clients.contains_key(&None));
        assert!(state.http_clients.contains_key(&Some("http://proxy.example.com:8080".to_string())));
        

        // Test get_client for None
        assert!(state.get_client(None).is_ok());
        // Test get_client for http proxy
        assert!(state.get_client(Some("http://proxy.example.com:8080")).is_ok());
        // Test get_client for socks5 proxy
        assert!(state.get_client(Some("socks5://user:pass@another.proxy:1080")).is_ok());
         // Test get_client for a non-existent proxy
        assert!(state.get_client(Some("http://other.proxy")).is_err());
    }

    #[tokio::test]
    async fn test_appstate_new_with_invalid_and_unsupported_proxies() {
         let groups = vec![
            KeyGroup { // Valid http proxy
                name: "g_http".to_string(),
                api_keys: vec!["key_http".to_string()],
                proxy_url: Some("http://good.proxy:8080".to_string()),
                target_url: "target_http".to_string(),
            },
            KeyGroup { // Invalid URL syntax
                name: "g_invalid_url".to_string(),
                api_keys: vec!["key_invalid".to_string()],
                proxy_url: Some("::not a proxy url::".to_string()),
                target_url: "target_invalid".to_string(),
            },
             KeyGroup { // Unsupported scheme (ftp)
                name: "g_unsupported".to_string(),
                api_keys: vec!["key_unsupported".to_string()],
                proxy_url: Some("ftp://unsupported.proxy".to_string()),
                target_url: "target_unsupported".to_string(),
            },
             KeyGroup { // Empty proxy string (should be ignored)
                name: "g_empty".to_string(),
                api_keys: vec!["key_empty".to_string()],
                proxy_url: Some("  ".to_string()),
                target_url: "target_empty".to_string(),
            },
        ];
        let config = create_test_state_config(groups);
        let state_result = AppState::new(&config); // Should succeed, but log warnings

        assert!(state_result.is_ok());
        let state = state_result.unwrap();

        // Should have 2 clients: base (None) and the valid http proxy
        // Invalid/unsupported ones should be skipped during initialization.
        assert_eq!(state.http_clients.len(), 2);
        assert!(state.http_clients.contains_key(&None));
        assert!(state.http_clients.contains_key(&Some("http://good.proxy:8080".to_string())));

        // Check that getting clients for the skipped proxies results in an error
        assert!(state.get_client(Some("::not a proxy url::")).is_err());
        assert!(state.get_client(Some("ftp://unsupported.proxy")).is_err());
        assert!(state.get_client(Some("  ")).is_err()); // Getting with the empty string should also fail

        // Getting the valid ones should work
        assert!(state.get_client(None).is_ok());
        assert!(state.get_client(Some("http://good.proxy:8080")).is_ok());
    }
}
