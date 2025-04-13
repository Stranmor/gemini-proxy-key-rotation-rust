// src/key_manager.rs

use crate::config::AppConfig; // Needed for KeyManager::new
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use chrono_tz::Europe::Moscow;
use chrono_tz::Tz;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

// --- Structures moved from state.rs ---

/// Represents the rate limit state of an individual API key.
#[derive(Debug, Clone, Default)]
pub struct KeyState {
    /// `true` if the key is currently considered rate-limited.
    is_limited: bool,
    /// The UTC time when the rate limit should expire. `None` if not limited.
    reset_time: Option<DateTime<Utc>>,
}

/// Contains all necessary information for a single API key instance used in the rotation.
/// This structure flattens information from the key's original `KeyGroup`.
#[derive(Debug, Clone)]
pub struct FlattenedKeyInfo {
    /// The actual API key string.
    pub key: String,
    /// The optional upstream proxy URL associated with this key's group.
    pub proxy_url: Option<String>,
    /// The target API endpoint URL associated with this key's group.
    pub target_url: String,
    /// The name of the `KeyGroup` this key originally belonged to.
    pub group_name: String,
}

// --- KeyManager Structure and Implementation ---

/// Manages the pool of API keys, tracks their rate limit states, and provides
/// round-robin rotation logic to select the next available key.
#[derive(Debug)]
pub struct KeyManager {
    /// A flattened list containing an entry for every API key from all groups.
    /// This list is used for the round-robin rotation.
    all_keys: Vec<FlattenedKeyInfo>,
    /// The index into `all_keys` pointing to the *next* key to be considered for selection.
    /// Uses atomic operations for safe concurrent access.
    key_index: AtomicUsize,
    /// A map tracking the current rate limit `KeyState` for each *unique* API key string.
    /// Uses an `RwLock` to allow concurrent reads and exclusive writes.
    key_states: RwLock<HashMap<String, KeyState>>,
}

impl KeyManager {
    /// Creates a new `KeyManager` instance from the application configuration.
    ///
    /// It flattens all keys from all configured groups into a single list (`all_keys`)
    /// for rotation and initializes the rate limit state (`key_states`) for each unique key.
    pub fn new(config: &AppConfig) -> Self {
        info!("Initializing KeyManager: Flattening keys and initializing states...");

        let mut all_keys = Vec::new();
        let mut initial_key_states = HashMap::new();
        let mut processed_keys_count = 0;

        // Iterate through groups from the config
        for group in &config.groups {
            if group.api_keys.is_empty() {
                warn!(group_name = %group.name, "Skipping group with no API keys.");
                continue;
            }
             info!(group_name = %group.name, key_count = group.api_keys.len(), proxy = group.proxy_url.as_deref().unwrap_or("None"), target = %group.target_url, "Processing group for KeyManager");
            for key in &group.api_keys {
                 if key.trim().is_empty() {
                    warn!(group_name = %group.name, "Skipping empty API key string in group.");
                    continue;
                }
                let key_info = FlattenedKeyInfo {
                    key: key.clone(),
                    proxy_url: group.proxy_url.clone(),
                    target_url: group.target_url.clone(),
                    group_name: group.name.clone(),
                };
                all_keys.push(key_info);
                // Ensure each unique key has an entry in key_states
                initial_key_states
                    .entry(key.clone())
                    .or_insert_with(KeyState::default);
                processed_keys_count += 1;
            }
        }

        // This assertion is important. If validation passes in main, this should never fail.
        assert!(
            !all_keys.is_empty(),
            "Configuration resulted in zero usable API keys. This should have been caught during validation."
        );

        info!(
            "KeyManager: Flattened {} total API keys from {} groups into rotation list.",
            processed_keys_count,
            config.groups.len()
        );
        info!(
            "KeyManager: Initialized state for {} unique API keys.",
            initial_key_states.len()
        );

        Self {
            all_keys,
            key_index: AtomicUsize::new(0),
            key_states: RwLock::new(initial_key_states),
        }
    }

