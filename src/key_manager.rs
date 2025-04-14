// src/key_manager.rs

 use crate::config::AppConfig;
 use chrono::{DateTime, Duration as ChronoDuration, Utc};
 use chrono_tz::America::Los_Angeles; // Use Los_Angeles timezone (PST/PDT)
 use chrono_tz::Tz; // Import Tz trait
 use serde::{Deserialize, Serialize};
 use std::{
     collections::HashMap,
     fs as std_fs, // Import standard fs for rename
     io as std_io, // Import standard io for Error kind
     path::{Path, PathBuf}, // Added Path, PathBuf
     sync::atomic::{AtomicUsize, Ordering},
 };
 use tokio::fs::{self as async_fs, File as TokioFile}; // Added async_fs, TokioFile
 use tokio::io::AsyncWriteExt; // Added AsyncWriteExt
 use tokio::sync::RwLock;
 use tracing::{debug, error, info, warn};

 // --- Structures moved from state.rs ---

 /// Represents the rate limit state of an individual API key.
 /// Now serializable to persist state.
 #[derive(Debug, Clone, Default, Serialize, Deserialize)] // Added Serialize, Deserialize
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

 /// Manages the pool of API keys, tracks their rate limit states, provides
 /// round-robin rotation logic, and persists/loads state to avoid rechecking limited keys.
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
     /// The path to the file where key states are persisted.
     state_file_path: PathBuf,
 }

 impl KeyManager {
     /// Creates a new `KeyManager` instance from the application configuration and config file path.
     ///
     /// It flattens all keys, loads persisted state from `key_states.json` (if exists),
     /// initializes the rate limit state for each unique key, and stores the state file path.
     pub async fn new(config: &AppConfig, config_path: &Path) -> Self {
         info!("Initializing KeyManager: Flattening keys, loading persisted state, and initializing states...");

         // Determine state file path (config_dir/key_states.json)
         let state_file_path = config_path
             .parent()
             .unwrap_or_else(|| Path::new(".")) // Default to current dir if no parent
             .join("key_states.json");
         info!("Key state persistence file: {}", state_file_path.display());

         // --- Load persisted key states ---
         let persisted_states = load_key_states_from_file(&state_file_path).await;

         let mut all_keys = Vec::new();
         let mut initial_key_states = HashMap::new();
         let mut processed_keys_count = 0;
         let now = Utc::now();

         // --- Iterate through config groups ---
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

                 // --- Initialize key state, prioritizing persisted state ---
                 let state_to_insert = if let Some(persisted) = persisted_states.get(key) {
                     // Check if persisted limit has expired
                     if persisted.is_limited && persisted.reset_time.map_or(false, |rt| now >= rt) {
                         info!(api_key_preview = Self::preview(key), "Persisted limit for key has expired. Initializing as available.");
                         KeyState::default() // Limit expired, reset to default
                     } else {
                         if persisted.is_limited {
                              info!(api_key_preview = Self::preview(key), reset_time=?persisted.reset_time, "Loaded persisted rate limit state for key.");
                         }
                         persisted.clone() // Use valid persisted state
                     }
                 } else {
                     KeyState::default() // No persisted state, use default
                 };

                 initial_key_states.entry(key.clone()).or_insert(state_to_insert);
                 processed_keys_count += 1;
             }
         }

         // Clean up states for keys no longer in config (optional, but good practice)
         initial_key_states.retain(|key, _| {
             let key_in_config = all_keys.iter().any(|ki| ki.key == *key);
             if !key_in_config {
                 warn!(api_key_preview = Self::preview(key), "Removing state for key no longer present in configuration.");
             }
             key_in_config
         });

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
             "KeyManager: Initialized state for {} unique API keys ({} loaded from persistence).",
             initial_key_states.len(),
             persisted_states.len()
         );

         let manager = Self {
             all_keys,
             key_index: AtomicUsize::new(0),
             key_states: RwLock::new(initial_key_states),
             state_file_path,
         };

         // Perform an initial save synchronously before returning the manager.
         // This ensures the state file is consistent after initialization and cleanup/expiration checks.
         debug!("Performing initial state save/sync after KeyManager initialization.");
         if let Err(e) = manager.save_current_states().await {
              // Log error but continue, as the manager might still be partially usable in memory.
              // Subsequent saves might fix the file issue.
             error!(error = ?e, "Failed to perform initial save of key states. The state file might be outdated or missing.");
         } else {
             debug!("Initial state save completed successfully.");
         }

         manager // Return the initialized manager
     }

     /// Retrieves the next available API key information using a round-robin strategy.
     /// Checks persisted and in-memory state, skipping limited keys whose reset time hasn't passed.
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
                 .expect("Key state must exist for a key in all_keys list"); // Should always exist after init

             let now = Utc::now();
             let is_available = if key_state.is_limited {
                 key_state.reset_time.map_or_else(
                     || {
                         warn!(api_key_preview = Self::preview(&key_info.key), group = %key_info.group_name, "Key marked limited but has no reset time! Treating as unavailable.");
                         false // Should not happen, but treat as unavailable
                     },
                     |reset_time| {
                         if now >= reset_time {
                             debug!(api_key_preview = Self::preview(&key_info.key), group = %key_info.group_name, reset_time = %reset_time, "Limit expired for key (checked during get_next)");
                             // NOTE: We don't auto-reset the state here, only check availability.
                             // The state might get reset on the next `mark_key_as_limited` call
                             // or if persistence loads an expired state on restart.
                             true
                         } else {
                             debug!(api_key_preview = Self::preview(&key_info.key), group = %key_info.group_name, reset_time = %reset_time, "Key still limited");
                             false
                         }
                     },
                 )
             } else {
                 true // Not limited
             };

             if is_available {
                 self.key_index
                     .store((current_index + 1) % num_keys, Ordering::Relaxed);
                 debug!(api_key_preview = Self::preview(&key_info.key), group = %key_info.group_name, index = current_index, "Selected available API key");
                 return Some(key_info.clone());
             }
         }
         drop(key_states_guard); // Explicitly drop read lock

         warn!("KeyManager: All API keys are currently rate-limited or unavailable.");
         None
     }

     /// Marks a specific API key as rate-limited and persists the updated state to the file.
     ///
     /// Updates the key's state in memory and triggers an asynchronous save of all key states.
     pub async fn mark_key_as_limited(&self, api_key: &str) {
         let key_preview = Self::preview(api_key);
         let mut should_save = false;

         { // Scope for the write lock
             let mut key_states_guard = self.key_states.write().await;

             if let Some(key_state) = key_states_guard.get_mut(api_key) {
                 let now_utc = Utc::now();
                 let mut state_changed = false;

                 // Check if it was already limited but the limit has now expired
                 if key_state.is_limited && key_state.reset_time.map_or(false, |rt| now_utc >= rt) {
                     info!(api_key_preview=%key_preview, "Resetting previously expired limit before marking again.");
                     // No need to explicitly reset fields here, they will be overwritten below.
                      state_changed = true; // State is logically changing even if is_limited stays true
                 }

                 // Calculate new reset time only if not already limited with a future reset time
                 if !key_state.is_limited || state_changed {
                     warn!(api_key_preview = %key_preview, "Marking key as rate-limited");

                     // Calculate the start of the *next* day in the target timezone (Los Angeles)
                     let target_tz: Tz = Los_Angeles;
                     let now_in_target_tz = now_utc.with_timezone(&target_tz);
                     let tomorrow_naive_target = (now_in_target_tz + ChronoDuration::days(1)).date_naive();
                     let reset_time_naive_target = tomorrow_naive_target.and_hms_opt(0, 0, 0)
                                                .expect("Failed to calculate next midnight (00:00:00) in target timezone");

                     // Get the DateTime<Tz> representation of the reset time
                     // Use single() for unique mapping, could use earliest() or latest() if needed
                     let reset_time_target = reset_time_naive_target.and_local_timezone(target_tz).single()
                                        .expect("Failed to resolve reset time in target timezone (ambiguous time?)");

                     // Convert the reset time back to UTC for storage
                     let reset_time_utc = reset_time_target.with_timezone(&Utc);

                     key_state.is_limited = true;
                     key_state.reset_time = Some(reset_time_utc);
                     state_changed = true; // Ensure state_changed is true if we updated

                     info!(
                         api_key_preview=%key_preview,
                         reset_utc = %reset_time_utc,
                         reset_local = %reset_time_target, // Log local time for clarity
                         "Key limit set until next local midnight" // Generic message
                     );
                 } else {
                     // Already limited, and the reset time is still in the future. Log but don't change state.
                     debug!(api_key_preview = %key_preview, existing_reset_time = ?key_state.reset_time, "Key already marked as limited with a future reset time. Ignoring redundant mark.");
                 }

                 if state_changed {
                     should_save = true; // Mark that we need to save
                 }

             } else {
                 error!(api_key_preview = %key_preview, "Attempted to mark an unknown API key as limited - key not found in states map!");
             }
         } // Write lock released here

         // --- Persist state if changed ---
         if should_save {
             // Spawn a task to save the state asynchronously
             if let Err(e) = self.save_current_states().await {
                  error!(error = ?e, file_path = %self.state_file_path.display(), "Failed to save key states after marking key as limited");
                  // Continue operation even if save fails, but log the error
              }
         }
     }

     /// Asynchronously saves the current state of all keys to the state file using atomic write.
     async fn save_current_states(&self) -> Result<(), std_io::Error> { // Use std_io::Error
         let states_guard = self.key_states.read().await;
         let states_to_save = states_guard.clone(); // Clone the map to release the lock quickly
         drop(states_guard);

         debug!("Attempting to save {} key states to {}", states_to_save.len(), self.state_file_path.display());

         // Use compact JSON format for saving state
         let json_data = serde_json::to_string(&states_to_save)
             .map_err(|e| std_io::Error::new(std_io::ErrorKind::InvalidData, format!("Failed to serialize key states: {}", e)))?;

         // --- Atomic Write Implementation using std::fs::rename ---
         let temp_file_path = self.state_file_path.with_extension("json.tmp");

         // Block to ensure temp_file is dropped before rename
         {
             // 1. Write to temporary file asynchronously
             let mut temp_file = TokioFile::create(&temp_file_path).await?;
             temp_file.write_all(json_data.as_bytes()).await?;
             temp_file.sync_all().await?; // Ensure data is written to disk
             // temp_file is dropped here, closing the handle
         }

         // 2. Rename temporary file to the final destination using synchronous std::fs::rename
         // This is generally atomic on POSIX systems.
         match std_fs::rename(&temp_file_path, &self.state_file_path) {
             Ok(()) => {
                 info!("Successfully saved {} key states to {}", states_to_save.len(), self.state_file_path.display());
                 Ok(())
             }
             Err(e) => {
                 error!(error = ?e, temp_path = %temp_file_path.display(), final_path = %self.state_file_path.display(), "Failed to rename temp state file");
                 // Attempt to clean up the temporary file if rename failed
                 if let Err(remove_err) = async_fs::remove_file(&temp_file_path).await {
                     warn!(error = ?remove_err, temp_path = %temp_file_path.display(), "Failed to remove temporary state file after rename error");
                 }
                 Err(e) // Propagate the original rename error
             }
         }
     }

     /// Creates a short, safe preview of an API key string for logging purposes.
     #[inline]
     fn preview(key: &str) -> String {
         let len = key.chars().count();
         let end = std::cmp::min(4, len);
         format!("{}...", key.chars().take(end).collect::<String>())
     }
 }

 /// Helper function to load key states from the JSON file.
 /// Returns an empty HashMap if the file doesn't exist or fails to parse.
 async fn load_key_states_from_file(path: &Path) -> HashMap<String, KeyState> {
     match async_fs::read_to_string(path).await {
         Ok(json_data) => match serde_json::from_str::<std::collections::HashMap<String, KeyState>>(&json_data) { // Use fully qualified path
             Ok(states) => {
                 info!("Successfully loaded {} key states from {}", states.len(), path.display());
                 states
             }
             Err(e) => {
                 error!(error = %e, file_path = %path.display(), "Failed to parse key state file. Starting with empty states.");
                 HashMap::new() // Parse error, return empty
             }
         },
         Err(ref e) if e.kind() == std_io::ErrorKind::NotFound => { // Use std_io::ErrorKind
             info!("Key state file '{}' not found. Starting with empty states.", path.display());
             HashMap::new() // File not found, return empty
         }
         Err(e) => {
             error!(error = %e, file_path = %path.display(), "Failed to read key state file. Starting with empty states.");
             HashMap::new() // Other read error, return empty
         }
     }
 }


 #[cfg(test)]
 mod tests {
     use super::*;
     use crate::config::{KeyGroup, ServerConfig};
     use tempfile::tempdir; // Use tempdir for isolated tests
     use std::fs::File;
     use std::io::Write;
     use std::time::Duration; // Import Duration for tests
     use std::path::PathBuf; // Added import for create_temp_yaml_config

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

     // Helper to create a temporary config file needed for KeyManager::new
     fn create_temp_yaml_config(dir: &tempfile::TempDir) -> PathBuf {
         let file_path = dir.path().join("test_config.yaml");
         let content = r#"
 server:
   host: "0.0.0.0"
   port: 8080
 groups: [] # Groups will be provided programmatically in tests
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

         // --- Create a persisted state file ---
         let future_reset = Utc::now() + ChronoDuration::hours(1);
         let past_reset = Utc::now() - ChronoDuration::hours(1);
         let persisted_states: HashMap<String, KeyState> = [
             ("key_limited".to_string(), KeyState { is_limited: true, reset_time: Some(future_reset) }),
             ("key_expired".to_string(), KeyState { is_limited: true, reset_time: Some(past_reset) }),
             ("key_nolimit".to_string(), KeyState { is_limited: false, reset_time: None }),
             ("key_not_in_config".to_string(), KeyState { is_limited: true, reset_time: Some(future_reset) }), // This should be removed
         ].iter().cloned().collect();
         let json_data = serde_json::to_string(&persisted_states).unwrap();
         async_fs::write(&state_path, json_data).await.unwrap();

         // --- Config for KeyManager ---
         let groups = vec![
             KeyGroup {
                 name: "g1".to_string(),
                 api_keys: vec!["key_limited".to_string(), "key_expired".to_string(), "key_nolimit".to_string(), "key_new".to_string()],
                 proxy_url: None,
                 target_url: "t1".to_string(),
             }
         ];
         let config = create_test_config(groups);

         // --- Initialize KeyManager ---
         let manager = KeyManager::new(&config, &config_path).await;

         // --- Assertions ---
         let final_states = manager.key_states.read().await;
         assert_eq!(final_states.len(), 4, "Should contain only keys from config"); // key_not_in_config removed, key_new added

         // Check key_limited (persisted and valid)
         assert!(final_states["key_limited"].is_limited);
         assert_eq!(final_states["key_limited"].reset_time, Some(future_reset));

         // Check key_expired (persisted but expired, should be reset)
         assert!(!final_states["key_expired"].is_limited);
         assert!(final_states["key_expired"].reset_time.is_none());

         // Check key_nolimit (persisted as not limited)
         assert!(!final_states["key_nolimit"].is_limited);
         assert!(final_states["key_nolimit"].reset_time.is_none());

         // Check key_new (not in persisted, should be default)
         assert!(final_states.contains_key("key_new"));
         assert!(!final_states["key_new"].is_limited);
         assert!(final_states["key_new"].reset_time.is_none());

          // Check that key_not_in_config was removed
         assert!(!final_states.contains_key("key_not_in_config"));

         // Check state file path
         assert_eq!(manager.state_file_path, state_path);
     }

     #[tokio::test]
     async fn test_mark_key_as_limited_saves_state() {
         let dir = tempdir().unwrap();
         let config_path = create_temp_yaml_config(&dir);
         let state_path = dir.path().join("key_states.json");

         let groups = vec![
             KeyGroup { name: "g1".to_string(), api_keys: vec!["k1".to_string(), "k2".to_string()], proxy_url: None, target_url: "t1".to_string() }
         ];
         let config = create_test_config(groups);
         let manager = KeyManager::new(&config, &config_path).await;

         // Mark k1 as limited
         manager.mark_key_as_limited("k1").await;

         // --- Verify state saved to file ---
         let saved_json = async_fs::read_to_string(&state_path).await.expect("State file should exist after save");
         let saved_states: HashMap<String, KeyState> = serde_json::from_str(&saved_json)
             .expect("Should parse saved JSON state file");

         assert_eq!(saved_states.len(), 2);
         assert!(saved_states.contains_key("k1"));
         assert!(saved_states["k1"].is_limited);
         assert!(saved_states["k1"].reset_time.is_some());
         assert!(saved_states["k1"].reset_time.unwrap() > Utc::now()); // Reset time should be in the future

         assert!(saved_states.contains_key("k2"));
         assert!(!saved_states["k2"].is_limited); // k2 should be unaffected
     }

     #[tokio::test]
     async fn test_get_next_key_skips_persisted_limited_key() {
         let dir = tempdir().unwrap();
         let config_path = create_temp_yaml_config(&dir);
         let state_path = dir.path().join("key_states.json");

         // Persist k1 as limited
         let future_reset = Utc::now() + ChronoDuration::hours(1);
         let persisted: HashMap<String, KeyState> = [("k1".to_string(), KeyState { is_limited: true, reset_time: Some(future_reset) })].iter().cloned().collect();
         async_fs::write(&state_path, serde_json::to_string(&persisted).unwrap()).await.unwrap();

         let groups = vec![
             KeyGroup { name: "g1".to_string(), api_keys: vec!["k1".to_string(), "k2".to_string()], proxy_url: None, target_url: "t1".to_string() }
         ];
         let config = create_test_config(groups);
         let manager = KeyManager::new(&config, &config_path).await;

         // First get should skip k1 (due to loaded state) and return k2
         let key_info1 = manager.get_next_available_key_info().await.unwrap();
         assert_eq!(key_info1.key, "k2");
         assert_eq!(manager.key_index.load(Ordering::Relaxed), 0); // Index points after k2

         // Second get should also return k2 (k1 still limited)
         let key_info2 = manager.get_next_available_key_info().await.unwrap();
         assert_eq!(key_info2.key, "k2");
         assert_eq!(manager.key_index.load(Ordering::Relaxed), 0);
     }

      #[tokio::test]
     async fn test_initial_save_syncs_state_after_loading() {
         let dir = tempdir().unwrap();
         let config_path = create_temp_yaml_config(&dir);
         let state_path = dir.path().join("key_states.json");

         // Persist k1 as limited but expired, k2 as not in config
         let past_reset = Utc::now() - ChronoDuration::hours(1);
         let persisted: HashMap<String, KeyState> = [
             ("k1_expired".to_string(), KeyState { is_limited: true, reset_time: Some(past_reset) }),
             ("k2_removed".to_string(), KeyState { is_limited: false, reset_time: None })
         ].iter().cloned().collect();
         async_fs::write(&state_path, serde_json::to_string(&persisted).unwrap()).await.unwrap();

         // Config only has k1_expired and a new key k3
         let groups = vec![
             KeyGroup { name: "g1".to_string(), api_keys: vec!["k1_expired".to_string(), "k3_new".to_string()], proxy_url: None, target_url: "t1".to_string() }
         ];
         let config = create_test_config(groups);

         // Initialize KeyManager - this should trigger the initial save task
         let _manager = KeyManager::new(&config, &config_path).await;

         // Give the async initial save task time to complete
         tokio::time::sleep(Duration::from_millis(100)).await;

         // Read the state file *after* initialization
         let saved_json = async_fs::read_to_string(&state_path).await.expect("State file should exist");
         let saved_states: HashMap<String, KeyState> = serde_json::from_str(&saved_json).expect("Should parse saved JSON");

         assert_eq!(saved_states.len(), 2, "Saved state should only contain keys from current config");

         // k1_expired should now be saved as non-limited (reset during load)
         assert!(saved_states.contains_key("k1_expired"));
         assert!(!saved_states["k1_expired"].is_limited);
         assert!(saved_states["k1_expired"].reset_time.is_none());

         // k3_new should be saved as default (non-limited)
         assert!(saved_states.contains_key("k3_new"));
         assert!(!saved_states["k3_new"].is_limited);
         assert!(saved_states["k3_new"].reset_time.is_none());

         // k2_removed should not be present in the saved file
         assert!(!saved_states.contains_key("k2_removed"));
     }

     // NOTE: Re-add tests for basic functionality like round-robin if they were removed,
     // ensuring they use the new `KeyManager::new(&config, &config_path).await` signature.
     // Example: test_get_next_key_round_robin

      #[tokio::test]
     async fn test_get_next_key_round_robin_with_persistence() {
         let dir = tempdir().unwrap();
         let config_path = create_temp_yaml_config(&dir); // Need a dummy config file path
         // No state file initially

         let groups = vec![KeyGroup {
             name: "g1".to_string(),
             api_keys: vec!["k1".to_string(), "k2".to_string(), "k3".to_string()],
             proxy_url: None,
             target_url: "t1".to_string(),
         }];
         let config = create_test_config(groups);
         let manager = KeyManager::new(&config, &config_path).await;

         let key_info1 = manager.get_next_available_key_info().await.unwrap();
         assert_eq!(key_info1.key, "k1");
         assert_eq!(manager.key_index.load(Ordering::Relaxed), 1);

         let key_info2 = manager.get_next_available_key_info().await.unwrap();
         assert_eq!(key_info2.key, "k2");
         assert_eq!(manager.key_index.load(Ordering::Relaxed), 2);

         let key_info3 = manager.get_next_available_key_info().await.unwrap();
         assert_eq!(key_info3.key, "k3");
         assert_eq!(manager.key_index.load(Ordering::Relaxed), 0);

         let key_info4 = manager.get_next_available_key_info().await.unwrap();
         assert_eq!(key_info4.key, "k1");
         assert_eq!(manager.key_index.load(Ordering::Relaxed), 1);
     }

     // TODO: Add tests for error handling during file IO (read/write/rename failures)
     // This might require mocking filesystem operations or carefully setting permissions.
 }