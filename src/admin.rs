// src/admin.rs

use crate::{
    config::{self, AppConfig},
    error::{AppError, Result},
    key_manager::{KeyState, KeyStatus as KmKeyStatus},
    state::AppState,
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
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};
use chrono::{DateTime, Utc};
use cookie::{time::Duration, SameSite};
use http::HeaderName;
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use sysinfo::System;
use tokio::sync::Mutex;
use tower_cookies::{Cookie, Cookies};
use tracing::warn;

// Define the custom header name for CSRF token
static X_CSRF_TOKEN: HeaderName = HeaderName::from_static("x-csrf-token");

/// Collector for system information.
/// Holds a single `System` instance to avoid repeated initialization
/// and to allow for more accurate CPU readings over time.
#[derive(Debug)]
pub struct SystemInfoCollector {
    system: Mutex<System>,
}

impl SystemInfoCollector {
    /// Creates a new `SystemInfoCollector`.
    pub fn new() -> Self {
        Self {
            system: Mutex::new(System::new_all()),
        }
    }

    /// Returns the current memory usage in MB.
    pub async fn get_memory_usage(&self) -> u64 {
        let mut sys = self.system.lock().await;
        sys.refresh_memory();
        sys.used_memory() / 1024 / 1024
    }

    /// Returns the current global CPU usage percentage.
    /// This requires a short delay between refreshes to get an accurate reading.
    pub async fn get_cpu_usage(&self) -> f64 {
        let mut sys = self.system.lock().await;
        sys.refresh_cpu();
        // Drop the lock to allow other tasks to run during sleep
        drop(sys);

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let mut sys = self.system.lock().await;
        sys.refresh_cpu();
        sys.global_cpu_info().cpu_usage() as f64
    }

    /// Returns the disk usage in MB.
    /// Placeholder for now.
    pub fn get_disk_usage(&self) -> u64 {
        // TODO: Implement actual disk usage detection using sysinfo
        0
    }
}

impl Default for SystemInfoCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Administrative API routes
pub fn admin_routes() -> Router<Arc<AppState>> {
    // Routes that require CSRF protection
    let protected_routes = Router::new()
        .route("/admin/login", post(login))
        .route("/admin/keys", post(add_keys))
        .route("/admin/keys", delete(delete_keys))
        .route("/admin/keys/:key_id/verify", post(verify_key))
        .route("/admin/keys/:key_id/reset", post(reset_key))
        .route("/admin/config", put(update_config))
        .route_layer(middleware::from_fn(csrf_middleware));

    // Combine all admin routes
    Router::new()
        .route("/admin", get(serve_dashboard))
        .route("/admin/health", get(detailed_health))
        .route("/admin/keys", get(list_keys))
        .route("/admin/config", get(get_config))
        .route("/admin/metrics", get(get_metrics_summary))
        .route("/admin/csrf-token", get(get_csrf_token))
        .merge(protected_routes)
}

/// Detailed health check response
#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
pub struct ServerInfo {
    pub host: String,
    pub port: u16,
    pub rust_version: String,
    pub build_info: BuildInfo,
}

#[derive(Debug, Serialize)]
pub struct BuildInfo {
    pub version: String,
    pub git_hash: String,
    pub build_date: String,
    pub target: String,
}

#[derive(Debug, Serialize)]
pub struct KeyStatus {
    pub total_keys: usize,
    pub active_keys: usize,
    pub limited_keys: usize,
    pub invalid_keys: usize,
    pub temporarily_unavailable_keys: usize,
    pub groups: Vec<GroupStatus>,
}

#[derive(Debug, Serialize)]
pub struct GroupStatus {
    pub name: String,
    pub total_keys: usize,
    pub active_keys: usize,
    pub proxy_url: Option<String>,
    pub target_url: String,
}