    /// Retrieves the next available API key information using a round-robin strategy.
    ///
    /// This method iterates through the `all_keys` list, starting from the current `key_index`,
    /// checking the `key_states` map for each key. It skips keys that are marked as
    /// rate-limited and whose `reset_time` has not yet passed.
    ///
    /// If an available key is found, its `FlattenedKeyInfo` is cloned and returned,
    /// and the internal `key_index` is advanced for the next call.
    ///
    /// If all keys are currently rate-limited, it returns `None`.
    pub async fn get_next_available_key_info(&self) -> Option<FlattenedKeyInfo> {
        if self.all_keys.is_empty() {
             warn!("KeyManager: No API keys available in the flattened list.");
            return None;
        }

        let key_states_guard = self.key_states.read().await; // Read lock needed to check status
        let start_index = self.key_index.load(Ordering::Relaxed);
        let num_keys = self.all_keys.len();

        for i in 0..num_keys {
            let current_index = (start_index + i) % num_keys;
            let key_info = &self.all_keys[current_index];

            let key_state = key_states_guard
                .get(&key_info.key)
                .expect("Key state must exist for a key in all_keys list");

            let now = Utc::now();
            let is_available = if key_state.is_limited {
                key_state.reset_time.map_or_else(|| {
                   warn!(api_key_preview = Self::preview(&key_info.key), group = %key_info.group_name, "Key marked limited but has no reset time!");
                   false
               }, |reset_time| if now >= reset_time {
                       debug!(api_key_preview = Self::preview(&key_info.key), group = %key_info.group_name, reset_time = %reset_time, "Limit expired for key");
                        true
                     } else {
                         debug!(api_key_preview = Self::preview(&key_info.key), group = %key_info.group_name, reset_time = %reset_time, "Key still limited");
                         false
                   })
            } else {
                true
            };

            if is_available {
                self.key_index
                    .store((current_index + 1) % num_keys, Ordering::Relaxed);
                debug!(api_key_preview = Self::preview(&key_info.key), group = %key_info.group_name, index = current_index, "Selected available API key");
                return Some(key_info.clone());
            }
        }
        drop(key_states_guard);

        warn!("KeyManager: All API keys are currently rate-limited or unavailable.");
        None
    }

    /// Marks a specific API key as rate-limited.
    ///
    /// This updates the key's state in the `key_states` map, setting `is_limited` to `true`
    /// and calculating the `reset_time` to be 10:00 AM Moscow Time on the current or next day.
    ///
    /// # Arguments
    ///
    /// * `api_key` - The string slice representing the API key to mark as limited.
    pub async fn mark_key_as_limited(&self, api_key: &str) {
        let mut key_states_guard = self.key_states.write().await;

        if let Some(key_state) = key_states_guard.get_mut(api_key) {
            let key_preview = Self::preview(api_key);

            if key_state.is_limited {
                 if let Some(reset_time) = key_state.reset_time {
                    if Utc::now() >= reset_time {
                        info!(api_key_preview=%key_preview, "Resetting previously expired limit before marking again.");
                    }
                }
            }

            warn!(api_key_preview = %key_preview, "Marking key as rate-limited");

            let now_utc = Utc::now();
            let moscow_tz: Tz = Moscow;
            let now_moscow = now_utc.with_timezone(&moscow_tz);

            let mut reset_time_moscow = now_moscow
                .date_naive()
                .and_hms_opt(10, 0, 0)
                .expect("Valid time components")
                .and_local_timezone(moscow_tz)
                .unwrap();

            if now_moscow >= reset_time_moscow {
                reset_time_moscow += ChronoDuration::days(1);
                 debug!(api_key_preview=%key_preview, "Current Moscow time >= 10:00, setting reset for tomorrow 10:00 MSK");
            } else {
                debug!(api_key_preview=%key_preview, "Current Moscow time < 10:00, setting reset for today 10:00 MSK");
            }

            let reset_time_utc = reset_time_moscow.with_timezone(&Utc);

            key_state.is_limited = true;
            key_state.reset_time = Some(reset_time_utc);

            info!(api_key_preview=%key_preview, reset_utc = %reset_time_utc, reset_msk = %reset_time_moscow, "Key limit set");
        } else {
            error!(api_key=%api_key, "Attempted to mark an unknown API key as limited - key not found in states map!");
        }
    }

    /// Creates a short, safe preview of an API key string for logging purposes.
    /// Shows the first few characters followed by '...'.
    #[inline]
    fn preview(key: &str) -> String {
        let len = key.chars().count();
        let end = std::cmp::min(4, len); // Use len directly to handle short strings correctly
        format!("{}...", key.chars().take(end).collect::<String>())
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{KeyGroup, ServerConfig}; // Needed for creating AppConfig

    // Helper function to create a basic AppConfig for testing
    fn create_test_config(groups: Vec<KeyGroup>) -> AppConfig {
        AppConfig {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 8080,
            },
            groups,
        }
    }

