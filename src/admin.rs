// src/admin.rs

use crate::{
    config::{self, AppConfig},
    error::{AppError, Result},
    key_manager::{FlattenedKeyInfo, KeyManager, KeyManagerTrait},
    state::{AppState, KeyState},
};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Json, Response},
    routing::{delete, get, post, put},
    Router,
};
use chrono::{DateTime, Utc};
use cookie::{time::Duration as CookieDuration, SameSite};
use http::HeaderName;
use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use sysinfo::{Disks, System};
use tokio::sync::Mutex;
use tower_cookies::{Cookie, CookieManagerLayer, Cookies};
use tracing::{error, info, warn};

// --- Constants ---

/// The name of the custom header for the CSRF token.
static X_CSRF_TOKEN: HeaderName = HeaderName::from_static("x-csrf-token");
/// The name of the cookie storing the admin authentication token.
const ADMIN_TOKEN_COOKIE: &str = "admin_token";
/// The name of the cookie storing the CSRF token.
const CSRF_TOKEN_COOKIE: &str = "csrf_token";
/// Default system refresh interval in seconds.
// --- System Info Collector ---
/// Collector for system information.
///
/// This struct holds a `System` instance and is designed to be updated by a background task.
/// This avoids blocking request threads for expensive data collection like CPU usage.
#[derive(Debug)]
pub struct SystemInfoCollector {
    // We use a Mutex as sysinfo::System is not Sync.
    system: Mutex<System>,
}

impl SystemInfoCollector {
    /// Creates a new `SystemInfoCollector` and performs an initial data refresh.
    pub fn new() -> Self {
        let mut system = System::new_all();
        system.refresh_all(); // Initial refresh
        Self {
            system: Mutex::new(system),
        }
    }

    /// Spawns a background task to periodically refresh system data.
    ///
    /// This should be called once when the application starts.
    pub fn spawn_background_refresh(self: Arc<Self>, interval: std::time::Duration) {
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(interval);
            // The first tick completes immediately, so we skip it to wait for the first interval.
            timer.tick().await;
            loop {
                timer.tick().await;
                if let Ok(mut sys) = self.system.try_lock() {
                    // Refresh only what we need to be more efficient.
                    sys.refresh_cpu_specifics(sysinfo::CpuRefreshKind::everything());
                    sys.refresh_memory();
                } else {
                    warn!("Failed to acquire system lock for refresh, skipping this cycle");
                }
            }
        });
    }

    /// Returns the current memory usage in MB. Reads recently refreshed data.
    pub async fn get_memory_usage(&self) -> u64 {
        let sys = self.system.lock().await;
        sys.used_memory() / (1024 * 1024)
    }

    /// Returns the current global CPU usage percentage. Reads recently refreshed data.
    pub async fn get_cpu_usage(&self) -> f64 {
        let sys = self.system.lock().await;
        let cpus = sys.cpus();
        if cpus.is_empty() {
            0.0
        } else {
            cpus.iter().map(|cpu| cpu.cpu_usage() as f64).sum::<f64>() / cpus.len() as f64
        }
    }

    /// Returns the total memory in MB.
    pub async fn get_total_memory(&self) -> u64 {
        let sys = self.system.lock().await;
        sys.total_memory() / (1024 * 1024)
    }

    /// Returns the total used disk space in MB. Reads recently refreshed data.
    pub async fn get_disk_usage(&self) -> u64 {
        let disks = Disks::new_with_refreshed_list();
        disks
            .iter()
            .map(|disk| disk.total_space() - disk.available_space())
            .sum::<u64>()
            / (1024 * 1024)
    }

    /// Returns the OS information.
    pub async fn get_os_info(&self) -> String {
        System::long_os_version()
            .or_else(System::os_version)
            .unwrap_or_else(|| "Unknown OS".to_string())
    }

    /// Returns the number of CPUs.
    pub async fn get_num_cpus(&self) -> usize {
        let sys = self.system.lock().await;
        sys.cpus().len()
    }
}

