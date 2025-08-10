// src/storage/redis.rs

use crate::config::AppConfig;
use crate::error::Result;
use crate::key_manager::FlattenedKeyInfo;
use crate::storage::{KeyState, KeyStateStore, KeyStore};
use async_trait::async_trait;
use deadpool_redis::{Connection as RedisConnection, Pool};
use redis::AsyncCommands;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, trace, warn};

const ROTATION_SET_KEY: &str = "rotation_keys";
const ROTATION_COUNTER_KEY: &str = "rotation_counter";

/// Redis implementation of key storage
pub struct RedisStore {
    pool: Pool,
    key_prefix: String,
    key_info_map: Arc<HashMap<String, FlattenedKeyInfo>>,
}

impl RedisStore {
    pub async fn new(
        pool: Pool,
        config: &AppConfig,
        key_info_map: &HashMap<String, FlattenedKeyInfo>,
    ) -> Result<Self> {
        let key_prefix = config
            .redis_key_prefix
            .clone()
            .unwrap_or_else(|| "gemini_proxy:".to_string());

        let keys_from_config: Vec<String> = key_info_map.keys().cloned().collect();
        let mut conn = pool.get().await?;
        let rotation_set_key = format!("{key_prefix}{ROTATION_SET_KEY}");

        if config.server.test_mode {
            Self::clear_redis_for_test_mode(&mut conn, &key_prefix, &rotation_set_key).await?;
        }

        let key_count: usize = conn.scard(&rotation_set_key).await?;
        if key_count == 0 {
            info!("Redis is empty. Initializing from config.");
            Self::initialize_redis_from_config(
                &mut conn,
                &key_prefix,
                &keys_from_config,
                key_info_map,
            )
            .await?;
        } else {
            info!(
                "Found {} keys in Redis set '{}'. Skipping initialization.",
                key_count, rotation_set_key
            );
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

    async fn initialize_redis_from_config(
        conn: &mut RedisConnection,
        key_prefix: &str,
        keys_from_config: &[String],
        key_info_map: &HashMap<String, FlattenedKeyInfo>,
    ) -> Result<()> {
        info!(
            "Initializing Redis with {} keys from config...",
            keys_from_config.len()
        );
        let rotation_set_key = format!("{key_prefix}{ROTATION_SET_KEY}");
        let mut pipe = redis::pipe();
        pipe.atomic();
        pipe.sadd(&rotation_set_key, keys_from_config);

        for key in keys_from_config {
            let state_key = format!("{key_prefix}key_state:{key}");
            let group_name = key_info_map
                .get(key)
                .map_or("unknown", |info| &info.group_name);
            pipe.hset_multiple(
                &state_key,
                &[
                    ("is_blocked", "false"),
                    ("consecutive_failures", "0"),
                    ("group_name", group_name),
                ],
            );
        }

        let _: () = pipe.query_async(conn).await?;
        info!("Successfully initialized Redis state.");
        Ok(())
    }

    async fn clear_redis_for_test_mode(
        conn: &mut RedisConnection,
        key_prefix: &str,
        rotation_set_key: &str,
    ) -> Result<()> {
        info!("Test mode: Forcing re-initialization of Redis from config.");
        let all_keys: Vec<String> = conn.smembers(rotation_set_key).await?;
        if !all_keys.is_empty() {
            let state_keys: Vec<_> = all_keys
                .iter()
                .map(|k| format!("{key_prefix}key_state:{k}"))
                .collect();
            let _: () = conn.del(state_keys).await?;
        }
        let _: () = conn.del(rotation_set_key).await?;

        if key_prefix.contains("test") || key_prefix.contains("TEST") {
            let counter_pattern = format!("{key_prefix}{ROTATION_COUNTER_KEY}:*");
            let counters: Vec<String> = conn.keys(&counter_pattern).await?;
            if !counters.is_empty() {
                let _: () = conn.del(counters).await?;
            }
        } else {
            warn!("Skipping KEYS command cleanup in production environment for safety");
        }
        info!("Cleared stale KeyManager keys from Redis for test isolation.");
        Ok(())
    }

    async fn get_connection(&self) -> Result<RedisConnection> {
        self.pool.get().await.map_err(Into::into)
    }

    fn parse_key_state(&self, key: &str, redis_state: HashMap<String, String>) -> KeyState {
        KeyState {
            key: key.to_string(),
            group_name: redis_state
                .get("group_name")
                .cloned()
                .unwrap_or_else(|| "unknown".to_string()),
            is_blocked: redis_state
                .get("is_blocked")
                .and_then(|s| s.parse().ok())
                .unwrap_or(false),
            consecutive_failures: redis_state
                .get("consecutive_failures")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            last_failure: redis_state
                .get("last_failure")
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc)),
        }
    }
}

#[async_trait]
impl KeyStore for RedisStore {
    async fn get_candidate_keys(&self) -> Result<Vec<String>> {
        trace!("RedisStore::get_candidate_keys: start");
        let mut conn = self.get_connection().await?;
        trace!("RedisStore::get_candidate_keys: got connection");
        let keys: Vec<String> = conn.smembers(self.prefix_key(ROTATION_SET_KEY)).await?;
        trace!("RedisStore::get_candidate_keys: found {} keys", keys.len());
        Ok(keys)
    }

