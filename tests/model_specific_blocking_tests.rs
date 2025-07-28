// tests/model_specific_blocking_tests.rs

use chrono::{Duration, Utc};
use gemini_proxy_key_rotation_rust::key_manager::{KeyManager, ModelBlockState};
use gemini_proxy_key_rotation_rust::config::{AppConfig, KeyGroup, ServerConfig};
use tempfile::tempdir;

fn create_test_config() -> AppConfig {
    AppConfig {
        server: ServerConfig {
            port: 8080,
            top_p: None,
            admin_token: None,
        },
        groups: vec![
            KeyGroup {
                name: "test_group".to_string(),
                api_keys: vec!["test_key_1".to_string(), "test_key_2".to_string()],
                proxy_url: None,
                target_url: "https://generativelanguage.googleapis.com".to_string(),
                top_p: None,
            }
        ],

        internal_retries: 2,
        temporary_block_minutes: 5,
    }
}

#[tokio::test]
async fn test_model_specific_key_blocking() {
    let temp_dir = tempdir().unwrap();
    let config_path = temp_dir.path().join("test_config.yaml");
    let config = create_test_config();
    
    let mut key_manager = KeyManager::new(&config, &config_path).await;
    
    // Initially, both keys should be available for any model
    assert!(key_manager.is_key_available_for_model("test_key_1", Some("gemini-pro")));
    assert!(key_manager.is_key_available_for_model("test_key_1", Some("gemini-flash")));
    assert!(key_manager.is_key_available_for_model("test_key_2", Some("gemini-pro")));
    
    // Block key_1 for gemini-pro model
    assert!(key_manager.mark_key_as_limited_for_model("test_key_1", "gemini-pro"));
    
    // key_1 should be blocked for gemini-pro but available for gemini-flash
    assert!(!key_manager.is_key_available_for_model("test_key_1", Some("gemini-pro")));
    assert!(key_manager.is_key_available_for_model("test_key_1", Some("gemini-flash")));
    
    // key_2 should still be available for both models
    assert!(key_manager.is_key_available_for_model("test_key_2", Some("gemini-pro")));
    assert!(key_manager.is_key_available_for_model("test_key_2", Some("gemini-flash")));
    
    // Test getting next available key for specific model
    let key_for_pro = key_manager.get_next_available_key_info_for_model(Some("gemini-pro"));
    assert!(key_for_pro.is_some());
    assert_eq!(key_for_pro.unwrap().key, "test_key_2");
    
    let key_for_flash = key_manager.get_next_available_key_info_for_model(Some("gemini-flash"));
    assert!(key_for_flash.is_some());
    // Should get key_1 since it's available for gemini-flash
    assert_eq!(key_for_flash.unwrap().key, "test_key_1");
}

#[tokio::test]
async fn test_model_block_cleanup() {
    let temp_dir = tempdir().unwrap();
    let config_path = temp_dir.path().join("test_config.yaml");
    let config = create_test_config();
    
    let mut key_manager = KeyManager::new(&config, &config_path).await;
    
    // Manually add an expired model block
    if let Some(key_state) = key_manager.get_key_states_mut().get_mut("test_key_1") {
        key_state.model_blocks.insert(
            "gemini-pro".to_string(),
            ModelBlockState {
                blocked_until: Utc::now() - Duration::hours(1), // Expired 1 hour ago
                reason: "429 quota exceeded".to_string(),
            }
        );
    }
    
    // Verify the block exists in the data structure (even if expired)
    let key_states = key_manager.get_key_states();
    let key_state = key_states.get("test_key_1").unwrap();
    assert!(key_state.model_blocks.contains_key("gemini-pro"));
    
    // Run cleanup
    let cleaned_count = key_manager.cleanup_expired_model_blocks();
    assert_eq!(cleaned_count, 1);
    
    // Key should be available after cleanup
    assert!(key_manager.is_key_available_for_model("test_key_1", Some("gemini-pro")));
}

#[tokio::test]
async fn test_model_stats() {
    let temp_dir = tempdir().unwrap();
    let config_path = temp_dir.path().join("test_config.yaml");
    let config = create_test_config();
    
    let mut key_manager = KeyManager::new(&config, &config_path).await;
    
    // Block both keys for different models
    key_manager.mark_key_as_limited_for_model("test_key_1", "gemini-pro");
    key_manager.mark_key_as_limited_for_model("test_key_2", "gemini-pro");
    key_manager.mark_key_as_limited_for_model("test_key_1", "gemini-flash");
    
    let stats = key_manager.get_model_block_stats();
    assert_eq!(stats.get("gemini-pro"), Some(&2)); // 2 keys blocked for gemini-pro
    assert_eq!(stats.get("gemini-flash"), Some(&1)); // 1 key blocked for gemini-flash
    
    let blocked_models_info = key_manager.get_blocked_models_info();
    assert_eq!(blocked_models_info.len(), 2); // 2 models have blocks
    
    // Find gemini-pro in the results
    let gemini_pro_info = blocked_models_info.iter()
        .find(|(model, _, _)| model == "gemini-pro")
        .unwrap();
    assert_eq!(gemini_pro_info.1, 2); // 2 keys blocked
}