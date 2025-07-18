// src/admin.rs

use crate::config::AppConfig;
use crate::error::{AppError, Result};
use crate::key_manager::{KeyStatus as KmKeyStatus}; // Renamed to avoid conflict
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, Json},
    routing::{delete, get, post, put},
    Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use sysinfo::{System};
use tokio::sync::Mutex;
use tracing::{warn};

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
    Router::new()
        .route("/admin", get(serve_dashboard))
        .route("/admin/health", get(detailed_health))
        .route("/admin/keys", get(list_keys))
        .route("/admin/keys/:key_id", delete(remove_key))
        .route("/admin/keys", post(add_key))
        .route("/admin/config", get(get_config))
        .route("/admin/config", put(update_config))
        .route("/admin/metrics", get(get_metrics_summary))
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
pub struct AddKeyRequest {
    pub group_name: String,
    pub api_key: String,
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

/// Detailed health check endpoint
///
/// # Errors
///
/// This function currently does not return any errors, but is declared as `Result`
/// for future compatibility where I/O or other operations might fail.
#[axum::debug_handler]
pub async fn detailed_health(State(state): State<Arc<AppState>>) -> Result<Json<DetailedHealthStatus>> {
    let all_key_info = state.key_manager.get_all_key_info();
    let key_states = state.key_manager.get_key_states().await;
    let now = Utc::now();

    let mut active_keys = 0;
    let mut limited_keys = 0;
    let mut invalid_keys = 0;
    let mut temp_unavailable_keys = 0;

    for key_info in &all_key_info {
        match key_states.get(&key_info.key) {
            Some(state) => match state.status {
                KmKeyStatus::Available => active_keys += 1,
                KmKeyStatus::RateLimited => {
                    if state.reset_time.is_some_and(|rt| now >= rt) {
                        active_keys += 1;
                    } else {
                        limited_keys += 1;
                    }
                }
                KmKeyStatus::Invalid => invalid_keys += 1,
                KmKeyStatus::TemporarilyUnavailable => {
                    if state.reset_time.is_some_and(|rt| now >= rt) {
                        active_keys += 1;
                    } else {
                        temp_unavailable_keys += 1;
                    }
                }
            },
            None => active_keys += 1, // Default to active if no state
        }
    }

    // Build group status
    let mut groups_map: HashMap<String, GroupStatus> = HashMap::new();
    for key_info in &all_key_info {
        let entry = groups_map.entry(key_info.group_name.clone()).or_insert_with(|| {
            GroupStatus {
                name: key_info.group_name.clone(),
                total_keys: 0,
                active_keys: 0,
                proxy_url: key_info.proxy_url.clone(),
                target_url: key_info.target_url.clone(),
            }
        });
        entry.total_keys += 1;

        // Check if this specific key is active
        let is_active = match key_states.get(&key_info.key) {
            Some(state) => match state.status {
                KmKeyStatus::Available => true,
                KmKeyStatus::RateLimited | KmKeyStatus::TemporarilyUnavailable => {
                    state.reset_time.is_some_and(|rt| now >= rt)
                }
                KmKeyStatus::Invalid => false,
            },
            None => true, // Default to active
        };
        if is_active {
            entry.active_keys += 1;
        }
    }
    let group_statuses = groups_map.into_values().collect();

    // Build proxy status
    let proxy_status = HashMap::new();
    // TODO: Implement proxy health checks

    let uptime = state.start_time.elapsed().as_secs();

    let health_status = DetailedHealthStatus {
        status: "healthy".to_string(),
        timestamp: now,
        version: option_env!("CARGO_PKG_VERSION").unwrap_or("N/A").to_string(),
        uptime_seconds: uptime,
        server_info: ServerInfo {
            host: "0.0.0.0".to_string(),
            port: state.config.server.port,
            rust_version: "N/A".to_string(), // sysinfo doesn't provide this
            build_info: BuildInfo {
                version: option_env!("CARGO_PKG_VERSION").unwrap_or("N/A").to_string(),
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
    let key_states = state.key_manager.get_key_states().await;
    let all_key_info = state.key_manager.get_all_key_info();
    let now = Utc::now();

    let mut keys = Vec::new();

    for key_info in &all_key_info {
        // Filter by group if specified
        if let Some(ref group_filter) = query.group {
            if &key_info.group_name != group_filter {
                continue;
            }
        }

        let (status_str, reset_time) = match key_states.get(&key_info.key) {
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
        };

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

/// Add a new API key
///
/// # Errors
///
/// Returns an error as this feature is not yet implemented.
#[axum::debug_handler]
pub async fn add_key(
    State(_state): State<Arc<AppState>>,
    Json(_request): Json<AddKeyRequest>,
) -> Result<Json<serde_json::Value>> {
    // TODO: Implement dynamic key addition
    warn!("Dynamic key addition not yet implemented");
    Err(AppError::Internal(
        "Dynamic key addition not implemented".to_string(),
    ))
}

/// Remove an API key
///
/// # Errors
///
/// Returns an error as this feature is not yet implemented.
#[axum::debug_handler]
pub async fn remove_key(
    State(_state): State<Arc<AppState>>,
    Path(key_id): Path<String>,
) -> Result<StatusCode> {
    // TODO: Implement dynamic key removal
    warn!(key_id = %key_id, "Dynamic key removal not yet implemented");
    Err(AppError::Internal(
        "Dynamic key removal not implemented".to_string(),
    ))
}

/// Get current configuration
///
/// # Errors
///
/// Returns an error as this feature is not yet implemented.
#[axum::debug_handler]
pub async fn get_config(State(state): State<Arc<AppState>>) -> Result<Json<AppConfig>> {
    Ok(Json(state.config.clone()))
}

/// Update configuration
///
/// # Errors
///
/// Returns an error as this feature is not yet implemented.
#[axum::debug_handler]
pub async fn update_config(
    State(_state): State<Arc<AppState>>,
    Json(_request): Json<ConfigUpdateRequest>,
) -> Result<Json<serde_json::Value>> {
    // TODO: Implement configuration updates
    Err(AppError::Internal(
        "Config updates not implemented".to_string(),
    ))
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
/// Serve the admin dashboard
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