impl Default for SystemInfoCollector {
    fn default() -> Self {
        Self::new()
    }
}

// --- Router Definition ---

/// Defines all administrative API routes.
pub fn admin_routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    use crate::middleware::rate_limit_middleware;

    // Routes that require admin authentication and CSRF protection
    // Order of middleware matters: auth first, then CSRF.
    let authed_routes = Router::new()
        .route("/keys", post(add_keys))
        .route("/keys", delete(delete_keys))
        .route("/keys/:key_id/verify", post(verify_key))
        .route("/keys/:key_id/reset", post(reset_key))
        .route("/config", put(update_config))
        .route_layer(middleware::from_fn(csrf_middleware))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::admin_auth_middleware,
        ));

    // Combine all admin routes under a common `/admin` prefix.
    Router::new().nest(
        "/admin",
        Router::new()
            .route("/", get(serve_dashboard))
            .route("/health", get(detailed_health))
            .route("/keys", get(list_keys))
            .route("/keys-page", get(serve_keys_management_page))
            .route("/config", get(get_config))
            .route("/metrics", get(get_metrics_summary))
            .route("/model-stats", get(get_model_stats))
            .route("/csrf-token", get(get_csrf_token))
            .route("/login", post(login))
            .merge(authed_routes)
            .layer(CookieManagerLayer::new())
            .layer(middleware::from_fn_with_state(state, rate_limit_middleware)), // Add rate limiting to all admin routes
    )
}

// --- Request/Response Structs ---

