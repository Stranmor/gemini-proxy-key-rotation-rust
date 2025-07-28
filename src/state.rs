// src/state.rs

use crate::admin::SystemInfoCollector;
use crate::config::AppConfig;
use crate::error::{AppError, ProxyConfigErrorData, ProxyConfigErrorKind, Result};
use crate::key_manager::KeyManager;
use reqwest::{Client, ClientBuilder, Proxy};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{Instrument, debug, error, info, instrument, warn};
use url::Url;

/// Представляет общее состояние приложения, доступное для всех обработчиков Axum.
#[derive(Debug)]
pub struct AppState {
    pub key_manager: Arc<RwLock<KeyManager>>,
    http_clients: Arc<RwLock<HashMap<Option<String>, Arc<Client>>>>,
    pub start_time: Instant,
    pub config: Arc<RwLock<AppConfig>>,
    pub system_info: SystemInfoCollector,
    pub config_path: PathBuf,
}

/// Создает `HashMap` HTTP-клиентов на основе предоставленной конфигурации.
///
/// Эта функция инкапсулирует логику для:
/// 1. Создания базового клиента (без прокси).
/// 2. Поиска уникальных URL-адресов прокси в конфигурации.
/// 3. Создания отдельного клиента для каждого уникального прокси.
///
/// # Errors
///
/// Возвращает `Err`, если:
/// - Не удается создать базовый HTTP-клиент (фатальная ошибка).
/// - URL-адрес прокси имеет синтаксически неверный формат или неподдерживаемую схему.
/// - Происходит другая непредвиденная ошибка во время сборки клиента.
#[instrument(level = "info", skip_all, name = "build_http_clients")]
async fn build_http_clients(config: &AppConfig) -> Result<HashMap<Option<String>, Arc<Client>>> {
    info!("Building HTTP clients based on configuration...");
    let mut http_clients = HashMap::new();

    // Определяем размер пула соединений на основе количества ключей, с минимальным порогом
    let total_key_count: usize = config
        .groups
        .iter()
        .flat_map(|g| &g.api_keys)
        .filter(|k| !k.trim().is_empty())
        .count()
        .max(10); // Гарантируем не менее 10 возможных соединений даже при малом количестве ключей
    debug!(
        pool.max_idle_per_host = total_key_count,
        "Calculated max idle connections per host"
    );

    // Централизованная функция конфигурации клиента
    let configure_builder = |builder: ClientBuilder| -> ClientBuilder {
        builder
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(300)) // Общий таймаут запроса
            .pool_idle_timeout(Duration::from_secs(90)) // Держать неактивные соединения открытыми 90с
            .pool_max_idle_per_host(total_key_count) // Настроить размер пула на основе ключей
            .tcp_keepalive(Some(Duration::from_secs(60))) // Включить TCP keepalive
    };

    // 1. Создаем базовый клиент (без прокси) - это ДОЛЖНО получиться
    let base_client = configure_builder(Client::builder()).build().map_err(|e| {
        // Структурированная ошибка для сбоя базового клиента - это фатально
        error!(error = ?e, "Failed to build base HTTP client (no proxy). This is required.");
        AppError::HttpClientBuildError {
            source: e,
            proxy_url: None,
        }
    })?;
    http_clients.insert(None, Arc::new(base_client));
    info!(client.type = "base", "Base HTTP client (no proxy) created successfully.");

    // 2. Собираем уникальные URL-адреса прокси из конфигурации
    let unique_proxy_urls: HashSet<String> = config
        .groups
        .iter()
        .filter_map(|g| g.proxy_url.as_ref()) // Получаем Option<&String>
        .filter(|url_str| !url_str.trim().is_empty()) // Отфильтровываем пустые строки
        .cloned() // Клонируем String
        .collect();
    debug!(
        proxy.count = unique_proxy_urls.len(),
        ?unique_proxy_urls,
        "Found unique proxy URLs for client creation"
    );

    // 3. Создаем клиенты для каждого уникального URL-адреса прокси
    for proxy_url_str in unique_proxy_urls {
        let proxy_span = tracing::info_span!("create_proxy_client", proxy.url = %proxy_url_str);
        let client_result: Result<Client> = async {
            // Сначала парсим URL, сопоставляем ошибку с конкретным ProxyConfigErrorKind
            let parsed_proxy_url = Url::parse(&proxy_url_str).map_err(|e| {
                error!(error = %e, "Failed to parse proxy URL string.");
                AppError::ProxyConfigError(ProxyConfigErrorData {
                    url: proxy_url_str.clone(),
                    kind: ProxyConfigErrorKind::UrlParse(e),
                })
            })?;

            let scheme = parsed_proxy_url.scheme().to_lowercase();
            debug!(proxy.scheme = %scheme, "Parsed proxy scheme");

            // Создаем объект прокси, сопоставляем ошибки с конкретным ProxyConfigErrorKind
            let proxy = match scheme.as_str() {
                "http" => Proxy::http(&proxy_url_str),
                "https" => Proxy::https(&proxy_url_str),
                "socks5" => Proxy::all(&proxy_url_str),
                _ => {
                    error!(proxy.scheme = %scheme, "Unsupported proxy scheme");
                    return Err(AppError::ProxyConfigError(ProxyConfigErrorData {
                        url: proxy_url_str.clone(),
                        kind: ProxyConfigErrorKind::UnsupportedScheme(scheme.to_string()),
                    }));
                }
            }
            .map_err(|e| {
                error!(error = %e, proxy.scheme = %scheme, "Invalid proxy definition");
                AppError::ProxyConfigError(ProxyConfigErrorData {
                    url: proxy_url_str.clone(),
                    kind: ProxyConfigErrorKind::InvalidDefinition(e.to_string()),
                })
            })?;
            debug!("Proxy object created successfully");

            // Собираем клиент с прокси
            configure_builder(Client::builder())
                .proxy(proxy)
                .build()
                .map_err(|e| {
                    error!(proxy.scheme = %scheme, error = ?e, "Failed to build reqwest client for proxy.");
                    AppError::HttpClientBuildError {
                        source: e,
                        proxy_url: Some(proxy_url_str.clone()),
                    }
                })
        }
        .instrument(proxy_span)
        .await;

        // Обрабатываем результат создания клиента
        match client_result {
            Ok(proxy_client) => {
                info!(proxy.url = %proxy_url_str, "HTTP client created successfully for proxy.");
                http_clients.insert(Some(proxy_url_str.clone()), Arc::new(proxy_client));
            }
            Err(e) => {
                match e {
                    AppError::ProxyConfigError(_) => {
                        error!(proxy.url = %proxy_url_str, error = ?e, "Critical proxy configuration error. Aborting client creation process.");
                        return Err(e); // Быстрый выход при ошибках конфигурации
                    }
                    AppError::HttpClientBuildError {
                        ref source,
                        proxy_url: Some(ref url),
                    } => {
                        warn!(proxy.url = %url, error = ?source, "Skipping client creation for this proxy due to build error. Groups using this proxy might fail.");
                        // Логируем и продолжаем при ошибках сборки
                    }
                    _ => {
                        error!(proxy.url = %proxy_url_str, error = ?e, "Unexpected error during proxy client creation. Aborting.");
                        return Err(e); // Выход при других непредвиденных ошибках
                    }
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

impl AppState {
    /// Создает новый `AppState`. Инициализирует KeyManager и предварительно создает HTTP-клиенты.
    ///
    /// # Errors
    ///
    /// Возвращает `Err`, если не удается создать `KeyManager` или `http_clients`.
    #[instrument(level = "info", skip(config, config_path), fields(config.path = %config_path.display()))]
    pub async fn new(config: &AppConfig, config_path: &Path) -> Result<Self> {
        info!("Creating shared AppState...");

        // Сначала инициализируем KeyManager
        let key_manager = KeyManager::new(config, config_path).await;

        // Создаем все HTTP-клиенты с помощью вспомогательной функции
        let http_clients = build_http_clients(config).await?;

        Ok(Self {
            key_manager: Arc::new(RwLock::new(key_manager)),
            http_clients: Arc::new(RwLock::new(http_clients)),
            start_time: Instant::now(),
            config: Arc::new(RwLock::new(config.clone())),
            system_info: SystemInfoCollector::new(),
            config_path: config_path.to_path_buf(),
        })
    }

    /// Перезагружает `KeyManager` и `http_clients` из текущей конфигурации.
    /// Это позволяет выполнять горячую перезагрузку API-ключей и конфигураций прокси без перезапуска сервера.
    ///
    /// # Errors
    ///
    /// Возвращает `Err`, если какая-либо часть реконструкции состояния завершается неудачно.
    #[instrument(level = "info", skip(self))]
    pub async fn reload_state_from_config(&self) -> Result<()> {
        info!(
            "Attempting to reload full application state (KeyManager, HttpClients) from configuration..."
        );
        let config_guard = self.config.read().await;

        // --- Создаем новый KeyManager ---
        let new_key_manager = KeyManager::new(&config_guard, &self.config_path).await;

        // --- Создаем новые HttpClients с помощью вспомогательной функции ---
        // Вспомогательная функция содержит надежную обработку ошибок, которую мы хотим использовать.
        let new_http_clients = build_http_clients(&config_guard).await?;

        // Освобождаем блокировку чтения перед получением блокировок записи
        drop(config_guard);

        // --- Атомарно обновляем состояние ---
        *self.key_manager.write().await = new_key_manager;
        *self.http_clients.write().await = new_http_clients;

        info!("Application state (KeyManager, HttpClients) reloaded successfully.");
        Ok(())
    }

    /// Возвращает ссылку на соответствующий HTTP-клиент.
    ///
    /// # Errors
    ///
    /// Возвращает `AppError::Internal`, если запрошенный клиент (определяемый `proxy_url` Option)
    /// не был найден в предварительно созданном отображении клиентов. Это указывает на логическую ошибку,
    /// так как все необходимые клиенты должны были быть инициализированы при запуске.
    #[instrument(level = "debug", skip(self), fields(proxy.url = ?proxy_url))]
    pub async fn get_client(&self, proxy_url: Option<&str>) -> Result<Arc<Client>> {
        let clients_guard = self.http_clients.read().await;
        let key = proxy_url.map(String::from);

        clients_guard.get(&key).cloned().ok_or_else(|| {
            let msg = proxy_url.map_or_else(
                || "Requested base HTTP client (None proxy) was unexpectedly missing.".to_string(),
                |p_url| format!("Requested HTTP client for proxy '{p_url}' was not found/initialized in AppState."),
            );
            error!(proxy.url = ?proxy_url, error.message = %msg, "HTTP client lookup failed");
            AppError::Internal(msg)
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
    use tracing::warn; // Import warn for logging in tests specifically

    const DEFAULT_TARGET_URL_STR: &str = "https://generativelanguage.googleapis.com";

    fn create_test_state_config(groups: Vec<KeyGroup>) -> AppConfig {
        AppConfig {
            server: ServerConfig {
                port: 8080,
                top_p: None,
                admin_token: None,
            },
            groups,
            rate_limit_behavior: Default::default(),
            internal_retries: 2,
            temporary_block_minutes: 5,
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
            top_p: None,
        }];
        let config = create_test_state_config(groups);
        let state_result = AppState::new(&config, &dummy_path).await;

        assert!(state_result.is_ok());
        let state = state_result.unwrap();
        let clients_guard = state.http_clients.read().await;
        assert_eq!(clients_guard.len(), 1);
        assert!(clients_guard.contains_key(&None)); // Base client only
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

        // Mock server or use potentially invalid ports to test resilience
        let http_proxy_url = "http://127.0.0.1:34567"; // Use a likely free port
        let socks_proxy_url = "socks5://127.0.0.1:34568"; // Use a likely free port

        let groups = vec![
            KeyGroup {
                name: "g_http".to_string(),
                api_keys: vec!["key_http".to_string()],
                proxy_url: Some(http_proxy_url.to_string()),
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
                top_p: None,
            },
            KeyGroup {
                name: "g_socks".to_string(),
                api_keys: vec!["key_socks".to_string()],
                proxy_url: Some(socks_proxy_url.to_string()),
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
                top_p: None,
            },
            KeyGroup {
                // Same HTTP proxy, should reuse client map entry
                name: "g_http_dup".to_string(),
                api_keys: vec!["key_http2".to_string()],
                proxy_url: Some(http_proxy_url.to_string()),
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
                top_p: None,
            },
            KeyGroup {
                name: "g_no_proxy".to_string(),
                api_keys: vec!["key_none".to_string()],
                proxy_url: None,
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
                top_p: None,
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
        let clients_guard = state.http_clients.read().await;

        assert!(clients_guard.contains_key(&None)); // Base client must exist

        let http_key = Some(http_proxy_url.to_string());
        let socks_key = Some(socks_proxy_url.to_string());

        let http_created = clients_guard.contains_key(&http_key);
        let socks_created = clients_guard.contains_key(&socks_key);

        // We expect all clients to be created successfully if URLs are valid syntactically
        assert!(http_created, "HTTP proxy client was not created");
        assert!(socks_created, "SOCKS5 proxy client was not created");
        assert_eq!(
            clients_guard.len(),
            3,
            "Expected Base + HTTP + SOCKS clients"
        ); // 1 base + 2 unique proxies
        drop(clients_guard);

        // Verify get_client behavior
        assert!(
            state.get_client(http_key.as_deref()).await.is_ok(),
            "get_client failed for created HTTP proxy"
        );
        assert!(
            state.get_client(socks_key.as_deref()).await.is_ok(),
            "get_client failed for created SOCKS5 proxy"
        );
        assert!(state.get_client(None).await.is_ok()); // Check base client retrieval
        assert!(state.get_client(Some("http://other.proxy")).await.is_err()); // Check non-existent proxy
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
            top_p: None,
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
            top_p: None,
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
                top_p: None,
            },
            // Use a socks URL that might cause build issues or is hard to resolve
            KeyGroup {
                name: "g_socks_fail_build".to_string(),
                api_keys: vec!["k2".to_string()],
                // Provide a URL that might fail build if socks feature isn't compiled correctly or has issues
                proxy_url: Some("socks5://invalid-host-that-causes-build-error:1080".to_string()),
                target_url: DEFAULT_TARGET_URL_STR.to_string(),
                top_p: None,
            },
        ];
        let config = create_test_state_config(groups);
        let state_result = AppState::new(&config, &dummy_path).await;

        // Check the result: AppState::new should either succeed (skipping the build error)
        // or return a ProxyConfigError if the test URL caused a definition error.
        match state_result {
            Ok(state) => {
                let clients_guard = state.http_clients.read().await;
                assert!(clients_guard.contains_key(&None)); // Base client
                let http_key = Some("http://127.0.0.1:34569".to_string());
                let socks_key =
                    Some("socks5://invalid-host-that-causes-build-error:1080".to_string());
                let http_created = clients_guard.contains_key(&http_key);
                let socks_created = clients_guard.contains_key(&socks_key);
                assert!(http_created, "Valid HTTP client should have been created");

                let expected_clients =
                    1 + (if http_created { 1 } else { 0 }) + (if socks_created { 1 } else { 0 });
                assert_eq!(
                    clients_guard.len(),
                    expected_clients,
                    "Unexpected number of clients created"
                );
                drop(clients_guard);

                assert!(state.get_client(http_key.as_deref()).await.is_ok());
                if socks_created {
                    assert!(state.get_client(socks_key.as_deref()).await.is_ok());
                    warn!(
                        "SOCKS client build succeeded unexpectedly in test - test might not cover build failure path"
                    );
                } else {
                    assert!(state.get_client(socks_key.as_deref()).await.is_err());
                }
            }
            Err(AppError::ProxyConfigError(data)) => {
                // If the "invalid" URL actually caused a config error (likely InvalidDefinition), this is an acceptable outcome.
                warn!(error = ?data, "Test URL caused ProxyConfigError instead of HttpClientBuildError. Treating as acceptable test outcome.");
                assert_eq!(
                    data.url,
                    "socks5://invalid-host-that-causes-build-error:1080"
                );
            }
            Err(e) => {
                // Any other error type is unexpected and should fail the test
                panic!("AppState::new failed with unexpected error type: {e:?}");
            }
        }
    }
}