    #[tokio::test] // Tests need async runtime because KeyManager methods are async
    async fn test_key_manager_initialization_simple() {
        let groups = vec![KeyGroup {
            name: "g1".to_string(),
            api_keys: vec!["key1".to_string(), "key2".to_string()],
            proxy_url: None,
            target_url: "target1".to_string(),
        }];
        let config = create_test_config(groups);
        let manager = KeyManager::new(&config);

        assert_eq!(manager.all_keys.len(), 2);
        assert_eq!(manager.all_keys[0].key, "key1");
        assert_eq!(manager.all_keys[1].key, "key2");
        assert_eq!(manager.all_keys[0].group_name, "g1");
        assert_eq!(manager.all_keys[1].group_name, "g1");
        assert_eq!(manager.key_index.load(Ordering::Relaxed), 0);

        let states = manager.key_states.read().await;
        assert_eq!(states.len(), 2);
        assert!(states.contains_key("key1"));
        assert!(states.contains_key("key2"));
        assert!(!states["key1"].is_limited);
        assert!(!states["key2"].is_limited);
    }

    #[tokio::test]
    async fn test_key_manager_initialization_multiple_groups() {
        let groups = vec![
            KeyGroup {
                name: "g1".to_string(),
                api_keys: vec!["key1".to_string()],
                proxy_url: Some("proxy1".to_string()),
                target_url: "target1".to_string(),
            },
            KeyGroup {
                name: "g2".to_string(),
                api_keys: vec!["key2".to_string(), "key3".to_string()],
                proxy_url: None,
                target_url: "target2".to_string(),
            },
             KeyGroup { // Group with duplicate key
                name: "g3".to_string(),
                api_keys: vec!["key1".to_string()], // Duplicate key1
                proxy_url: None,
                target_url: "target3".to_string(),
            },
        ];
        let config = create_test_config(groups);
        let manager = KeyManager::new(&config);

        // Flattened list should contain all keys, including duplicates
        assert_eq!(manager.all_keys.len(), 4);
        // Check order: keys from g1, then g2, then g3
        assert_eq!(manager.all_keys[0].key, "key1");
        assert_eq!(manager.all_keys[0].group_name, "g1");
        assert_eq!(manager.all_keys[0].proxy_url, Some("proxy1".to_string()));

        assert_eq!(manager.all_keys[1].key, "key2");
        assert_eq!(manager.all_keys[1].group_name, "g2");
        assert!(manager.all_keys[1].proxy_url.is_none());

        assert_eq!(manager.all_keys[2].key, "key3");
        assert_eq!(manager.all_keys[2].group_name, "g2");

        assert_eq!(manager.all_keys[3].key, "key1"); // Duplicate key1 from g3
        assert_eq!(manager.all_keys[3].group_name, "g3");
        assert!(manager.all_keys[3].proxy_url.is_none());


        let states = manager.key_states.read().await;
        // States map should only contain unique keys
        assert_eq!(states.len(), 3);
        assert!(states.contains_key("key1"));
        assert!(states.contains_key("key2"));
        assert!(states.contains_key("key3"));
    }

     #[tokio::test]
    async fn test_key_manager_initialization_empty_key_string() {
        let groups = vec![KeyGroup {
            name: "g1".to_string(),
            api_keys: vec!["key1".to_string(), "  ".to_string(), "key2".to_string()], // Empty string inside
            proxy_url: None,
            target_url: "target1".to_string(),
        }];
        let config = create_test_config(groups);
        let manager = KeyManager::new(&config);

        // Empty string should be skipped during flattening
        assert_eq!(manager.all_keys.len(), 2);
        assert_eq!(manager.all_keys[0].key, "key1");
        assert_eq!(manager.all_keys[1].key, "key2");

        let states = manager.key_states.read().await;
        assert_eq!(states.len(), 2); // State only for valid keys
    }

    #[tokio::test]
    async fn test_get_next_key_round_robin() {
        let groups = vec![KeyGroup {
            name: "g1".to_string(),
            api_keys: vec!["k1".to_string(), "k2".to_string(), "k3".to_string()],
            proxy_url: None,
            target_url: "t1".to_string(),
        }];
        let config = create_test_config(groups);
        let manager = KeyManager::new(&config);

        let key_info1 = manager.get_next_available_key_info().await.unwrap();
        assert_eq!(key_info1.key, "k1");
        assert_eq!(manager.key_index.load(Ordering::Relaxed), 1); // Index advanced

        let key_info2 = manager.get_next_available_key_info().await.unwrap();
        assert_eq!(key_info2.key, "k2");
        assert_eq!(manager.key_index.load(Ordering::Relaxed), 2);

        let key_info3 = manager.get_next_available_key_info().await.unwrap();
        assert_eq!(key_info3.key, "k3");
        assert_eq!(manager.key_index.load(Ordering::Relaxed), 0); // Wrapped around

        let key_info4 = manager.get_next_available_key_info().await.unwrap();
        assert_eq!(key_info4.key, "k1");
        assert_eq!(manager.key_index.load(Ordering::Relaxed), 1);
    }