#[derive(Debug, Serialize, Deserialize)]
pub struct DetailedHealthStatus {
    pub status: String,
    pub timestamp: DateTime<Utc>,
    pub version: String,
    pub uptime_seconds: u64,
    pub server_info: ServerInfo,
    pub key_status: KeyStatus,
    pub proxy_status: HashMap<String, ProxyStatus>,
    pub system_info: SystemInfo,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerInfo {
    pub host: String,
    pub port: u16,
    pub rust_version: String,
    pub build_info: BuildInfo,
    pub os_info: String,
    pub num_cpus: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BuildInfo {
    pub version: String,
    pub git_hash: String,
    pub build_date: String,
    pub target: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KeyStatus {
    pub total_keys: usize,
    pub active_keys: usize,
    pub limited_keys: usize,
    pub invalid_keys: usize,
    pub temporarily_unavailable_keys: usize,
    pub groups: Vec<GroupStatus>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GroupStatus {
    pub name: String,
    pub total_keys: usize,
    pub active_keys: usize,
    pub proxy_url: Option<String>,
    pub target_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProxyStatus {
    pub url: String,
    pub status: String,
    pub last_check: DateTime<Utc>,
    pub groups_using: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemInfo {
    pub memory_usage_mb: u64,
    pub total_memory_mb: u64,
    pub cpu_usage_percent: f64,
    pub disk_usage_mb: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AddKeysRequest {
    pub group_name: String,
    pub api_keys: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteKeysRequest {
    pub group_name: String,
    pub api_keys: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelStats {
    pub model: String,
    pub blocked_keys_count: usize,
    pub next_reset_time: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelStatsResponse {
    pub models: Vec<ModelStats>,
    pub total_keys: usize,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KeyInfo {
    pub id: String,
    pub group_name: String,
    pub key_preview: String,
    pub status: String,
    pub last_used: Option<DateTime<Utc>>,
    pub reset_time: Option<DateTime<Utc>>,
}

impl KeyInfo {
    /// Creates a new `KeyInfo` for API responses from internal key data.
    fn new(key_info: &FlattenedKeyInfo, key_state: Option<&KeyState>) -> Self {
        let (status_str, reset_time) = get_key_status_str(key_state);
        let key_preview = Self::create_key_preview(&key_info.key);
        Self {
            id: format!("{:x}", md5::compute(&key_info.key)),
            group_name: key_info.group_name.clone(),
            key_preview,
            status: status_str.to_string(),
            last_used: None, // TODO: Track last usage time in KeyManager
            reset_time,
        }
    }

    /// Creates a safe preview of the API key for display purposes.
    fn create_key_preview(key: &str) -> String {
        if key.len() > 10 {
            format!("{}...{}", &key[..6], &key[key.len() - 4..])
        } else {
            key.to_string()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ListKeysQuery {
    pub group: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginRequest {
    // For production, consider using a secret-wrapper type to prevent accidental logging.
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CsrfTokenResponse {
    pub csrf_token: String,
}

// --- Middleware ---

/// Constant-time string comparison to prevent timing attacks
fn secure_compare(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    
    let mut result = 0u8;
    for (byte_a, byte_b) in a.bytes().zip(b.bytes()) {
        result |= byte_a ^ byte_b;
    }
    result == 0
}

/// Middleware for Cross-Site Request Forgery (CSRF) protection.
///
/// It expects a `csrf_token` in a cookie and a matching token in the `X-CSRF-Token` header.
async fn csrf_middleware(cookies: Cookies, req: Request<Body>, next: Next) -> Result<Response> {
    let cookie_token = cookies
        .get(CSRF_TOKEN_COOKIE)
        .map(|cookie| cookie.value().to_string());

    let header_token = req
        .headers()
        .get(&X_CSRF_TOKEN)
        .and_then(|value| value.to_str().ok())
        .map(String::from);

    match (cookie_token, header_token) {
        (Some(c_token), Some(h_token)) 
            if !c_token.is_empty() 
            && c_token.len() <= 128  // Reasonable limit for CSRF tokens
            && h_token.len() <= 128
            && secure_compare(&c_token, &h_token) => {
            info!("CSRF token matched.");
            Ok(next.run(req).await)
        }
        _ => {
            warn!("CSRF token mismatch or missing. Access forbidden.");
            Err(AppError::Csrf)
        }
    }
}

// --- Route Handlers ---

/// Provides a detailed health check of the application.
#[axum::debug_handler]
pub async fn detailed_health(
    State(state): State<Arc<AppState>>,
) -> Result<Json<DetailedHealthStatus>> {
    let key_manager_guard = state.key_manager.read().await;
    let now = Utc::now();

    let key_status = calculate_key_status_summary(&key_manager_guard, now).await?;
    let proxy_status = HashMap::new(); // TODO: Implement proxy health checks.
    let uptime = state.start_time.elapsed().as_secs();
    let config_guard = state.config.read().await;

    let health_status = DetailedHealthStatus {
        status: "healthy".to_string(),
        timestamp: now,
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: uptime,
        server_info: ServerInfo {
            host: "0.0.0.0".to_string(), // Host is no longer part of config
            port: config_guard.server.port,
            rust_version: option_env!("RUSTC_VERSION").unwrap_or("N/A").to_string(),
            build_info: BuildInfo {
                version: env!("CARGO_PKG_VERSION").to_string(),
                git_hash: option_env!("GIT_HASH").unwrap_or("N/A").to_string(),
                build_date: option_env!("BUILD_DATE").unwrap_or("N/A").to_string(),
                target: option_env!("TARGET").unwrap_or("N/A").to_string(),
            },
            os_info: state.system_info.get_os_info().await,
            num_cpus: state.system_info.get_num_cpus().await,
        },
        key_status,
        proxy_status,
        system_info: SystemInfo {
            memory_usage_mb: state.system_info.get_memory_usage().await,
            total_memory_mb: state.system_info.get_total_memory().await,
            cpu_usage_percent: state.system_info.get_cpu_usage().await,
            disk_usage_mb: state.system_info.get_disk_usage().await,
        },
    };

    Ok(Json(health_status))
}

/// Lists API keys with optional filtering by group and status.
#[axum::debug_handler]
pub async fn list_keys(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListKeysQuery>,
) -> Result<Json<Vec<KeyInfo>>> {
    let key_manager_guard = state.key_manager.read().await;
    let key_states = key_manager_guard.get_key_states().await?;
    let all_key_info = key_manager_guard.get_all_key_info().await;

    let keys = all_key_info
        .values()
        .filter(|key_info| {
            query
                .group
                .as_ref()
                .is_none_or(|g| g == &key_info.group_name)
        })
        .filter_map(|key_info| {
            let key_state = key_states.get(&key_info.key);
            let api_key_info = KeyInfo::new(key_info, key_state);
            if query
                .status
                .as_ref()
                .is_none_or(|s| s == &api_key_info.status)
            {
                Some(api_key_info)
            } else {
                None
            }
        })
        .collect();

    Ok(Json(keys))
}

/// Verifies a single API key by making a test request to its target service.
#[axum::debug_handler]
pub async fn verify_key(
    State(_state): State<Arc<AppState>>,
    Path(key_id): Path<String>,
) -> Result<StatusCode> {
    // This functionality is complex with Redis and needs careful implementation.
    // For now, we return OK but log that it's not implemented.
    warn!(
        "verify_key endpoint called for key_id: {}, but is not implemented with Redis backend yet.",
        key_id
    );
    Ok(StatusCode::NOT_IMPLEMENTED)
}

/// Resets the status of a single API key to 'Available'.
#[axum::debug_handler]
pub async fn reset_key(
    State(_state): State<Arc<AppState>>,
    Path(key_id): Path<String>,
) -> Result<StatusCode> {
    // This functionality is complex with Redis and needs careful implementation.
    // For now, we return OK but log that it's not implemented.
    warn!(
        "reset_key endpoint called for key_id: {}, but is not implemented with Redis backend yet.",
        key_id
    );
    Ok(StatusCode::NOT_IMPLEMENTED)
}

/// Returns the current application configuration.
#[axum::debug_handler]
pub async fn get_config(State(state): State<Arc<AppState>>) -> Result<Json<AppConfig>> {
    let config = state.config.read().await;
    Ok(Json(config.clone()))
}

/// Принимает новую конфигурацию, отправляет ее в канал для фоновой обработки.
#[axum::debug_handler]
pub async fn update_config(
    State(state): State<Arc<AppState>>,
    Json(new_config): Json<AppConfig>,
) -> Result<StatusCode> {
    info!("Received request to update application configuration.");
    create_and_send_new_config(&state, "admin_update", |_| Ok(new_config)).await?;
    info!("Configuration update command sent successfully.");
    Ok(StatusCode::ACCEPTED)
}

/// Добавляет новые ключи API в указанную группу и отправляет обновленную конфигурацию на перезагрузку.
#[axum::debug_handler]
pub async fn add_keys(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AddKeysRequest>,
) -> Result<StatusCode> {
    info!("Received request to add keys to group '{}'.", request.group_name);
    create_and_send_new_config(&state, "admin_add_keys", |config| {
        let group = config
            .groups
            .iter_mut()
            .find(|g| g.name == request.group_name)
            .ok_or_else(|| {
                warn!(
                    "Failed to add keys: Group '{}' not found.",
                    request.group_name
                );
                AppError::NotFound(format!("Group '{}' not found", request.group_name))
            })?;

        let mut added_count = 0;
        for key in request.api_keys {
            let trimmed_key = key.trim();
            if !trimmed_key.is_empty() && !group.api_keys.iter().any(|k| k == trimmed_key) {
                group.api_keys.push(trimmed_key.to_string());
                added_count += 1;
            }
        }
        info!(
            "Prepared {} new keys for group '{}'.",
            added_count, request.group_name
        );
        Ok(config.clone())
    })
    .await?;
    Ok(StatusCode::ACCEPTED)
}

/// Удаляет указанные ключи API из группы и отправляет обновленную конфигурацию на перезагрузку.
#[axum::debug_handler]
pub async fn delete_keys(
    State(state): State<Arc<AppState>>,
    Json(request): Json<DeleteKeysRequest>,
) -> Result<StatusCode> {
    info!(
        "Received request to delete keys from group '{}'.",
        request.group_name
    );
    create_and_send_new_config(&state, "admin_delete_keys", |config| {
        let group = config
            .groups
            .iter_mut()
            .find(|g| g.name == request.group_name)
            .ok_or_else(|| {
                warn!(
                    "Failed to delete keys: Group '{}' not found.",
                    request.group_name
                );
                AppError::NotFound(format!("Group '{}' not found", request.group_name))
            })?;

        let keys_to_delete: HashSet<_> = request.api_keys.iter().map(String::as_str).collect();
        let initial_count = group.api_keys.len();
        group
            .api_keys
            .retain(|k| !keys_to_delete.contains(k.as_str()));
        let deleted_count = initial_count - group.api_keys.len();
        info!(
            "Prepared deletion of {} keys from group '{}'.",
            deleted_count, request.group_name
        );
        Ok(config.clone())
    })
    .await?;
    Ok(StatusCode::ACCEPTED)
}

/// Provides a summary of application metrics (placeholder).
#[axum::debug_handler]
pub async fn get_metrics_summary(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>> {
    info!("Metrics summary requested. (Placeholder)");
    Ok(Json(serde_json::json!({
        "message": "Metrics collection not yet implemented.",
        "note": "This endpoint will provide detailed application metrics in the future."
    })))
}

/// Provides statistics about model-specific key blocking.
#[axum::debug_handler]
pub async fn get_model_stats(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ModelStatsResponse>> {
    info!("Model statistics requested.");

    let key_manager_guard = state.key_manager.read().await;
    // This needs to be reimplemented for Redis
    let total_keys = key_manager_guard.get_all_key_info().await.len();

    let models = vec![]; // Placeholder

    Ok(Json(ModelStatsResponse {
        models,
        total_keys,
        timestamp: Utc::now(),
    }))
}

/// Handles admin login by setting a secure, HttpOnly cookie with the admin token.
#[axum::debug_handler]
pub async fn login(
    State(state): State<Arc<AppState>>,
    jar: Cookies,
    Json(request): Json<LoginRequest>,
) -> Result<impl IntoResponse> {
    let config = state.config.read().await;
    let expected_token = config.server.admin_token.as_deref();

    match expected_token {
        Some(token) if !token.is_empty() && request.token == token => {
            let is_test_mode = config.server.test_mode;
            let cookie = Cookie::build((ADMIN_TOKEN_COOKIE, token.to_string()))
                .path("/")
                .http_only(true)
                // In production, the cookie must be secure. In test mode, it should not be.
                .secure(!is_test_mode)
                .same_site(SameSite::Strict)
                .max_age(CookieDuration::days(7))
                .build();
            info!("Admin login successful.");
            Ok((jar.add(cookie), StatusCode::OK))
        }
        _ => {
            warn!("Failed admin login attempt: Invalid token or no token configured.");
            Err(AppError::Unauthorized)
        }
    }
}

/// Generates a cryptographically secure CSRF token, sets it as a cookie, and returns it in the response body.
#[axum::debug_handler]
pub async fn get_csrf_token(
    State(state): State<Arc<AppState>>,
    jar: Cookies,
) -> Result<impl IntoResponse> {
    // Generate a cryptographically secure token using 32 random bytes
    let mut token_bytes = [0u8; 32];
    thread_rng().fill(&mut token_bytes);

    // Convert to hex string for easier handling
    let token = hex::encode(token_bytes);

    let config = state.config.read().await;
    let is_test_mode = config.server.test_mode;

    let cookie = Cookie::build((CSRF_TOKEN_COOKIE, token.clone()))
        .path("/")
        // In production, the cookie must be secure. In test mode, it should not be.
        .secure(!is_test_mode)
        .same_site(SameSite::Strict)
        // This cookie should be readable by JS, so it must NOT be HttpOnly.
        // It is session-based for stricter security (no max_age).
        .build();

    info!("Generated new cryptographically secure CSRF token.");
    Ok((
        jar.add(cookie),
        Json(CsrfTokenResponse { csrf_token: token }),
    ))
}

// --- HTML Serving Handlers ---

/// Serves the main admin dashboard HTML page.
#[axum::debug_handler]
pub async fn serve_dashboard() -> Html<String> {
    Html(include_str!("../static/dashboard.html").to_string())
}

/// Serves the key management HTML page.
#[axum::debug_handler]
pub async fn serve_keys_management_page() -> Html<String> {
    Html(include_str!("../static/keys_management.html").to_string())
}

// --- Helper Functions ---

/// A generic helper to create a new configuration based on a modification
/// function and send it to the background worker channel.
async fn create_and_send_new_config<F>(
    state: &Arc<AppState>,
    source: &str,
    modification: F,
) -> Result<()>
where
    F: FnOnce(&mut AppConfig) -> Result<AppConfig>,
{
    let mut config_clone = state.config.read().await.clone();
    let new_config = modification(&mut config_clone)?;

    // The send method returns the number of active receivers.
    // If it's 0, it means the worker has died, which is a critical state.
    if state.config_update_tx.send(new_config).is_err() {
        let msg = "Configuration update channel is closed. The background worker may have crashed.";
        error!("{}", msg);
        return Err(AppError::Internal(msg.to_string()));
    }

    info!(
        "Successfully sent configuration update command from source: '{}'",
        source
    );
    Ok(())
}

/// This function is executed by the background worker to apply configuration changes.
/// It contains the logic previously in `modify_config_and_reload`.
pub async fn reload_state_from_config(
    state: Arc<AppState>,
    mut new_config: AppConfig,
) -> Result<()> {
    let source = "background_worker";
    // Preserve the original test_mode flag to ensure it's not overwritten by the incoming config
    // or lost during the reload process. This is crucial for test environments.
    let is_test_mode = state.config.read().await.server.test_mode;

    // Restore the test_mode flag before validation and reloading.
    new_config.server.test_mode = is_test_mode;

    if !config::validate_config(&mut new_config, source) {
        let msg =
            format!("Validation failed for new configuration from '{source}'; changes not saved.");
        error!("{}", msg);
        return Err(AppError::Config(msg));
    }

    config::save_config(&new_config, &state.config_path).await?;
    info!("Configuration saved to disk from '{}'.", source);

    // Perform the state reload logic directly here to avoid RwLock deadlocks.
    let new_http_clients = crate::state::build_http_clients(&new_config).await?;
    let new_key_manager = KeyManager::new(&new_config, state.redis_pool.clone()).await?;

    // Atomically swap all parts of the state that depend on the configuration.
    let mut config_guard = state.config.write().await;
    let mut http_clients_guard = state.http_clients.write().await;
    let mut key_manager_guard = state.key_manager.write().await;

    *config_guard = new_config;
    *http_clients_guard = new_http_clients;
    *key_manager_guard = new_key_manager;

    info!(
        "Application state reloaded successfully after config update from '{}'.",
        source
    );

    Ok(())
}

/// Calculates a summary of key statuses and group information.
async fn calculate_key_status_summary(
    key_manager_guard: &tokio::sync::RwLockReadGuard<'_, crate::key_manager::KeyManager>,
    _now: DateTime<Utc>,
) -> Result<KeyStatus> {
    let all_key_info = key_manager_guard.get_all_key_info().await;
    let key_states = key_manager_guard.get_key_states().await?;

    let mut summary = KeyStatus {
        total_keys: all_key_info.len(),
        active_keys: 0,
        limited_keys: 0,
        invalid_keys: 0,
        temporarily_unavailable_keys: 0,
        groups: Vec::new(),
    };

    let mut groups_map: HashMap<String, GroupStatus> = HashMap::new();

    for key_info in all_key_info.values() {
        let (status_str, _) = get_key_status_str(key_states.get(&key_info.key));
        match status_str {
            "available" => summary.active_keys += 1,
            "limited" => summary.limited_keys += 1,
            "invalid" => summary.invalid_keys += 1,
            "unavailable" => summary.temporarily_unavailable_keys += 1,
            _ => warn!(
                "Unknown key status '{}' for key in group '{}'.",
                status_str, key_info.group_name
            ),
        }

        let entry = groups_map
            .entry(key_info.group_name.clone())
            .or_insert_with(|| GroupStatus {
                name: key_info.group_name.clone(),
                total_keys: 0,
                active_keys: 0,
                proxy_url: key_info.proxy_url.clone(),
                target_url: key_info.target_url.clone(),
            });
        entry.total_keys += 1;
        if status_str == "available" {
            entry.active_keys += 1;
        }
    }

    summary.groups = groups_map.into_values().collect();
    Ok(summary)
}

/// Returns a string representation of the key's status and its potential reset time.
fn get_key_status_str(key_state: Option<&KeyState>) -> (&'static str, Option<DateTime<Utc>>) {
    match key_state {
        Some(state) => {
            if state.is_blocked {
                ("blocked", state.last_failure)
            } else {
                ("available", None)
            }
        }
        None => ("available", None), // Default to 'available' if no state is recorded yet.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
        routing::post,
    };
    use tower::util::ServiceExt;
    use tower_cookies::CookieManagerLayer;

    // --- Unit Tests ---
    #[test]
    fn test_get_key_status_str() {
        let now = Utc::now();
        assert_eq!(get_key_status_str(None), ("available", None));

        let state_available = KeyState {
            key: "test".to_string(),
            group_name: "test".to_string(),
            is_blocked: false,
            consecutive_failures: 0,
            last_failure: None,
        };
        assert_eq!(
            get_key_status_str(Some(&state_available)),
            ("available", None)
        );

        let state_blocked = KeyState {
            key: "test".to_string(),
            group_name: "test".to_string(),
            is_blocked: true,
            consecutive_failures: 3,
            last_failure: Some(now),
        };
        assert_eq!(
            get_key_status_str(Some(&state_blocked)),
            ("blocked", Some(now))
        );
    }

    #[test]
    fn test_create_key_preview() {
        assert_eq!(KeyInfo::create_key_preview("short"), "short");
        assert_eq!(
            KeyInfo::create_key_preview("sk-1234567890abcdef"),
            "sk-123...cdef"
        );
        assert_eq!(
            KeyInfo::create_key_preview("very_long_api_key_string_here"),
            "very_l...here"
        );
    }

    // --- Middleware Tests ---

    /// Creates a test app with the CSRF middleware applied.
    fn csrf_app() -> Router {
        Router::new()
            .route("/", post(|| async { StatusCode::OK }))
            .route_layer(middleware::from_fn(csrf_middleware))
            .layer(CookieManagerLayer::new())
    }
    #[tokio::test]
    async fn test_csrf_middleware_success() {
        let app = csrf_app();
        let token = "correct_csrf_token";

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header(header::COOKIE, format!("{CSRF_TOKEN_COOKIE}={token}"))
                    .header(&X_CSRF_TOKEN, token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_csrf_middleware_no_header() {
        let app = csrf_app();
        let token = "correct_csrf_token";

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header(header::COOKIE, format!("{CSRF_TOKEN_COOKIE}={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_csrf_middleware_no_cookie() {
        let app = csrf_app();
        let token = "correct_csrf_token";

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header(&X_CSRF_TOKEN, token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_csrf_middleware_mismatch() {
        let app = csrf_app();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header(
                        header::COOKIE,
                        format!("{CSRF_TOKEN_COOKIE}=token_in_cookie"),
                    )
                    .header(&X_CSRF_TOKEN, "token_in_header")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    #[tokio::test]
    async fn test_csrf_middleware_empty_tokens() {
        let app = csrf_app();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header(header::COOKIE, format!("{CSRF_TOKEN_COOKIE}="))
                    .header(&X_CSRF_TOKEN, "")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
