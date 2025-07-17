// src/key_manager.rs

use crate::config::{AppConfig, RateLimitBehavior};
use crate::error::{AppError, Result as AppResult}; // Use AppResult alias where appropriate
use chrono::{DateTime, Duration as ChronoDuration, NaiveDateTime, TimeZone, Utc}; // ENSURED TimeZone is imported
use chrono_tz::America::Los_Angeles; // Use Los_Angeles timezone (PST/PDT)
use chrono_tz::Tz; // Import Tz trait
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs as std_fs, // Use standard fs for rename
    io as std_io, // Import standard io for Error kind
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    }, // Added Arc for mutex cloning
};
use tokio::fs::{self as async_fs};
use tokio::sync::{Mutex, RwLock};
use tokio::task; // For spawning async save task
use tracing::Instrument; // Explicitly import the Instrument trait
use tracing::{debug, error, info, warn};
use uuid::Uuid; // For unique temporary file names

// --- Structures ---

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[derive(Default)]
pub enum KeyStatus {
    #[default]
    Available,
    RateLimited,
    Invalid,
    TemporarilyUnavailable,
}


#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct KeyState {
    pub status: KeyStatus,
    pub reset_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct FlattenedKeyInfo {
    pub key: String,
    pub proxy_url: Option<String>,
    pub target_url: String,
    pub group_name: String,
    pub top_p: Option<f32>,
    // Add original index within the group for state lookup if needed later
    // pub original_group_index: usize,
}
// --- KeyManager ---

#[derive(Debug)]
pub struct KeyManager {
    // Store keys grouped by their original group name.
    // The outer Vec represents groups, the tuple holds (group_name, keys_in_group).
    // Order of groups is preserved based on config processing order.
    grouped_keys: Vec<(String, Vec<FlattenedKeyInfo>)>,
    // Index of the group to try next.
    current_group_index: AtomicUsize,
    // Index of the key to try next *within each group*. The order matches `grouped_keys`.
    key_indices_per_group: Vec<AtomicUsize>,
    key_states: Arc<RwLock<HashMap<String, KeyState>>>,
    state_file_path: PathBuf,
    save_mutex: Arc<Mutex<()>>,
    rate_limit_behavior: crate::config::RateLimitBehavior, // Add the new field
}

