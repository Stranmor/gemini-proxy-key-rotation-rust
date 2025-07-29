// src/key_manager.rs

use crate::config::AppConfig;
use crate::error::{AppError, Result};
use crate::state::KeyState;
use deadpool_redis::redis::AsyncCommands;
use deadpool_redis::Pool;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, instrument, warn};

// --- Constants for Redis Keys ---

/// A Redis SET holding all active API keys for rotation.
const ROTATION_SET_KEY: &str = "keys:rotation_set";
/// A Redis string used as an atomic counter for round-robin indexing.
const ROTATION_COUNTER_KEY: &str = "keys:rotation_counter";

/// Returns the Redis key for the HASH storing a single key's state.
fn key_state_key(api_key: &str) -> String {
    format!("key:state:{api_key}")
}


// --- Data Structures ---

#[derive(Debug, Clone)]
pub struct FlattenedKeyInfo {
    pub key: String,
    pub proxy_url: Option<String>,
    pub target_url: String,
    pub group_name: String,
    pub top_p: Option<f32>,
}

pub struct KeyManager {
    redis_pool: Pool,
    key_prefix: String,
    /// In-memory map for quick lookups of key metadata (like URLs) after getting a key from Redis.
    /// This is populated from the config file at startup and assumed to be static until restart.
    key_info_map: Arc<RwLock<HashMap<String, FlattenedKeyInfo>>>,
}

impl fmt::Debug for KeyManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyManager")
            .field("key_prefix", &self.key_prefix)
            .field("key_info_map", &self.key_info_map)
            .finish_non_exhaustive() // Indicates that redis_pool is omitted
    }
}


// --- Implementation ---

impl KeyManager {
    #[instrument(level = "info", skip(config, redis_pool))]
    pub async fn new(config: &AppConfig, redis_pool: Pool) -> Result<Self> {
        info!("Initializing KeyManager with Redis...");
        let mut conn = redis_pool.get().await?;
        let key_prefix = config.redis_key_prefix.clone().unwrap_or_default();

        let mut key_info_map = HashMap::new();
        let mut keys_from_config = Vec::new();

        // 1. Populate in-memory map and a temporary list of keys from the config file.
        for group in &config.groups {
            for api_key in &group.api_keys {
                if api_key.trim().is_empty() {
                    warn!(group.name = %group.name, "Skipping empty API key string.");
                    continue;
                }
                let key_info = FlattenedKeyInfo {
                    key: api_key.clone(),
                    proxy_url: group.proxy_url.clone(),
                    target_url: group.target_url.clone(),
                    group_name: group.name.clone(),
                    top_p: group.top_p,
                };
                key_info_map.insert(api_key.clone(), key_info);
                keys_from_config.push(api_key.clone());
            }
        }

        // 2. Determine initialization strategy.
        let rotation_set_key = format!("{key_prefix}{ROTATION_SET_KEY}");

        // In test mode, ALWAYS clear and re-initialize Redis from the current config
        // to ensure test isolation. This happens regardless of what's already in Redis.
        if config.server.test_mode {
            info!("Test mode detected. Forcing re-initialization of Redis from config.");

            // Clear any existing keys for this test to ensure isolation
            let all_keys_in_redis: Vec<String> = conn.smembers(&rotation_set_key).await?;
            for key in &all_keys_in_redis {
                 let _: () = conn.del(format!("{}{}", key_prefix, key_state_key(key))).await?;
            }
            let _: () = conn.del(&rotation_set_key).await?;
            let counter_key = format!("{key_prefix}{ROTATION_COUNTER_KEY}");
            let counters: Vec<String> = conn.keys(format!("{counter_key}:*")).await?;
            if !counters.is_empty() {
                let _: () = conn.del(counters).await?;
            }
            info!("Cleared any stale KeyManager keys from Redis for test isolation.");

            initialize_redis_from_config(&mut conn, &key_prefix, &keys_from_config, &key_info_map).await?;
        } else {
            let key_count: usize = conn.scard(&rotation_set_key).await?;
            if key_count == 0 {
                info!("Production mode and Redis is empty. Initializing from config.");
                initialize_redis_from_config(&mut conn, &key_prefix, &keys_from_config, &key_info_map).await?;
            } else {
                info!(
                    "Found {} keys in Redis set '{}'. Skipping initialization from config (production mode).",
                    key_count, rotation_set_key
                );
            }
        }

        Ok(Self {
            redis_pool,
            key_prefix,
            key_info_map: Arc::new(RwLock::new(key_info_map)),
        })
    }

