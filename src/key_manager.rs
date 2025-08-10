// src/key_manager.rs
// Refactored key manager with clear separation of concerns

use crate::config::AppConfig;
use crate::core::KeySelector;
use crate::error::Result;
use crate::storage::{InMemoryStore, KeyState, KeyStore, RedisStore};
use deadpool_redis::Pool;
use secrecy::{ExposeSecret, Secret};
use serde::{Deserialize, Deserializer, Serializer};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, trace, warn};

const DEFAULT_GROUP_ID: &str = "default";

/// Flattened key information for easier access
#[derive(Clone)]
pub struct FlattenedKeyInfo {
    pub key: Secret<String>,
    pub group_name: String,
    pub target_url: String,
    pub proxy_url: Option<String>,
}

impl std::fmt::Debug for FlattenedKeyInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlattenedKeyInfo")
            .field("key", &"[REDACTED]")
            .field("group_name", &self.group_name)
            .field("target_url", &self.target_url)
            .field("proxy_url", &self.proxy_url)
            .finish()
    }
}

// Serialization helpers for Secret<String>
pub fn serialize<S>(secret: &Secret<String>, serializer: S) -> std::result::Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(secret.expose_secret())
}

pub fn deserialize<'de, D>(deserializer: D) -> std::result::Result<Secret<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(Secret::new(s))
}

/// Trait for key management operations
#[async_trait::async_trait]
pub trait KeyManagerTrait: Send + Sync {
    async fn get_next_available_key_info(
        &self,
        group_name: Option<&str>,
    ) -> Result<Option<FlattenedKeyInfo>>;

    async fn handle_api_failure(&self, api_key: &str, is_terminal: bool) -> Result<()>;

    async fn handle_rate_limit(&self, api_key: &str, duration: Duration) -> Result<()>;

    async fn get_key_states(&self) -> Result<HashMap<String, KeyState>>;

    async fn get_all_key_info(&self) -> HashMap<String, FlattenedKeyInfo>;

    async fn reload(&mut self, config: &AppConfig, redis_pool: Option<Pool>) -> Result<()>;
}

/// Simplified key manager with clear separation of concerns
pub struct KeyManager {
    store: Arc<dyn KeyStore>,
    key_info_map: Arc<HashMap<String, FlattenedKeyInfo>>,
    selector: KeySelector,
    max_failures_threshold: u32,
}

impl KeyManager {
    pub async fn new(config: &AppConfig, redis_pool: Option<Pool>) -> Result<Self> {
        trace!("KeyManager::new started");

        let key_info_map = Self::build_key_info_map(config);

        let store: Arc<dyn KeyStore> = match redis_pool {
            Some(pool) => {
                info!("Redis pool provided. KeyManager will operate in Redis mode.");
                let redis_store = RedisStore::new(pool, config, &key_info_map).await?;
                Arc::new(redis_store)
            }
            None => {
                info!("No Redis pool provided. KeyManager will operate in in-memory mode.");
                let in_memory_store = InMemoryStore::new(&key_info_map);
                Arc::new(in_memory_store)
            }
        };

        let selector = KeySelector::with_round_robin();

        trace!("KeyManager::new finished");
        Ok(Self {
            store,
            key_info_map: Arc::new(key_info_map),
            selector,
            max_failures_threshold: config.max_failures_threshold.unwrap_or(3),
        })
    }

    fn build_key_info_map(config: &AppConfig) -> HashMap<String, FlattenedKeyInfo> {
        config
            .groups
            .iter()
            .flat_map(|group| {
                group.api_keys.iter().map(|api_key| {
                    let flattened_info = FlattenedKeyInfo {
                        key: Secret::new(api_key.clone()),
                        group_name: group.name.clone(),
                        target_url: group.target_url.clone(),
                        proxy_url: group.proxy_url.clone(),
                    };
                    (api_key.clone(), flattened_info)
                })
            })
            .collect()
    }

    pub fn preview_key(key: &Secret<String>) -> String {
        let key_str = key.expose_secret();
        Self::preview_key_str(key_str)
    }

