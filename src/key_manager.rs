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

/// Represents the state of an individual API key regarding rate limits.
#[derive(Debug, Clone, Default)]
pub struct KeyState {
    /// Indicates if the key is currently rate-limited.
    is_limited: bool,
    /// The UTC time when the rate limit is expected to reset.
    reset_time: Option<DateTime<Utc>>,
}

/// Holds information about a single API key, flattened from its group config.
#[derive(Debug, Clone)]
pub struct FlattenedKeyInfo {
    /// The API key itself.
    pub key: String,
    /// The proxy URL associated with this key's group (if any).
    pub proxy_url: Option<String>,
    /// The target URL associated with this key's group.
    pub target_url: String,
    /// The name of the group this key belongs to (for logging).
    pub group_name: String,
}

// --- KeyManager Structure and Implementation ---

/// Manages the pool of API keys, their states, and rotation.
#[derive(Debug)]
pub struct KeyManager {
    /// Flattened list of all keys from all groups, with associated info.
    all_keys: Vec<FlattenedKeyInfo>,
    /// Atomic counter for round-robin index into `all_keys`.
    key_index: AtomicUsize,
    /// Tracks the rate limit status of each unique API key.
    /// Key: API Key string
    /// Value: `KeyState`
    key_states: RwLock<HashMap<String, KeyState>>,
}

impl KeyManager {
    /// Creates a new `KeyManager` by flattening keys from the configuration.
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

    /// Retrieves the next available `FlattenedKeyInfo` using a round-robin strategy
    /// across the flattened list of all keys, skipping limited keys.
    ///
    /// # Returns
    ///
    /// An `Option<FlattenedKeyInfo>` containing the info for the next available key,
    /// or `None` if all keys are currently limited.
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

    /// Marks an API key as rate-limited until the next reset time (10:00 AM Moscow Time).
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

    /// Helper to get a short preview of an API key for logging.
    #[inline]
    fn preview(key: &str) -> String {
        let len = key.chars().count();
        let end = std::cmp::min(4, len.saturating_sub(1));
        format!("{}...", key.chars().take(end).collect::<String>())
    }
}