     #[tokio::test]
    async fn test_mark_and_skip_limited_key() {
        let groups = vec![KeyGroup {
            name: "g1".to_string(),
            api_keys: vec!["k1".to_string(), "k2".to_string()],
            proxy_url: None,
            target_url: "t1".to_string(),
        }];
        let config = create_test_config(groups);
        let manager = KeyManager::new(&config);

        // Mark k1 as limited
        manager.mark_key_as_limited("k1").await;
        { // Check state
            let states = manager.key_states.read().await;
            assert!(states["k1"].is_limited);
            assert!(states["k1"].reset_time.is_some());
            assert!(!states["k2"].is_limited);
        }

        // First get_next should return k2 (skipping k1)
        let key_info1 = manager.get_next_available_key_info().await.unwrap();
        assert_eq!(key_info1.key, "k2");
        // Index should now point after k2 (which is 0, wrapping around)
        assert_eq!(manager.key_index.load(Ordering::Relaxed), 0);

         // Second get_next should also return k2 (as k1 is still limited)
        let key_info2 = manager.get_next_available_key_info().await.unwrap();
        assert_eq!(key_info2.key, "k2");
         // Index should advance again, pointing after k2 (0)
         assert_eq!(manager.key_index.load(Ordering::Relaxed), 0);

    }

     #[tokio::test]
    async fn test_get_next_key_all_limited() {
        let groups = vec![KeyGroup {
            name: "g1".to_string(),
            api_keys: vec!["k1".to_string(), "k2".to_string()],
            proxy_url: None,
            target_url: "t1".to_string(),
        }];
        let config = create_test_config(groups);
        let manager = KeyManager::new(&config);

        // Mark both keys as limited
        manager.mark_key_as_limited("k1").await;
        manager.mark_key_as_limited("k2").await;

        // get_next should return None
        let key_info = manager.get_next_available_key_info().await;
        assert!(key_info.is_none());
    }

    #[tokio::test]
    async fn test_limit_reset_logic() {
         let groups = vec![KeyGroup {
            name: "g1".to_string(),
            api_keys: vec!["k1".to_string()],
            proxy_url: None,
            target_url: "t1".to_string(),
        }];
        let config = create_test_config(groups);
        let manager = KeyManager::new(&config);

        // Mark k1 as limited
        manager.mark_key_as_limited("k1").await;

        // Manually modify the reset time to be in the past
        let past_time = Utc::now() - ChronoDuration::minutes(1);
         {
            let mut states = manager.key_states.write().await;
            states.get_mut("k1").unwrap().reset_time = Some(past_time);
         }

        // Now, get_next should consider the key available again
        let key_info = manager.get_next_available_key_info().await.unwrap();
        assert_eq!(key_info.key, "k1");

        // Check if the state was implicitly reset (optional, depends on desired behavior)
        // Current implementation doesn't automatically reset the flag, only checks time.
        // Let's verify the state remains limited but reset_time is in the past.
        {
             let states = manager.key_states.read().await;
             assert!(states["k1"].is_limited); // Flag isn't auto-reset
             assert!(states["k1"].reset_time.unwrap() < Utc::now());
        }

        // Calling mark_key_as_limited again should reset the timer to the future
        manager.mark_key_as_limited("k1").await;
        {
             let states = manager.key_states.read().await;
             assert!(states["k1"].is_limited);
             assert!(states["k1"].reset_time.unwrap() > Utc::now());
        }
    }

     #[tokio::test]
    async fn test_preview_helper() {
        assert_eq!(KeyManager::preview(""), "...");
        assert_eq!(KeyManager::preview("k"), "k...");
        assert_eq!(KeyManager::preview("ke"), "ke...");
        assert_eq!(KeyManager::preview("key"), "key...");
        assert_eq!(KeyManager::preview("key1"), "key1...");
        assert_eq!(KeyManager::preview("key12"), "key1..."); // Takes first 4
        assert_eq!(KeyManager::preview("key123456"), "key1...");
    }

}