// tests/lib_module_tests.rs

use gemini_proxy::{create_router, AppConfig, AppError, AppState, ErrorContext};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::fs;

#[tokio::test]
async fn test_create_router() {
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
    target_url: "https://generativelanguage.googleapis.com"
"#;

    fs::write(&config_path, config_content).await.unwrap();

    let config = gemini_proxy::config::load_config(&config_path).unwrap();
    let (app_state, _) = AppState::new(&config, &config_path).await.unwrap();
    let app_state = Arc::new(app_state);

    let router = create_router(app_state);

    // Проверяем, что роутер создается без ошибок
    let _service = router.into_make_service();
    // Роутер создан успешно, если мы дошли до этой точки
}

#[tokio::test]
async fn test_run_function() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("test_config.yaml");

    let config_content = r#"
server:
  port: 8082
  max_tokens_per_request: 250000
  test_mode: true
  connect_timeout_secs: 10
  request_timeout_secs: 60

groups:
  - name: "test-group"
    api_keys:
      - "test-key-1"
    target_url: "https://generativelanguage.googleapis.com"
"#;

    fs::write(&config_path, config_content).await.unwrap();

    let result = gemini_proxy::run(Some(config_path)).await;
    assert!(
        result.is_ok(),
        "Run function should succeed with valid config"
    );

    let (_router, config) = result.unwrap();
    assert_eq!(config.server.port, 8082);
    assert_eq!(config.groups.len(), 1);
}

#[test]
fn test_app_config_creation() {
    let config = AppConfig {
        server: gemini_proxy::config::ServerConfig {
            port: 8080,
            max_tokens_per_request: Some(250_000),
            test_mode: false,
            connect_timeout_secs: 10,
            request_timeout_secs: 60,
            admin_token: None,
            top_p: None,
            tokenizer_type: None,
        },
        groups: vec![gemini_proxy::config::KeyGroup {
            name: "test-group".to_string(),
            api_keys: vec!["key1".to_string()],
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

    assert_eq!(config.server.port, 8080);
    assert_eq!(config.groups.len(), 1);
    assert_eq!(config.groups[0].name, "test-group");
}

#[test]
fn test_error_context_creation() {
    let context = ErrorContext::new("test_operation")
        .with_metadata("key1", "value1")
        .with_metadata("key2", "value2");

    assert_eq!(context.operation, "test_operation");
    assert_eq!(context.metadata.get("key1"), Some(&"value1".to_string()));
    assert_eq!(context.metadata.get("key2"), Some(&"value2".to_string()));
}

#[test]
fn test_app_error_creation() {
    let error = AppError::Internal {
        message: "Test error".to_string(),
    };

    let error_string = format!("{error}");
    assert!(error_string.contains("Test error"));
}