    async fn get_next_rotation_index(&self, group_id: &str) -> Result<usize> {
        trace!(
            "RedisStore::get_next_rotation_index: start for group '{}'",
            group_id
        );
        let mut conn = self.get_connection().await?;
        trace!("RedisStore::get_next_rotation_index: got connection");
        let counter_key = self.prefix_key(&format!("{ROTATION_COUNTER_KEY}:{group_id}"));
        let index: usize = conn.incr(&counter_key, 1).await?;
        trace!(
            "RedisStore::get_next_rotation_index: new index is {}",
            index
        );
        Ok(index)
    }

    async fn update_failure_state(
        &self,
        api_key: &str,
        is_terminal: bool,
        max_failures: u32,
    ) -> Result<KeyState> {
        trace!(
            "RedisStore::update_failure_state: start for key '{}'",
            api_key
        );
        let mut conn = self.get_connection().await?;
        trace!("RedisStore::update_failure_state: got connection");
        let state_key = self.prefix_key(&format!("key_state:{api_key}"));

        let new_failure_count: u32 = conn.hincr(&state_key, "consecutive_failures", 1).await?;
        let now = chrono::Utc::now().to_rfc3339();
        let _: () = conn.hset(&state_key, "last_failure", &now).await?;

        let should_block = is_terminal || new_failure_count >= max_failures;
        if should_block {
            let _: () = conn.hset(&state_key, "is_blocked", true).await?;
        }

        trace!(
            "RedisStore::update_failure_state: updated state for key '{}'",
            api_key
        );
        Ok(KeyState {
            key: api_key.to_string(),
            group_name: self
                .key_info_map
                .get(api_key)
                .map_or_else(|| "unknown".to_string(), |i| i.group_name.clone()),
            is_blocked: should_block,
            consecutive_failures: new_failure_count,
            last_failure: Some(chrono::Utc::now()),
        })
    }

    async fn get_key_state(&self, key: &str) -> Result<Option<KeyState>> {
        trace!("RedisStore::get_key_state: start for key '{}'", key);
        let mut conn = self.get_connection().await?;
        trace!("RedisStore::get_key_state: got connection");
        let state_key = self.prefix_key(&format!("key_state:{key}"));
        let redis_state: HashMap<String, String> = conn.hgetall(&state_key).await?;
        trace!("RedisStore::get_key_state: got hgetall result");

        if redis_state.is_empty() {
            return Ok(None);
        }

        let state = self.parse_key_state(key, redis_state);
        Ok(Some(state))
    }

    async fn get_all_key_states(&self) -> Result<HashMap<String, KeyState>> {
        let keys = self.get_candidate_keys().await?;
        let mut states: HashMap<String, KeyState> = HashMap::new();
        for key in &keys {
            if let Ok(Some(state)) = self.get_key_state(key).await {
                states.insert(key.to_string(), state);
            }
        }
        Ok(states)
    }

    async fn set_key_rate_limited(&self, api_key: &str, duration: Duration) -> Result<()> {
        let mut conn = self.get_connection().await?;
        let state_key = self.prefix_key(&format!("key_state:{api_key}"));
        
        let mut pipe = redis::pipe();
        pipe.atomic();
        pipe.hset(&state_key, "is_blocked", true);
        pipe.expire(&state_key, duration.as_secs() as i64);

        let _: () = pipe.query_async(&mut conn).await?;
        
        warn!(
            api_key.preview = %crate::key_manager::KeyManager::preview_key_str(api_key),
            duration = ?duration,
            "API key has been temporarily rate-limited."
        );
        Ok(())
    }
}

#[async_trait]
impl KeyStateStore for RedisStore {
    async fn initialize_keys(&self, keys: &[String]) -> Result<()> {
        let mut conn = self.get_connection().await?;
        let rotation_set_key = self.prefix_key(ROTATION_SET_KEY);

        let _: () = conn.sadd(&rotation_set_key, keys).await?;

        for key in keys {
            let state_key = self.prefix_key(&format!("key_state:{key}"));
            let _: () = conn
                .hset_multiple(
                    &state_key,
                    &[
                        ("is_blocked", "false"),
                        ("consecutive_failures", "0"),
                        ("group_name", "unknown"),
                    ],
                )
                .await?;
        }

        Ok(())
    }

    async fn reset_key_state(&self, key: &str) -> Result<()> {
        let mut conn = self.get_connection().await?;
        let state_key = self.prefix_key(&format!("key_state:{key}"));

        let _: () = conn
            .hset_multiple(
                &state_key,
                &[("is_blocked", "false"), ("consecutive_failures", "0")],
            )
            .await?;

        Ok(())
    }

    async fn get_keys_by_group(&self, group_name: &str) -> Result<Vec<String>> {
        let all_states = self.get_all_key_states().await?;
        let keys = all_states
            .values()
            .filter(|state| state.group_name == group_name)
            .map(|state| state.key.clone())
            .collect();
        Ok(keys)
    }

    async fn is_key_available(&self, key: &str) -> Result<bool> {
        match self.get_key_state(key).await? {
            Some(state) => Ok(state.is_available()),
            None => Ok(false),
        }
    }
}