impl KeyManager {
    #[tracing::instrument(level = "info", skip(config, config_path))]
    pub async fn new(config: &AppConfig, config_path: &Path) -> Self {
        info!("Initializing KeyManager...");
        let state_file_path = config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("key_states.json");
        let state_file_path_display = state_file_path.display().to_string(); // Capture for logs
        info!(key_state.path = %state_file_path_display, "Key state persistence file");

        let persisted_states = load_key_states_from_file(&state_file_path).await;
        let mut grouped_keys_map: HashMap<String, Vec<FlattenedKeyInfo>> = HashMap::new();
        let mut initial_key_states = HashMap::new();
        let mut processed_keys_count = 0;
        let now = Utc::now();

        // First pass: Collect keys into a map grouped by name to preserve group structure
        for group in &config.groups {
            if group.api_keys.is_empty() {
                warn!(group.name = %group.name, "Skipping group with no API keys.");
                continue;
            }
            info!(
               group.name = %group.name,
               group.key_count = group.api_keys.len(),
               group.proxy_url = group.proxy_url.as_deref().unwrap_or("None"),
               group.target_url = %group.target_url,
               "Processing group for KeyManager"
            );
            let group_entry = grouped_keys_map.entry(group.name.clone()).or_default();
            for key in &group.api_keys {
                if key.trim().is_empty() {
                    warn!(group.name = %group.name, "Skipping empty API key string in group.");
                    continue;
                }
                let key_info = FlattenedKeyInfo {
                    key: key.clone(),
                    proxy_url: group.proxy_url.clone(),
                    target_url: group.target_url.clone(),
                    group_name: group.name.clone(),
                    top_p: group.top_p,
                };
                group_entry.push(key_info); // Add key to its group in the map

                // Process state (this part remains largely the same)
                let state_to_insert = persisted_states.get(key).map_or_else(KeyState::default, |persisted| {
                    let is_expired = persisted.reset_time.is_some_and(|rt| now >= rt);
                    match persisted.status {
                        KeyStatus::RateLimited | KeyStatus::TemporarilyUnavailable if is_expired => {
                            info!(api_key.preview = %Self::preview(key), group.name = %group.name, "Persisted limit for key has expired. Initializing as available.");
                            KeyState::default()
                        }
                        KeyStatus::Invalid => {
                            info!(api_key.preview = %Self::preview(key), group.name = %group.name, "Loaded persisted invalid state for key.");
                            persisted.clone()
                        }
                        KeyStatus::RateLimited | KeyStatus::TemporarilyUnavailable => {
                            info!(api_key.preview = %Self::preview(key), group.name = %group.name, key.status = ?persisted.status, key.reset_time = ?persisted.reset_time, "Loaded persisted limited state for key.");
                            persisted.clone()
                        }
                        KeyStatus::Available => persisted.clone(),
                    }
                });
                initial_key_states
                    .entry(key.clone())
                    .or_insert(state_to_insert);
                processed_keys_count += 1;
            }
        }

        // Convert map to Vec to maintain a specific order for round-robin
        // Sort by group name to ensure consistent ordering across restarts, if desired.
        // Alternatively, could retain the order from config.groups if the map insertion order isn't guaranteed.
        // Let's use the order from config.groups for predictability.
        let mut grouped_keys: Vec<(String, Vec<FlattenedKeyInfo>)> = Vec::with_capacity(config.groups.len());
        let mut key_indices_per_group: Vec<AtomicUsize> = Vec::with_capacity(config.groups.len());

        // Iterate through config.groups again to maintain the original order
        let mut active_group_count = 0;
        for group_config in &config.groups {
             if let Some(keys) = grouped_keys_map.remove(&group_config.name) {
                 if !keys.is_empty() {
                     grouped_keys.push((group_config.name.clone(), keys));
                     key_indices_per_group.push(AtomicUsize::new(0));
                     active_group_count += 1;
                 }
             }
         }

        // Clean up states for keys no longer in config (needs adaptation)
        let all_keys_in_config: std::collections::HashSet<String> = grouped_keys
            .iter()
            .flat_map(|(_, keys)| keys.iter().map(|ki| ki.key.clone()))
            .collect();

        initial_key_states.retain(|key, _| {
            let key_in_config = all_keys_in_config.contains(key);
            if !key_in_config {
                warn!(api_key.preview = %Self::preview(key), "Removing state for key no longer present in configuration.");
            }
            key_in_config
        });


        if processed_keys_count == 0 { // Check processed keys, not grouped_keys.is_empty() which might be true if all groups had empty keys
            error!("KeyManager Initialization Error: No usable API keys found after processing configuration. Application might not function correctly.");
        } else if active_group_count == 0 {
             error!("KeyManager Initialization Error: Keys were processed, but no active groups were formed. Check group definitions.");
        }

        info!(
            key_manager.total_keys = processed_keys_count,
            key_manager.total_groups = active_group_count, // Log active groups
            "KeyManager: Grouped keys into rotation list."
        );
        info!(
            key_manager.state_count = initial_key_states.len(),
            key_manager.persisted_count = persisted_states.len(),
            "KeyManager: Initialized key states."
        );

        let manager = Self {
            grouped_keys,
            current_group_index: AtomicUsize::new(0),
            key_indices_per_group,
            key_states: Arc::new(RwLock::new(initial_key_states)),
            state_file_path: state_file_path.clone(), // Use cloned path
            save_mutex: Arc::new(Mutex::new(())),
            rate_limit_behavior: config.rate_limit_behavior.clone(), // Store the behavior
        };

        debug!(key_state.path = %state_file_path_display, "Performing initial state save/sync after KeyManager initialization.");
        if let Err(e) = manager.save_current_states().await {
            error!(key_state.path = %state_file_path_display, error = ?e, "Failed to perform initial save of key states. The state file might be outdated or missing.");
        } else {
            debug!(key_state.path = %state_file_path_display, "Initial state save completed successfully.");
        }
        manager
    }

    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_next_available_key_info(&self) -> Option<FlattenedKeyInfo> {
        if self.grouped_keys.is_empty() {
            warn!(
                key_manager.status = "empty",
                "No key groups configured or available. Cannot provide a key."
            );
            return None;
        }

        let num_groups = self.grouped_keys.len();
        // Relaxed ordering should be sufficient for these counters
        let initial_group_index = self.current_group_index.load(Ordering::Relaxed);

        let key_states_guard = self.key_states.read().await; // Lock state map for reading

        for group_offset in 0..num_groups {
            let current_group_idx = (initial_group_index + group_offset) % num_groups;
            let Some((group_name, keys_in_group)) = self.grouped_keys.get(current_group_idx) else {
                error!(group.index = current_group_idx, "Internal inconsistency: Group index out of bounds. Skipping.");
                continue; // Should not happen
            };

            if keys_in_group.is_empty() {
                debug!(group.index = current_group_idx, group.name = %group_name, "Skipping empty group");
                continue; // Skip empty groups
            }

            let num_keys_in_group = keys_in_group.len();
            let Some(group_key_index_atomic) = self.key_indices_per_group.get(current_group_idx) else {
                error!(group.index = current_group_idx, group.name=%group_name, "Internal inconsistency: Missing key index for group. Skipping.");
                continue; // Should not happen
            };
            let initial_key_index_in_group = group_key_index_atomic.load(Ordering::Relaxed);

             debug!(group.index = current_group_idx, group.name = %group_name, group.key_count = num_keys_in_group, group.start_key_index = initial_key_index_in_group, "Searching within group");

            for key_offset in 0..num_keys_in_group {
                let current_key_idx_in_group = (initial_key_index_in_group + key_offset) % num_keys_in_group;
                let Some(key_info) = keys_in_group.get(current_key_idx_in_group) else {
                    error!(group.index = current_group_idx, group.name=%group_name, key.index=current_key_idx_in_group, "Internal inconsistency: Key index out of bounds within group. Skipping key.");
                    continue; // Should not happen
                };

                let key_preview = Self::preview(&key_info.key);

                let Some(key_state) = key_states_guard.get(&key_info.key) else {
                    error!(api_key.preview = %key_preview, group.name = %group_name, "Internal inconsistency: Key found in group but missing from state map! Skipping.");
                    continue;
                };

                let now = Utc::now();
                let is_expired = key_state.reset_time.is_some_and(|rt| now >= rt);

                let is_available = match key_state.status {
                    KeyStatus::Available => true,
                    KeyStatus::RateLimited | KeyStatus::TemporarilyUnavailable if is_expired => true,
                    KeyStatus::Invalid => false, // Explicitly handle Invalid
                    _ => false,
                };

                if is_available {
                    if key_state.status != KeyStatus::Available {
                        debug!(api_key.preview = %key_preview, group.name = %group_name, "Limit previously set but now expired");
                    }

                    // --- Found an available key ---
                    // The NEXT key in THIS group should be tried on the next call for this group.
                    let next_key_idx_in_group = (current_key_idx_in_group + 1) % num_keys_in_group;
                    group_key_index_atomic.store(next_key_idx_in_group, Ordering::Relaxed);

                    // The NEXT group should be tried on the next global call.
                    let next_group_idx = (current_group_idx + 1) % num_groups;
                    self.current_group_index.store(next_group_idx, Ordering::Relaxed);

                    debug!(
                       api_key.preview = %key_preview,
                       group.name = %group_name,
                       group.index = current_group_idx,
                       key.index_in_group = current_key_idx_in_group,
                       group.next_key_index = next_key_idx_in_group,
                       manager.next_group_index = next_group_idx,
                       "Selected available API key using group round-robin"
                    );
                    // Release read lock before returning
                    drop(key_states_guard);
                    return Some(key_info.clone());
                }
                // Key is limited, log why it was skipped
                debug!(
                    api_key.preview = %key_preview,
                    group.name = %group_name,
                    key.index_in_group = current_key_idx_in_group,
                    reason = "limited",
                    key.reset_time = ?key_state.reset_time,
                    key.status = ?key_state.status,
                    "Skipped key in group"
                );
            } // End inner loop (keys in group)
             debug!(group.index = current_group_idx, group.name = %group_name, "No available key found in this group during this pass.");
        // If we exhausted this group and found no key, reset its index for the next full rotation.
        group_key_index_atomic.store(0, Ordering::Relaxed);
    } // End outer loop (groups)

        // If we exit the loop, no available key was found in any group
        drop(key_states_guard); // Release read lock
        warn!(
            key_manager.status = "all_limited",
            "All API keys checked across all groups are currently rate-limited or unavailable."
        );
        None
    }

