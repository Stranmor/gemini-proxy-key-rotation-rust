// src/key_manager.rs

use crate::config::AppConfig;
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KeyState {
    is_limited: bool,
    reset_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct FlattenedKeyInfo {
    pub key: String,
    pub proxy_url: Option<String>,
    pub target_url: String,
    pub group_name: String,
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
                };
                group_entry.push(key_info); // Add key to its group in the map

                // Process state (this part remains largely the same)
                let state_to_insert = if let Some(persisted) = persisted_states.get(key) {
                    if persisted.is_limited && persisted.reset_time.map_or(false, |rt| now >= rt) {
                        info!(api_key.preview = %Self::preview(key), group.name = %group.name, "Persisted limit for key has expired. Initializing as available.");
                        KeyState::default()
                    } else {
                        if persisted.is_limited {
                            info!(api_key.preview = %Self::preview(key), group.name = %group.name, key.reset_time = ?persisted.reset_time, "Loaded persisted rate limit state for key.");
                        }
                        persisted.clone()
                    }
                } else {
                    KeyState::default()
                };
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
            let (group_name, keys_in_group) = match self.grouped_keys.get(current_group_idx) {
                 Some(group_data) => group_data,
                 None => {
                     error!(group.index = current_group_idx, "Internal inconsistency: Group index out of bounds. Skipping.");
                     continue; // Should not happen
                 }
             };

            if keys_in_group.is_empty() {
                debug!(group.index = current_group_idx, group.name = %group_name, "Skipping empty group");
                continue; // Skip empty groups
            }

            let num_keys_in_group = keys_in_group.len();
            let group_key_index_atomic = match self.key_indices_per_group.get(current_group_idx) {
                Some(atomic_idx) => atomic_idx,
                None => {
                    error!(group.index = current_group_idx, group.name=%group_name, "Internal inconsistency: Missing key index for group. Skipping.");
                    continue; // Should not happen
                }
            };
            let initial_key_index_in_group = group_key_index_atomic.load(Ordering::Relaxed);

             debug!(group.index = current_group_idx, group.name = %group_name, group.key_count = num_keys_in_group, group.start_key_index = initial_key_index_in_group, "Searching within group");

            for key_offset in 0..num_keys_in_group {
                let current_key_idx_in_group = (initial_key_index_in_group + key_offset) % num_keys_in_group;
                let key_info = match keys_in_group.get(current_key_idx_in_group) {
                     Some(ki) => ki,
                     None => {
                         error!(group.index = current_group_idx, group.name=%group_name, key.index=current_key_idx_in_group, "Internal inconsistency: Key index out of bounds within group. Skipping key.");
                         continue; // Should not happen
                     }
                 };

                let key_preview = Self::preview(&key_info.key);

                let key_state = match key_states_guard.get(&key_info.key) {
                    Some(state) => state,
                    None => {
                        error!(api_key.preview = %key_preview, group.name = %group_name, "Internal inconsistency: Key found in group but missing from state map! Skipping.");
                        continue;
                    }
                };

                let now = Utc::now();
                let is_available = !key_state.is_limited || key_state.reset_time.map_or(true, |rt| now >= rt);

                if is_available {
                    if key_state.is_limited {
                        debug!(api_key.preview = %key_preview, group.name = %group_name, "Limit previously set but now expired");
                        // State will be reset in mark_key_as_limited if it's called again for this key.
                        // If another key gets limited first, the reset happens during the next load/initialization.
                    }

                    // --- Found an available key ---
                    let next_key_idx_in_group = (current_key_idx_in_group + 1) % num_keys_in_group;
                    let next_group_idx = (current_group_idx + 1) % num_groups;

                    // Update indices atomically
                    group_key_index_atomic.store(next_key_idx_in_group, Ordering::Relaxed);
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
                } else {
                    // Key is limited, log why it was skipped
                    debug!(
                       api_key.preview = %key_preview,
                       group.name = %group_name,
                       key.index_in_group = current_key_idx_in_group,
                       reason = "limited",
                       key.reset_time = ?key_state.reset_time,
                       "Skipped key in group"
                    );
                }
            } // End inner loop (keys in group)
             debug!(group.index = current_group_idx, group.name = %group_name, "No available key found in this group during this pass.");
        } // End outer loop (groups)

        // If we exit the loop, no available key was found in any group
        drop(key_states_guard); // Release read lock
        warn!(
            key_manager.status = "all_limited",
            "All API keys checked across all groups are currently rate-limited or unavailable."
        );
        None
    }

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
                    group_name_for_log = found_key_info.group_name.clone();
                 }

                let now_utc = Utc::now();
                let mut state_changed = false;

                // Check if the limit had actually expired before we got the write lock
                let had_expired = key_state.is_limited && key_state.reset_time.map_or(false, |rt| now_utc >= rt);
                if had_expired {
                    info!(group.name=%group_name_for_log, "Resetting previously expired limit before marking again.");
                    // Explicitly reset the state if it had expired
                    key_state.is_limited = false;
                    key_state.reset_time = None;
                    state_changed = true; // Mark as changed so save is triggered if needed
                }

                use crate::config::RateLimitBehavior;

                // Mark as limited only if not already limited OR if it just expired and needs re-limiting
                if !key_state.is_limited || had_expired {
                    warn!(group.name=%group_name_for_log, behavior = ?self.rate_limit_behavior, "Marking key as rate-limited");

                    match self.rate_limit_behavior {
                        RateLimitBehavior::BlockUntilMidnight => {
                            let target_tz: Tz = Los_Angeles;
                            let now_in_target_tz = now_utc.with_timezone(&target_tz);
                            let tomorrow_naive_target =
                                (now_in_target_tz + ChronoDuration::days(1)).date_naive();
                            // Expect is okay here, failure indicates a chrono logic error
                            let reset_time_naive_target: NaiveDateTime = tomorrow_naive_target
                                .and_hms_opt(0, 0, 0)
                                .expect("Failed to calculate next midnight (00:00:00) in target timezone");

                            let (reset_time_utc, local_log_str): (DateTime<Utc>, String) = match target_tz
                                .from_local_datetime(&reset_time_naive_target)
                            {
                                // Use TimeZone trait method
                                chrono::LocalResult::Single(dt_target) => {
                                    (dt_target.with_timezone(&Utc), dt_target.to_string())
                                }
                                chrono::LocalResult::Ambiguous(dt1, dt2) => {
                                    // Log ambiguity and choice
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
                                    // Log failure and fallback
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

                            key_state.is_limited = true;
                            key_state.reset_time = Some(reset_time_utc);
                            state_changed = true; // Ensure state_changed is true after modification

                            // Log the final calculated reset times
                            info!(
                                group.name=%group_name_for_log,
                                key.reset_time.utc = %reset_time_utc,
                                key.reset_time.local = %local_log_str, // Local representation in target TZ
                                key.reset_time.tz = ?target_tz, // The target TZ used
                                "Key limit set until next local midnight"
                            );
                        }
                        RateLimitBehavior::RetryNextKey => {
                             info!(group.name=%group_name_for_log, "Setting minimal reset time for immediate retry.");
                             key_state.is_limited = true;
                             key_state.reset_time = Some(Utc::now() + ChronoDuration::milliseconds(1));
                             state_changed = true;
                        }
                    }
                } else {
                    // Already limited with a future reset time
                    debug!(
                        group.name=%group_name_for_log,
                        key.reset_time = ?key_state.reset_time,
                        "Key already marked as limited with a future reset time. Ignoring redundant mark."
                    );
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
            let state_file_path_clone = self.state_file_path.clone();
            let states_clone = Arc::clone(&self.key_states);
            let save_mutex_clone = Arc::clone(&self.save_mutex);
            let state_file_path_display = state_file_path_clone.display().to_string(); // Capture display string

            task::spawn(async move {
                // Use instrument to add context to the save task logs
                let save_span = tracing::info_span!("async_save_key_state", key_state.path = %state_file_path_display);
                async move {
                    let _save_guard = save_mutex_clone.lock().await;
                    debug!("Acquired save mutex lock");
                    let states_guard = states_clone.read().await;
                    let states_to_save = states_guard.clone();
                    let state_count = states_to_save.len();
                    drop(states_guard); // Release read lock before potentially long save

                    if let Err(e) =
                        Self::save_states_to_file_impl(&state_file_path_clone, &states_to_save)
                            .await
                    {
                        // Structured error log within the span
                        error!(state.count = state_count, error = ?e, "Async save task failed");
                    } else {
                        debug!(
                            state.count = state_count,
                            "Async save task completed successfully"
                        );
                    }
                }
                .instrument(save_span)
                .await // Apply instrumentation here
            });
        }
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
        let temp_path = parent_dir.join(&temp_filename); // Borrow temp_filename
        let temp_path_display = temp_path.display().to_string(); // Capture for logs

        // Serialize JSON
        let json_data = serde_json::to_string_pretty(states).map_err(|e| {
            error!(error = %e, "Failed to serialize key states to JSON");
            std_io::Error::new(
                std_io::ErrorKind::InvalidData,
                format!("Failed to serialize key states: {}", e),
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
        debug!(temp_file.path = %temp_path_display, "Attempting atomic rename");
        if let Err(e) = std_fs::rename(&temp_path, final_path) {
            error!(temp_file.path = %temp_path_display, final_file.path = %final_path.display(), error = ?e, "Failed atomic rename of state file");
            // Attempt cleanup on failure
            if let Err(rm_err) = std_fs::remove_file(&temp_path) {
                warn!(temp_file.path = %temp_path_display, error = ?rm_err, "Failed to remove temporary file after rename failure");
            }
            return Err(e);
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
}

/// Helper function to load key states from the JSON file, with recovery attempt from temp file.
#[tracing::instrument(level = "info", skip(path), fields(key_state.path = %path.display()))]
async fn load_key_states_from_file(path: &Path) -> HashMap<String, KeyState> {
    let base_filename = path.file_name().unwrap_or_default().to_string_lossy();
    let parent_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let path_display = path.display().to_string(); // Capture display string

    let mut recovered_from_temp = false;
    let mut recovered_states = HashMap::new();

    match async_fs::read_to_string(path).await {
        Ok(json_data) => {
            // Attempt to clean up any old temp files on successful load
            cleanup_temp_files(parent_dir, &base_filename).await;
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
    if let Some(temp_path) = find_latest_temp_file(parent_dir, &base_filename).await {
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
                                "Failed to rename recovered temp state file to main path. State recovered in memory, but file system may be inconsistent."
                            );
                        } else {
                            info!(final_file.path = %path_display, "Successfully renamed recovered temp state file to main path.");
                            // Clean up potentially other old temp files after successful rename
                            cleanup_temp_files(parent_dir, &base_filename).await;
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
#[tracing::instrument(level = "debug", skip(dir, base_filename))]
async fn find_latest_temp_file(dir: &Path, base_filename: &str) -> Option<PathBuf> {
    let mut latest_mod_time: Option<std::time::SystemTime> = None;
    let mut latest_temp_file: Option<PathBuf> = None;
    let temp_prefix = format!(".{}.", base_filename);
    let temp_suffix = ".tmp";
    debug!(temp_file.prefix = %temp_prefix, temp_file.suffix = %temp_suffix, directory = %dir.display(), "Searching for latest temporary file");

    if let Ok(mut read_dir) = async_fs::read_dir(dir).await {
        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let path = entry.path();
            if path.is_file() {
                if let Some(filename) = path.file_name().map(|n| n.to_string_lossy()) {
                    if filename.starts_with(&temp_prefix) && filename.ends_with(temp_suffix) {
                        debug!(temp_file.path = %path.display(), "Found potential temporary file");
                        if let Ok(metadata) = entry.metadata().await {
                            if let Ok(modified) = metadata.modified() {
                                if latest_mod_time.map_or(true, |latest| modified > latest) {
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
        warn!(directory = %dir.display(), "Could not read directory to find temp files");
    }

    if let Some(ref p) = latest_temp_file {
        debug!(temp_file.path = %p.display(), "Found latest temporary file");
    } else {
        debug!("No suitable temporary file found");
    }
    latest_temp_file
}

/// Cleans up all temporary state files matching the pattern in a directory.
#[tracing::instrument(level = "debug", skip(dir, base_filename))]
async fn cleanup_temp_files(dir: &Path, base_filename: &str) {
    let temp_prefix = format!(".{}.", base_filename);
    let temp_suffix = ".tmp";
    debug!(temp_file.prefix = %temp_prefix, temp_file.suffix = %temp_suffix, directory = %dir.display(), "Cleaning up temporary files");
    let mut cleaned_count = 0;

    if let Ok(mut read_dir) = async_fs::read_dir(dir).await {
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
        warn!(directory = %dir.display(), "Could not read directory to clean temp files");
    }
    debug!(cleaned_count, "Temporary file cleanup finished");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{KeyGroup, ServerConfig};
    use std::fs::{self as sync_fs, File};
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::time::sleep;

    fn create_test_config(groups: Vec<KeyGroup>) -> AppConfig {
        AppConfig {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 8080,
            },
            groups,
            rate_limit_behavior: crate::config::RateLimitBehavior::default(), // Add the new field with default
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
    async fn get_manager_indices(manager: &KeyManager) -> (usize, Vec<usize>) {
       let group_idx = manager.current_group_index.load(Ordering::Relaxed);
       let key_indices = manager.key_indices_per_group.iter().map(|a| a.load(Ordering::Relaxed)).collect();
       (group_idx, key_indices)
   }

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
                    is_limited: true,
                    reset_time: Some(future_reset),
                },
            ),
            (
                "key_expired".to_string(),
                KeyState {
                    is_limited: true,
                    reset_time: Some(past_reset),
                },
            ),
            (
                "key_nolimit".to_string(),
                KeyState {
                    is_limited: false,
                    reset_time: None,
                },
            ),
            (
                "key_not_in_config".to_string(),
                KeyState {
                    is_limited: true,
                    reset_time: Some(future_reset),
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
        }];
        let config = create_test_config(groups);
        let manager = KeyManager::new(&config, &config_path).await;
        let final_states = manager.key_states.read().await;

        assert_eq!(final_states.len(), 4);
        assert!(final_states["key_limited"].is_limited);
        assert_eq!(final_states["key_limited"].reset_time, Some(future_reset));
        assert!(!final_states["key_expired"].is_limited);
        assert!(final_states["key_expired"].reset_time.is_none());
        assert!(!final_states["key_nolimit"].is_limited);
        assert!(final_states["key_nolimit"].reset_time.is_none());
        assert!(final_states.contains_key("key_new"));
        assert!(!final_states["key_new"].is_limited);
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
        assert!(saved_states["k1"].is_limited);
        assert!(saved_states["k1"].reset_time.is_some());
        assert!(saved_states["k1"].reset_time.unwrap() > Utc::now());
        assert!(!saved_states["k2"].is_limited);

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
                is_limited: true,
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
        let past_reset = Utc::now() - ChronoDuration::hours(1);
        let persisted: HashMap<String, KeyState> = [
            (
                "k1_expired".to_string(),
                KeyState {
                    is_limited: true,
                    reset_time: Some(past_reset),
                },
            ),
            (
                "k2_removed".to_string(),
                KeyState {
                    is_limited: false,
                    reset_time: None,
                },
            ),
        ]
        .iter()
        .cloned()
        .collect();
        sync_fs::write(&state_path, serde_json::to_string(&persisted).unwrap()).unwrap();
        let groups = vec![KeyGroup {
            name: "g1".to_string(),
            api_keys: vec!["k1_expired".to_string(), "k3_new".to_string()],
            proxy_url: None,
            target_url: "t1".to_string(),
        }];
        let config = create_test_config(groups);
        let _manager = KeyManager::new(&config, &config_path).await; // Manager creation triggers initial save

        sleep(Duration::from_millis(50)).await; // Allow time for async save

        let saved_json = sync_fs::read_to_string(&state_path).expect("State file should exist");
        let saved_states: HashMap<String, KeyState> =
            serde_json::from_str(&saved_json).expect("Should parse saved JSON");
        assert_eq!(saved_states.len(), 2);
        assert!(
            !saved_states["k1_expired"].is_limited,
            "Expired key should be reset"
        );
        assert!(saved_states["k1_expired"].reset_time.is_none());
        assert!(
            !saved_states["k3_new"].is_limited,
            "New key should be available"
        );
        assert!(saved_states["k3_new"].reset_time.is_none());
        assert!(
            !saved_states.contains_key("k2_removed"),
            "Removed key should not be present"
        );
    }

    #[tokio::test]
    async fn test_get_next_key_group_round_robin() {
        let dir = tempdir().unwrap();
        let config_path = create_temp_yaml_config(&dir);
        let groups = vec![
             KeyGroup { // Group 0
                 name: "g1".to_string(),
                 api_keys: vec!["g1k1".to_string(), "g1k2".to_string()],
                 proxy_url: None, target_url: "t1".to_string(),
             },
             KeyGroup { // Group 1
                 name: "g2".to_string(),
                 api_keys: vec!["g2k1".to_string()],
                 proxy_url: None, target_url: "t2".to_string(),
             },
             KeyGroup { // Group 2
                 name: "g3".to_string(),
                 api_keys: vec!["g3k1".to_string(), "g3k2".to_string(), "g3k3".to_string()],
                 proxy_url: None, target_url: "t3".to_string(),
            },
         ];
        let config = create_test_config(groups);
        let manager = KeyManager::new(&config, &config_path).await;

        assert_eq!(manager.grouped_keys.len(), 3);
        assert_eq!(manager.key_indices_per_group.len(), 3);

        // Expected sequence: g1k1, g2k1, g3k1, g1k2, g2k1, g3k2, g1k1, g2k1, g3k3, ...
        let key1 = manager.get_next_available_key_info().await.unwrap(); // g1k1
        assert_eq!(key1.key, "g1k1");
        assert_eq!(get_manager_indices(&manager).await, (1, vec![1, 0, 0])); // Next group 1, g1 index 1

        let key2 = manager.get_next_available_key_info().await.unwrap(); // g2k1
        assert_eq!(key2.key, "g2k1");
        // Group g2 has only one key, so its index should wrap back to 0 after selection.
        assert_eq!(get_manager_indices(&manager).await, (2, vec![1, 0, 0])); // Next group 2, g2 index 0

        let key3 = manager.get_next_available_key_info().await.unwrap(); // g3k1
        assert_eq!(key3.key, "g3k1");
        // Corrected assertion: Group g2 index should be 0.
        assert_eq!(get_manager_indices(&manager).await, (0, vec![1, 0, 1])); // Next group 0, g3 index 1

        let key4 = manager.get_next_available_key_info().await.unwrap(); // g1k2
        assert_eq!(key4.key, "g1k2");
        // Corrected assertion: Group g2 index should remain 0 as we selected from g1.
        assert_eq!(get_manager_indices(&manager).await, (1, vec![0, 0, 1])); // Next group 1, g1 index 0

        let key5 = manager.get_next_available_key_info().await.unwrap(); // g2k1 (again)
        assert_eq!(key5.key, "g2k1");
        // Group g2 index wraps to 0 again.
        assert_eq!(get_manager_indices(&manager).await, (2, vec![0, 0, 1])); // Next group 2, g2 index 0

        let key6 = manager.get_next_available_key_info().await.unwrap(); // g3k2
        assert_eq!(key6.key, "g3k2");
        assert_eq!(get_manager_indices(&manager).await, (0, vec![0, 0, 2])); // Next group 0, g3 index 2

        let key7 = manager.get_next_available_key_info().await.unwrap(); // g1k1 (again)
        assert_eq!(key7.key, "g1k1");
        assert_eq!(get_manager_indices(&manager).await, (1, vec![1, 0, 2])); // Next group 1, g1 index 1

        let key8 = manager.get_next_available_key_info().await.unwrap(); // g2k1 (again)
        assert_eq!(key8.key, "g2k1");
        // Group g2 index wraps to 0 again.
        assert_eq!(get_manager_indices(&manager).await, (2, vec![1, 0, 2])); // Next group 2, g2 index 0

        let key9 = manager.get_next_available_key_info().await.unwrap(); // g3k3
        assert_eq!(key9.key, "g3k3");
        // Corrected assertion: g2 index should be 0. g3 index wraps to 0.
        assert_eq!(get_manager_indices(&manager).await, (0, vec![1, 0, 0])); // Next group 0, g3 index 0
    }

     #[tokio::test]
     async fn test_get_next_key_skips_limited_keys_and_groups() {
         let dir = tempdir().unwrap();
         let config_path = create_temp_yaml_config(&dir);
         let groups = vec![
             KeyGroup { // Group 0
                 name: "g1".to_string(),
                 api_keys: vec!["g1k1".to_string(), "g1k2".to_string()],
                 proxy_url: None, target_url: "t1".to_string(),
             },
             KeyGroup { // Group 1 - All keys will be limited
                 name: "g2".to_string(),
                 api_keys: vec!["g2k1_lim".to_string(), "g2k2_lim".to_string()],
                 proxy_url: None, target_url: "t2".to_string(),
             },
             KeyGroup { // Group 2
                 name: "g3".to_string(),
                 api_keys: vec!["g3k1".to_string()],
                 proxy_url: None, target_url: "t3".to_string(),
             },
         ];
         let config = create_test_config(groups);
         let manager = KeyManager::new(&config, &config_path).await;

         // Limit all keys in g2
         manager.mark_key_as_limited("g2k1_lim").await;
         manager.mark_key_as_limited("g2k2_lim").await;
         sleep(Duration::from_millis(250)).await; // Wait for save state tasks if needed

         // Expected sequence: g1k1, g3k1, g1k2, g3k1, g1k1, g3k1 ... (skipping g2)
         let key1 = manager.get_next_available_key_info().await.unwrap(); // g1k1 (starts at group 0)
         assert_eq!(key1.key, "g1k1");
         assert_eq!(get_manager_indices(&manager).await, (1, vec![1, 0, 0])); // Next group 1 (g2), g1 index 1

         let key2 = manager.get_next_available_key_info().await.unwrap(); // g3k1 (skips g2)
         assert_eq!(key2.key, "g3k1");
         // Group g3 has only one key, its index should wrap to 0.
         assert_eq!(get_manager_indices(&manager).await, (0, vec![1, 0, 0])); // Next group 0 (g1), g3 index 0
 
         let key3 = manager.get_next_available_key_info().await.unwrap(); // g1k2
         assert_eq!(key3.key, "g1k2");
         // Corrected assertion: Group g3 index should be 0.
         assert_eq!(get_manager_indices(&manager).await, (1, vec![0, 0, 0])); // Next group 1 (g2), g1 index 0
 
         let key4 = manager.get_next_available_key_info().await.unwrap(); // g3k1 (again, skips g2)
         assert_eq!(key4.key, "g3k1");
         // Group g3 index wraps to 0 again.
         assert_eq!(get_manager_indices(&manager).await, (0, vec![0, 0, 0])); // Next group 0 (g1), g3 index 0
 
         let key5 = manager.get_next_available_key_info().await.unwrap(); // g1k1 (again)
         assert_eq!(key5.key, "g1k1");
         assert_eq!(get_manager_indices(&manager).await, (1, vec![1, 0, 0])); // Next group 1 (g2), g1 index 1
     }

     #[tokio::test]
     async fn test_get_next_key_returns_none_when_all_limited() {
         let dir = tempdir().unwrap();
         let config_path = create_temp_yaml_config(&dir);
         let groups = vec![
             KeyGroup { name: "g1".to_string(), api_keys: vec!["g1k1".to_string()], proxy_url: None, target_url: "t1".to_string(), },
             KeyGroup { name: "g2".to_string(), api_keys: vec!["g2k1".to_string()], proxy_url: None, target_url: "t2".to_string(), },
         ];
         let config = create_test_config(groups);
         let manager = KeyManager::new(&config, &config_path).await;

         manager.mark_key_as_limited("g1k1").await;
         manager.mark_key_as_limited("g2k1").await;
         sleep(Duration::from_millis(250)).await;

         let result = manager.get_next_available_key_info().await;
         assert!(result.is_none());
     }


    #[tokio::test]
    async fn test_load_recovers_from_temp_file() {
        let dir = tempdir().unwrap();
        let config_path = create_temp_yaml_config(&dir);
        let state_path = dir.path().join("key_states.json");
        let base_filename = state_path.file_name().unwrap().to_string_lossy();
        let temp_state_path = dir
            .path()
            .join(format!(".{}.recover_test.tmp", base_filename));

        let future_reset = Utc::now() + ChronoDuration::hours(1);
        let temp_states: HashMap<String, KeyState> = [(
            "key_in_temp".to_string(),
            KeyState {
                is_limited: true,
                reset_time: Some(future_reset),
            },
        )]
        .iter()
        .cloned()
        .collect();
        sync_fs::write(
            &temp_state_path,
            serde_json::to_string(&temp_states).unwrap(),
        )
        .unwrap();
        sync_fs::remove_file(&state_path).ok(); // Ensure main file doesn't exist

        let groups = vec![KeyGroup {
            name: "g1".to_string(),
            api_keys: vec!["key_in_temp".to_string()],
            proxy_url: None,
            target_url: "t1".to_string(),
        }];
        let config = create_test_config(groups);
        let manager = KeyManager::new(&config, &config_path).await; // Should trigger recovery

        let loaded_states = manager.key_states.read().await;
        assert_eq!(loaded_states.len(), 1);
        assert!(loaded_states["key_in_temp"].is_limited);
        assert_eq!(loaded_states["key_in_temp"].reset_time, Some(future_reset));
        assert!(
            state_path.exists(),
            "Main state file should exist after recovery"
        );
        assert!(
            !temp_state_path.exists(),
            "Temp state file should be removed after successful recovery rename"
        );
    }

    #[tokio::test]
    async fn test_load_does_not_recover_from_corrupted_temp_file() {
        let dir = tempdir().unwrap();
        let config_path = create_temp_yaml_config(&dir);
        let state_path = dir.path().join("key_states.json");
        let base_filename = state_path.file_name().unwrap().to_string_lossy();
        let temp_state_path = dir
            .path()
            .join(format!(".{}.corrupt_test.tmp", base_filename));

        sync_fs::write(&temp_state_path, b"this is not valid json { ").unwrap();
        sync_fs::remove_file(&state_path).ok(); // Ensure main file doesn't exist

        let groups = vec![KeyGroup {
            name: "g1".to_string(),
            api_keys: vec!["key1".to_string()],
            proxy_url: None,
            target_url: "t1".to_string(),
        }];
        let config = create_test_config(groups);
        let manager = KeyManager::new(&config, &config_path).await; // Should fail recovery

        let loaded_states = manager.key_states.read().await;
        // State should contain the key from the config with default state, as recovery failed
        assert_eq!(
            loaded_states.len(),
            1,
            "State should contain the key from config"
        );
        assert!(
            loaded_states.contains_key("key1"),
            "State must contain 'key1' from config"
        );
        let key1_state = &loaded_states["key1"];
        assert!(
            !key1_state.is_limited,
            "Key 'key1' should have default state (not limited)"
        );
        assert!(
            key1_state.reset_time.is_none(),
            "Key 'key1' should have default state (no reset time)"
        );
        // The main state file should now exist because KeyManager::new performs an initial save
        assert!(
            state_path.exists(),
            "Main state file should exist after failed recovery and initial save"
        );
        assert!(
            !temp_state_path.exists(),
            "Corrupt temp state file should be removed after failed recovery attempt"
        );
    }

    #[tokio::test]
    async fn test_mark_key_as_limited_block_until_midnight() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("test_config.yaml"); // Need a path even if file is not read
        let groups = vec![KeyGroup {
            name: "g1".to_string(),
            api_keys: vec!["k1".to_string()],
            proxy_url: None,
            target_url: "t1".to_string(),
        }];
        let config = AppConfig {
            server: ServerConfig::default(),
            groups,
            rate_limit_behavior: crate::config::RateLimitBehavior::BlockUntilMidnight,
        };
        let manager = KeyManager::new(&config, &config_path).await;

        manager.mark_key_as_limited("k1").await;
        let states = manager.key_states.read().await;
        let state = &states["k1"];

        assert!(state.is_limited);
        assert!(state.reset_time.is_some());

        let reset_time_utc = state.reset_time.unwrap();
        let target_tz: Tz = Los_Angeles;
        let now_in_target_tz = Utc::now().with_timezone(&target_tz);
        let reset_time_in_target_tz = reset_time_utc.with_timezone(&target_tz);

        // Check if reset time is the next midnight in target timezone
        // It should be after now in the target timezone
        assert!(reset_time_in_target_tz > now_in_target_tz);
        // And the time part should be 00:00:00
        assert_eq!(reset_time_in_target_tz.time().format("%H:%M:%S").to_string(), "00:00:00");
    }

    #[tokio::test]
    async fn test_mark_key_as_limited_retry_next_key() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("test_config.yaml"); // Need a path even if file is not read
        let groups = vec![KeyGroup {
            name: "g1".to_string(),
            api_keys: vec!["k1".to_string()],
            proxy_url: None,
            target_url: "t1".to_string(),
        }];
        let config = AppConfig {
            server: ServerConfig::default(),
            groups,
            rate_limit_behavior: crate::config::RateLimitBehavior::RetryNextKey,
        };
        let manager = KeyManager::new(&config, &config_path).await;

        let now_before_mark = Utc::now();
        manager.mark_key_as_limited("k1").await;
        let now_after_mark = Utc::now();

        let states = manager.key_states.read().await;
        let state = &states["k1"];

        assert!(state.is_limited);
        assert!(state.reset_time.is_some());

        let reset_time_utc = state.reset_time.unwrap();

        // For RetryNextKey, the reset time should be very close to now
        // We check if it's within a small time window around Utc::now()
        let time_difference = reset_time_utc.signed_duration_since(now_before_mark);
        // Assuming the async operations and test execution don't take more than a few seconds
        // This assertion might need tuning based on test environment performance
        assert!(time_difference >= ChronoDuration::zero() && time_difference < ChronoDuration::seconds(5));
    }
}
