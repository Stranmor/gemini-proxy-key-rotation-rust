use crate::config::AppConfig;
use crate::error::Result;
use crate::state::KeyState;
use axum::async_trait;
use deadpool_redis::Pool;
use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, instrument, warn};

const ROTATION_SET_KEY: &str = "rotation_set";
const ROTATION_COUNTER_KEY: &str = "rotation_counter";

fn key_state_key(api_key: &str) -> String {
    format!("key_state:{}", api_key)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlattenedKeyInfo {
    pub key: String,
    pub group_name: String,
    pub target_url: String,
    pub proxy_url: Option<String>,
}

/// Manages API key rotation and state.
/// Can operate with a Redis backend for persistence or in-memory for testing.
#[async_trait]
pub trait KeyManagerTrait: Send + Sync {
    async fn get_next_available_key_info(
        &self,
        group_name: Option<&str>,
    ) -> Result<Option<FlattenedKeyInfo>>;
    async fn handle_api_failure(&self, api_key: &str, is_terminal: bool) -> Result<()>;
    async fn get_key_states(&self) -> Result<HashMap<String, KeyState>>;
    fn get_all_key_info(&self) -> HashMap<String, FlattenedKeyInfo>;
}

#[derive(Clone)]
pub struct KeyManager {
    redis_pool: Option<Pool>,
    key_prefix: String,
    key_info_map: Arc<RwLock<HashMap<String, FlattenedKeyInfo>>>,
    // Fields for in-memory mode, crucial for test isolation
    in_memory_key_states: Arc<RwLock<HashMap<String, KeyState>>>,
    in_memory_counters: Arc<RwLock<HashMap<String, AtomicUsize>>>,
}

impl KeyManager {
    #[instrument(skip(config), name = "key_manager_init")]
    pub async fn new(config: &AppConfig, redis_pool: Option<Pool>) -> Result<Self> {
        let key_prefix = config.redis_key_prefix.clone().unwrap_or_else(|| "gemini_proxy:".to_string());
        let mut key_info_map = HashMap::new();
        let mut keys_from_config = Vec::new();

        for group in &config.groups {
            for api_key in &group.api_keys {
                let flattened_info = FlattenedKeyInfo {
                    key: api_key.clone(),
                    group_name: group.name.clone(),
                    target_url: group.target_url.clone(),
                    proxy_url: group.proxy_url.clone(),
                };
                key_info_map.insert(api_key.clone(), flattened_info);
                keys_from_config.push(api_key.clone());
            }
        }

        let mut in_memory_key_states = HashMap::new();
        for (key, info) in &key_info_map {
            in_memory_key_states.insert(key.clone(), KeyState {
                key: key.clone(),
                group_name: info.group_name.clone(),
                is_blocked: false,
                consecutive_failures: 0,
                last_failure: None,
            });
        }

        if let Some(pool) = redis_pool.as_ref() {
            info!("Redis pool provided. Initializing Redis state...");
            let mut conn = pool.get().await?;
            let rotation_set_key = format!("{key_prefix}{ROTATION_SET_KEY}");

            if config.server.test_mode {
                info!("Test mode detected. Forcing re-initialization of Redis from config.");
                // Simplified cleanup logic for clarity
                let all_keys_in_redis: Vec<String> = conn.smembers(&rotation_set_key).await?;
                if !all_keys_in_redis.is_empty() {
                    let state_keys: Vec<String> = all_keys_in_redis.iter().map(|k| format!("{}{}", key_prefix, key_state_key(k))).collect();
                    let _: () = conn.del(state_keys).await?;
                }
                let _: () = conn.del(&rotation_set_key).await?;
                let counter_pattern = format!("{key_prefix}{ROTATION_COUNTER_KEY}:*");
                let counters: Vec<String> = conn.keys(&counter_pattern).await?;
                if !counters.is_empty() {
                    let _: () = conn.del(counters).await?;
                }
                 let _: () = conn.del(format!("{key_prefix}{ROTATION_COUNTER_KEY}")).await?;
                info!("Cleared stale KeyManager keys from Redis for test isolation.");
            }
            
            let key_count: usize = conn.scard(&rotation_set_key).await?;
            if key_count == 0 {
                info!("Redis is empty. Initializing from config.");
                initialize_redis_from_config(
                    &mut conn,
                    &key_prefix,
                    &keys_from_config,
                    &key_info_map,
                )
                .await?;
            } else {
                info!(
                    "Found {} keys in Redis set '{}'. Skipping initialization from config.",
                    key_count, rotation_set_key
                );
            }
        } else {
            info!("No Redis pool provided. KeyManager will operate in in-memory mode.");
        }

        Ok(Self {
            redis_pool,
            key_prefix,
            key_info_map: Arc::new(RwLock::new(key_info_map)),
            in_memory_key_states: Arc::new(RwLock::new(in_memory_key_states)),
            in_memory_counters: Arc::new(RwLock::new(HashMap::new())),
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
        // If Redis is not available, fall back to in-memory rotation
        if self.redis_pool.is_none() {
            return self.get_next_available_key_info_memory(group_name).await;
        }

        let redis_pool = self.redis_pool.as_ref().unwrap();
        let mut conn = redis_pool.get().await?;
        let key_info_map_guard = self.key_info_map.read().await;

        // 1. Get all keys from the global set.
        let all_keys_from_redis: Vec<String> =
            conn.smembers(self.prefix_key(ROTATION_SET_KEY)).await?;

        // 2. Filter keys by the requested group, if provided.
        let mut candidate_keys: Vec<String> = if let Some(gn) = group_name {
            all_keys_from_redis
                .into_iter()
                .filter(|k| {
                    key_info_map_guard
                        .get(k)
                        .is_some_and(|info| info.group_name == gn)
                })
                .collect()
        } else {
            all_keys_from_redis
        };

        // 3. Sort the candidate keys to ensure a deterministic round-robin order.
        candidate_keys.sort();

        if candidate_keys.is_empty() {
            warn!(
                group_name = group_name,
                "No keys available for the specified group in Redis."
            );
            return Ok(None);
        }

        // 4. Atomically get the next key index for the group.
        let group_id = group_name.unwrap_or("__default_all_keys__");
        let counter_key = self.prefix_key(&format!("{}:{}", ROTATION_COUNTER_KEY, group_id));
        let index: usize = conn.incr(&counter_key, 1).await?;

        // 5. Try to find an unblocked key starting from the calculated index.
        for i in 0..candidate_keys.len() {
            let final_index = (index + i) % candidate_keys.len();
            let selected_key_str = &candidate_keys[final_index];

            let state_key = self.prefix_key(&key_state_key(selected_key_str));
            let is_blocked: bool = conn.hget(&state_key, "is_blocked").await?;

            if !is_blocked {
                if let Some(key_info) = key_info_map_guard.get(selected_key_str) {
                    debug!(
                        api_key.preview = %Self::preview(&key_info.key),
                        group = %key_info.group_name,
                        index = final_index,
                        "Selected available API key from Redis (round-robin)"
                    );
                    return Ok(Some(key_info.clone()));
                }
            }
        }

        warn!(
            group_name = group_name,
            "All keys for the specified group are currently blocked in Redis."
        );
        Ok(None)
    }

    async fn get_next_available_key_info_memory(
        &self,
        group_name: Option<&str>,
    ) -> Result<Option<FlattenedKeyInfo>> {
        let key_info_map_guard = self.key_info_map.read().await;
        let states_guard = self.in_memory_key_states.read().await;

        // 1. Filter keys by group first
        let mut candidate_keys: Vec<&FlattenedKeyInfo> = if let Some(gn) = group_name {
            key_info_map_guard
                .values()
                .filter(|info| info.group_name == gn)
                .collect()
        } else {
            key_info_map_guard.values().collect()
        };

        // 2. Sort them for deterministic order
        candidate_keys.sort_by(|a, b| a.key.cmp(&b.key));

        if candidate_keys.is_empty() {
            warn!(group_name = group_name, "No keys found for the specified group in memory.");
            return Ok(None);
        }

        // 3. Atomically get the next index for the group
        let group_id = group_name.unwrap_or("__default_all_keys__").to_string();
        let mut counters_guard = self.in_memory_counters.write().await;
        let counter = counters_guard
            .entry(group_id)
            .or_insert_with(|| AtomicUsize::new(0));
        let index = counter.fetch_add(1, Ordering::SeqCst);

        // 4. Try to find an unblocked key starting from the calculated index
        for i in 0..candidate_keys.len() {
            let final_index = (index + i) % candidate_keys.len();
            let selected_key_info = candidate_keys[final_index];

            if states_guard
                .get(&selected_key_info.key)
                .map_or(true, |s| !s.is_blocked)
            {
                debug!(
                    api_key.preview = %Self::preview(&selected_key_info.key),
                    group = %selected_key_info.group_name,
                    index = final_index,
                    "Selected available API key from memory (round-robin)"
                );
                return Ok(Some(selected_key_info.clone()));
            }
        }

        warn!(group_name = group_name, "All keys for the specified group are currently blocked in memory.");
        Ok(None)
    }

    #[instrument(level = "warn", skip(self, api_key), fields(api_key.preview = %KeyManager::preview(api_key), is_terminal))]
    pub async fn handle_api_failure(&self, api_key: &str, is_terminal: bool) -> Result<()> {
        if self.redis_pool.is_none() {
            let mut states_guard = self.in_memory_key_states.write().await;
            if let Some(state) = states_guard.get_mut(api_key) {
                state.consecutive_failures += 1;
                state.last_failure = Some(chrono::Utc::now());
                if is_terminal {
                    state.is_blocked = true;
                }
                if state.consecutive_failures >= 3 {
                    state.is_blocked = true;
                }
                 warn!(
                    api_key.preview = %KeyManager::preview(api_key),
                    is_terminal = is_terminal,
                    is_blocked = state.is_blocked,
                    failures = state.consecutive_failures,
                    "API failure handled in memory mode."
                );
            }
            return Ok(());
        }

        let redis_pool = self.redis_pool.as_ref().unwrap();
        let mut conn = redis_pool.get().await?;
        let state_key = self.prefix_key(&key_state_key(api_key));

        // Use HINCRBY to atomically increment the failure count
        let new_failure_count: i64 = conn.hincr(&state_key, "consecutive_failures", 1).await?;

        // Set the last failure time
        let now = chrono::Utc::now().to_rfc3339();
        let _: () = conn.hset(&state_key, "last_failure", &now).await?;

        // Block the key if the failure is terminal or the threshold is reached
        let max_failures = 3i64; // Default threshold

        if is_terminal || new_failure_count >= max_failures {
            let _: () = conn.hset(&state_key, "is_blocked", true).await?;
            warn!(
                api_key.preview = %KeyManager::preview(api_key),
                is_terminal = is_terminal,
                failures = new_failure_count,
                "API key has been blocked."
            );
        }
        Ok(())
    }

    /// Returns a clone of the in-memory key info map.
    pub async fn get_all_key_info(&self) -> HashMap<String, FlattenedKeyInfo> {
        self.key_info_map.read().await.clone()
    }

    /// Fetches the state of all keys from Redis or memory.
    #[instrument(level = "debug", skip(self))]
    pub async fn get_key_states(&self) -> Result<HashMap<String, KeyState>> {
        if self.redis_pool.is_none() {
            let states_guard = self.in_memory_key_states.read().await;
            return Ok(states_guard.clone());
        }

        let redis_pool = self.redis_pool.as_ref().unwrap();
        let mut conn = redis_pool.get().await?;
        let rotation_set_key = self.prefix_key(ROTATION_SET_KEY);
        let all_keys: Vec<String> = conn.smembers(&rotation_set_key).await?;
        let mut states = HashMap::new();

        for key in all_keys {
            let state_key = self.prefix_key(&key_state_key(&key));
            let redis_state: HashMap<String, String> = conn.hgetall(&state_key).await?;

            let last_failure = redis_state
                .get("last_failure")
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));

            let state = KeyState {
                key: key.clone(),
                group_name: self
                    .key_info_map
                    .read()
                    .await
                    .get(&key)
                    .map_or_else(|| "unknown".to_string(), |info| info.group_name.clone()),
                is_blocked: redis_state
                    .get("is_blocked")
                    .and_then(|s| s.parse::<bool>().ok())
                    .unwrap_or(false),
                consecutive_failures: redis_state
                    .get("consecutive_failures")
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0),
                last_failure,
            };
            states.insert(key, state);
        }

        Ok(states)
    }

    fn preview(key: &str) -> String {
        if key.len() > 8 {
            format!("{}...{}", &key[..4], &key[key.len() - 4..])
        } else {
            key.to_string()
        }
    }
}

