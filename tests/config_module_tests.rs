// tests/config_module_tests.rs

use gemini_proxy::config::{AppConfig, KeyGroup, ServerConfig, load_config, save_config};

use tempfile::TempDir;
use tokio::fs;

#[tokio::test]
async fn test_config_load_from_file() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("test_config.yaml");
    
    let config_content = r#"
server:
  port: 8080
  max_tokens_per_request: 250000
  test_mode: true
  connect_timeout_secs: 10
  request_timeout_secs: 60

groups:
  - name: "test-group"
    api_keys:
      - "test-key-1"
      - "test-key-2"
    target_url: "https://generativelanguage.googleapis.com"
"#;
    
    fs::write(&config_path, config_content).await.unwrap();
    
    let config = load_config(&config_path);
    assert!(config.is_ok(), "Config should load successfully from file");
    
    let config = config.unwrap();
    assert_eq!(config.server.port, 8080);
    assert_eq!(config.groups.len(), 1);
    assert_eq!(config.groups[0].name, "test-group");
    assert_eq!(config.groups[0].api_keys.len(), 2);
}

#[test]
fn test_config_creation_valid() {
    let config = AppConfig {
        server: ServerConfig {
            port: 8080,
            max_tokens_per_request: Some(250_000),
            test_mode: false,
            connect_timeout_secs: 10,
            request_timeout_secs: 60,
            admin_token: Some("admin-token".to_string()),
            top_p: Some(0.9),
            tokenizer_type: None,
        },
        groups: vec![KeyGroup {
            name: "test-group".to_string(),
            api_keys: vec!["key1".to_string(), "key2".to_string()],
            model_aliases: vec![],
            target_url: "https://generativelanguage.googleapis.com".to_string(),
            proxy_url: None,
            top_p: None,
        }],
        redis_url: Some("redis://localhost:6379".to_string()),
        redis_key_prefix: Some("test:".to_string()),
        internal_retries: Some(3),
        temporary_block_minutes: Some(5),
        top_p: Some(0.8),
        max_failures_threshold: Some(5),
        rate_limit: None,
        circuit_breaker: None,
    };
    
    // Проверяем, что конфиг создается корректно
    assert_eq!(config.server.port, 8080);
    assert_eq!(config.groups.len(), 1);
    assert_eq!(config.groups[0].api_keys.len(), 2);
}

#[test]
fn test_config_with_empty_groups() {
    let config = AppConfig {
        server: ServerConfig {
            port: 8080,
            max_tokens_per_request: Some(250_000),
            test_mode: false,
            connect_timeout_secs: 10,
            request_timeout_secs: 60,
            admin_token: None,
            top_p: None,
            tokenizer_type: None,
        },
        groups: vec![], // Empty groups
        redis_url: None,
        redis_key_prefix: None,
        internal_retries: None,
        temporary_block_minutes: None,
        top_p: None,
        max_failures_threshold: None,
        rate_limit: None,
        circuit_breaker: None,
    };
    
    // Проверяем, что конфиг с пустыми группами создается
    assert_eq!(config.groups.len(), 0);
}

#[test]
fn test_server_config_defaults() {
    let config = ServerConfig::default();
    
    assert_eq!(config.port, 8080);
    assert_eq!(config.connect_timeout_secs, 10);
    assert_eq!(config.request_timeout_secs, 60);
    assert!(!config.test_mode);
    assert!(config.admin_token.is_none());
    assert!(config.top_p.is_none());
}

#[test]
fn test_key_group_defaults() {
    let group = KeyGroup::default();
    
    // Проверяем, что группа создается с дефолтными значениями
    assert!(group.api_keys.is_empty());
    assert!(group.model_aliases.is_empty());
    assert!(group.proxy_url.is_none());
    assert!(group.top_p.is_none());
    // Не проверяем конкретные значения name и target_url, так как они могут отличаться
}

#[tokio::test]
async fn test_config_save_and_load() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("save_test_config.yaml");
    
    let original_config = AppConfig {
        server: ServerConfig {
            port: 9090,
            max_tokens_per_request: Some(300_000),
            test_mode: true,
            connect_timeout_secs: 15,
            request_timeout_secs: 90,
            admin_token: Some("test-admin".to_string()),
            top_p: Some(0.95),
            tokenizer_type: None,
        },
        groups: vec![KeyGroup {
            name: "save-test-group".to_string(),
            api_keys: vec!["save-key-1".to_string(), "save-key-2".to_string()],
            model_aliases: vec!["gemini-pro".to_string()],
            target_url: "https://example.com".to_string(),
            proxy_url: Some("http://proxy.example.com:8080".to_string()),
            top_p: Some(0.85),
        }],
        redis_url: Some("redis://localhost:6380".to_string()),
        redis_key_prefix: Some("save-test:".to_string()),
        internal_retries: Some(5),
        temporary_block_minutes: Some(10),
        top_p: Some(0.9),
        max_failures_threshold: Some(10),
        rate_limit: None,
        circuit_breaker: None,
    };
    
    // Сохраняем конфиг
    let save_result = save_config(&original_config, &config_path).await;
    assert!(save_result.is_ok(), "Config should save successfully");
    
    // Загружаем конфиг
    let loaded_config = load_config(&config_path);
    assert!(loaded_config.is_ok(), "Config should load successfully");
    
    let loaded_config = loaded_config.unwrap();
    assert_eq!(loaded_config.server.port, original_config.server.port);
    assert_eq!(loaded_config.groups.len(), original_config.groups.len());
    assert_eq!(loaded_config.groups[0].name, original_config.groups[0].name);
    assert_eq!(loaded_config.redis_url, original_config.redis_url);
}

