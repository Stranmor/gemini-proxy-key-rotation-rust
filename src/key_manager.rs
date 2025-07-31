use crate::config::AppConfig;
use crate::error::{AppError, Result};
use crate::state::KeyState;
use axum::async_trait;
use deadpool_redis::Pool;
use redis::aio::MultiplexedConnection;
use redis::{AsyncCommands};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, instrument, warn};

// --- Constants ---
const ROTATION_SET_KEY: &str = "rotation_set";
const ROTATION_COUNTER_KEY: &str = "rotation_counter";
const DEFAULT_GROUP_ID: &str = "__default_all_keys__";

// --- Helper Functions ---
fn key_state_key(api_key: &str) -> String {
    format!("key_state:{api_key}")
}

// --- Data Structures ---
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlattenedKeyInfo {
    pub key: String,
    pub group_name: String,
    pub target_url: String,
    pub proxy_url: Option<String>,
    // Suggestion: Move max_failures_threshold here from AppConfig for per-group limits
    // pub max_failures_threshold: u32, 
}

// --- Main Trait for KeyManager ---
#[async_trait]
pub trait KeyManagerTrait: Send + Sync {
    async fn get_next_available_key_info(
        &self,
        group_name: Option<&str>,
    ) -> Result<Option<FlattenedKeyInfo>>;
    async fn handle_api_failure(&self, api_key: &str, is_terminal: bool) -> Result<()>;
    async fn get_key_states(&self) -> Result<HashMap<String, KeyState>>;
    async fn get_all_key_info(&self) -> HashMap<String, FlattenedKeyInfo>;
}

// --- Storage Abstraction (`KeyStore` Trait) ---
#[async_trait]
trait KeyStore: Send + Sync {
    async fn get_candidate_keys(&self) -> Result<Vec<String>>;
    async fn get_next_rotation_index(&self, group_id: &str) -> Result<usize>;
    async fn update_failure_state(&self, api_key: &str, is_terminal: bool, max_failures: u32) -> Result<KeyState>;
    async fn get_key_state(&self, key: &str) -> Result<Option<KeyState>>;
    async fn get_all_key_states(&self) -> Result<HashMap<String, KeyState>>;
}

// --- KeyManager Implementation ---
#[derive(Clone)]
pub struct KeyManager {
    store: Arc<dyn KeyStore>,
    key_info_map: Arc<HashMap<String, FlattenedKeyInfo>>,
    max_failures_threshold: u32,
}

impl KeyManager {
    #[instrument(skip(config, redis_pool), name = "key_manager_init")]
    pub async fn new(config: &AppConfig, redis_pool: Option<Pool>) -> Result<Self> {
        let key_info_map = Self::build_key_info_map(config);
        
        let store: Arc<dyn KeyStore> = if let Some(pool) = redis_pool {
            info!("Redis pool provided. KeyManager will operate in Redis mode.");
            let redis_store = RedisStore::new(pool, config, &key_info_map).await?;
            Arc::new(redis_store)
        } else {
            info!("No Redis pool provided. KeyManager will operate in in-memory mode.");
            let in_memory_store = InMemoryStore::new(&key_info_map);
            Arc::new(in_memory_store)
        };
        
        Ok(Self {
            store,
            key_info_map: Arc::new(key_info_map),
            max_failures_threshold: config.max_failures_threshold.unwrap_or(3),
        })
    }

    fn build_key_info_map(config: &AppConfig) -> HashMap<String, FlattenedKeyInfo> {
        config.groups.iter().flat_map(|group| {
            group.api_keys.iter().map(|api_key| {
                let flattened_info = FlattenedKeyInfo {
                    key: api_key.clone(),
                    group_name: group.name.clone(),
                    target_url: group.target_url.clone(),
                    proxy_url: group.proxy_url.clone(),
                };
                (api_key.clone(), flattened_info)
            })
        }).collect()
    }
    
    pub fn preview_key(key: &str) -> String {
        if key.len() > 8 {
            format!("{}...{}", &key[..4], &key[key.len() - 4..])
        } else {
            key.to_string()
        }
    }