    /// Marks a key as rate-limited.
    ///
    /// This function updates the in-memory state of a key to indicate it has hit a rate limit.
    /// The state is then persisted to a file asynchronously.
    ///
    /// # Panics
    ///
    /// This function will panic if it fails to calculate the next midnight time in the target timezone.
    /// This would indicate a critical logic error in the `chrono` or `chrono-tz` libraries.
    #[tracing::instrument(level = "warn", skip(self, api_key), fields(api_key.preview = %Self::preview(api_key)))]
    pub async fn mark_key_as_limited(&self, api_key: &str) {
        let key_preview = Self::preview(api_key); // Keep for simpler access within scope
        let mut should_save = false;
        let mut group_name_for_log = "unknown".to_string(); // Default if not found

        {
            // Scope for RwLockWriteGuard
            let mut key_states_guard = self.key_states.write().await;
            if let Some(key_state) = key_states_guard.get_mut(api_key) {
                // Find group name for context (best effort - search grouped_keys)
                 if let Some(found_key_info) = self.grouped_keys.iter()
                     .flat_map(|(_, keys)| keys.iter())
                     .find(|k| k.key == api_key) {
                    group_name_for_log.clone_from(&found_key_info.group_name);
                 }

                let now_utc = Utc::now();
                let mut state_changed = false;

                // Check if the limit had actually expired before we got the write lock
                let is_expired = key_state.reset_time.is_some_and(|rt| now_utc >= rt);
                let is_available = key_state.status == KeyStatus::Available || is_expired;

                if !is_available {
                     debug!(
                        group.name=%group_name_for_log,
                        key.status = ?key_state.status,
                        key.reset_time = ?key_state.reset_time,
                        "Key already marked as limited with a future reset time. Ignoring redundant mark."
                    );
                } else {
                    warn!(group.name=%group_name_for_log, behavior = ?self.rate_limit_behavior, "Marking key as rate-limited");

                    match self.rate_limit_behavior {
                        RateLimitBehavior::BlockUntilMidnight => {
                            let target_tz: Tz = Los_Angeles;
                            let now_in_target_tz = now_utc.with_timezone(&target_tz);
                            let tomorrow_naive_target =
                                (now_in_target_tz + ChronoDuration::days(1)).date_naive();
                            let reset_time_naive_target: NaiveDateTime = tomorrow_naive_target
                                .and_hms_opt(0, 0, 0)
                                .expect("Failed to calculate next midnight (00:00:00) in target timezone");

                            let (reset_time_utc, local_log_str): (DateTime<Utc>, String) = match target_tz
                                .from_local_datetime(&reset_time_naive_target)
                            {
                                chrono::LocalResult::Single(dt_target) => {
                                    (dt_target.with_timezone(&Utc), dt_target.to_string())
                                }
                                chrono::LocalResult::Ambiguous(dt1, dt2) => {
                                    warn!(
                                        group.name=%group_name_for_log,
                                        target.naive_time = %reset_time_naive_target,
                                        target.tz = ?target_tz,
                                        ambiguous_time1 = %dt1,
                                        ambiguous_time2 = %dt2,
                                        "Ambiguous local time calculated for reset, choosing earlier time."
                                    );
                                    (dt1.with_timezone(&Utc), dt1.to_string())
                                }
                                chrono::LocalResult::None => {
                                    error!(
                                        group.name=%group_name_for_log,
                                        target.naive_time = %reset_time_naive_target,
                                        target.tz = ?target_tz,
                                        "Calculated reset time does not exist in the target timezone! Falling back to UTC + 24h."
                                    );
                                    let fallback_utc = now_utc + ChronoDuration::hours(24);
                                    (fallback_utc, "N/A (non-existent local time)".to_string())
                                }
                            };

                            key_state.status = KeyStatus::RateLimited;
                            key_state.reset_time = Some(reset_time_utc);
                            state_changed = true;

                            info!(
                                group.name=%group_name_for_log,
                                key.reset_time.utc = %reset_time_utc,
                                key.reset_time.local = %local_log_str,
                                key.reset_time.tz = ?target_tz,
                                "Key limit set until next local midnight"
                            );
                        }
                        RateLimitBehavior::RetryNextKey => {
                             info!(group.name=%group_name_for_log, "Setting minimal reset time for immediate retry.");
                             key_state.status = KeyStatus::RateLimited;
                             key_state.reset_time = Some(Utc::now() + ChronoDuration::milliseconds(1));
                             state_changed = true;
                        }
                    }
                }

                // Trigger save only if state actually changed
                if state_changed {
                    should_save = true;
                }
            } else {
                // This is an error: trying to mark a key not managed by the manager
                error!(api_key.preview = %key_preview, "Attempted to mark an unknown API key as limited - key not found in states map!");
            }
        } // Write lock released here

        // Spawn save task only if needed
        if should_save {
            self.spawn_save_task();
        }
    }

    #[tracing::instrument(level = "warn", skip(self, api_key), fields(api_key.preview = %Self::preview(api_key)))]
    pub async fn mark_key_as_invalid(&self, api_key: &str) {
        let mut should_save = false;
        {
            let mut key_states_guard = self.key_states.write().await;
            if let Some(key_state) = key_states_guard.get_mut(api_key) {
                if key_state.status != KeyStatus::Invalid {
                    warn!("Marking key as permanently invalid");
                    key_state.status = KeyStatus::Invalid;
                    key_state.reset_time = None; // No reset for invalid keys
                    should_save = true;
                } else {
                    debug!("Key already marked as invalid. Ignoring redundant mark.");
                }
            } else {
                error!("Attempted to mark an unknown API key as invalid!");
            }
        }

        if should_save {
            self.spawn_save_task();
        }
    }

    #[tracing::instrument(level = "warn", skip(self, api_key), fields(api_key.preview = %Self::preview(api_key)))]
    pub async fn mark_key_as_temporarily_unavailable(
        &self,
        api_key: &str,
        duration: ChronoDuration,
    ) {
        let mut should_save = false;
        {
            let mut key_states_guard = self.key_states.write().await;
            if let Some(key_state) = key_states_guard.get_mut(api_key) {
                let reset_time = Utc::now() + duration;
                warn!(?duration, %reset_time, "Marking key as temporarily unavailable");
                key_state.status = KeyStatus::TemporarilyUnavailable;
                key_state.reset_time = Some(reset_time);
                should_save = true;
            } else {
                error!("Attempted to mark an unknown API key as temporarily unavailable!");
            }
        }

        if should_save {
            self.spawn_save_task();
        }
    }

