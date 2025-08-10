// tests/main_tests.rs

use gemini_proxy::{config, run};
use std::path::PathBuf;
use tempfile::TempDir;
use tokio::fs;

#[tokio::test]
async fn test_run_with_valid_config() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("test_config.yaml");
    
    // Создаем валидный конфиг
    let config_content = r#"
server:
  port: 8081
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
    
    let result = run(Some(config_path)).await;
    assert!(result.is_ok(), "Run should succeed with valid config");
}

#[tokio::test]
async fn test_run_with_invalid_config() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("invalid_config.yaml");
    
    // Создаем невалидный конфиг
    let config_content = r#"
server:
  port: "invalid_port"
groups: []
"#;
    
    fs::write(&config_path, config_content).await.unwrap();
    
    let result = run(Some(config_path)).await;
    assert!(result.is_err(), "Run should fail with invalid config");
}

#[tokio::test]
async fn test_config_load_valid() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("test_config.yaml");
    
    // Создаем валидный конфиг
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
    
    let result = config::load_config(&config_path);
    assert!(result.is_ok(), "Config loading should succeed for valid config");
    
    let config = result.unwrap();
    assert_eq!(config.server.port, 8080);
    assert_eq!(config.groups.len(), 1);
    assert_eq!(config.groups[0].name, "test-group");
}

#[tokio::test]
async fn test_config_load_invalid() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("invalid_config.yaml");
    
    // Создаем невалидный конфиг
    let config_content = r#"
server:
  port: "invalid_port"
groups: []
"#;
    
    fs::write(&config_path, config_content).await.unwrap();
    
    let result = config::load_config(&config_path);
    assert!(result.is_err(), "Config loading should fail for invalid config");
}

#[tokio::test]
async fn test_config_load_missing_file() {
    let result = config::load_config(&PathBuf::from("nonexistent.yaml"));
    assert!(result.is_err(), "Config loading should fail for missing file");
}

#[tokio::test]
async fn test_config_save_and_load() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("save_test_config.yaml");
    
    // Создаем тестовый конфиг
    let original_config = config::AppConfig {
        server: config::ServerConfig {
            port: 9090,
            max_tokens_per_request: Some(300_000),
            test_mode: true,
            connect_timeout_secs: 15,
            request_timeout_secs: 90,
            admin_token: Some("test-admin".to_string()),
            top_p: Some(0.95),
            tokenizer_type: None,
        },
        groups: vec![config::KeyGroup {
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
    let save_result = config::save_config(&original_config, &config_path).await;
    assert!(save_result.is_ok(), "Config should save successfully");
    
    // Загружаем конфиг
    let loaded_config = config::load_config(&config_path);
    assert!(loaded_config.is_ok(), "Config should load successfully");
    
    let loaded_config = loaded_config.unwrap();
    assert_eq!(loaded_config.server.port, original_config.server.port);
    assert_eq!(loaded_config.groups.len(), original_config.groups.len());
    assert_eq!(loaded_config.groups[0].name, original_config.groups[0].name);
    assert_eq!(loaded_config.redis_url, original_config.redis_url);
}