    fn log_key_selection(&self, key_info: &FlattenedKeyInfo, rotation_method: &str, total_candidates: usize) {
        info!(
            event = "key_selected",
            api_key.preview = %Self::preview_key(&key_info.key),
            group = %key_info.group_name,
            rotation_method,
            total_candidates,
            "API key selected for request"
        );
    }

    fn log_failure_handling(&self, api_key: &str, is_terminal: bool, state: &KeyState) {
        if state.is_blocked {
            warn!(
                event = "key_blocked",
                api_key.preview = %Self::preview_key(api_key),
                is_terminal,
                failures = state.consecutive_failures,
                max_failures = self.max_failures_threshold,
                block_reason = if is_terminal { "terminal_error" } else { "failure_threshold" },
                "API key has been blocked due to failures"
            );
        } else {
            info!(
                event = "key_failure_recorded",
                api_key.preview = %Self::preview_key(api_key),
                is_terminal,
                failures = state.consecutive_failures,
                max_failures = self.max_failures_threshold,
                "API key failure recorded, key still available"
            );
        }
    }
}

// --- KeyManager Trait Implementation ---
#[async_trait]
impl KeyManagerTrait for KeyManager {
    #[instrument(level = "debug", skip(self), fields(group_name))]
    async fn get_next_available_key_info(
        &self,
        group_name: Option<&str>,
    ) -> Result<Option<FlattenedKeyInfo>> {
        let all_keys = self.store.get_candidate_keys().await?;
        
        let mut candidate_keys: Vec<_> = all_keys.iter().filter_map(|key| {
            self.key_info_map.get(key)
        }).filter(|info| {
            group_name.is_none_or(|gn| info.group_name == gn)
        }).collect();

        candidate_keys.sort_by(|a, b| a.key.cmp(&b.key));
        
        if candidate_keys.is_empty() {
            warn!(group_name, "No keys available for the specified group.");
            return Ok(None);
        }
        
        let group_id = group_name.unwrap_or(DEFAULT_GROUP_ID);
        let start_index = self.store.get_next_rotation_index(group_id).await?;

        for i in 0..candidate_keys.len() {
            let key_info = candidate_keys[(start_index + i) % candidate_keys.len()];
            if let Ok(Some(state)) = self.store.get_key_state(&key_info.key).await {
                if !state.is_blocked {
                    self.log_key_selection(key_info, "round_robin", candidate_keys.len());
                    return Ok(Some(key_info.clone()));
                }
            } else {
                 // If state is missing or there's an error, assume it's usable
                 self.log_key_selection(key_info, "round_robin", candidate_keys.len());
                 return Ok(Some(key_info.clone()));
            }
        }
        
        warn!(group_name, "All keys for the specified group are currently blocked.");
        Ok(None)
    }

    #[instrument(level = "warn", skip(self, api_key), fields(api_key.preview = %KeyManager::preview_key(api_key), is_terminal))]
    async fn handle_api_failure(&self, api_key: &str, is_terminal: bool) -> Result<()> {
        let updated_state = self.store.update_failure_state(api_key, is_terminal, self.max_failures_threshold).await?;
        self.log_failure_handling(api_key, is_terminal, &updated_state);
        Ok(())
    }

    #[instrument(level = "debug", skip(self))]
    async fn get_key_states(&self) -> Result<HashMap<String, KeyState>> {
        self.store.get_all_key_states().await
    }

    async fn get_all_key_info(&self) -> HashMap<String, FlattenedKeyInfo> {
        self.key_info_map.as_ref().clone()
    }
}


// --- Redis Store Implementation ---
struct RedisStore {
    pool: Pool,
    key_prefix: String,
    key_info_map: Arc<HashMap<String, FlattenedKeyInfo>>,
}