    /// Spawns a Tokio task to save the current key states to the file.
    fn spawn_save_task(&self) {
        let state_file_path_clone = self.state_file_path.clone();
        let states_clone = Arc::clone(&self.key_states);
        let save_mutex_clone = Arc::clone(&self.save_mutex);
        let state_file_path_display = state_file_path_clone.display().to_string();

        task::spawn(async move {
            let save_span = tracing::info_span!("async_save_key_state", key_state.path = %state_file_path_display);
            async move {
                let _save_guard = save_mutex_clone.lock().await;
                debug!("Acquired save mutex lock");
                let states_guard = states_clone.read().await;
                let states_to_save = states_guard.clone();
                let state_count = states_to_save.len();
                drop(states_guard);

                if let Err(e) =
                    Self::save_states_to_file_impl(&state_file_path_clone, &states_to_save).await
                {
                    error!(state.count = state_count, error = ?e, "Async save task failed");
                } else {
                    debug!(state.count = state_count, "Async save task completed successfully");
                }
            }
            .instrument(save_span)
            .await;
        });
    }

    /// Asynchronously saves the current state (used for initial save). Requires external lock.
    #[tracing::instrument(level = "debug", skip(self))]
    async fn save_current_states(&self) -> AppResult<()> {
        let _save_guard = self.save_mutex.lock().await; // Acquire lock
        debug!(key_state.path = %self.state_file_path.display(), "Acquired save mutex lock for initial save");
        let states_guard = self.key_states.read().await;
        let states_to_save = states_guard.clone();
        let state_count = states_to_save.len();
        drop(states_guard); // Release read lock

        Self::save_states_to_file_impl(&self.state_file_path, &states_to_save).await
             .map_err(|io_err| {
                 // Structured error log
                 error!(key_state.path = %self.state_file_path.display(), state.count = state_count, error = ?io_err, "Initial save failed");
                 AppError::Io(io_err)
             })
    }

    /// Implementation detail: Performs the atomic save operation.
    #[tracing::instrument(level = "debug", skip(final_path, states), fields(key_state.path = %final_path.display(), state.count = states.len()))]
    async fn save_states_to_file_impl(
        final_path: &Path,
        states: &HashMap<String, KeyState>,
    ) -> std_io::Result<()> {
        debug!("Attempting atomic save");
        let parent_dir = final_path.parent().ok_or_else(|| {
            error!("State file path has no parent directory"); // Log error before returning
            std_io::Error::new(
                std_io::ErrorKind::InvalidInput,
                "State file path has no parent directory",
            )
        })?;
        // Create dir only if needed, log potential error
        if let Err(e) = async_fs::create_dir_all(parent_dir).await {
            error!(directory = %parent_dir.display(), error = ?e, "Failed to ensure parent directory exists for state file");
            return Err(e);
        }

        let base_filename = final_path.file_name().unwrap_or_default().to_string_lossy();
        let temp_filename = format!(".{}.{}.tmp", base_filename, Uuid::new_v4());
        let temp_path = Path::new("/tmp").join(&temp_filename);
        let temp_path_display = temp_path.display().to_string(); // Capture for logs

        // Serialize JSON
        let json_data = serde_json::to_string_pretty(states).map_err(|e| {
            error!(error = %e, "Failed to serialize key states to JSON");
            std_io::Error::new(
                std_io::ErrorKind::InvalidData,
                format!("Failed to serialize key states: {e}"),
            )
        })?;

        // Write to temp file
        debug!(temp_file.path = %temp_path_display, "Writing state to temporary file");
        if let Err(e) = async_fs::write(&temp_path, json_data.as_bytes()).await {
            error!(temp_file.path = %temp_path_display, error = ?e, "Failed to write to temporary state file");
            // Attempt cleanup on failure
            if let Err(rm_err) = std_fs::remove_file(&temp_path) {
                warn!(temp_file.path = %temp_path_display, error = ?rm_err, "Failed to remove temporary file after write failure");
            }
            return Err(e);
        }

        // Atomic rename
        // Copy and remove, which is more reliable across different filesystems (like /tmp to a mounted volume)
        debug!(temp_file.path = %temp_path_display, "Copying temporary file to final destination");
        if let Err(e) = async_fs::copy(&temp_path, final_path).await {
            error!(temp_file.path = %temp_path_display, final_file.path = %final_path.display(), error = ?e, "Failed to copy state file from temp to final destination");
            // Attempt cleanup on failure
            if let Err(rm_err) = async_fs::remove_file(&temp_path).await {
                warn!(temp_file.path = %temp_path_display, error = ?rm_err, "Failed to remove temporary file after copy failure");
            }
            return Err(e);
        }

        // Clean up the temporary file after successful copy
        if let Err(e) =  async_fs::remove_file(&temp_path).await {
            warn!(temp_file.path = %temp_path_display, error = ?e, "Failed to remove temporary file after successful copy.");
            // This is not a fatal error, so we don't return an error
        }
        info!("Successfully saved key states atomically"); // Info level for successful save
        Ok(())
    }

    /// Generates a shortened preview of an API key for logging.
    #[inline]
    fn preview(key: &str) -> String {
        let len = key.chars().count();
        let end = std::cmp::min(6, len); // Show first 6 chars max
                                         // Show last 4 if length > 10 (6 + 4), otherwise show only first part
        let start = if len > 10 { len - 4 } else { len };
        if len > 10 {
            format!(
                "{}...{}",
                key.chars().take(end).collect::<String>(),
                key.chars().skip(start).collect::<String>()
            )
        } else {
            format!("{}...", key.chars().take(end).collect::<String>())
        }
    }

    pub async fn get_key_states(&self) -> HashMap<String, KeyState> {
        self.key_states.read().await.clone()
    }

    /// Provides a flattened list of all key info for admin/debug purposes.
    pub fn get_all_key_info(&self) -> Vec<FlattenedKeyInfo> {
        self.grouped_keys
            .iter()
            .flat_map(|(_, keys)| keys.clone())
            .collect()
    }
}

