// tests/storage_tests.rs

use gemini_proxy::{
    key_manager::FlattenedKeyInfo,
    storage::{memory::InMemoryStore, traits::KeyStore},
};
use secrecy::Secret;
use std::collections::HashMap;

#[tokio::test]
async fn test_memory_store_basic_operations() {
    let mut key_info_map = HashMap::new();
    key_info_map.insert(
        "test-key".to_string(),
        FlattenedKeyInfo {
            key: Secret::new("test-key".to_string()),
            group_name: "test-group".to_string(),
            target_url: "https://example.com".to_string(),
            proxy_url: None,
        },
    );

    let store = InMemoryStore::new(&key_info_map);

    // Should have candidate keys
    let keys = store.get_candidate_keys().await;
    assert!(keys.is_ok());
    let keys = keys.unwrap();
    assert_eq!(keys.len(), 1);
    assert!(keys.contains(&"test-key".to_string()));
}

#[tokio::test]
async fn test_memory_store_rotation_index() {
    let key_info_map = HashMap::new();
    let store = InMemoryStore::new(&key_info_map);

    // Test rotation index increment
    let index1 = store.get_next_rotation_index("test-group").await;
    assert!(index1.is_ok());

    let index2 = store.get_next_rotation_index("test-group").await;
    assert!(index2.is_ok());

    // Should increment
    assert!(index2.unwrap() > index1.unwrap());
}

#[tokio::test]
async fn test_memory_store_different_groups() {
    let key_info_map = HashMap::new();
    let store = InMemoryStore::new(&key_info_map);

    // Test different group counters
    let index1 = store.get_next_rotation_index("group1").await.unwrap();
    let index2 = store.get_next_rotation_index("group2").await.unwrap();
    let index3 = store.get_next_rotation_index("group1").await.unwrap();

    // Different groups should have independent counters
    assert_eq!(index1, 0);
    assert_eq!(index2, 0);
    assert_eq!(index3, 1);
}

#[tokio::test]
async fn test_memory_store_multiple_keys() {
    let mut key_info_map = HashMap::new();
    let keys = vec!["key1", "key2", "key3"];

    for key in &keys {
        key_info_map.insert(
            key.to_string(),
            FlattenedKeyInfo {
                key: Secret::new(key.to_string()),
                group_name: "test-group".to_string(),
                target_url: "https://example.com".to_string(),
                proxy_url: None,
            },
        );
    }

    let store = InMemoryStore::new(&key_info_map);

    // Should have all candidate keys
    let candidate_keys = store.get_candidate_keys().await.unwrap();
    assert_eq!(candidate_keys.len(), 3);

    for key in &keys {
        assert!(candidate_keys.contains(&key.to_string()));
    }
}
