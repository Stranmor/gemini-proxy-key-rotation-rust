//! Common test utilities and fixtures

use gemini_proxy::{config::AppConfig, error::Result, AppState};
use std::{path::PathBuf, sync::Arc};
use tempfile::TempDir;
use tokio::sync::broadcast;

/// Test configuration builder
pub struct TestConfigBuilder {
    config: AppConfig,
}

impl TestConfigBuilder {
    pub fn new() -> Self {
        Self {
            config: AppConfig::default(),
        }
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.config.server.port = port;
        self
    }

    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        if self.config.groups.is_empty() {
            self.config.groups.push(gemini_proxy::config::KeyGroup {
                name: "test".to_string(),
                api_keys: vec![],
                target_url: "https://generativelanguage.googleapis.com/v1beta/openai/".to_string(),
                proxy_url: None,
            });
        }
        self.config.groups[0].api_keys.push(key.into());
        self
    }

    pub fn with_redis_url(mut self, url: impl Into<String>) -> Self {
        self.config.redis_url = Some(url.into());
        self
    }

    pub fn build(self) -> AppConfig {
        self.config
    }
}

impl Default for TestConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Test application state builder
pub struct TestAppStateBuilder {
    config: AppConfig,
    temp_dir: Option<TempDir>,
}

impl TestAppStateBuilder {
    pub fn new() -> Self {
        Self {
            config: TestConfigBuilder::new()
                .with_api_key("test-key-1")
                .with_api_key("test-key-2")
                .build(),
            temp_dir: None,
        }
    }

    pub fn with_config(mut self, config: AppConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_temp_dir(mut self) -> Self {
        self.temp_dir = Some(TempDir::new().expect("Failed to create temp dir"));
        self
    }

    pub async fn build(self) -> Result<(Arc<AppState>, Option<TempDir>)> {
        let config_path = if let Some(ref temp_dir) = self.temp_dir {
            temp_dir.path().join("config.yaml")
        } else {
            PathBuf::from("test-config.yaml")
        };

        let (state, _rx) = AppState::new(&self.config, &config_path).await?;
        Ok((Arc::new(state), self.temp_dir))
    }
}

impl Default for TestAppStateBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Mock HTTP server for testing upstream services
pub struct MockServer {
    pub server: wiremock::MockServer,
}

impl MockServer {
    pub async fn start() -> Self {
        Self {
            server: wiremock::MockServer::start().await,
        }
    }

    pub fn uri(&self) -> String {
        self.server.uri()
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.uri(), path)
    }
}

/// Test utilities for Redis
pub mod redis {
    use super::*;

    pub fn get_test_redis_url() -> Option<String> {
        std::env::var("TEST_REDIS_URL").ok()
            .or_else(|| Some("redis://localhost:6379/15".to_string()))
    }

    pub async fn cleanup_test_keys(redis_url: &str, prefix: &str) -> Result<()> {
        use redis::AsyncCommands;

        let client = redis::Client::open(redis_url)?;
        let mut conn = client.get_async_connection().await?;

        let pattern = format!("{}*", prefix);
        let keys: Vec<String> = conn.keys(pattern).await?;

        if !keys.is_empty() {
            conn.del(keys).await?;
        }

        Ok(())
    }
}

/// Test utilities for HTTP requests
pub mod http {
    use axum::{body::Body, http::Request};
    use serde_json::Value;

    pub fn json_request(method: &str, uri: &str, body: Value) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    pub fn get_request(uri: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .unwrap()
    }

    pub fn post_request(uri: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }
}

/// Test assertions and utilities
pub mod assertions {
    use axum::{body::to_bytes, response::Response};
    use serde_json::Value;

    pub async fn assert_json_response(response: Response, expected_status: u16) -> Value {
        assert_eq!(response.status().as_u16(), expected_status);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body)
            .expect("Response body should be valid JSON");

        json
    }

    pub async fn assert_error_response(response: Response, expected_status: u16, error_type: &str) -> Value {
        let json = assert_json_response(response, expected_status).await;

        assert!(json.get("type").is_some(), "Error response should have 'type' field");
        assert_eq!(
            json["type"].as_str().unwrap(),
            error_type,
            "Error type mismatch"
        );

        json
    }

    pub async fn assert_success_response(response: Response) -> Value {
        assert!(response.status().is_success(), "Response should be successful");

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body)
            .expect("Response body should be valid JSON");

        json
    }
}

/// Environment setup for tests
pub fn setup_test_env() {
    std::env::set_var("RUST_LOG", "debug");
    let _ = tracing_subscriber::fmt::try_init();
}

/// Cleanup function for tests
pub fn cleanup_test_env() {
    // Clean up any test-specific environment variables
    std::env::remove_var("TEST_REDIS_URL");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_builder() {
        let config = TestConfigBuilder::new()
            .with_port(8080)
            .with_api_key("test-key")
            .build();

        assert_eq!(config.server.port, 8080);
        assert_eq!(config.groups.len(), 1);
        assert_eq!(config.groups[0].api_keys.len(), 1);
        assert_eq!(config.groups[0].api_keys[0], "test-key");
    }

    #[tokio::test]
    async fn test_app_state_builder() {
        let config = TestConfigBuilder::new()
            .with_api_key("test-key")
            .build();

        let result = TestAppStateBuilder::new()
            .with_config(config)
            .with_temp_dir()
            .build()
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_mock_server() {
        let mock_server = MockServer::start().await;
        let uri = mock_server.uri();

        assert!(uri.starts_with("http://"));
        assert!(uri.contains("127.0.0.1"));
    }
}