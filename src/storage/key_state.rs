// src/storage/key_state.rs

use serde::{Deserialize, Serialize};

/// Represents the state of a single API key
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct KeyState {
    pub key: String,
    pub group_name: String,
    pub is_blocked: bool,
    pub consecutive_failures: u32,
    pub last_failure: Option<chrono::DateTime<chrono::Utc>>,
}

impl KeyState {
    /// Create a new key state with default values
    pub fn new(key: String, group_name: String) -> Self {
        Self {
            key,
            group_name,
            is_blocked: false,
            consecutive_failures: 0,
            last_failure: None,
        }
    }

    /// Check if the key should be blocked based on failure count
    pub fn should_block(&self, max_failures: u32, is_terminal: bool) -> bool {
        is_terminal || self.consecutive_failures >= max_failures
    }

    /// Record a failure and update state
    pub fn record_failure(&mut self, is_terminal: bool, max_failures: u32) {
        self.consecutive_failures += 1;
        self.last_failure = Some(chrono::Utc::now());

        if self.should_block(max_failures, is_terminal) {
            self.is_blocked = true;
        }
    }

    /// Reset the key state to available
    pub fn reset(&mut self) {
        self.is_blocked = false;
        self.consecutive_failures = 0;
        self.last_failure = None;
    }

    /// Check if the key is available for use
    pub fn is_available(&self) -> bool {
        !self.is_blocked
    }
}