/// Helper function to load key states from the JSON file, with recovery attempt from temp file.
#[tracing::instrument(level = "info", skip(path), fields(key_state.path = %path.display()))]
async fn load_key_states_from_file(path: &Path) -> HashMap<String, KeyState> {
    let base_filename = path.file_name().unwrap_or_default().to_string_lossy();
    let _parent_dir = path.parent().unwrap_or_else(|| Path::new(".")); // Keep for context, but mark as unused for now
    let path_display = path.display().to_string(); // Capture display string

    let mut recovered_from_temp = false;
    let mut recovered_states = HashMap::new();

    match async_fs::read_to_string(path).await {
        Ok(json_data) => {
            // Attempt to clean up any old temp files on successful load
            cleanup_temp_files(&base_filename).await;
            match serde_json::from_str::<HashMap<String, KeyState>>(&json_data) {
                Ok(states) => {
                    info!(state.count = states.len(), "Successfully loaded key states");
                    return states;
                }
                Err(e) => {
                    error!(error = %e, "Failed to parse key state file (JSON invalid). Attempting recovery.");
                }
            }
        }
        Err(ref e) if e.kind() == std_io::ErrorKind::NotFound => {
            // This is not an error, just information
            info!("Key state file not found. Checking for temporary recovery file.");
        }
        Err(e) => {
            // Log actual IO errors
            error!(error = %e, "Failed to read key state file due to IO error. Attempting recovery.");
        }
    }

    // Attempt recovery from temp file if main file failed or not found
    if let Some(temp_path) = find_latest_temp_file(&base_filename).await {
        let temp_path_display = temp_path.display().to_string(); // Capture display string
        warn!(temp_file.path = %temp_path_display, "Attempting recovery from temporary state file.");
        match async_fs::read_to_string(&temp_path).await {
            Ok(temp_json_data) => {
                match serde_json::from_str::<HashMap<String, KeyState>>(&temp_json_data) {
                    Ok(states) => {
                        info!(state.count = states.len(), temp_file.path = %temp_path_display, "Successfully recovered key states from temporary file");
                        // Attempt to rename recovered file to main path
                        if let Err(rename_err) = std_fs::rename(&temp_path, path) {
                            error!(
                                temp_file.path = %temp_path_display,
                                final_file.path = %path_display,
                                error = ?rename_err,
                                "Failed to copy recovered temp state file to main path. State recovered in memory, but file system may be inconsistent."
                            );
                       } else {
                           info!(final_file.path = %path_display, "Successfully copied recovered temp state file to main path.");
                            if let Err(rm_err) = async_fs::remove_file(&temp_path).await {
                               warn!(temp_file.path = %temp_path_display, error = ?rm_err, "Failed to remove temporary file after successful recovery.");
                           }
                           // Clean up potentially other old temp files after successful copy
                           cleanup_temp_files(&base_filename).await;
                       }
                        recovered_from_temp = true;
                        recovered_states = states; // Store recovered states
                    }
                    Err(parse_e) => {
                        error!(temp_file.path = %temp_path_display, error = %parse_e, "Failed to parse temporary key state file (JSON invalid). Recovery failed.");
                        // Attempt to remove corrupt temp file
                        if let Err(rm_err) = std_fs::remove_file(&temp_path) {
                            warn!(temp_file.path = %temp_path_display, error = ?rm_err, "Failed to remove corrupt temporary file after parse failure");
                        }
                        recovered_states = HashMap::new(); // Ensure empty map on parse failure
                    }
                }
            }
            Err(read_e) => {
                error!(temp_file.path = %temp_path_display, error = %read_e, "Failed to read temporary key state file. Recovery failed.");
                // Attempt to remove unreadable temp file
                if let Err(rm_err) = std_fs::remove_file(&temp_path) {
                    warn!(temp_file.path = %temp_path_display, error = ?rm_err, "Failed to remove unreadable temporary file");
                }
                recovered_states = HashMap::new(); // Ensure empty map on read failure
            }
        }
    } else {
        info!("No temporary state file found for recovery.");
    }

    // Return recovered states if successful, otherwise empty map
    if recovered_from_temp {
        recovered_states
    } else {
        warn!("Recovery failed or no file found. Starting with empty key states.");
        HashMap::new()
    }
}

/// Finds the most recently modified temporary state file matching the pattern.
#[tracing::instrument(level = "debug", skip(base_filename))]
async fn find_latest_temp_file(base_filename: &str) -> Option<PathBuf> {
    let mut latest_mod_time: Option<std::time::SystemTime> = None;
    let mut latest_temp_file: Option<PathBuf> = None;
    let temp_prefix = format!(".{base_filename}.");
    let temp_suffix = ".tmp";
    let temp_dir = Path::new("/tmp");
    debug!(temp_file.prefix = %temp_prefix, temp_file.suffix = %temp_suffix, directory = %temp_dir.display(), "Searching for latest temporary file");

    if let Ok(mut read_dir) = async_fs::read_dir(temp_dir).await {
        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let path = entry.path();
            if path.is_file() {
                if let Some(filename) = path.file_name().map(|n| n.to_string_lossy()) {
                    if filename.starts_with(&temp_prefix) && filename.ends_with(temp_suffix) {
                        debug!(temp_file.path = %path.display(), "Found potential temporary file");
                        if let Ok(metadata) = entry.metadata().await {
                            if let Ok(modified) = metadata.modified() {
                                if latest_mod_time.is_none() || modified > latest_mod_time.unwrap() {
                                    debug!(temp_file.path = %path.display(), ?modified, "Updating latest temporary file");
                                    latest_mod_time = Some(modified);
                                    latest_temp_file = Some(path.clone());
                                }
                            } else {
                                warn!(temp_file.path = %path.display(), "Could not get modified time for temp file");
                            }
                        } else {
                            warn!(temp_file.path = %path.display(), "Could not get metadata for temp file");
                        }
                    }
                }
            }
        }
    } else {
        warn!(directory = %temp_dir.display(), "Could not read directory to find temp files");
    }

    if let Some(ref p) = latest_temp_file {
        debug!(temp_file.path = %p.display(), "Found latest temporary file");
    } else {
        debug!("No suitable temporary file found");
    }
    latest_temp_file
}

