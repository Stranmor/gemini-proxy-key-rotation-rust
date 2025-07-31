// src/state.rs

use crate::admin::SystemInfoCollector;
use crate::config::AppConfig;
use crate::handlers::base::ResponseHandler;
use crate::middleware::rate_limit::RateLimitStore;
use crate::handlers::{
    invalid_api_key::InvalidApiKeyHandler, rate_limit::RateLimitHandler, success::SuccessHandler,
    terminal_error::TerminalErrorHandler,
};
use crate::key_manager::KeyManager;
use crate::metrics::MetricsCollector;
use std::fmt;
use crate::error::{AppError, ProxyConfigErrorData, ProxyConfigErrorKind, Result};
use deadpool_redis::{Config, Pool, Runtime};
use reqwest::{Client, ClientBuilder, Proxy};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, error, info, instrument, warn, Instrument};
use url::Url;

/// Represents the state of a single API key, designed to be stored in Redis.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct KeyState {
    pub key: String,
    pub group_name: String,
    pub is_blocked: bool,
    pub consecutive_failures: u32,
    pub last_failure: Option<chrono::DateTime<chrono::Utc>>,
}

/// Represents the shared application state, accessible by all Axum handlers.
pub struct AppState {
    pub redis_pool: Option<Pool>,
    pub key_manager: Arc<RwLock<KeyManager>>,
    http_clients: Arc<RwLock<HashMap<Option<String>, Arc<Client>>>>,
    pub response_handlers: Arc<Vec<Box<dyn ResponseHandler>>>,
    pub start_time: Instant,
    pub config: Arc<RwLock<AppConfig>>,
    pub system_info: SystemInfoCollector,
    pub config_path: PathBuf,
    pub metrics: Arc<MetricsCollector>,
    pub rate_limit_store: RateLimitStore,
}

impl fmt::Debug for AppState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppState")
            .field("http_clients", &self.http_clients)
            .field("start_time", &self.start_time)
            .field("config", &self.config)
            .field("system_info", &self.system_info)
            .field("config_path", &self.config_path)
            .field("redis_pool_present", &self.redis_pool.is_some())
            .finish_non_exhaustive()
    }
}

/// Configuration for HTTP client connection pooling.
#[derive(Debug, Clone)]
struct ClientPoolConfig {
    max_idle_per_host: usize,
    idle_timeout: Duration,
    keepalive: Duration,
    connect_timeout: Duration,
    request_timeout: Duration,
}

impl ClientPoolConfig {
    fn new(key_count: usize, server_config: &crate::config::ServerConfig) -> Self {
        Self {
            max_idle_per_host: key_count.max(10),
            idle_timeout: Duration::from_secs(90),
            keepalive: Duration::from_secs(60),
            connect_timeout: Duration::from_secs(server_config.connect_timeout_secs),
            request_timeout: Duration::from_secs(server_config.request_timeout_secs),
        }
    }
}

/// Builder for HTTP clients with consistent configuration.
struct HttpClientBuilder {
    pool_config: ClientPoolConfig,
}

impl HttpClientBuilder {
    fn new(pool_config: ClientPoolConfig) -> Self {
        Self { pool_config }
    }

    /// Configures a ClientBuilder with consistent settings.
    fn configure_builder(&self, builder: ClientBuilder) -> ClientBuilder {
        builder
            .connect_timeout(self.pool_config.connect_timeout)
            .timeout(self.pool_config.request_timeout)
            .pool_idle_timeout(self.pool_config.idle_timeout)
            .pool_max_idle_per_host(self.pool_config.max_idle_per_host)
            .tcp_keepalive(Some(self.pool_config.keepalive))
    }

    /// Creates a base HTTP client without proxy.
    fn build_base_client(&self) -> Result<Client> {
        self.configure_builder(Client::builder())
            .build()
                .map_err(|e| {
                error!(error = ?e, "Failed to build base HTTP client (no proxy). This is required.");
                    AppError::HttpClientBuildError {
                        source: e,
                    proxy_url: None,
                    }
                })
        }

