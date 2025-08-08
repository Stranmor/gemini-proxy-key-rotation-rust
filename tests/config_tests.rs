// tests/config_tests.rs

use gemini_proxy::config::{validation::ConfigValidator, AppConfig, KeyGroup, ServerConfig};

#[test]
fn test_config_validation_valid_config() {
    let config = AppConfig {
        server: ServerConfig {
            max_tokens_per_request: Some(250_000),
            port: 8080,
            top_p: Some(0.9),
            admin_token: Some("test-token".to_string()),
            test_mode: false,
            connect_timeout_secs: 10,
            request_timeout_secs: 60,
            tokenizer_type: None,
        },
        groups: vec![KeyGroup {
            name: "test-group".to_string(),
            api_keys: vec!["key1".to_string(), "key2".to_string()],
            model_aliases: vec![],
            target_url: "https://example.com".to_string(),
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

    let result = ConfigValidator::validate(&config);
    assert!(result.is_ok(), "Valid config should pass validation");
}

#[test]
fn test_config_validation_empty_groups() {
    let config = AppConfig {
        server: ServerConfig {
            max_tokens_per_request: Some(250_000),
            port: 8080,
            top_p: None,
            admin_token: None,
            test_mode: false,
            connect_timeout_secs: 10,
            request_timeout_secs: 60,
            tokenizer_type: None,
        },
        groups: vec![], // Empty groups should fail
        redis_url: None,
        redis_key_prefix: None,
        internal_retries: None,
        temporary_block_minutes: None,
        top_p: None,
        max_failures_threshold: None,
        rate_limit: None,
        circuit_breaker: None,
    };

    let result = ConfigValidator::validate(&config);
    assert!(result.is_err(), "Empty groups should fail validation");
}
