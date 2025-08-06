use gemini_proxy::{error::AppError, run};
use std::fs::File;
use std::io::Write;
use tempfile::tempdir;

#[tokio::test]
async fn test_run_successful_startup() {
    let temp_dir = tempdir().expect("Failed to create temp directory");
    let config_path = temp_dir.path().join("config.yaml");

    let mut temp_config = File::create(&config_path).expect("Failed to create temp config file");
    let config_content = r#"
redis_url: "redis://127.0.0.1:6379/1"
server:
  port: 8080
  admin_token: "test_token"
  test_mode: true
groups:
  - name: "default"
    target_url: "http://127.0.0.1:1" # Dummy URL, doesn't matter for this test
    api_keys: ["key1"]
"#;
    temp_config
        .write_all(config_content.as_bytes())
        .expect("Failed to write to temp config");

    // This test now verifies that `run` can successfully initialize the application
    // state and router when provided with a valid configuration.
    // It no longer tests the `main` function directly, as `main` is not part of the
    // library's public API and testing it requires fragile workarounds.
    let result = run(Some(config_path)).await;

    assert!(result.is_ok(), "run() should succeed with a valid config");
}

#[tokio::test]
async fn test_run_fails_with_invalid_config() {
    let temp_dir = tempdir().expect("Failed to create temp directory");
    let config_path = temp_dir.path().join("config.yaml");

    let mut temp_config = File::create(&config_path).expect("Failed to create temp config file");
    // Invalid structure (port is a string)
    let config_content = r#"
server:
  port: "not-a-number"
  admin_token: "test_token"
groups: []
"#;
    temp_config
        .write_all(config_content.as_bytes())
        .expect("Failed to write to temp config");

    let result = run(Some(config_path)).await;

    println!("Result: {:?}", result);
    assert!(matches!(result, Err(AppError::ConfigParse { .. }) | Err(AppError::ConfigValidation { .. })));
}

#[tokio::test]
async fn test_run_fails_without_config_file() {
    let temp_dir = tempdir().expect("Failed to create temp directory");

    // Set CONFIG_PATH to a non-existent file in our temp directory
    let config_path = temp_dir.path().join("non_existent_config.yaml");

    let result = run(Some(config_path)).await;

    // Expect a config error because the file is required if the path is set,
    // and default values are not sufficient to run.
    println!("Result: {:?}", result);
    assert!(matches!(result, Err(AppError::ConfigParse { .. }) | Err(AppError::ConfigValidation { .. })));
}