impl RedisStore {
    async fn new(pool: Pool, config: &AppConfig, key_info_map: &HashMap<String, FlattenedKeyInfo>) -> Result<Self> {
        let key_prefix = config.redis_key_prefix.clone().unwrap_or_else(|| "gemini_proxy:".to_string());
        let keys_from_config: Vec<String> = key_info_map.keys().cloned().collect();

        let mut conn = pool.get().await?;
        let rotation_set_key = format!("{key_prefix}{ROTATION_SET_KEY}");

        if config.server.test_mode {
            Self::clear_redis_for_test_mode(&mut conn, &key_prefix, &rotation_set_key).await?;
        }

        let key_count: usize = conn.scard(&rotation_set_key).await?;
        if key_count == 0 {
            info!("Redis is empty. Initializing from config.");
            Self::initialize_redis_from_config(&mut conn, &key_prefix, &keys_from_config, key_info_map).await?;
        } else {
            info!("Found {} keys in Redis set '{}'. Skipping initialization.", key_count, rotation_set_key);
        }

        Ok(Self {
            pool,
            key_prefix,
            key_info_map: Arc::new(key_info_map.clone()),
        })
    }

    fn prefix_key(&self, key: &str) -> String {
        format!("{}{}", self.key_prefix, key)
    }

    #[instrument(skip_all)]
    async fn initialize_redis_from_config(
        conn: &mut MultiplexedConnection,
        key_prefix: &str,
        keys_from_config: &[String],
        key_info_map: &HashMap<String, FlattenedKeyInfo>,
    ) -> Result<()> {
        info!("Initializing Redis with {} keys from config...", keys_from_config.len());
        let rotation_set_key = format!("{key_prefix}{ROTATION_SET_KEY}");
        let mut pipe = redis::pipe();
        pipe.atomic();
        pipe.sadd(&rotation_set_key, keys_from_config);

        for key in keys_from_config {
            let state_key = format!("{key_prefix}{}", key_state_key(key));
            let group_name = key_info_map.get(key).map_or("unknown", |info| &info.group_name);
            pipe.hset_multiple(&state_key, &[("is_blocked", "false"), ("consecutive_failures", "0"), ("group_name", group_name)]);
        }

        let _: () = pipe.query_async(conn).await?;
        info!("Successfully initialized Redis state.");
        Ok(())
    }

    async fn clear_redis_for_test_mode(
        conn: &mut MultiplexedConnection,
        key_prefix: &str,
        rotation_set_key: &str,
    ) -> Result<()> {
        info!("Test mode: Forcing re-initialization of Redis from config.");
        let all_keys: Vec<String> = conn.smembers(rotation_set_key).await?;
        if !all_keys.is_empty() {
            let state_keys: Vec<_> = all_keys.iter().map(|k| format!("{key_prefix}{}", key_state_key(k))).collect();
            let _: () = conn.del(state_keys).await?;
        }
        let _: () = conn.del(rotation_set_key).await?;
        
        // WARNING: Using KEYS in production is dangerous. This is acceptable ONLY for test isolation.
        // A better approach for tests is to use a unique key prefix per test run or a separate Redis DB.
        let counter_pattern = format!("{key_prefix}{ROTATION_COUNTER_KEY}:*");
        let counters: Vec<String> = conn.keys(&counter_pattern).await?;
        if !counters.is_empty() {
            let _: () = conn.del(counters).await?;
        }
        info!("Cleared stale KeyManager keys from Redis for test isolation.");
        Ok(())
    }
}

#[async_trait]
impl KeyStore for RedisStore {
    async fn get_candidate_keys(&self) -> Result<Vec<String>> {
        let mut conn = self.pool.get().await?;
        // PERF: SMEMBERS can be slow if the set is very large.
        // A more performant model for large key sets with many groups would be to have a
        // Redis Set *per group*, e.g., "rotation_set:group_a", "rotation_set:group_b".
        // This would avoid fetching all keys and filtering in the application.
        let keys: Vec<String> = conn.smembers(self.prefix_key(ROTATION_SET_KEY)).await?;
        Ok(keys)
    }

    async fn get_next_rotation_index(&self, group_id: &str) -> Result<usize> {
        let mut conn = self.pool.get().await?;
        let counter_key = self.prefix_key(&format!("{ROTATION_COUNTER_KEY}:{group_id}"));
        let index: usize = conn.incr(&counter_key, 1).await?;
        Ok(index)
    }

