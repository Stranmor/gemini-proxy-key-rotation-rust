// tests/key_manager_simple_tests.rs

use gemini_proxy::key_manager::{KeyManager, KeyManagerTrait};
use gemini_proxy::config::{AppConfig, KeyGroup, ServerConfig};

#[tokio::test]
async fn test_key_manager_creation() {
    let config = AppConfig {
        server: ServerConfig {
            port: 8080,
            max_tokens_per_request: Some(250_000),
            test_mode: true,
            connect_timeout_secs: 10,
            request_timeout_secs: 60,
            admin_token: None,
            top_p: None,
            tokenizer_type: None,
        },
        groups: vec![KeyGroup {
            name: "test-group".to_string(),
            api_keys: vec!["test-key-1".to_string(), "test-key-2".to_string()],
            model_aliases: vec![],
            target_url: "https://generativelanguage.googleapis.com".to_string(),
            proxy_url: None,
            top_p: None,
        }],
        redis_url: None,
        redis_key_prefix: None,
        internal_retries: None,
        temporary_block_minutes: None,
        top_p: None,
        max_failures_threshold: None,
        rate_limit: None,
        circuit_breaker: None,
    };
    
    let key_manager = KeyManager::new(&config, None).await;
    assert!(key_manager.is_ok(), "KeyManager should be created successfully");
}

#[tokio::test]
async fn test_key_manager_get_next_available_key() {
    let config = AppConfig {
        server: ServerConfig {
            port: 8080,
            max_tokens_per_request: Some(250_000),
            test_mode: true,
            connect_timeout_secs: 10,
            request_timeout_secs: 60,
            admin_token: None,
            top_p: None,
            tokenizer_type: None,
        },
        groups: vec![KeyGroup {
            name: "test-group".to_string(),
            api_keys: vec!["test-key-1".to_string(), "test-key-2".to_string()],
            model_aliases: vec![],
            target_url: "https://generativelanguage.googleapis.com".to_string(),
            proxy_url: None,
            top_p: None,
        }],
        redis_url: None,
        redis_key_prefix: None,
        internal_retries: None,
        temporary_block_minutes: None,
        top_p: None,
        max_failures_threshold: None,
        rate_limit: None,
        circuit_breaker: None,
    };
    
    let key_manager = KeyManager::new(&config, None).await.unwrap();
    
    let key_info = key_manager.get_next_available_key_info(Some("test-group")).await;
    assert!(key_info.is_ok(), "Should get available key");
    
    let key_info = key_info.unwrap();
    assert!(key_info.is_some(), "Should have available key");
}

#[tokio::test]
async fn test_key_manager_preview() {
    let key = "sk-1234567890abcdef1234567890abcdef";
    let preview = KeyManager::preview_key(&secrecy::Secret::new(key.to_string()));
    
    // Проверяем, что preview скрывает большую часть ключа
    assert!(preview.len() < key.len());
    assert!(preview.contains("sk-"));
    // Проверяем, что ключ замаскирован (может содержать * или другие символы)
    assert!(!preview.contains("1234567890abcdef1234567890abcdef"));
}

#[tokio::test]
async fn test_key_manager_get_all_key_info() {
    let config = AppConfig {
        server: ServerConfig {
            port: 8080,
            max_tokens_per_request: Some(250_000),
            test_mode: true,
            connect_timeout_secs: 10,
            request_timeout_secs: 60,
            admin_token: None,
            top_p: None,
            tokenizer_type: None,
        },
        groups: vec![KeyGroup {
            name: "test-group".to_string(),
            api_keys: vec!["test-key-1".to_string(), "test-key-2".to_string()],
            model_aliases: vec![],
            target_url: "https://generativelanguage.googleapis.com".to_string(),
            proxy_url: None,
            top_p: None,
        }],
        redis_url: None,
        redis_key_prefix: None,
        internal_retries: None,
        temporary_block_minutes: None,
        top_p: None,
        max_failures_threshold: None,
        rate_limit: None,
        circuit_breaker: None,
    };
    
    let key_manager = KeyManager::new(&config, None).await.unwrap();
    
    let all_keys = key_manager.get_all_key_info().await;
    assert_eq!(all_keys.len(), 2, "Should have 2 keys");
}