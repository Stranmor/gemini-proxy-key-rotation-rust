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
     sync::{atomic::{AtomicUsize, Ordering}, Arc}, // Added Arc for mutex cloning
 };
 use tokio::fs::{self as async_fs};
 use tokio::sync::{Mutex, RwLock};
 use tokio::task; // For spawning async save task
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
 }

 // --- KeyManager ---

 #[derive(Debug)]
 pub struct KeyManager {
     all_keys: Vec<FlattenedKeyInfo>,
     key_index: AtomicUsize,
     key_states: Arc<RwLock<HashMap<String, KeyState>>>,
     state_file_path: PathBuf,
     save_mutex: Arc<Mutex<()>>,
 }

 impl KeyManager {
     pub async fn new(config: &AppConfig, config_path: &Path) -> Self {
         info!("Initializing KeyManager: Flattening keys, loading persisted state, and initializing states...");
         let state_file_path = config_path
             .parent()
             .unwrap_or_else(|| Path::new("."))
             .join("key_states.json");
         info!("Key state persistence file: {}", state_file_path.display());

         let persisted_states = load_key_states_from_file(&state_file_path).await;
         let mut all_keys = Vec::new();
         let mut initial_key_states = HashMap::new();
         let mut processed_keys_count = 0;
         let now = Utc::now();

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

                 let state_to_insert = if let Some(persisted) = persisted_states.get(key) {
                     if persisted.is_limited && persisted.reset_time.map_or(false, |rt| now >= rt) {
                         info!(api_key_preview = Self::preview(key), "Persisted limit for key has expired. Initializing as available.");
                         KeyState::default()
                     } else {
                         if persisted.is_limited {
                              info!(api_key_preview = Self::preview(key), reset_time=?persisted.reset_time, "Loaded persisted rate limit state for key.");
                         }
                         persisted.clone()
                     }
                 } else {
                     KeyState::default()
                 };
                 initial_key_states.entry(key.clone()).or_insert(state_to_insert);
                 processed_keys_count += 1;
             }
         }

         initial_key_states.retain(|key, _| {
             let key_in_config = all_keys.iter().any(|ki| ki.key == *key);
             if !key_in_config {
                 warn!(api_key_preview = Self::preview(key), "Removing state for key no longer present in configuration.");
             }
             key_in_config
         });

         if all_keys.is_empty() {
              error!("KeyManager Initialization Error: No usable API keys found after processing configuration. Application might not function correctly.");
          }

         info!(
             "KeyManager: Flattened {} total API keys from {} groups into rotation list.",
             processed_keys_count,
             config.groups.len()
         );
         info!(
             "KeyManager: Initialized state for {} unique API keys ({} potentially loaded from persistence).",
             initial_key_states.len(),
             persisted_states.len()
         );

         let manager = Self {
             all_keys,
             key_index: AtomicUsize::new(0),
             key_states: Arc::new(RwLock::new(initial_key_states)),
             state_file_path,
             save_mutex: Arc::new(Mutex::new(())),
         };

         debug!("Performing initial state save/sync after KeyManager initialization.");
         if let Err(e) = manager.save_current_states().await {
             error!(error = ?e, "Failed to perform initial save of key states. The state file might be outdated or missing.");
         } else {
             debug!("Initial state save completed successfully.");
         }
         manager
     }

     pub async fn get_next_available_key_info(&self) -> Option<FlattenedKeyInfo> {
         if self.all_keys.is_empty() {
             warn!("KeyManager: No API keys available in the flattened list. Cannot provide a key.");
             return None;
         }
         let key_states_guard = self.key_states.read().await;
         let start_index = self.key_index.load(Ordering::Relaxed);
         let num_keys = self.all_keys.len();

         for i in 0..num_keys {
             let current_index = (start_index + i) % num_keys;
             let key_info = self.all_keys.get(current_index)?;
             let key_state = match key_states_guard.get(&key_info.key) {
                 Some(state) => state,
                 None => {
                     error!(api_key_preview = Self::preview(&key_info.key), group = %key_info.group_name, "Internal inconsistency: Key found in rotation list but missing from state map!");
                     continue;
                 }
             };

             let now = Utc::now();
             let is_available = if key_state.is_limited {
                 key_state.reset_time.map_or(false, |rt| now >= rt)
             } else {
                 true
             };

             if is_available {
                  if key_state.is_limited {
                       debug!(api_key_preview = Self::preview(&key_info.key), group = %key_info.group_name, "Limit expired for key (checked during get_next)");
                  }
                 self.key_index.store((current_index + 1) % num_keys, Ordering::Relaxed);
                 debug!(api_key_preview = Self::preview(&key_info.key), group = %key_info.group_name, index = current_index, "Selected available API key");
                 return Some(key_info.clone());
             } else {
                  debug!(api_key_preview = Self::preview(&key_info.key), group = %key_info.group_name, reset_time = ?key_state.reset_time, "Key still limited");
             }
         }
         drop(key_states_guard);
         warn!("KeyManager: All API keys are currently rate-limited or unavailable.");
         None
     }

     pub async fn mark_key_as_limited(&self, api_key: &str) {
         let key_preview = Self::preview(api_key);
         let mut should_save = false;

         {
             let mut key_states_guard = self.key_states.write().await;
             if let Some(key_state) = key_states_guard.get_mut(api_key) {
                 let now_utc = Utc::now();
                 let mut state_changed = false;

                 if key_state.is_limited && key_state.reset_time.map_or(false, |rt| now_utc >= rt) {
                     info!(api_key_preview=%key_preview, "Resetting previously expired limit before marking again.");
                     state_changed = true;
                 }

                 if !key_state.is_limited || state_changed {
                     warn!(api_key_preview = %key_preview, "Marking key as rate-limited");
                     let target_tz: Tz = Los_Angeles;
                     let now_in_target_tz = now_utc.with_timezone(&target_tz);
                     let tomorrow_naive_target = (now_in_target_tz + ChronoDuration::days(1)).date_naive();
                     let reset_time_naive_target: NaiveDateTime = tomorrow_naive_target.and_hms_opt(0, 0, 0)
                         .expect("Failed to calculate next midnight (00:00:00) in target timezone");

                     let (reset_time_utc, local_log_str): (DateTime<Utc>, String) =
                         match target_tz.from_local_datetime(&reset_time_naive_target) { // Use TimeZone trait method
                            chrono::LocalResult::Single(dt_target) => {
                                (dt_target.with_timezone(&Utc), dt_target.to_string())
                            }
                            chrono::LocalResult::Ambiguous(dt1, dt2) => {
                                warn!(naive_time = %reset_time_naive_target, tz = ?target_tz, dt1 = %dt1, dt2 = %dt2, "Ambiguous local time calculated for reset, choosing earlier time.");
                                (dt1.with_timezone(&Utc), dt1.to_string())
                            }
                            chrono::LocalResult::None => {
                                error!(naive_time = %reset_time_naive_target, tz = ?target_tz, "Calculated reset time does not exist in the target timezone! Falling back to UTC + 24h.");
                                let fallback_utc = now_utc + ChronoDuration::hours(24);
                                (fallback_utc, "N/A (non-existent local time)".to_string())
                            }
                       };

                     key_state.is_limited = true;
                     key_state.reset_time = Some(reset_time_utc);
                     state_changed = true;

                     info!(
                         api_key_preview=%key_preview,
                         reset_utc = %reset_time_utc,
                         reset_local = %local_log_str,
                         "Key limit set until next local midnight"
                     );
                 } else {
                     debug!(api_key_preview = %key_preview, existing_reset_time = ?key_state.reset_time, "Key already marked as limited with a future reset time. Ignoring redundant mark.");
                 }

                 if state_changed {
                     should_save = true;
                 }
             } else {
                 error!(api_key_preview = %key_preview, "Attempted to mark an unknown API key as limited - key not found in states map!");
             }
         } // Write lock released

         if should_save {
             let state_file_path_clone = self.state_file_path.clone();
             let states_clone = Arc::clone(&self.key_states);
             let save_mutex_clone = Arc::clone(&self.save_mutex);

             task::spawn(async move {
                 let _save_guard = save_mutex_clone.lock().await;
                 debug!("Async save task acquired save mutex lock for {}", state_file_path_clone.display());
                 let states_guard = states_clone.read().await;
                 let states_to_save = states_guard.clone();
                 drop(states_guard);

                 if let Err(e) = Self::save_states_to_file_impl(&state_file_path_clone, &states_to_save).await {
                     error!(error = ?e, file_path = %state_file_path_clone.display(), "Async save task failed to save key states");
                 } else {
                     debug!("Async save task completed successfully for {}", state_file_path_clone.display());
                 }
             });
         }
     }

     /// Asynchronously saves the current state (used for background saves and initial save).
     async fn save_current_states(&self) -> AppResult<()> {
         let _save_guard = self.save_mutex.lock().await;
         debug!("Acquired save mutex lock for save: {}", self.state_file_path.display());
         let states_guard = self.key_states.read().await;
         let states_to_save = states_guard.clone();
         drop(states_guard);
         Self::save_states_to_file_impl(&self.state_file_path, &states_to_save).await
             .map_err(AppError::Io)
     }


     /// Implementation detail: Performs the atomic save operation.
     async fn save_states_to_file_impl(
         final_path: &Path,
         states: &HashMap<String, KeyState>
     ) -> std_io::Result<()> {
         debug!("Attempting atomic save of {} key states to {}", states.len(), final_path.display());
         let parent_dir = final_path.parent().ok_or_else(|| {
             std_io::Error::new(std_io::ErrorKind::InvalidInput, "State file path has no parent directory")
         })?;
         async_fs::create_dir_all(parent_dir).await?;
         let base_filename = final_path.file_name().unwrap_or_default().to_string_lossy();
         let temp_filename = format!(".{}.{}.tmp", base_filename, Uuid::new_v4());
         let temp_path = parent_dir.join(temp_filename);

         let json_data = serde_json::to_string_pretty(states).map_err(|e| {
             error!(error = %e, "Failed to serialize key states to JSON");
             std_io::Error::new(std_io::ErrorKind::InvalidData, format!("Failed to serialize key states: {}", e))
         })?;

         debug!("Writing state to temporary file: {}", temp_path.display());
         if let Err(e) = async_fs::write(&temp_path, json_data.as_bytes()).await {
             error!(error = ?e, temp_file = %temp_path.display(), "Failed to write to temporary state file");
             let _ = std_fs::remove_file(&temp_path);
             return Err(e);
         }

         debug!("Attempting atomic rename from {} to {}", temp_path.display(), final_path.display());
         if let Err(e) = std_fs::rename(&temp_path, final_path) {
             error!(error = ?e, temp_file = %temp_path.display(), final_file = %final_path.display(), "Failed atomic rename of state file");
             let _ = std_fs::remove_file(&temp_path);
             return Err(e);
         }

         info!("Successfully saved {} key states atomically to {}", states.len(), final_path.display());
         Ok(())
     }

     #[inline]
     fn preview(key: &str) -> String {
         let len = key.chars().count();
         let end = std::cmp::min(6, len);
         let start = if len > 8 { len - 4 } else { end };
         if len > 8 {
             format!("{}...{}", key.chars().take(end).collect::<String>(), key.chars().skip(start).collect::<String>())
         } else {
             format!("{}...", key.chars().take(end).collect::<String>())
         }
     }
 }

 /// Helper function to load key states from the JSON file, with recovery attempt from temp file.
 async fn load_key_states_from_file(path: &Path) -> HashMap<String, KeyState> {
      let base_filename = path.file_name().unwrap_or_default().to_string_lossy();
      let parent_dir = path.parent().unwrap_or_else(|| Path::new("."));

      let mut recovered_from_temp = false;
      let mut recovered_states = HashMap::new();

     match async_fs::read_to_string(path).await {
         Ok(json_data) => {
             cleanup_temp_files(parent_dir, &base_filename).await;
             match serde_json::from_str::<HashMap<String, KeyState>>(&json_data) {
                 Ok(states) => {
                     info!("Successfully loaded {} key states from {}", states.len(), path.display());
                     return states;
                 }
                 Err(e) => {
                     error!(error = %e, file_path = %path.display(), "Failed to parse key state file (JSON invalid). Attempting recovery.");
                 }
             }
         },
         Err(ref e) if e.kind() == std_io::ErrorKind::NotFound => {
             warn!("Key state file '{}' not found. Checking for temporary recovery file.", path.display());
         }
         Err(e) => {
             error!(error = %e, file_path = %path.display(), "Failed to read key state file due to IO error. Attempting recovery.");
         }
     }

     if let Some(temp_path) = find_latest_temp_file(parent_dir, &base_filename).await {
          warn!(temp_file = %temp_path.display(), "Attempting recovery from temporary state file.");
          match async_fs::read_to_string(&temp_path).await {
              Ok(temp_json_data) => {
                  match serde_json::from_str::<HashMap<String, KeyState>>(&temp_json_data) {
                      Ok(states) => {
                          info!("Successfully recovered {} key states from temporary file {}", states.len(), temp_path.display());
                          if let Err(rename_err) = std_fs::rename(&temp_path, path) {
                              error!(error = ?rename_err, temp_file = %temp_path.display(), "Failed to rename recovered temp state file to main path. State recovered in memory, but file system may be inconsistent.");
                          } else {
                              info!("Successfully renamed recovered temp state file to main path: {}", path.display());
                              cleanup_temp_files(parent_dir, &base_filename).await;
                          }
                          recovered_from_temp = true;
                          recovered_states = states;
                      }
                      Err(parse_e) => {
                          error!(error = %parse_e, temp_file = %temp_path.display(), "Failed to parse temporary key state file (JSON invalid). Recovery failed.");
                          let _ = std_fs::remove_file(&temp_path);
                          // Ensure we return empty map if temp file parsing fails
                          recovered_states = HashMap::new(); // Reset recovered states
                      }
                  }
              }
              Err(read_e) => {
                  error!(error = %read_e, temp_file = %temp_path.display(), "Failed to read temporary key state file. Recovery failed.");
                  let _ = std_fs::remove_file(&temp_path);
                   // Ensure we return empty map if temp file reading fails
                   recovered_states = HashMap::new(); // Reset recovered states
              }
          }
     } else {
         info!("No temporary state file found for recovery.");
     }

     if recovered_from_temp {
         recovered_states
     } else {
         warn!("Recovery failed or no file found. Starting with empty key states.");
         HashMap::new()
     }
 }

 /// Finds the most recently modified temporary state file matching the pattern.
 async fn find_latest_temp_file(dir: &Path, base_filename: &str) -> Option<PathBuf> {
     let mut latest_mod_time: Option<std::time::SystemTime> = None;
     let mut latest_temp_file: Option<PathBuf> = None;
     let temp_prefix = format!(".{}.", base_filename);
     let temp_suffix = ".tmp";

     if let Ok(mut read_dir) = async_fs::read_dir(dir).await {
         while let Ok(Some(entry)) = read_dir.next_entry().await {
             let path = entry.path();
             if path.is_file() {
                  if let Some(filename) = path.file_name().map(|n| n.to_string_lossy()) {
                      if filename.starts_with(&temp_prefix) && filename.ends_with(temp_suffix) {
                           if let Ok(metadata) = entry.metadata().await {
                                if let Ok(modified) = metadata.modified() {
                                     if latest_mod_time.map_or(true, |latest| modified > latest) {
                                         latest_mod_time = Some(modified);
                                         latest_temp_file = Some(path.clone());
                                     }
                                }
                           }
                      }
                  }
             }
         }
     }
     latest_temp_file
 }

 /// Cleans up all temporary state files matching the pattern in a directory.
 async fn cleanup_temp_files(dir: &Path, base_filename: &str) {
      let temp_prefix = format!(".{}.", base_filename);
      let temp_suffix = ".tmp";
      if let Ok(mut read_dir) = async_fs::read_dir(dir).await {
           while let Ok(Some(entry)) = read_dir.next_entry().await {
               let path = entry.path();
               if path.is_file() {
                    if let Some(filename) = path.file_name().map(|n| n.to_string_lossy()) {
                         if filename.starts_with(&temp_prefix) && filename.ends_with(temp_suffix) {
                              warn!(temp_file = %path.display(), "Cleaning up leftover temporary state file.");
                              if let Err(e) = async_fs::remove_file(&path).await {
                                   error!(error = ?e, temp_file = %path.display(), "Failed during cleanup of temporary state file.");
                              }
                         }
                    }
               }
           }
      }
  }

 #[cfg(test)]
 mod tests {
     use super::*;
     use crate::config::{KeyGroup, ServerConfig};
     use tempfile::tempdir;
     use std::fs::{self as sync_fs, File};
     use std::io::Write;
     use std::time::Duration;
     use std::path::PathBuf;
     use tokio::time::sleep;

     fn create_test_config(groups: Vec<KeyGroup>) -> AppConfig {
         AppConfig {
             server: ServerConfig { host: "0.0.0.0".to_string(), port: 8080 },
             groups,
         }
     }

     fn create_temp_yaml_config(dir: &tempfile::TempDir) -> PathBuf {
         let file_path = dir.path().join("test_config.yaml");
         let content = r#"
 server:
   host: "0.0.0.0"
   port: 8080
 groups: []
 "#;
         let mut file = File::create(&file_path).unwrap();
         writeln!(file, "{}", content).unwrap();
         file_path
     }

     #[tokio::test]
     async fn test_key_manager_initialization_loads_persisted_state() {
         let dir = tempdir().unwrap();
         let config_path = create_temp_yaml_config(&dir);
         let state_path = dir.path().join("key_states.json");

         let future_reset = Utc::now() + ChronoDuration::hours(1);
         let past_reset = Utc::now() - ChronoDuration::hours(1);
         let persisted_states: HashMap<String, KeyState> = [
             ("key_limited".to_string(), KeyState { is_limited: true, reset_time: Some(future_reset) }),
             ("key_expired".to_string(), KeyState { is_limited: true, reset_time: Some(past_reset) }),
             ("key_nolimit".to_string(), KeyState { is_limited: false, reset_time: None }),
             ("key_not_in_config".to_string(), KeyState { is_limited: true, reset_time: Some(future_reset) }),
         ].iter().cloned().collect();
         let json_data = serde_json::to_string(&persisted_states).unwrap();
         sync_fs::write(&state_path, json_data).unwrap();

         let groups = vec![
             KeyGroup {
                 name: "g1".to_string(),
                 api_keys: vec!["key_limited".to_string(), "key_expired".to_string(), "key_nolimit".to_string(), "key_new".to_string()],
                 proxy_url: None, target_url: "t1".to_string(),
             }
         ];
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
         File::create(&state_path).unwrap().write_all(b"initial_content").unwrap();

         let groups = vec![KeyGroup { name: "g1".to_string(), api_keys: vec!["k1".to_string(), "k2".to_string()], proxy_url: None, target_url: "t1".to_string() }];
         let config = create_test_config(groups);
         let manager = KeyManager::new(&config, &config_path).await;

         sleep(Duration::from_millis(50)).await;
         let initial_saved_json = sync_fs::read_to_string(&state_path).expect("State file should exist after init");
         let initial_saved_states: HashMap<String, KeyState> = serde_json::from_str(&initial_saved_json).expect("Should parse initial JSON");
         assert_eq!(initial_saved_states.len(), 2);

         manager.mark_key_as_limited("k1").await;
         sleep(Duration::from_millis(250)).await;

         let saved_json = sync_fs::read_to_string(&state_path).expect("State file should exist after save");
         let saved_states: HashMap<String, KeyState> = serde_json::from_str(&saved_json).expect("Should parse saved JSON");

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
                      if filename.starts_with(&format!(".{}.", base_filename)) && filename.ends_with(".tmp") {
                          error!("Found unexpected temp file: {}", path.display());
                          found_temp = true;
                      }
                  }
             }
         }
         assert!(!found_temp, "Temporary state file should not exist after successful save");
     }

     #[tokio::test]
     async fn test_get_next_key_skips_persisted_limited_key() {
         let dir = tempdir().unwrap();
         let config_path = create_temp_yaml_config(&dir);
         let state_path = dir.path().join("key_states.json");
         let future_reset = Utc::now() + ChronoDuration::hours(1);
         let persisted: HashMap<String, KeyState> = [("k1".to_string(), KeyState { is_limited: true, reset_time: Some(future_reset) })].iter().cloned().collect();
         sync_fs::write(&state_path, serde_json::to_string(&persisted).unwrap()).unwrap();

         let groups = vec![KeyGroup { name: "g1".to_string(), api_keys: vec!["k1".to_string(), "k2".to_string()], proxy_url: None, target_url: "t1".to_string() }];
         let config = create_test_config(groups);
         let manager = KeyManager::new(&config, &config_path).await;

         let key_info1 = manager.get_next_available_key_info().await.unwrap();
         assert_eq!(key_info1.key, "k2");
         assert_eq!(manager.key_index.load(Ordering::Relaxed), 0);
         let key_info2 = manager.get_next_available_key_info().await.unwrap();
         assert_eq!(key_info2.key, "k2");
         assert_eq!(manager.key_index.load(Ordering::Relaxed), 0);
     }

     #[tokio::test]
     async fn test_initial_save_syncs_state_after_loading() {
         let dir = tempdir().unwrap();
         let config_path = create_temp_yaml_config(&dir);
         let state_path = dir.path().join("key_states.json");
         let past_reset = Utc::now() - ChronoDuration::hours(1);
         let persisted: HashMap<String, KeyState> = [
             ("k1_expired".to_string(), KeyState { is_limited: true, reset_time: Some(past_reset) }),
             ("k2_removed".to_string(), KeyState { is_limited: false, reset_time: None })
         ].iter().cloned().collect();
         sync_fs::write(&state_path, serde_json::to_string(&persisted).unwrap()).unwrap();
         let groups = vec![KeyGroup { name: "g1".to_string(), api_keys: vec!["k1_expired".to_string(), "k3_new".to_string()], proxy_url: None, target_url: "t1".to_string() }];
         let config = create_test_config(groups);
         let _manager = KeyManager::new(&config, &config_path).await;

         let saved_json = sync_fs::read_to_string(&state_path).expect("State file should exist");
         let saved_states: HashMap<String, KeyState> = serde_json::from_str(&saved_json).expect("Should parse saved JSON");
         assert_eq!(saved_states.len(), 2);
         assert!(!saved_states["k1_expired"].is_limited);
         assert!(saved_states["k1_expired"].reset_time.is_none());
         assert!(!saved_states["k3_new"].is_limited);
         assert!(saved_states["k3_new"].reset_time.is_none());
         assert!(!saved_states.contains_key("k2_removed"));
     }

     #[tokio::test]
     async fn test_get_next_key_round_robin_with_persistence() {
         let dir = tempdir().unwrap();
         let config_path = create_temp_yaml_config(&dir);
         let groups = vec![KeyGroup { name: "g1".to_string(), api_keys: vec!["k1".to_string(), "k2".to_string(), "k3".to_string()], proxy_url: None, target_url: "t1".to_string() }];
         let config = create_test_config(groups);
         let manager = KeyManager::new(&config, &config_path).await;
         assert_eq!(manager.get_next_available_key_info().await.unwrap().key, "k1");
         assert_eq!(manager.get_next_available_key_info().await.unwrap().key, "k2");
         assert_eq!(manager.get_next_available_key_info().await.unwrap().key, "k3");
         assert_eq!(manager.get_next_available_key_info().await.unwrap().key, "k1");
     }

     #[tokio::test]
     async fn test_load_recovers_from_temp_file() {
         let dir = tempdir().unwrap();
         let config_path = create_temp_yaml_config(&dir);
         let state_path = dir.path().join("key_states.json");
         let base_filename = state_path.file_name().unwrap().to_string_lossy();
         let temp_state_path = dir.path().join(format!(".{}.recover_test.tmp", base_filename));

         let future_reset = Utc::now() + ChronoDuration::hours(1);
         let temp_states: HashMap<String, KeyState> = [("key_in_temp".to_string(), KeyState { is_limited: true, reset_time: Some(future_reset) })].iter().cloned().collect();
         sync_fs::write(&temp_state_path, serde_json::to_string(&temp_states).unwrap()).unwrap();
         sync_fs::remove_file(&state_path).ok();

         let groups = vec![KeyGroup { name: "g1".to_string(), api_keys: vec!["key_in_temp".to_string()], proxy_url: None, target_url: "t1".to_string() }];
         let config = create_test_config(groups);
         let manager = KeyManager::new(&config, &config_path).await;

         let loaded_states = manager.key_states.read().await;
         assert_eq!(loaded_states.len(), 1);
         assert!(loaded_states["key_in_temp"].is_limited);
         assert_eq!(loaded_states["key_in_temp"].reset_time, Some(future_reset));
         assert!(state_path.exists(), "Main state file should exist after recovery");
         assert!(!temp_state_path.exists(), "Temp state file should be removed after successful recovery rename");
     }

     #[tokio::test]
     async fn test_load_does_not_recover_from_corrupted_temp_file() {
         let dir = tempdir().unwrap();
         let config_path = create_temp_yaml_config(&dir);
         let state_path = dir.path().join("key_states.json");
         let base_filename = state_path.file_name().unwrap().to_string_lossy();
         let temp_state_path = dir.path().join(format!(".{}.corrupt_test.tmp", base_filename));

         sync_fs::write(&temp_state_path, b"this is not valid json { ").unwrap();
         sync_fs::remove_file(&state_path).ok();

         let groups = vec![KeyGroup { name: "g1".to_string(), api_keys: vec!["key1".to_string()], proxy_url: None, target_url: "t1".to_string() }];
         let config = create_test_config(groups);
         let manager = KeyManager::new(&config, &config_path).await;

         let loaded_states = manager.key_states.read().await;
         // State should contain the key from the config with default state, as recovery failed
         assert_eq!(loaded_states.len(), 1, "State should contain the key from config");
         assert!(loaded_states.contains_key("key1"), "State must contain 'key1' from config");
         let key1_state = &loaded_states["key1"];
         assert!(!key1_state.is_limited, "Key 'key1' should have default state (not limited)");
         assert!(key1_state.reset_time.is_none(), "Key 'key1' should have default state (no reset time)");
         // The main state file should now exist because KeyManager::new performs an initial save
         assert!(state_path.exists(), "Main state file should exist after failed recovery and initial save");
         assert!(!temp_state_path.exists(), "Corrupt temp state file should be removed after failed recovery attempt");
     }
 }