#[derive(Debug, Serialize)]
pub struct ProxyStatus {
    pub url: String,
    pub status: String,
    pub last_check: DateTime<Utc>,
    pub groups_using: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SystemInfo {
    pub memory_usage_mb: u64,
    pub cpu_usage_percent: f64,
    pub disk_usage_mb: u64,
}

/// Key management request/response types
#[derive(Debug, Deserialize)]
pub struct KeysUpdateRequest {
    pub group_name: String,
    pub api_keys: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct KeyInfo {
    pub id: String,
    pub group_name: String,
    pub key_preview: String,
    pub status: String,
    pub last_used: Option<DateTime<Utc>>,
    pub reset_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct ListKeysQuery {
    pub group: Option<String>,
    pub status: Option<String>,
}

/// Configuration update request
#[derive(Debug, Deserialize)]
pub struct ConfigUpdateRequest {
    pub config: AppConfig,
    pub restart_required: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    token: String,
}

#[derive(Debug, Serialize)]
pub struct CsrfTokenResponse {
    csrf_token: String,
}

/// Detailed health check endpoint
///
/// # Errors
///
/// This function currently does not return any errors, but is declared as `Result`
/// for future compatibility where I/O or other operations might fail.
#[axum::debug_handler]
pub async fn detailed_health(
    State(state): State<Arc<AppState>>,
) -> Result<Json<DetailedHealthStatus>> {
    let key_manager_guard = state.key_manager.read().await;
    let all_key_info = key_manager_guard.get_all_key_info();
    let _key_states = key_manager_guard.get_key_states();
    let now = Utc::now();
    let mut active_keys = 0;
    let mut limited_keys = 0;
    let mut invalid_keys = 0;
    let mut temp_unavailable_keys = 0;

    for key_info in &all_key_info {
        let (status_str, _) = get_key_status_str(key_manager_guard.get_key_states().get(&key_info.key), now);
        match status_str {
            "available" => active_keys += 1,
            "limited" => limited_keys += 1,
            "invalid" => invalid_keys += 1,
            "unavailable" => temp_unavailable_keys += 1,
            _ => warn!(
                "Unknown key status '{}' encountered during health check",
                status_str
            ),
        }
    }

    // Build group status
    let mut groups_map: HashMap<String, GroupStatus> = HashMap::new();
    for key_info in &all_key_info {
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

        let (status_str, _) = get_key_status_str(key_manager_guard.get_key_states().get(&key_info.key), now);
        if status_str == "available" {
            entry.active_keys += 1;
        }
    }
    let group_statuses = groups_map.into_values().collect();

    // Build proxy status
    let proxy_status = HashMap::new();
    // TODO: Implement proxy health checks

    let uptime = state.start_time.elapsed().as_secs();
    let config_guard = state.config.read().await;

    let health_status = DetailedHealthStatus {
        status: "healthy".to_string(),
        timestamp: now,
        version: option_env!("CARGO_PKG_VERSION")
            .unwrap_or("N/A")
            .to_string(),
        uptime_seconds: uptime,
        server_info: ServerInfo {
            host: "0.0.0.0".to_string(),
            port: config_guard.server.port,
            rust_version: "N/A".to_string(), // sysinfo doesn't provide this
            build_info: BuildInfo {
                version: option_env!("CARGO_PKG_VERSION")
                    .unwrap_or("N/A")
                    .to_string(),
                git_hash: "N/A".to_string(),
                build_date: "N/A".to_string(),
                target: "N/A".to_string(),
            },
        },
        key_status: KeyStatus {
            total_keys: all_key_info.len(),
            active_keys,
            limited_keys,
            invalid_keys,
            temporarily_unavailable_keys: temp_unavailable_keys,
            groups: group_statuses,
        },
        proxy_status,
        system_info: SystemInfo {
            memory_usage_mb: state.system_info.get_memory_usage().await,
            cpu_usage_percent: state.system_info.get_cpu_usage().await,
            disk_usage_mb: state.system_info.get_disk_usage(),
        },
    };

    Ok(Json(health_status))
}

/// List API keys
///
/// # Errors
///
/// This function currently does not return errors but is declared as `Result`
/// for future compatibility.
#[axum::debug_handler]
#[allow(clippy::significant_drop_tightening)]
pub async fn list_keys(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListKeysQuery>,
) -> Result<Json<Vec<KeyInfo>>> {
    let key_manager_guard = state.key_manager.read().await;
    let key_states = key_manager_guard.get_key_states();
    let all_key_info = key_manager_guard.get_all_key_info();
    let now = Utc::now();

    let mut keys = Vec::new();

    for key_info in &all_key_info {
        // Filter by group if specified
        if let Some(ref group_filter) = query.group {
            if &key_info.group_name != group_filter {
                continue;
            }
        }

        let (status_str, reset_time) = get_key_status_str(key_states.get(&key_info.key), now);

        // Filter by status if specified
        if let Some(ref status_filter) = query.status {
            if status_str != status_filter {
                continue;
            }
        }

        keys.push(KeyInfo {
            id: format!("{:x}", md5::compute(&key_info.key)),
            group_name: key_info.group_name.clone(),
            key_preview: format!(
                "{}...{}",
                &key_info.key[..6.min(key_info.key.len())],
                if key_info.key.len() > 10 {
                    &key_info.key[key_info.key.len() - 4..]
                } else {
                    ""
                }
            ),
            status: status_str.to_string(),
            last_used: None, // TODO: Track last usage
            reset_time,
        });
    }
    Ok(Json(keys))
}

/// Verify a single API key by making a test request.
///
/// # Errors
///
/// Returns 401 if unauthorized.
#[axum::debug_handler]
pub async fn verify_key(
    State(state): State<Arc<AppState>>,
    jar: Cookies,
    auth_header: Option<TypedHeader<Authorization<Bearer>>>,
    Path(key_id): Path<String>,
) -> Result<StatusCode> {
    require_admin_token(&state, &jar, auth_header, "verify_key").await?;

    let mut key_manager_guard = state.key_manager.write().await;

    // 1. Get key info under the write lock
    let (key_to_verify, proxy_url) =
        match key_manager_guard.get_key_info_by_id(&key_id) {
            Some(info) => (info.key.clone(), info.proxy_url.clone()),
            None => {
                return Err(AppError::NotFound(format!(
                    "Key with ID '{}' not found",
                    key_id
                )));
            }
        };

    // 2. Perform verification and update status within the single write lock
    let client = state.get_client(proxy_url.as_deref()).await?;
    let verification_result = key_manager_guard
        .perform_key_verification(&key_to_verify, &client)
        .await;

    if key_manager_guard.update_key_status_from_verification(&key_to_verify, verification_result) {
        key_manager_guard.save_states().await?;
    }

    Ok(StatusCode::OK)
}

/// Reset the status of a single API key.
///
/// # Errors
///
/// Returns 401 if unauthorized.
#[axum::debug_handler]
pub async fn reset_key(
    State(state): State<Arc<AppState>>,
    jar: Cookies,
    auth_header: Option<TypedHeader<Authorization<Bearer>>>,
    Path(key_id): Path<String>,
) -> Result<StatusCode> {
    require_admin_token(&state, &jar, auth_header, "reset_key").await?;

    let mut key_manager_guard = state.key_manager.write().await;
    if key_manager_guard.reset_key_status(&key_id) {
        key_manager_guard.save_states().await?;
    }

    Ok(StatusCode::OK)
}
///
///
/// # Errors
///
/// Returns 401 if unauthorized, 404 if group not found, 500 on other errors.
#[axum::debug_handler]
pub async fn add_keys(
    State(state): State<Arc<AppState>>,
    jar: Cookies,
    auth_header: Option<TypedHeader<Authorization<Bearer>>>,
    Json(request): Json<KeysUpdateRequest>,
) -> Result<StatusCode> {
    // 1. Authentication
    require_admin_token(&state, &jar, auth_header, "add_keys").await?;

    // Hold write lock for the entire operation to ensure atomicity
    let mut config_write_guard = state.config.write().await;

    // 2. Modify in-memory config
    let group_name = request.group_name;
    let keys_to_add = request.api_keys;

    let group = config_write_guard
        .groups
        .iter_mut()
        .find(|g| g.name == group_name)
        .ok_or_else(|| AppError::NotFound(format!("Group '{group_name}' not found")))?;

    for key in keys_to_add {
        if !key.trim().is_empty() && !group.api_keys.contains(&key) {
            group.api_keys.push(key);
        }
    }

    // 3. Validate before saving
    if !config::validate_config(&mut config_write_guard, "admin_add_keys") {
        return Err(AppError::Config(
            "Validation failed after adding keys; changes not saved.".to_string(),
        ));
    }

    // 4. Persist to file
    config::save_config(&config_write_guard, &state.config_path).await?;

    // Release lock before reloading to avoid deadlock
    drop(config_write_guard);

    // 5. Reload State
    state.reload_state_from_config().await?;

    Ok(StatusCode::OK)
}

/// Remove API keys from a group
///
/// # Errors
///
/// Returns 401 if unauthorized, 404 if group not found, 500 on other errors.
#[axum::debug_handler]
pub async fn delete_keys(
    State(state): State<Arc<AppState>>,
    jar: Cookies,
    auth_header: Option<TypedHeader<Authorization<Bearer>>>,
    Json(request): Json<KeysUpdateRequest>,
) -> Result<StatusCode> {
    // 1. Authentication
    require_admin_token(&state, &jar, auth_header, "delete_keys").await?;

    // Hold write lock for the entire operation to ensure atomicity
    let mut config_write_guard = state.config.write().await;

    // 2. Modify in-memory config
    let group_name = request.group_name;
    let keys_to_delete: std::collections::HashSet<_> = request.api_keys.into_iter().collect();

    let group = config_write_guard
        .groups
        .iter_mut()
        .find(|g| g.name == group_name)
        .ok_or_else(|| AppError::NotFound(format!("Group '{group_name}' not found")))?;

    group.api_keys.retain(|k| !keys_to_delete.contains(k));

    // 3. Validate before saving
    if !config::validate_config(&mut config_write_guard, "admin_delete_keys") {
        return Err(AppError::Config(
            "Validation failed after deleting keys; changes not saved.".to_string(),
        ));
    }

    // 4. Persist to file
    config::save_config(&config_write_guard, &state.config_path).await?;

    // Release lock before reloading to avoid deadlock
    drop(config_write_guard);

    // 5. Reload State
    state.reload_state_from_config().await?;

    Ok(StatusCode::OK)
}

/// Get current configuration
///
/// # Errors
///
/// Returns an error as this feature is not yet implemented.
#[axum::debug_handler]
pub async fn get_config(State(state): State<Arc<AppState>>) -> Result<Json<AppConfig>> {
    let config = state.config.read().await;
    Ok(Json(config.clone()))
}

/// Update configuration
///
/// # Errors
///
/// Returns an error as this feature is not yet implemented.
#[axum::debug_handler]
pub async fn update_config(
    State(state): State<Arc<AppState>>,
    jar: Cookies,
    auth_header: Option<TypedHeader<Authorization<Bearer>>>,
    Json(mut new_config): Json<AppConfig>,
) -> Result<StatusCode> {
    // 1. Authentication
    require_admin_token(&state, &jar, auth_header, "update_config").await?;

    // 2. Validation
    if !config::validate_config(&mut new_config, "admin_update") {
        return Err(AppError::Config(
            "Validation failed for the new configuration".to_string(),
        ));
    }

    // 3. Persist to file
    config::save_config(&new_config, &state.config_path).await?;

    // 4. Update in-memory config
    let mut config_write_guard = state.config.write().await;
    *config_write_guard = new_config;
    drop(config_write_guard);

    // 5. Reload State
    state.reload_state_from_config().await?;

    Ok(StatusCode::OK)
}

/// Get metrics summary
///
/// # Errors
///
/// This function currently does not return errors but is declared as `Result`
/// for future compatibility.
#[axum::debug_handler]
pub async fn get_metrics_summary(State(_state): State<Arc<AppState>>) -> Result<Json<()>> {
    // TODO: Collect actual metrics
    Ok(Json(()))
}

/// Returns a string representation of the key's status.
fn get_key_status_str(
    key_state: Option<&KeyState>,
    now: DateTime<Utc>,
) -> (&'static str, Option<DateTime<Utc>>) {
    match key_state {
        Some(state) => {
            let is_expired = state.reset_time.is_some_and(|rt| now >= rt);
            let status = match state.status {
                KmKeyStatus::Available => "available",
                KmKeyStatus::RateLimited if is_expired => "available",
                KmKeyStatus::RateLimited => "limited",
                KmKeyStatus::Invalid => "invalid",
                KmKeyStatus::TemporarilyUnavailable if is_expired => "available",
                KmKeyStatus::TemporarilyUnavailable => "unavailable",
            };
            (status, state.reset_time)
        }
        None => ("available", None), // Default to active if no state
    }
}
/// Serve the admin dashboard
#[axum::debug_handler]
pub async fn serve_dashboard(State(state): State<Arc<AppState>>) -> Html<String> {
    let content = include_str!("../static/dashboard.html").to_string();

    // Inject system info
    let mem_usage = state.system_info.get_memory_usage().await;
    let cpu_usage = state.system_info.get_cpu_usage().await;

    let system_info_html = format!(
        "<div class=\"p-4 bg-gray-800 rounded-lg shadow-md\">
<h2 class=\"text-xl font-bold mb-2\">System Info</h2>
<p><strong>Memory Usage:</strong> {mem_usage} MB</p>
<p><strong>CPU Usage:</strong> {cpu_usage:.2} %</p>
</div>"
    );

    // A bit of a hacky way to inject the info.
    // A proper templating engine would be better.
    let final_content = content.replace("<!-- SYSINFO_PLACEHOLDER -->", &system_info_html);

    Html(final_content)
}

/// Generates a CSRF token, sets it as a cookie, and returns it in the response body.
#[axum::debug_handler]
pub async fn get_csrf_token(jar: Cookies) -> Result<impl IntoResponse> {
    let token: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();

    let cookie = Cookie::build(("csrf_token", token.clone()))
        .path("/")
        .secure(true)
        .same_site(SameSite::Strict)
        .build();

    Ok((
        jar.add(cookie),
        Json(CsrfTokenResponse { csrf_token: token }),
    ))
}

/// CSRF protection middleware.
async fn csrf_middleware(
    cookies: Cookies,
    req: Request<Body>,
    next: Next,
) -> std::result::Result<Response, AppError> {
    // Extract the CSRF token from the cookie
    let cookie_token = cookies
        .get("csrf_token")
        .map(|cookie| cookie.value().to_string());

    // Extract the CSRF token from the header
    let header_token = req
        .headers()
        .get(&X_CSRF_TOKEN)
        .and_then(|value| value.to_str().ok())
        .map(String::from);

    match (cookie_token, header_token) {
        (Some(c_token), Some(h_token)) if !c_token.is_empty() && c_token == h_token => {
            // Tokens match, proceed with the request
            Ok(next.run(req).await)
        }
        _ => {
            // Tokens do not match or are missing
            warn!("CSRF token mismatch or missing");
            Err(AppError::Csrf)
        }
    }
}

/// New login endpoint to set HttpOnly cookie
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
            let cookie = Cookie::build(("admin_token", token.to_string()))
                .path("/")
                .http_only(true)
                .secure(true)
                .same_site(SameSite::Strict)
                .max_age(Duration::days(7)) // Uses time::Duration
                .build();
            Ok((jar.add(cookie), StatusCode::OK))
        }
        _ => {
            warn!("Failed login attempt");
            Err(AppError::Unauthorized)
        }
    }
}