    /// Creates an HTTP client with the specified proxy.
    async fn build_proxy_client(&self, proxy_url: &str) -> Result<Client> {
        let proxy_span = tracing::info_span!("create_proxy_client", proxy.url = %proxy_url);
        
        async {
            let parsed_proxy_url = Url::parse(proxy_url).map_err(|e| {
                error!(error = %e, "Failed to parse proxy URL string.");
                AppError::ProxyConfigError(ProxyConfigErrorData {
                    url: proxy_url.to_string(),
                    kind: ProxyConfigErrorKind::UrlParse(e),
                })
            })?;

            let scheme = parsed_proxy_url.scheme().to_lowercase();
            debug!(proxy.scheme = %scheme, "Parsed proxy scheme");

            let proxy = self.create_proxy_from_scheme(&scheme, proxy_url)?;
            debug!("Proxy object created successfully");

            self.configure_builder(Client::builder())
                .proxy(proxy)
                .build()
                .map_err(|e| {
                    error!(proxy.scheme = %scheme, error = ?e, "Failed to build reqwest client for proxy.");
                    AppError::HttpClientBuildError {
                        source: e,
                        proxy_url: Some(proxy_url.to_string()),
                    }
                })
                    }
        .instrument(proxy_span)
                .await
    }

    /// Creates a proxy object based on the URL scheme.
    fn create_proxy_from_scheme(&self, scheme: &str, proxy_url: &str) -> Result<Proxy> {
        match scheme {
            "http" => Proxy::http(proxy_url),
            "https" => Proxy::https(proxy_url),
            "socks5" => Proxy::all(proxy_url),
            _ => {
                error!(proxy.scheme = %scheme, "Unsupported proxy scheme");
                return Err(AppError::ProxyConfigError(ProxyConfigErrorData {
                    url: proxy_url.to_string(),
                    kind: ProxyConfigErrorKind::UnsupportedScheme(scheme.to_string()),
                }));
            }
        }
        .map_err(|e| {
            error!(error = %e, proxy.scheme = %scheme, "Invalid proxy definition");
            AppError::ProxyConfigError(ProxyConfigErrorData {
                url: proxy_url.to_string(),
                kind: ProxyConfigErrorKind::InvalidDefinition(e.to_string()),
            })
        })
    }
}

/// Builds HTTP clients for all unique proxy configurations.
///
/// This function encapsulates the logic for:
/// 1. Creating a base client (no proxy).
/// 2. Finding unique proxy URLs in the configuration.
/// 3. Creating a separate client for each unique proxy.
///
/// # Errors
///
/// Returns `Err` if:
/// - The base HTTP client cannot be created (fatal).
/// - A proxy URL is syntactically invalid or has an unsupported scheme.
/// - Another unexpected error occurs during client building.
#[instrument(level = "info", skip_all, name = "build_http_clients")]
async fn build_http_clients(config: &AppConfig) -> Result<HashMap<Option<String>, Arc<Client>>> {
    info!("Building HTTP clients based on configuration...");
    let mut http_clients = HashMap::new();

    // Calculate connection pool configuration
    let total_key_count: usize = config
        .groups
        .iter()
        .flat_map(|g| &g.api_keys)
        .filter(|k| !k.trim().is_empty())
        .count()
        .max(10);
    
    debug!(
        pool.max_idle_per_host = total_key_count,
        "Calculated max idle connections per host"
        );

    let pool_config = ClientPoolConfig::new(total_key_count, &config.server);
    let client_builder = HttpClientBuilder::new(pool_config);

    // 1. Create the base client (no proxy) - this MUST succeed
    let base_client = client_builder.build_base_client()?;
    http_clients.insert(None, Arc::new(base_client));
    info!(client.type = "base", "Base HTTP client (no proxy) created successfully.");

    // 2. Collect unique proxy URLs from the configuration
    let unique_proxy_urls: HashSet<String> = config
        .groups
        .iter()
        .filter_map(|g| g.proxy_url.as_ref())
        .filter(|url_str| !url_str.trim().is_empty())
        .cloned()
        .collect();
    
    debug!(
        proxy.count = unique_proxy_urls.len(),
        ?unique_proxy_urls,
        "Found unique proxy URLs for client creation"
        );

    // 3. Create clients for each unique proxy URL
    for proxy_url_str in unique_proxy_urls {
        match client_builder.build_proxy_client(&proxy_url_str).await {
            Ok(proxy_client) => {
                info!(proxy.url = %proxy_url_str, "HTTP client created successfully for proxy.");
                http_clients.insert(Some(proxy_url_str.clone()), Arc::new(proxy_client));
            }
            Err(e) => {
                if should_fail_fast(&e) {
                    error!(proxy.url = %proxy_url_str, error = ?e, "Critical proxy configuration error. Aborting client creation process.");
                    return Err(e);
                } else {
                    warn!(proxy.url = %proxy_url_str, error = ?e, "Skipping client creation for this proxy. Groups using this proxy might fail.");
                }
            }
        }
    }

    info!(
        client.count = http_clients.len(),
        "Finished building HTTP clients."
        );
    Ok(http_clients)
}