    async fn update_failure_state(&self, api_key: &str, is_terminal: bool, max_failures: u32) -> Result<KeyState> {
        let mut conn = self.pool.get().await?;
        let state_key = self.prefix_key(&key_state_key(api_key));
        
        let new_failure_count: u32 = conn.hincr(&state_key, "consecutive_failures", 1).await?;
        let now = chrono::Utc::now().to_rfc3339();
        let _: () = conn.hset(&state_key, "last_failure", &now).await?;

        let should_block = is_terminal || new_failure_count >= max_failures;
        if should_block {
            let _: () = conn.hset(&state_key, "is_blocked", true).await?;
        }

        Ok(KeyState {
            key: api_key.to_string(),
            group_name: self.key_info_map.get(api_key).map_or_else(|| "unknown".to_string(), |i| i.group_name.clone()),
            is_blocked: should_block,
            consecutive_failures: new_failure_count,
            last_failure: Some(chrono::Utc::now()),
        })
    }
    
    async fn get_key_state(&self, key: &str) -> Result<Option<KeyState>> {
        let mut conn = self.pool.get().await?;
        let state_key = self.prefix_key(&key_state_key(key));
        let redis_state: HashMap<String, String> = conn.hgetall(&state_key).await?;

        if redis_state.is_empty() {
            return Ok(None);
        }

        let state = KeyState {
            key: key.to_string(),
            group_name: redis_state.get("group_name").cloned().unwrap_or_else(|| "unknown".to_string()),
            is_blocked: redis_state.get("is_blocked").and_then(|s| s.parse().ok()).unwrap_or(false),
            consecutive_failures: redis_state.get("consecutive_failures").and_then(|s| s.parse().ok()).unwrap_or(0),
            last_failure: redis_state.get("last_failure").and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).map(|dt| dt.with_timezone(&chrono::Utc)),
        };
        Ok(Some(state))
    }

    async fn get_all_key_states(&self) -> Result<HashMap<String, KeyState>> {
        let keys = self.get_candidate_keys().await?;
        let mut states = HashMap::new();
        for key in keys {
            if let Ok(Some(state)) = self.get_key_state(&key).await {
                states.insert(key, state);
            }
        }
        Ok(states)
    }
}


// --- In-Memory Store Implementation ---
struct InMemoryStore {
    key_states: Arc<RwLock<HashMap<String, KeyState>>>,
    counters: Arc<RwLock<HashMap<String, AtomicUsize>>>,
}

impl InMemoryStore {
    fn new(key_info_map: &HashMap<String, FlattenedKeyInfo>) -> Self {
        let key_states = key_info_map.iter().map(|(key, info)| {
            let state = KeyState {
                key: key.clone(),
                group_name: info.group_name.clone(),
                is_blocked: false,
                consecutive_failures: 0,
                last_failure: None,
            };
            (key.clone(), state)
        }).collect();

        Self {
            key_states: Arc::new(RwLock::new(key_states)),
            counters: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl KeyStore for InMemoryStore {
    async fn get_candidate_keys(&self) -> Result<Vec<String>> {
        let states_guard = self.key_states.read().await;
        Ok(states_guard.keys().cloned().collect())
    }

    async fn get_next_rotation_index(&self, group_id: &str) -> Result<usize> {
        let mut counters_guard = self.counters.write().await;
        let counter = counters_guard
            .entry(group_id.to_string())
            .or_insert_with(|| AtomicUsize::new(0));
        Ok(counter.fetch_add(1, Ordering::SeqCst))
    }

    async fn update_failure_state(&self, api_key: &str, is_terminal: bool, max_failures: u32) -> Result<KeyState> {
        let mut states_guard = self.key_states.write().await;
        if let Some(state) = states_guard.get_mut(api_key) {
            state.consecutive_failures += 1;
            state.last_failure = Some(chrono::Utc::now());
            if is_terminal || state.consecutive_failures >= max_failures {
                state.is_blocked = true;
            }
            return Ok(state.clone());
        }
        Err(AppError::NotFound(format!("API Key '{api_key}' not found in memory store.")))
    }
    
    async fn get_key_state(&self, key: &str) -> Result<Option<KeyState>> {
        let states_guard = self.key_states.read().await;
        Ok(states_guard.get(key).cloned())
    }

    async fn get_all_key_states(&self) -> Result<HashMap<String, KeyState>> {
        let states_guard = self.key_states.read().await;
        Ok(states_guard.clone())
    }
}