/// Middleware to verify admin token from Cookie or Bearer token.
async fn require_admin_token(
    state: &Arc<AppState>,
    jar: &Cookies,
    auth_header: Option<TypedHeader<Authorization<Bearer>>>,
    endpoint_name: &str,
) -> Result<()> {
    let config = state.config.read().await;
    let expected_token = config.server.admin_token.as_deref();

    let token_from_cookie = jar.get("admin_token").map(|c| c.value().to_string());

    let token_from_header = auth_header.map(|h| h.token().to_string());

    let provided_token = token_from_cookie.or(token_from_header);

    match (expected_token, provided_token) {
        (Some(expected), Some(provided)) if !expected.is_empty() && provided == expected => Ok(()),
        (Some(""), _) => {
            warn!(
                "Admin endpoint ({}) accessed but no admin_token is configured",
                endpoint_name
            );
            Err(AppError::Unauthorized)
        }
        _ => {
            warn!(
                "Unauthorized admin access attempt for endpoint: {}",
                endpoint_name
            );
            Err(AppError::Unauthorized)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;
    use crate::key_manager::{KeyState, KeyStatus as KmKeyStatus};
    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
        middleware::from_fn_with_state,
        routing::get,
        Router,
    };
    use axum_extra::headers::Authorization;
    use chrono::{Duration, Utc};
    use std::sync::Arc;
use tower::util::ServiceExt;
    use tower_cookies::CookieManagerLayer;
    use tempfile::TempDir;
 
     // Handler that will be protected by the middleware
     async fn protected_handler() -> StatusCode {
        StatusCode::OK
    }

    // Middleware test wrapper
    async fn auth_middleware_wrapper(
        State(state): State<Arc<AppState>>,
        jar: Cookies,
        auth_header: Option<TypedHeader<Authorization<Bearer>>>,
        req: Request<Body>,
        next: Next,
    ) -> Response {
        match require_admin_token(&state, &jar, auth_header, "test").await {
            Ok(_) => next.run(req).await,
            Err(e) => e.into_response(),
        }
    }

    #[test]
    fn test_get_key_status_str() {
        let now = Utc::now();
        let past = now - Duration::seconds(10);
        let future = now + Duration::seconds(10);

        // 1. No state (None) -> should be "available"
        let (status, reset_time) = get_key_status_str(None, now);
        assert_eq!(status, "available");
        assert_eq!(reset_time, None);

        // 2. Available state
        let state_available = KeyState {
            status: KmKeyStatus::Available,
            reset_time: None,
        };
        let (status, reset_time) = get_key_status_str(Some(&state_available), now);
        assert_eq!(status, "available");
        assert_eq!(reset_time, None);

        // 3. RateLimited, not expired
        let state_limited_pending = KeyState {
            status: KmKeyStatus::RateLimited,
            reset_time: Some(future),
        };
        let (status, reset_time) = get_key_status_str(Some(&state_limited_pending), now);
        assert_eq!(status, "limited");
        assert_eq!(reset_time, Some(future));

        // 4. RateLimited, expired
        let state_limited_expired = KeyState {
            status: KmKeyStatus::RateLimited,
            reset_time: Some(past),
        };
        let (status, reset_time) = get_key_status_str(Some(&state_limited_expired), now);
        assert_eq!(status, "available");
        assert_eq!(reset_time, Some(past));

        // 5. Invalid
        let state_invalid = KeyState {
            status: KmKeyStatus::Invalid,
            reset_time: None,
        };
        let (status, reset_time) = get_key_status_str(Some(&state_invalid), now);
        assert_eq!(status, "invalid");
        assert_eq!(reset_time, None);

        // 6. TemporarilyUnavailable, not expired
        let state_unavailable_pending = KeyState {
            status: KmKeyStatus::TemporarilyUnavailable,
            reset_time: Some(future),
        };
        let (status, reset_time) = get_key_status_str(Some(&state_unavailable_pending), now);
        assert_eq!(status, "unavailable");
        assert_eq!(reset_time, Some(future));

        // 7. TemporarilyUnavailable, expired
        let state_unavailable_expired = KeyState {
            status: KmKeyStatus::TemporarilyUnavailable,
            reset_time: Some(past),
        };
        let (status, reset_time) = get_key_status_str(Some(&state_unavailable_expired), now);
        assert_eq!(status, "available");
        assert_eq!(reset_time, Some(past));
    }

    // --- Tests for require_admin_token ---
 
    async fn setup_state(admin_token: Option<String>) -> (Arc<AppState>, TempDir) {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_path = temp_dir.path().to_path_buf();
        let mut config = AppConfig {
            server: ServerConfig {
                admin_token,
                ..Default::default()
            },
            ..Default::default()
        };
        let app_state = Arc::new(
            AppState::new(&mut config, &config_path)
                .await
                .unwrap(),
        );
        (app_state, temp_dir)
    }
 
     fn app(state: Arc<AppState>) -> Router {
        Router::new()
            .route("/", get(protected_handler))
            .route_layer(from_fn_with_state(state.clone(), auth_middleware_wrapper))
            .layer(CookieManagerLayer::new())
            .with_state(state)
    }

    #[tokio::test]
    async fn test_require_admin_token_no_token_provided() {
        let (state, _temp_dir) = setup_state(Some("secret_token".to_string())).await;
        let app = app(state);
 
         let response = app
             .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_require_admin_token_bearer_valid() {
        let (state, _temp_dir) = setup_state(Some("secret_token".to_string())).await;
        let app = app(state);
 
         let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header(header::AUTHORIZATION, "Bearer secret_token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_require_admin_token_cookie_valid() {
        let (state, _temp_dir) = setup_state(Some("secret_token".to_string())).await;
        let app = app(state);
 
         let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header(header::COOKIE, "admin_token=secret_token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_require_admin_token_bearer_invalid() {
        let (state, _temp_dir) = setup_state(Some("secret_token".to_string())).await;
        let app = app(state);
 
         let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header(header::AUTHORIZATION, "Bearer wrong_token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_require_admin_token_cookie_invalid() {
        let (state, _temp_dir) = setup_state(Some("secret_token".to_string())).await;
        let app = app(state);
 
         let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header(header::COOKIE, "admin_token=wrong_token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_require_admin_token_no_token_configured() {
        let (state, _temp_dir) = setup_state(Some("".to_string())).await;
        let app = app(state);
 
         let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header(header::AUTHORIZATION, "Bearer any_token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}