    pub fn preview_key_str(key: &str) -> String {
        if key.len() > 8 {
            format!("{}...{}", &key[..4], &key[key.len() - 4..])
        } else {
            key.to_string()
        }
    }

    fn filter_keys_by_group<'a>(&'a self, group_name: Option<&str>) -> Vec<&'a FlattenedKeyInfo> {
        self.key_info_map
            .values()
            .filter(|info| group_name.map_or(true, |gn| info.group_name == gn))
            .collect()
    }
}

#[async_trait::async_trait]
impl KeyManagerTrait for KeyManager {
    async fn get_next_available_key_info(
        &self,
        group_name: Option<&str>,
    ) -> Result<Option<FlattenedKeyInfo>> {
        trace!("get_next_available_key_info: start");

        let all_keys = self.store.get_candidate_keys().await?;
        trace!(
            "get_next_available_key_info: got {} candidate keys",
            all_keys.len()
        );

        let mut candidate_keys = self.filter_keys_by_group(group_name);
        candidate_keys.retain(|info| all_keys.contains(info.key.expose_secret()));
        candidate_keys.sort_by(|a, b| a.key.expose_secret().cmp(b.key.expose_secret()));

        if candidate_keys.is_empty() {
            warn!(group_name, "No keys available for the specified group.");
            return Ok(None);
        }

        let group_id = group_name.unwrap_or(DEFAULT_GROUP_ID);

        match self
            .selector
            .select_available_key(candidate_keys.as_slice(), group_id, self.store.clone())
            .await?
        {
            Some(key_info) => Ok(Some(key_info)),
            None => {
                warn!(
                    group_name,
                    "All keys for the specified group are currently blocked."
                );
                Ok(None)
            }
        }
    }

    async fn handle_api_failure(&self, api_key: &str, is_terminal: bool) -> Result<()> {
        let updated_state = self
            .store
            .update_failure_state(api_key, is_terminal, self.max_failures_threshold)
            .await?;

        self.log_failure_handling(api_key, is_terminal, &updated_state);
        Ok(())
    }

    async fn handle_rate_limit(&self, api_key: &str, duration: Duration) -> Result<()> {
        self.store.set_key_rate_limited(api_key, duration).await
    }

    async fn get_key_states(&self) -> Result<HashMap<String, KeyState>> {
        self.store.get_all_key_states().await
    }

    async fn get_all_key_info(&self) -> HashMap<String, FlattenedKeyInfo> {
        self.key_info_map.as_ref().clone()
    }

    async fn reload(&mut self, config: &AppConfig, redis_pool: Option<Pool>) -> Result<()> {
        info!("Reloading KeyManager state from new configuration...");
        let new_key_info_map = Self::build_key_info_map(config);

        let new_store: Arc<dyn KeyStore> = match redis_pool {
            Some(pool) => Arc::new(RedisStore::new(pool, config, &new_key_info_map).await?),
            None => Arc::new(InMemoryStore::new(&new_key_info_map)),
        };

        self.store = new_store;
        self.key_info_map = Arc::new(new_key_info_map);
        self.max_failures_threshold = config.max_failures_threshold.unwrap_or(3);

        info!("KeyManager reloaded successfully.");
        Ok(())
    }
}

impl KeyManager {
    fn log_failure_handling(&self, api_key: &str, is_terminal: bool, state: &KeyState) {
        if state.is_blocked {
            warn!(
                event = "key_blocked",
                api_key.preview = %Self::preview_key_str(api_key),
                is_terminal,
                failures = state.consecutive_failures,
                max_failures = self.max_failures_threshold,
                block_reason = if is_terminal { "terminal_error" } else { "failure_threshold" },
                "API key has been blocked due to failures"
            );
        } else {
            info!(
                event = "key_failure_recorded",
                api_key.preview = %Self::preview_key_str(api_key),
                is_terminal,
                failures = state.consecutive_failures,
                max_failures = self.max_failures_threshold,
                "API key failure recorded, key still available"
            );
        }
    }
}
