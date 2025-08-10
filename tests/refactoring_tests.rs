// tests/refactoring_tests.rs

use gemini_proxy::{
    config::{AppConfig, KeyGroup, ServerConfig},
    key_manager::{KeyManager, KeyManagerTrait},
    storage::{memory::InMemoryStore, traits::KeyStore},
};
use secrecy::{ExposeSecret, Secret};
use std::collections::HashMap;

#[tokio::test]
async fn test_key_selector_round_robin() {
    // Create test configuration
    let config = AppConfig {
        server: ServerConfig::default(),
        groups: vec![KeyGroup {
            name: "test_group".to_string(),
            api_keys: vec!["key1".to_string(), "key2".to_string(), "key3".to_string()],
            target_url: "https://api.example.com".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };

    // Create key manager
    let key_manager = KeyManager::new(&config, None).await.unwrap();

    // Test getting keys
    let key1 = key_manager
        .get_next_available_key_info(Some("test_group"))
        .await
        .unwrap();
    assert!(key1.is_some());

    let key2 = key_manager
        .get_next_available_key_info(Some("test_group"))
        .await
        .unwrap();
    assert!(key2.is_some());

    // Keys should be different due to round-robin
    assert_ne!(
        key1.as_ref().unwrap().key.expose_secret(),
        key2.as_ref().unwrap().key.expose_secret()
    );
}


#[tokio::test]
async fn test_memory_store_operations() {
    let mut key_info_map = HashMap::new();
    key_info_map.insert(
        "test_key".to_string(),
        gemini_proxy::key_manager::FlattenedKeyInfo {
            key: Secret::new("test_key".to_string()),
            group_name: "test_group".to_string(),
            target_url: "https://api.example.com".to_string(),
            proxy_url: None,
        },
    );

    let store = InMemoryStore::new(&key_info_map);

    // Test getting candidate keys
    let keys = store.get_candidate_keys().await.unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0], "test_key");

    // Test rotation index
    let index1 = store.get_next_rotation_index("test_group").await.unwrap();
    let index2 = store.get_next_rotation_index("test_group").await.unwrap();
    assert_eq!(index2, index1 + 1);

    // Test key state
    let state = store.get_key_state("test_key").await.unwrap();
    assert!(state.is_some());
    assert!(state.unwrap().is_available());
}
