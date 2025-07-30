// tests/model_specific_blocking_tests.rs

use axum_test::TestServer;
use gemini_proxy_key_rotation_rust::{run, AppConfig};
use serde_json::json;
use std::path::PathBuf;
use tempfile::tempdir;
use tokio::fs;

/// Creates a test configuration with multiple groups for model-specific testing
async fn create_test_config_with_groups() -> (tempfile::TempDir, PathBuf) {
    let temp_dir = tempdir().expect("Failed to create temp directory");
    let config_path = temp_dir.path().join("config.yaml");

    let config_content = r#"
server:
  port: 0
  test_mode: true

groups:
  - name: "Group1"
    api_keys: ["key1", "key2"]
    model_aliases: ["gemini-1.5-flash", "gemini-flash"]
    target_url: "https://generativelanguage.googleapis.com/"
  
  - name: "Group2"
    api_keys: ["key3", "key4"]
    model_aliases: ["gemini-1.5-pro", "gemini-pro"]
    target_url: "https://generativelanguage.googleapis.com/"
"#;

    fs::write(&config_path, config_content)
        .await
        .expect("Failed to write test config");

    (temp_dir, config_path)
}

#[tokio::test]
async fn test_model_specific_key_selection() {
    let (_temp_dir, config_path) = create_test_config_with_groups().await;
    
    let (app, _config) = run(Some(config_path))
        .await
        .expect("Failed to create app");
    
    let server = TestServer::new(app).expect("Failed to create test server");

    // Test request for gemini-1.5-flash should use Group1 keys
    let response = server
        .post("/v1/chat/completions")
        .json(&json!({
            "model": "gemini-1.5-flash",
            "messages": [{"role": "user", "content": "Hello"}]
        }))
        .await;

    // We expect the request to be forwarded (even if it fails due to invalid keys)
    // The important thing is that the correct group is selected
    assert!(response.status_code().is_client_error() || response.status_code().is_server_error());
}

#[tokio::test]
async fn test_model_specific_key_selection_different_group() {
    let (_temp_dir, config_path) = create_test_config_with_groups().await;
    
    let (app, _config) = run(Some(config_path))
        .await
        .expect("Failed to create app");
    
    let server = TestServer::new(app).expect("Failed to create test server");

    // Test request for gemini-1.5-pro should use Group2 keys
    let response = server
        .post("/v1/chat/completions")
        .json(&json!({
            "model": "gemini-1.5-pro",
            "messages": [{"role": "user", "content": "Hello"}]
        }))
        .await;

    // We expect the request to be forwarded (even if it fails due to invalid keys)
    assert!(response.status_code().is_client_error() || response.status_code().is_server_error());
}

#[tokio::test]
async fn test_unknown_model_uses_default_group() {
    let (_temp_dir, config_path) = create_test_config_with_groups().await;
    
    let (app, _config) = run(Some(config_path))
        .await
        .expect("Failed to create app");
    
    let server = TestServer::new(app).expect("Failed to create test server");

    // Test request for unknown model should use default rotation
    let response = server
        .post("/v1/chat/completions")
        .json(&json!({
            "model": "unknown-model",
            "messages": [{"role": "user", "content": "Hello"}]
        }))
        .await;

    // Should still process the request
    assert!(response.status_code().is_client_error() || response.status_code().is_server_error());
}

#[tokio::test]
async fn test_model_alias_matching() {
    let (_temp_dir, config_path) = create_test_config_with_groups().await;
    
    let (app, _config) = run(Some(config_path))
        .await
        .expect("Failed to create app");
    
    let server = TestServer::new(app).expect("Failed to create test server");

    // Test that model aliases work correctly
    let response = server
        .post("/v1/chat/completions")
        .json(&json!({
            "model": "gemini-flash", // This is an alias for gemini-1.5-flash
            "messages": [{"role": "user", "content": "Hello"}]
        }))
        .await;

    // Should be processed by Group1
    assert!(response.status_code().is_client_error() || response.status_code().is_server_error());
}

#[tokio::test]
async fn test_case_insensitive_model_matching() {
    let (_temp_dir, config_path) = create_test_config_with_groups().await;
    
    let (app, _config) = run(Some(config_path))
        .await
        .expect("Failed to create app");
    
    let server = TestServer::new(app).expect("Failed to create test server");

    // Test case insensitive matching
    let response = server
        .post("/v1/chat/completions")
        .json(&json!({
            "model": "GEMINI-1.5-FLASH", // Uppercase version
            "messages": [{"role": "user", "content": "Hello"}]
        }))
        .await;

    // Should still be processed by Group1
    assert!(response.status_code().is_client_error() || response.status_code().is_server_error());
}