    fn prefix_key(&self, key: &str) -> String {
        format!("{}{}", self.key_prefix, key)
    }

    #[instrument(level = "debug", skip(self), fields(group_name = group_name))]
    pub async fn get_next_available_key_info(
        &self,
        group_name: Option<&str>,
    ) -> Result<Option<FlattenedKeyInfo>> {
        let mut conn = self.redis_pool.get().await?;
        let key_info_map_guard = self.key_info_map.read().await;

        // 1. Get all keys from the global set.
        let all_keys_from_redis: Vec<String> = conn.smembers(self.prefix_key(ROTATION_SET_KEY)).await?;

        // 2. Filter keys by the requested group, if provided.
        let mut candidate_keys: Vec<String> = if let Some(gn) = group_name {
            all_keys_from_redis
                .into_iter()
                .filter(|k| key_info_map_guard.get(k).is_some_and(|info| info.group_name == gn))
                .collect()
        } else {
            all_keys_from_redis
        };

        // 3. Sort the candidate keys to ensure a deterministic round-robin order.
        candidate_keys.sort();

        if candidate_keys.is_empty() {
            warn!(
                group_name = group_name,
                "No keys available for the specified group."
            );
            return Ok(None);
        }

        // 4. Use a group-specific atomic counter for round-robin.
        let counter_key = group_name.map_or_else(
            || self.prefix_key(ROTATION_COUNTER_KEY),
            |gn| self.prefix_key(&format!("{ROTATION_COUNTER_KEY}:{gn}")),
        );
        let counter_val = conn.incr::<_, _, i64>(&counter_key, 1).await?;
        let start_index = ((counter_val - 1) as usize) % candidate_keys.len();

        // 5. Loop through candidate keys to find an available one, starting from the current index.
        for i in 0..candidate_keys.len() {
            let current_index = (start_index + i) % candidate_keys.len();
            let key = &candidate_keys[current_index];

            let state_json: Option<String> = conn.get(self.prefix_key(&key_state_key(key))).await?;

            if let Some(json_string) = state_json {
                match serde_json::from_str::<KeyState>(&json_string) {
                    Ok(state) => {
                        if !state.is_blocked {
                            let key_info_map_guard = self.key_info_map.read().await;
                            if let Some(key_info) = key_info_map_guard.get(key) {
                                debug!(api_key.preview = %Self::preview(key), "Selected available API key from Redis");
                                return Ok(Some(key_info.clone()));
                            } else {
                                warn!(api_key = %key, "Key found in Redis but not in config map. It might be stale.");
                            }
                        }
                    }
                    Err(e) => {
                        error!(api_key = %key, error = %e, "Failed to parse KeyState from Redis. Skipping key.");
                    }
                }
            } else {
                warn!(api_key = %key, "Key state not found in Redis. It might be stale.");
            }
        }

        warn!("All API keys checked in Redis are currently blocked or unavailable.");
        Ok(None)
    }

    #[instrument(level = "warn", skip(self, api_key), fields(api_key.preview = %KeyManager::preview(api_key), is_terminal))]
    pub async fn handle_api_failure(&self, api_key: &str, is_terminal: bool) -> Result<()> {
        let mut conn = self.redis_pool.get().await?;
        let state_key = self.prefix_key(&key_state_key(api_key));

        let state_json: Option<String> = conn.get(&state_key).await?;

        if let Some(json_string) = state_json {
            let mut state: KeyState = serde_json::from_str(&json_string)
                .map_err(|e| AppError::Internal(format!("Failed to parse KeyState for failure handling: {e}")))?;

            state.consecutive_failures += 1;
            state.last_failure = Some(chrono::Utc::now());

            // Block if the error is terminal (e.g., API_KEY_INVALID)
            if is_terminal {
                state.is_blocked = true;
                warn!(api_key.preview = %KeyManager::preview(api_key), "Blocking key due to terminal error.");
            }
            // Also block if consecutive failures reach the threshold.
            // This is a separate `if` so that a terminal error on the 3rd attempt still gets logged correctly
            // and ensures the key is blocked.
            if state.consecutive_failures >= 3 {
                state.is_blocked = true;
                warn!(api_key.preview = %KeyManager::preview(api_key), failures = state.consecutive_failures, "Blocking key due to excessive failures.");
            }

            let new_state_json = serde_json::to_string(&state)
                .map_err(|e| AppError::Internal(format!("Failed to serialize updated KeyState: {e}")))?;
            conn.set::<_, _, ()>(state_key, new_state_json).await?;
        } else {
            warn!(api_key = %api_key, "Attempted to handle failure for a key with no state in Redis.");
        }

        Ok(())
    }

