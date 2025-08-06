// src/storage/memory.rs

use crate::error::{AppError, Result};
use crate::key_manager_v2::FlattenedKeyInfo;
use crate::storage::{KeyState, KeyStateStore, KeyStore};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::trace;

/// In-memory implementation of key storage
pub struct InMemoryStore {
    key_states: Arc<RwLock<HashMap<String, KeyState>>>,
    counters: Arc<RwLock<HashMap<String, AtomicUsize>>>,
}

impl InMemoryStore {
    pub fn new(key_info_map: &HashMap<String, FlattenedKeyInfo>) -> Self {
        let key_states = key_info_map
            .iter()
            .map(|(key, info)| {
                let state = KeyState::new(key.clone(), info.group_name.clone());
                (key.clone(), state)
            })
            .collect();

        Self {
            key_states: Arc::new(RwLock::new(key_states)),
            counters: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl KeyStore for InMemoryStore {
    async fn get_candidate_keys(&self) -> Result<Vec<String>> {
        trace!("InMemoryStore::get_candidate_keys: waiting for read lock");
        let states_guard = self.key_states.read().await;
        trace!("InMemoryStore::get_candidate_keys: got read lock");
        Ok(states_guard.keys().cloned().collect())
    }

    async fn get_next_rotation_index(&self, group_id: &str) -> Result<usize> {
        trace!("InMemoryStore::get_next_rotation_index: waiting for write lock");
        let mut counters_guard = self.counters.write().await;
        trace!("InMemoryStore::get_next_rotation_index: got write lock");
        let counter = counters_guard
            .entry(group_id.to_string())
            .or_insert_with(|| AtomicUsize::new(0));
        Ok(counter.fetch_add(1, Ordering::SeqCst))
    }

    async fn update_failure_state(
        &self,
        api_key: &str,
        is_terminal: bool,
        max_failures: u32,
    ) -> Result<KeyState> {
        trace!("InMemoryStore::update_failure_state: waiting for write lock");
        let mut states_guard = self.key_states.write().await;
        trace!("InMemoryStore::update_failure_state: got write lock");

        if let Some(state) = states_guard.get_mut(api_key) {
            state.record_failure(is_terminal, max_failures);
            return Ok(state.clone());
        }

        Err(AppError::Validation {
            field: "api_key".to_string(),
            message: format!("API Key '{api_key}' not found."),
        })
    }

    async fn get_key_state(&self, key: &str) -> Result<Option<KeyState>> {
        trace!("InMemoryStore::get_key_state: waiting for read lock");
        let states_guard = self.key_states.read().await;
        trace!("InMemoryStore::get_key_state: got read lock");
        Ok(states_guard.get(key).cloned())
    }

    async fn get_all_key_states(&self) -> Result<HashMap<String, KeyState>> {
        trace!("InMemoryStore::get_all_key_states: waiting for read lock");
        let states_guard = self.key_states.read().await;
        trace!("InMemoryStore::get_all_key_states: got read lock");
        Ok(states_guard.clone())
    }
}

#[async_trait]
impl KeyStateStore for InMemoryStore {
    async fn initialize_keys(&self, keys: &[String]) -> Result<()> {
        let mut states_guard = self.key_states.write().await;

        for key in keys {
            if !states_guard.contains_key(key) {
                let state = KeyState::new(key.clone(), "unknown".to_string());
                states_guard.insert(key.clone(), state);
            }
        }

        Ok(())
    }

    async fn reset_key_state(&self, key: &str) -> Result<()> {
        let mut states_guard = self.key_states.write().await;

        if let Some(state) = states_guard.get_mut(key) {
            state.reset();
            Ok(())
        } else {
            Err(AppError::Validation {
                field: "key".to_string(),
                message: format!("Key '{key}' not found."),
            })
        }
    }

    async fn get_keys_by_group(&self, group_name: &str) -> Result<Vec<String>> {
        let states_guard = self.key_states.read().await;
        let keys = states_guard
            .values()
            .filter(|state| state.group_name == group_name)
            .map(|state| state.key.clone())
            .collect();
        Ok(keys)
    }

    async fn is_key_available(&self, key: &str) -> Result<bool> {
        let states_guard = self.key_states.read().await;
        Ok(states_guard
            .get(key)
            .map(|state| state.is_available())
            .unwrap_or(false))
    }
}