/// Cleans up all temporary state files matching the pattern in a directory.
#[tracing::instrument(level = "debug", skip(base_filename))]
async fn cleanup_temp_files(base_filename: &str) {
    let temp_prefix = format!(".{base_filename}.");
    let temp_suffix = ".tmp";
    let temp_dir = Path::new("/tmp");

    debug!(temp_file.prefix = %temp_prefix, temp_file.suffix = %temp_suffix, directory = %temp_dir.display(), "Cleaning up temporary files");
    let mut cleaned_count = 0;

    if let Ok(mut read_dir) = async_fs::read_dir(temp_dir).await {
        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let path = entry.path();
            if path.is_file() {
                if let Some(filename) = path.file_name().map(|n| n.to_string_lossy()) {
                    if filename.starts_with(&temp_prefix) && filename.ends_with(temp_suffix) {
                        warn!(temp_file.path = %path.display(), "Cleaning up leftover temporary state file.");
                        if let Err(e) = async_fs::remove_file(&path).await {
                            error!(temp_file.path = %path.display(), error = ?e, "Failed during cleanup of temporary state file.");
                        } else {
                            cleaned_count += 1;
                            debug!(temp_file.path = %path.display(), "Successfully cleaned up temporary file.");
                        }
                    }
                }
            }
        }
    } else {
        warn!(directory = %temp_dir.display(), "Could not read directory to clean temp files");
    }
    debug!(cleaned_count, "Temporary file cleanup finished");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{KeyGroup, RateLimitBehavior, ServerConfig};
    use std::fs::{self as sync_fs, File};
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::time::sleep;

    fn create_test_config(groups: Vec<KeyGroup>) -> AppConfig {
        AppConfig {
            server: ServerConfig {
                port: 8080,
                cache_ttl_secs: 300,
                cache_max_size: 100,
                top_p: None,
            },
            groups,
            rate_limit_behavior: RateLimitBehavior::default(), // Add the new field with default
        }
    }

    fn create_temp_yaml_config(dir: &tempfile::TempDir) -> PathBuf {
        let file_path = dir.path().join("test_config.yaml");
        let content = r#"
 server:
   host: "0.0.0.0"
   port: 8080
 # No groups needed here, KeyManager uses AppConfig directly
 "#;
        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "{}", content).unwrap();
        file_path
    }

    // Helper to get the internal state for verification
    // async fn get_manager_indices(manager: &KeyManager) -> (usize, Vec<usize>) {
    //    let group_idx = manager.current_group_index.load(Ordering::Relaxed);
    //    let key_indices = manager.key_indices_per_group.iter().map(|a| a.load(Ordering::Relaxed)).collect();
    //    (group_idx, key_indices)
    // }

    #[tokio::test]
    async fn test_key_manager_initialization_loads_persisted_state() {
        let dir = tempdir().unwrap();
        let config_path = create_temp_yaml_config(&dir);
        let state_path = dir.path().join("key_states.json");

        let future_reset = Utc::now() + ChronoDuration::hours(1);
        let past_reset = Utc::now() - ChronoDuration::hours(1);
        let persisted_states: HashMap<String, KeyState> = [
            (
                "key_limited".to_string(),
                KeyState {
                    status: KeyStatus::RateLimited,
                    reset_time: Some(future_reset),
                },
            ),
            (
                "key_expired".to_string(),
                KeyState {
                    status: KeyStatus::RateLimited,
                    reset_time: Some(past_reset),
                },
            ),
            (
                "key_nolimit".to_string(),
                KeyState {
                    status: KeyStatus::Available,
                    reset_time: None,
                },
            ),
            (
                "key_not_in_config".to_string(),
                KeyState {
                    status: KeyStatus::Invalid,
                    reset_time: None,
                },
            ),
        ]
        .iter()
        .cloned()
        .collect();
        let json_data = serde_json::to_string(&persisted_states).unwrap();
        sync_fs::write(&state_path, json_data).unwrap();

        let groups = vec![KeyGroup {
            name: "g1".to_string(),
            api_keys: vec![
                "key_limited".to_string(),
                "key_expired".to_string(),
                "key_nolimit".to_string(),
                "key_new".to_string(),
            ],
            proxy_url: None,
            target_url: "t1".to_string(),
            top_p: None,
        }];
        let config = create_test_config(groups);
        let manager = KeyManager::new(&config, &config_path).await;
        let final_states = manager.key_states.read().await;

        assert_eq!(final_states.len(), 4);
        assert_eq!(final_states["key_limited"].status, KeyStatus::RateLimited);
        assert_eq!(final_states["key_limited"].reset_time, Some(future_reset));
        assert_eq!(final_states["key_expired"].status, KeyStatus::Available);
        assert!(final_states["key_expired"].reset_time.is_none());
        assert_eq!(final_states["key_nolimit"].status, KeyStatus::Available);
        assert!(final_states["key_nolimit"].reset_time.is_none());
        assert!(final_states.contains_key("key_new"));
        assert_eq!(final_states["key_new"].status, KeyStatus::Available);
        assert!(final_states["key_new"].reset_time.is_none());
        assert!(!final_states.contains_key("key_not_in_config"));
        assert_eq!(manager.state_file_path, state_path);
    }

    #[tokio::test]
    async fn test_mark_key_as_limited_saves_state_atomically() {
        let dir = tempdir().unwrap();
        let config_path = create_temp_yaml_config(&dir);
        let state_path = dir.path().join("key_states.json");
        File::create(&state_path)
            .unwrap()
            .write_all(b"initial_content")
            .unwrap();
        let groups = vec![KeyGroup {
            name: "g1".to_string(),
            api_keys: vec!["k1".to_string(), "k2".to_string()],
            proxy_url: None,
            target_url: "t1".to_string(),
            top_p: None,
        }];
        let config = create_test_config(groups);
        let manager = KeyManager::new(&config, &config_path).await;

        sleep(Duration::from_millis(50)).await;
        let initial_saved_json =
            sync_fs::read_to_string(&state_path).expect("State file should exist after init");
        let initial_saved_states: HashMap<String, KeyState> =
            serde_json::from_str(&initial_saved_json).expect("Should parse initial JSON");
        assert_eq!(initial_saved_states.len(), 2);

        manager.mark_key_as_limited("k1").await;
        sleep(Duration::from_millis(250)).await; // Wait for async save task

        let saved_json =
            sync_fs::read_to_string(&state_path).expect("State file should exist after save");
        let saved_states: HashMap<String, KeyState> =
            serde_json::from_str(&saved_json).expect("Should parse saved JSON");

        assert_eq!(saved_states.len(), 2);
        assert_eq!(saved_states["k1"].status, KeyStatus::RateLimited);
        assert!(saved_states["k1"].reset_time.is_some());
        sleep(Duration::from_millis(10)).await; // Ensure clock tick
        assert!(saved_states["k1"].reset_time.unwrap() < Utc::now());
        assert_eq!(saved_states["k2"].status, KeyStatus::Available);

        let base_filename = state_path.file_name().unwrap().to_string_lossy();
        let mut found_temp = false;
        for entry in sync_fs::read_dir(dir.path()).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                if let Some(filename) = path.file_name().map(|n| n.to_string_lossy()) {
                    if filename.starts_with(&format!(".{}.", base_filename))
                        && filename.ends_with(".tmp")
                    {
                        error!("Found unexpected temp file: {}", path.display());
                        found_temp = true;
                    }
                }
            }
        }
        assert!(
            !found_temp,
            "Temporary state file should not exist after successful save"
        );
    }

    #[tokio::test]
    async fn test_get_next_key_skips_persisted_limited_key() {
        let dir = tempdir().unwrap();
        let config_path = create_temp_yaml_config(&dir);
        let state_path = dir.path().join("key_states.json");
        let future_reset = Utc::now() + ChronoDuration::hours(1);
        let persisted: HashMap<String, KeyState> = [(
            "k1".to_string(),
            KeyState {
                status: KeyStatus::RateLimited,
                reset_time: Some(future_reset),
            },
        )]
        .iter()
        .cloned()
        .collect();
        sync_fs::write(&state_path, serde_json::to_string(&persisted).unwrap()).unwrap();

        let groups = vec![KeyGroup {
            name: "g1".to_string(),
            api_keys: vec!["k1".to_string(), "k2".to_string()],
            proxy_url: None,
            target_url: "t1".to_string(),
            top_p: None,
        }];
        let config = create_test_config(groups);
        let manager = KeyManager::new(&config, &config_path).await;

        let key_info1 = manager.get_next_available_key_info().await.unwrap();
        assert_eq!(key_info1.key, "k2"); // Should skip k1
        let key_info2 = manager.get_next_available_key_info().await.unwrap();
        assert_eq!(key_info2.key, "k2"); // Should loop back to k2 as k1 is still limited
    }

    #[tokio::test]
    async fn test_initial_save_syncs_state_after_loading() {
        let dir = tempdir().unwrap();
        let config_path = create_temp_yaml_config(&dir);
        let state_path = dir.path().join("key_states.json");

        // State file with an expired key and a key not in the new config
        let past_reset = Utc::now() - ChronoDuration::hours(1);
        let persisted: HashMap<String, KeyState> = [
            (
                "k1_expired".to_string(),
                KeyState {
                    status: KeyStatus::RateLimited,
                    reset_time: Some(past_reset),
                },
            ),
            (
                "k2_stale".to_string(),
                KeyState {
                    status: KeyStatus::Available,
                    reset_time: None,
                },
            ),
        ]
        .iter()
        .cloned()
        .collect();
        sync_fs::write(&state_path, serde_json::to_string(&persisted).unwrap()).unwrap();

        // New config only has k1_expired and a new key k3
        let groups = vec![KeyGroup {
            name: "g1".to_string(),
            api_keys: vec!["k1_expired".to_string(), "k3_new".to_string()],
            proxy_url: None,
            target_url: "t1".to_string(),
            top_p: None,
        }];
        let config = create_test_config(groups);

        // Init manager - this should trigger an initial save
        let _manager = KeyManager::new(&config, &config_path).await;
        sleep(Duration::from_millis(250)).await; // Wait for async save

        // Read the file back and check its contents
        let final_json = sync_fs::read_to_string(&state_path).unwrap();
        let final_states: HashMap<String, KeyState> = serde_json::from_str(&final_json).unwrap();

        // The final saved state should reflect the cleanup:
        // - k1_expired should be Available because its timer expired on load.
        // - k2_stale should be removed because it's not in the new config.
        // - k3_new should be added as Available.
        assert_eq!(final_states.len(), 2);
        assert!(final_states.contains_key("k1_expired"));
        assert!(final_states.contains_key("k3_new"));
        assert!(!final_states.contains_key("k2_stale"));
        assert_eq!(final_states["k1_expired"].status, KeyStatus::Available);
        assert!(final_states["k1_expired"].reset_time.is_none());
        assert_eq!(final_states["k3_new"].status, KeyStatus::Available);
    }

    #[tokio::test]
    async fn test_get_next_key_group_round_robin() {
        let dir = tempdir().unwrap();
        let config_path = create_temp_yaml_config(&dir);
        let groups = vec![
            KeyGroup {
                name: "g1".to_string(),
                api_keys: vec!["g1k1".to_string(), "g1k2".to_string()],
                proxy_url: None,
                target_url: "t1".to_string(),
                top_p: None,
            },
            KeyGroup {
                name: "g2".to_string(),
                api_keys: vec!["g2k1".to_string()],
                proxy_url: None,
                target_url: "t2".to_string(),
                top_p: None,
            },
            KeyGroup {
                name: "g3".to_string(),
                api_keys: vec!["g3k1".to_string(), "g3k2".to_string(), "g3k3".to_string()],
                proxy_url: None,
                target_url: "t3".to_string(),
                top_p: None,
            },
        ];
        let config = create_test_config(groups);
        let manager = KeyManager::new(&config, &config_path).await;

        // Expected sequence: g1k1, g2k1, g3k1, g1k2, g2k1 (loops), g3k2, g1k1 (loops), ...
        assert_eq!(manager.get_next_available_key_info().await.unwrap().key, "g1k1");
        assert_eq!(manager.get_next_available_key_info().await.unwrap().key, "g2k1");
        assert_eq!(manager.get_next_available_key_info().await.unwrap().key, "g3k1");
        assert_eq!(manager.get_next_available_key_info().await.unwrap().key, "g1k2");
        assert_eq!(manager.get_next_available_key_info().await.unwrap().key, "g2k1");
        assert_eq!(manager.get_next_available_key_info().await.unwrap().key, "g3k2");
        assert_eq!(manager.get_next_available_key_info().await.unwrap().key, "g1k1");
        assert_eq!(manager.get_next_available_key_info().await.unwrap().key, "g2k1");
        assert_eq!(manager.get_next_available_key_info().await.unwrap().key, "g3k3");
    }

     #[tokio::test]
     async fn test_get_next_key_skips_limited_keys_and_groups() {
         let dir = tempdir().unwrap();
         let config_path = create_temp_yaml_config(&dir);
         let groups = vec![
             KeyGroup { name: "g1".to_string(), api_keys: vec!["g1k1".to_string(), "g1k2".to_string()], proxy_url: None, target_url: "t1".to_string(), top_p: None },
             KeyGroup { name: "g2".to_string(), api_keys: vec!["g2k1".to_string()], proxy_url: None, target_url: "t2".to_string(), top_p: None },
             KeyGroup { name: "g3".to_string(), api_keys: vec!["g3k1".to_string()], proxy_url: None, target_url: "t3".to_string(), top_p: None },
         ];
         let mut config = create_test_config(groups);
         // Use BlockUntilMidnight to make test deterministic
         config.rate_limit_behavior = RateLimitBehavior::BlockUntilMidnight;
         let manager = KeyManager::new(&config, &config_path).await;

         // Limit g1k1 and all of g2
         manager.mark_key_as_limited("g1k1").await;
         manager.mark_key_as_limited("g2k1").await;
         sleep(Duration::from_millis(50)).await; // allow state to be saved

         // Expected sequence: g1k2 (starts at g1, skips g1k1), g3k1 (skips g2), g1k2 (wraps around)
         assert_eq!(manager.get_next_available_key_info().await.unwrap().key, "g1k2", "Should select g1k2 first");
         assert_eq!(manager.get_next_available_key_info().await.unwrap().key, "g3k1", "Should select g3k1 after skipping g2");
         assert_eq!(manager.get_next_available_key_info().await.unwrap().key, "g1k2", "Should wrap around and select g1k2 again");
     }

     #[tokio::test]
     async fn test_get_next_key_returns_none_when_all_limited() {
         let dir = tempdir().unwrap();
         let config_path = create_temp_yaml_config(&dir);
         let groups = vec![
             KeyGroup { name: "g1".to_string(), api_keys: vec!["g1k1".to_string()], proxy_url: None, target_url: "t1".to_string(), top_p: None },
             KeyGroup { name: "g2".to_string(), api_keys: vec!["g2k1".to_string()], proxy_url: None, target_url: "t2".to_string(), top_p: None },
         ];
         let mut config = create_test_config(groups);
         // Use BlockUntilMidnight to make test deterministic
         config.rate_limit_behavior = RateLimitBehavior::BlockUntilMidnight;
         let manager = KeyManager::new(&config, &config_path).await;

         manager.mark_key_as_limited("g1k1").await;
         manager.mark_key_as_limited("g2k1").await;
         sleep(Duration::from_millis(250)).await;

         assert!(manager.get_next_available_key_info().await.is_none(), "Should return None when all keys are limited");
     }

    #[tokio::test]
    async fn test_load_recovers_from_temp_file() {
        let dir = tempdir().unwrap();
        let final_path = dir.path().join("key_states.json");
        let base_filename = final_path.file_name().unwrap().to_string_lossy();
        let temp_filename = format!(".{}.{}.tmp", base_filename, Uuid::new_v4());
        let temp_path = dir.path().join(temp_filename);

        let expected_states: HashMap<String, KeyState> =
            [("recovered_key".to_string(), KeyState::default())]
                .iter()
                .cloned()
                .collect();
        let json_data = serde_json::to_string(&expected_states).unwrap();
        sync_fs::write(&temp_path, json_data).unwrap();

        // Ensure final file does not exist
        let _ = sync_fs::remove_file(&final_path);

        let loaded_states = load_key_states_from_file(&final_path).await;

        assert_eq!(loaded_states, expected_states);
        assert!(
            final_path.exists(),
            "Final file should be created from temp file"
        );
        assert!(
            !temp_path.exists(),
            "Temp file should be removed after successful recovery"
        );
    }

    #[tokio::test]
    async fn test_load_does_not_recover_from_corrupted_temp_file() {
        let dir = tempdir().unwrap();
        let final_path = dir.path().join("key_states.json");
        let base_filename = final_path.file_name().unwrap().to_string_lossy();
        let temp_filename = format!(".{}.{}.tmp", base_filename, Uuid::new_v4());
        let temp_path = dir.path().join(temp_filename);

        // Write corrupted JSON
        sync_fs::write(&temp_path, "{ not json }").unwrap();

        // Ensure final file does not exist
        let _ = sync_fs::remove_file(&final_path);

        let loaded_states = load_key_states_from_file(&final_path).await;

        assert!(
            loaded_states.is_empty(),
            "Should return empty map on recovery failure"
        );
        assert!(
            !final_path.exists(),
            "Final file should not be created on recovery failure"
        );
        assert!(
            !temp_path.exists(),
            "Corrupted temp file should be removed after failed recovery attempt"
        );
    }

    #[tokio::test]
    async fn test_mark_key_as_limited_block_until_midnight() {
        let dir = tempdir().unwrap();
        let config_path = create_temp_yaml_config(&dir);
        let groups = vec![KeyGroup { name: "g1".to_string(), api_keys: vec!["k1".to_string()], proxy_url: None, target_url: "t1".to_string(), top_p: None }];
        let mut config = create_test_config(groups);
        config.rate_limit_behavior = RateLimitBehavior::BlockUntilMidnight;
        let manager = KeyManager::new(&config, &config_path).await;

        manager.mark_key_as_limited("k1").await;

        let states = manager.key_states.read().await;
        let key_state = states.get("k1").unwrap();

        assert_eq!(key_state.status, KeyStatus::RateLimited);
        assert!(key_state.reset_time.is_some());
        let reset_time = key_state.reset_time.unwrap();
        assert!(reset_time > Utc::now());
        // Check that it's roughly 24h from now (could be less depending on time of day)
        assert!(reset_time < Utc::now() + ChronoDuration::hours(25));
    }

    #[tokio::test]
    async fn test_mark_key_as_limited_retry_next_key() {
        let dir = tempdir().unwrap();
        let config_path = create_temp_yaml_config(&dir);
        let groups = vec![KeyGroup { name: "g1".to_string(), api_keys: vec!["k1".to_string()], proxy_url: None, target_url: "t1".to_string(), top_p: None }];
        let mut config = create_test_config(groups);
        config.rate_limit_behavior = RateLimitBehavior::RetryNextKey;
        let manager = KeyManager::new(&config, &config_path).await;

        manager.mark_key_as_limited("k1").await;

        let states = manager.key_states.read().await;
        let key_state = states.get("k1").unwrap();

        assert_eq!(key_state.status, KeyStatus::RateLimited);
        assert!(key_state.reset_time.is_some());
        let reset_time = key_state.reset_time.unwrap();
        // Should be very close to now
        assert!(reset_time > Utc::now());
        assert!(reset_time < Utc::now() + ChronoDuration::seconds(1));
    }
}
