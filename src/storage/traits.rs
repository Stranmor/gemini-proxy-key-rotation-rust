// src/storage/traits.rs

use crate::error::Result;
use crate::storage::KeyState;
use async_trait::async_trait;
use std::collections::HashMap;
use std::time::Duration;

/// Trait for key storage operations
#[async_trait]
pub trait KeyStore: Send + Sync {
    /// Get all candidate keys for rotation
    async fn get_candidate_keys(&self) -> Result<Vec<String>>;

    /// Get the next rotation index for a group
    async fn get_next_rotation_index(&self, group_id: &str) -> Result<usize>;

    /// Update failure state for a key
    async fn update_failure_state(
        &self,
        api_key: &str,
        is_terminal: bool,
        max_failures: u32,
    ) -> Result<KeyState>;

    /// Get state for a specific key
    async fn get_key_state(&self, key: &str) -> Result<Option<KeyState>>;

    /// Get all key states
    async fn get_all_key_states(&self) -> Result<HashMap<String, KeyState>>;

    /// Temporarily block a key due to rate limiting
    async fn set_key_rate_limited(&self, api_key: &str, duration: Duration) -> Result<()>;
}

/// Trait for key state management operations
#[async_trait]
pub trait KeyStateStore: Send + Sync {
    /// Initialize key states from configuration
    async fn initialize_keys(&self, keys: &[String]) -> Result<()>;

    /// Reset key state (unblock and reset failure count)
    async fn reset_key_state(&self, key: &str) -> Result<()>;

    /// Get keys by group
    async fn get_keys_by_group(&self, group_name: &str) -> Result<Vec<String>>;

    /// Check if key is available (not blocked)
    async fn is_key_available(&self, key: &str) -> Result<bool>;
}