    /// Returns a clone of the in-memory key info map.
    pub async fn get_all_key_info(&self) -> HashMap<String, FlattenedKeyInfo> {
        self.key_info_map.read().await.clone()
    }

    /// Fetches the state of all keys from Redis.
    #[instrument(level = "debug", skip(self))]
    pub async fn get_key_states(&self) -> Result<HashMap<String, KeyState>> {
        let mut conn = self.redis_pool.get().await?;
        let all_keys: Vec<String> = conn.smembers(self.prefix_key(ROTATION_SET_KEY)).await?;
        let mut states = HashMap::new();

        for key in all_keys {
            let state_json: Option<String> = conn.get(self.prefix_key(&key_state_key(&key))).await?;
            if let Some(json_string) = state_json {
                match serde_json::from_str::<KeyState>(&json_string) {
                    Ok(state) => {
                        states.insert(key, state);
                    }
                    Err(e) => {
                        error!(api_key = %key, error = %e, "Failed to parse KeyState from Redis during bulk fetch.");
                    }
                }
            }
        }
        Ok(states)
    }

    /// Generates a shortened preview of an API key for logging.
    #[inline]
    fn preview(key: &str) -> String {
        let len = key.chars().count();
        let end = std::cmp::min(6, len);
        let start = if len > 10 { len - 4 } else { len };
        if len > 10 {
            format!(
                "{}...{}",
                key.chars().take(end).collect::<String>(),
                key.chars().skip(start).collect::<String>()
            )
        } else {
            format!("{}...", key.chars().take(end).collect::<String>())
        }
    }
}

/// Helper function to populate Redis with keys and their initial states.
async fn initialize_redis_from_config(
    conn: &mut deadpool_redis::Connection,
    key_prefix: &str,
    keys_from_config: &[String],
    key_info_map: &HashMap<String, FlattenedKeyInfo>,
) -> Result<()> {
    info!("Initializing Redis key set from config.yaml ({} keys)...", keys_from_config.len());
    if !keys_from_config.is_empty() {
        conn.sadd::<_, _, ()>(format!("{key_prefix}{ROTATION_SET_KEY}"), keys_from_config).await?;

        for api_key in keys_from_config {
            let state = KeyState {
                key: api_key.clone(),
                group_name: key_info_map
                    .get(api_key)
                    .map_or_else(|| "unknown".to_string(), |ki| ki.group_name.clone()),
                is_blocked: false,
                consecutive_failures: 0,
                last_failure: None,
            };
            let state_json = serde_json::to_string(&state)
                .map_err(|e| AppError::Internal(format!("Failed to serialize initial KeyState: {e}")))?;
            conn.set::<_, _, ()>(format!("{key_prefix}{}", key_state_key(api_key)), state_json).await?;
        }
        info!(
            "Added {} keys and their initial states to Redis.",
            keys_from_config.len()
        );
    } else {
        warn!("No API keys found in config.yaml to initialize Redis with.");
    }
    Ok(())
}


#[cfg(test)]
mod tests {
    // Note: These tests require a running Redis instance on the default port.
    // They should be run with `cargo test -- --test-threads=1` to avoid race conditions
    // with the shared Redis state.

    // Most of the old tests are difficult to adapt without significant mocking of Redis.
    // A full test suite would require a library like `redis-test` or manual setup/teardown logic.
    // For now, we will keep this empty as per the task instructions.
}
