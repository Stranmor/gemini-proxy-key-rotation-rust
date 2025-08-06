// src/core/key_rotation.rs

use crate::error::Result;
use crate::key_manager_v2::FlattenedKeyInfo;
use crate::storage::KeyStore;
use async_trait::async_trait;
use secrecy::ExposeSecret;
use std::sync::Arc;
use tracing::{info, trace};

/// Strategy for selecting the next key
#[async_trait]
pub trait KeyRotationStrategy: Send + Sync {
    async fn select_key(
        &self,
        candidates: &[&FlattenedKeyInfo],
        group_id: &str,
        store: Arc<dyn KeyStore>,
    ) -> Result<Option<FlattenedKeyInfo>>;
}

/// Round-robin key selection strategy
pub struct RoundRobinStrategy;

#[async_trait]
impl KeyRotationStrategy for RoundRobinStrategy {
    async fn select_key(
        &self,
        candidates: &[&FlattenedKeyInfo],
        group_id: &str,
        store: Arc<dyn KeyStore>,
    ) -> Result<Option<FlattenedKeyInfo>> {
        if candidates.is_empty() {
            return Ok(None);
        }

        let start_index = store.get_next_rotation_index(group_id).await?;

        for i in 0..candidates.len() {
            let key_info = candidates[(start_index + i) % candidates.len()];

            match store.get_key_state(key_info.key.expose_secret()).await? {
                Some(state) if state.is_available() => {
                    self.log_key_selection(key_info, candidates.len());
                    return Ok(Some((*key_info).clone()));
                }
                None => {
                    // Key state not found, assume it's available
                    self.log_key_selection(key_info, candidates.len());
                    return Ok(Some((*key_info).clone()));
                }
                _ => continue,
            }
        }

        Ok(None)
    }
}

impl RoundRobinStrategy {
    fn log_key_selection(&self, key_info: &FlattenedKeyInfo, total_candidates: usize) {
        info!(
            event = "key_selected",
            api_key.preview = %crate::key_manager_v2::KeyManager::preview_key(&key_info.key),
            group = %key_info.group_name,
            rotation_method = "round_robin",
            total_candidates,
            "API key selected for request"
        );
    }
}

/// High-level key selector that coordinates key selection
pub struct KeySelector {
    strategy: Box<dyn KeyRotationStrategy>,
}

impl KeySelector {
    pub fn new(strategy: Box<dyn KeyRotationStrategy>) -> Self {
        Self { strategy }
    }

    pub fn with_round_robin() -> Self {
        Self::new(Box::new(RoundRobinStrategy))
    }

    pub async fn select_available_key(
        &self,
        candidates: &[&FlattenedKeyInfo],
        group_id: &str,
        store: Arc<dyn KeyStore>,
    ) -> Result<Option<FlattenedKeyInfo>> {
        trace!("Selecting key from {} candidates", candidates.len());
        self.strategy.select_key(candidates, group_id, store).await
    }
}