/// Initializes Redis state from the configuration file.
#[instrument(skip(conn, key_prefix, keys_from_config, key_info_map))]
async fn initialize_redis_from_config(
    conn: &mut MultiplexedConnection,
    key_prefix: &str,
    keys_from_config: &[String],
    key_info_map: &HashMap<String, FlattenedKeyInfo>,
) -> Result<()> {
    info!(
        "Initializing Redis with {} keys from config...",
        keys_from_config.len()
    );
    let rotation_set_key = format!("{key_prefix}{ROTATION_SET_KEY}");

    // Use a pipeline for efficiency
    let mut pipe = redis::pipe();
    pipe.atomic(); // Make it a transaction

    // Add all keys to the rotation set
    pipe.sadd(&rotation_set_key, keys_from_config);

    // For each key, create its initial state hash
    for key in keys_from_config {
        let state_key = format!("{key_prefix}{}", key_state_key(key));
        let group_name = key_info_map
            .get(key)
            .map_or("unknown", |info| &info.group_name);
        pipe.hset_multiple(
            &state_key,
            &[
                ("key", key.as_str()),
                ("group_name", group_name),
                ("is_blocked", "false"),
                ("consecutive_failures", "0"),
            ],
        );
    }

    // Execute the pipeline
    let _: () = pipe.query_async(conn).await?;
    info!("Successfully initialized Redis state.");
    Ok(())
}