/// Determines if an error should cause the entire client building process to fail.
fn should_fail_fast(error: &AppError) -> bool {
    match error {
        AppError::ProxyConfigError(data) => {
            matches!(
                data.kind,
                ProxyConfigErrorKind::UrlParse(_) | ProxyConfigErrorKind::UnsupportedScheme(_)
            )
        }
        AppError::HttpClientBuildError { proxy_url: None, .. } => true, // Base client failure
        _ => false,
    }
}

impl AppState {
    /// Creates a new `AppState`. Initializes the Redis pool and pre-builds HTTP clients.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the Redis pool or `http_clients` cannot be created.
    #[instrument(level = "info", skip(config, config_path), fields(config.path = %config_path.display()))]
    pub async fn new(config: &AppConfig, config_path: &Path) -> Result<Self> {
        info!("Creating shared AppState...");

        let redis_pool = Self::create_redis_pool(config).await?;
        let key_manager =
            Arc::new(RwLock::new(KeyManager::new(config, redis_pool.clone()).await?));
        let http_clients = build_http_clients(config).await?;

        let response_handlers: Arc<Vec<Box<dyn ResponseHandler>>> = Arc::new(vec![
            Box::new(SuccessHandler),
            Box::new(crate::handlers::timeout::TimeoutHandler),
            Box::new(RateLimitHandler),
            Box::new(InvalidApiKeyHandler),
            Box::new(TerminalErrorHandler),
        ]);

        Ok(Self {
            redis_pool,
            key_manager,
            http_clients: Arc::new(RwLock::new(http_clients)),
            response_handlers,
            start_time: Instant::now(),
            config: Arc::new(RwLock::new(config.clone())),
            system_info: SystemInfoCollector::new(),
            config_path: config_path.to_path_buf(),
            metrics: Arc::new(MetricsCollector::new()),
            rate_limit_store: crate::middleware::rate_limit::create_rate_limit_store(),
        })
    }

    /// Creates a Redis connection pool if configured.
    async fn create_redis_pool(config: &AppConfig) -> Result<Option<Pool>> {
        if let Some(redis_url) = &config.redis_url {
            let redis_pool_config = Config::from_url(redis_url);
            let pool = redis_pool_config.create_pool(Some(Runtime::Tokio1))?;
            info!("Redis connection pool created successfully.");

            #[cfg(test)]
            Self::clear_redis_for_tests(&pool).await?;

            Ok(Some(pool))
        } else {
            info!("No Redis URL provided. Skipping Redis pool creation.");
            Ok(None)
}
    }

    /// Clears Redis database for test isolation.
    #[cfg(test)]
    async fn clear_redis_for_tests(pool: &Pool) -> Result<()> {
        use deadpool_redis::redis::cmd;
        let mut conn = pool.get().await?;
        let _: () = cmd("FLUSHDB").query_async(&mut conn).await?;
        info!("FLUSHDB command executed to clear Redis for test environment.");
        Ok(())
    }

    /// Reloads `http_clients` from the current configuration.
    /// This allows for hot-reloading of proxy configurations without a server restart.
    ///
    /// # Errors
    ///
    /// Returns `Err` if any part of the state reconstruction fails.
    pub async fn reload_state_from_config(&self) -> Result<()> {
        info!("Attempting to reload application state from configuration...");

        let config_guard = self.config.read().await;
        let new_http_clients = build_http_clients(&config_guard).await?;
        let new_key_manager = KeyManager::new(&config_guard, self.redis_pool.clone()).await?;
        drop(config_guard);

        // Atomically swap the http_clients and key_manager
        *self.http_clients.write().await = new_http_clients;
        *self.key_manager.write().await = new_key_manager;

        info!("Application state reloaded successfully.");
        Ok(())
    }

    /// Returns a reference to the appropriate HTTP client.
    ///
    /// # Errors
    ///
    /// Returns `AppError::Internal` if the requested client (identified by the `proxy_url` Option)
    /// was not found in the pre-built client map. This indicates a logic error,
    /// as all necessary clients should have been initialized at startup.
    #[instrument(level = "debug", skip(self), fields(proxy.url = ?proxy_url))]
    pub async fn get_client(&self, proxy_url: Option<&str>) -> Result<Arc<Client>> {
        let clients_guard = self.http_clients.read().await;
        let key = proxy_url.map(String::from);

        clients_guard.get(&key).cloned().ok_or_else(|| {
            let msg = proxy_url.map_or_else(
                || "Requested base HTTP client (None proxy) was unexpectedly missing.".to_string(),
                |p_url| {
                    format!(
                        "Requested HTTP client for proxy '{p_url}' was not found/initialized in AppState."
                    )
                },
            );
            error!(proxy.url = ?proxy_url, error.message = %msg, "HTTP client lookup failed");
            AppError::Internal(msg)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, KeyGroup, ServerConfig};
    use crate::error::ProxyConfigErrorKind;
    use std::fs::File;
    use tempfile::tempdir;
    
    const DEFAULT_TARGET_URL_STR: &str = "https://generativelanguage.googleapis.com";
    
    fn create_test_config(groups: Vec<KeyGroup>, with_redis: bool) -> AppConfig {
        AppConfig {
            server: ServerConfig {
                port: 8080,
                admin_token: None,
                ..Default::default()
            },
            groups,
            redis_url: if with_redis {
                Some("redis://redis:6379".to_string())
            } else {
                None
            },
            ..Default::default()
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
            model_aliases: vec![],
            proxy_url: None,
            target_url: DEFAULT_TARGET_URL_STR.to_string(),
            top_p: None,
        }];
        let config = create_test_config(groups, false);
        let state_result = AppState::new(&config, &dummy_path).await;

        assert!(
            state_result.is_ok(),
            "AppState::new failed unexpectedly: {:?}",
            state_result.err()
        );
        let state = state_result.unwrap();
        let clients_guard = state.http_clients.read().await;
        assert_eq!(clients_guard.len(), 1);
        drop(clients_guard);

        assert!(state.get_client(None).await.is_ok());
        assert!(
            state
                .get_client(Some("http://nonexistent.proxy"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_appstate_new_with_valid_proxies() {
        let dir = tempdir().unwrap();
        let dummy_path = create_dummy_config_path(&dir);

        let http_proxy_url = "http://127.0.0.1:34567";
        let socks_proxy_url = "socks5://127.0.0.1:34568";

        let groups = vec![
            KeyGroup {
                name: "g_http".to_string(),
                api_keys: vec!["key_http".to_string()],
                model_aliases: vec![],
                proxy_url: Some(http_proxy_url.to_string()),
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
                top_p: None,
            },
            KeyGroup {
                name: "g_socks".to_string(),
                api_keys: vec!["key_socks".to_string()],
                model_aliases: vec![],
                proxy_url: Some(socks_proxy_url.to_string()),
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
                top_p: None,
            },
            KeyGroup {
                name: "g_http_dup".to_string(),
                api_keys: vec!["key_http2".to_string()],
                model_aliases: vec![],
                proxy_url: Some(http_proxy_url.to_string()),
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
                top_p: None,
            },
            KeyGroup {
                name: "g_no_proxy".to_string(),
                api_keys: vec!["key_none".to_string()],
                model_aliases: vec![],
                proxy_url: None,
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
                top_p: None,
            },
        ];
        let config = create_test_config(groups, false);
        let state_result = AppState::new(&config, &dummy_path).await;

        assert!(
            state_result.is_ok(),
            "AppState::new failed unexpectedly: {:?}",
            state_result.err()
        );
        let state = state_result.unwrap();
        let clients_guard = state.http_clients.read().await;

        assert!(clients_guard.contains_key(&None));

        let http_key = Some(http_proxy_url.to_string());
        let socks_key = Some(socks_proxy_url.to_string());

        let http_created = clients_guard.contains_key(&http_key);
        let socks_created = clients_guard.contains_key(&socks_key);

        assert!(http_created, "HTTP proxy client was not created");
        assert!(socks_created, "SOCKS5 proxy client was not created");
        assert_eq!(
            clients_guard.len(),
            3,
            "Expected Base + HTTP + SOCKS clients"
        );
        drop(clients_guard);

        assert!(
            state.get_client(http_key.as_deref()).await.is_ok(),
            "get_client failed for created HTTP proxy"
        );
        assert!(
            state.get_client(socks_key.as_deref()).await.is_ok(),
            "get_client failed for created SOCKS5 proxy"
        );
        assert!(state.get_client(None).await.is_ok());
        assert!(state.get_client(Some("http://other.proxy")).await.is_err());
}
    #[tokio::test]
    async fn test_appstate_new_returns_err_on_invalid_url_syntax() {
        let dir = tempdir().unwrap();
        let dummy_path = create_dummy_config_path(&dir);

        let groups = vec![KeyGroup {
            name: "g_invalid_url".to_string(),
            api_keys: vec!["key_invalid".to_string()],
            model_aliases: vec![],
            proxy_url: Some("::not a proxy url::".to_string()),
            target_url: DEFAULT_TARGET_URL_STR.to_string(),
            top_p: None,
        }];
        let config = create_test_config(groups, false);
        let state_result = AppState::new(&config, &dummy_path).await;

        assert!(
            state_result.is_err(),
            "AppState::new should return Err for invalid proxy URL syntax"
        );
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
            model_aliases: vec![],
            proxy_url: Some("ftp://unsupported.proxy".to_string()),
            target_url: DEFAULT_TARGET_URL_STR.to_string(),
            top_p: None,
        }];
        let config = create_test_config(groups, false);
        let state_result = AppState::new(&config, &dummy_path).await;

        assert!(
            state_result.is_err(),
            "AppState::new should return Err for unsupported proxy scheme"
        );
        assert!(
            matches!(state_result.as_ref().err().unwrap(), AppError::ProxyConfigError(data) if matches!(data.kind, ProxyConfigErrorKind::UnsupportedScheme(_))),
            "Expected ProxyConfigError with UnsupportedScheme kind"
        );
    }

    #[tokio::test]
    async fn test_appstate_new_skips_client_on_build_error() {
        let dir = tempdir().unwrap();
        let dummy_path = create_dummy_config_path(&dir);

        let groups = vec![
            KeyGroup {
                name: "g_http_ok".to_string(),
                api_keys: vec!["k1".to_string()],
                model_aliases: vec![],
                proxy_url: Some("http://127.0.0.1:34569".to_string()),
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
                top_p: None,
            },
            KeyGroup {
                name: "g_build_error".to_string(),
                api_keys: vec!["k2".to_string()],
                model_aliases: vec![],
                proxy_url: Some("socks5://nonexistent-proxy-host.invalid:1080".to_string()),
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
                top_p: None,
            },
        ];
        let config = create_test_config(groups, false);
        let state_result = AppState::new(&config, &dummy_path).await;

        assert!(
            state_result.is_ok(),
            "AppState::new failed unexpectedly: {:?}",
            state_result.err()
        );
        let state = state_result.unwrap();
        let clients_guard = state.http_clients.read().await;

        assert!(clients_guard.contains_key(&None));
        let http_key = Some("http://127.0.0.1:34569".to_string());
        assert!(
            clients_guard.contains_key(&http_key),
            "Valid HTTP client should have been created"
        );

        let socks_key = Some("socks5://nonexistent-proxy-host.invalid:1080".to_string());
        assert!(
            clients_guard.contains_key(&socks_key),
            "SOCKS5 client should have been created (reqwest allows non-existent hosts)"
        );

        assert_eq!(
            clients_guard.len(),
            3,
            "Expected base, HTTP, and SOCKS5 clients"
        );
        drop(clients_guard);

        assert!(state.get_client(None).await.is_ok());
        assert!(state.get_client(http_key.as_deref()).await.is_ok());
        assert!(state.get_client(socks_key.as_deref()).await.is_ok());
    }
}
