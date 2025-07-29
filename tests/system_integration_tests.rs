// tests/system_integration_tests.rs

use axum::http::{Method, StatusCode};
use futures::future;
use gemini_proxy_key_rotation_rust::{
    config::{AppConfig, KeyGroup, ServerConfig},
    handler,
    state::AppState,
};
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
use std::{fs::File, sync::Arc};
use tempfile::tempdir;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

fn generate_prefix() -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(6)
        .map(char::from)
        .collect()
}

async fn create_test_system(
    internal_retries: u32,
) -> (Arc<AppState>, MockServer, tempfile::TempDir) {
    let server = MockServer::start().await;
    let temp_dir = tempdir().unwrap();
    let config_path = temp_dir.path().join("config.yaml");
    File::create(&config_path).unwrap();

    let test_group = KeyGroup {
        name: "test-group".to_string(),
        api_keys: vec!["test-key-1".to_string(), "test-key-2".to_string()],
        model_aliases: vec![],
        target_url: server.uri(),
        proxy_url: None,
        top_p: None,
    };

   let config = AppConfig {
       server: ServerConfig {
           port: 8080,
           top_p: None,
           admin_token: Some("test_token".to_string()),
           test_mode: true,
       },
       groups: vec![test_group],
       redis_url: "redis://127.0.0.1:6379/1".to_string(), // Use a single DB for system tests
       redis_key_prefix: Some(format!("test:{}", generate_prefix())),
       internal_retries,
       temporary_block_minutes: 1,
       top_p: None,
   };

    let app_state = Arc::new(AppState::new(&config, &config_path).await.unwrap());

    (app_state, server, temp_dir)
}

#[tokio::test]
async fn test_metrics_collection() {
    let (app_state, server, _temp_dir) = create_test_system(3).await;

    // Mock a successful request
    Mock::given(method("GET"))
        .and(path("/v1beta/openai/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": []})))
        .mount(&server)
        .await;

    // Make a request to generate metrics
    let request = axum::extract::Request::builder()
        .method(Method::GET)
        .uri("/v1/models")
        .body(axum::body::Body::empty())
        .unwrap();

    let _response = handler::proxy_handler(axum::extract::State(app_state.clone()), request)
        .await
        .unwrap();

    // Metrics should be recorded (this is more of a smoke test)
    // In a real scenario, you'd check the actual metrics values
    // but that requires more complex setup with the metrics registry
}
#[tokio::test]
async fn test_error_handling_and_recovery() {
    let (app_state, server, _temp_dir) = create_test_system(0).await; // Disable internal retries

    // Mock server error followed by success
    Mock::given(method("GET"))
        .and(path("/v1beta/openai/models"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .expect(1) // Should only be called once, as we fail fast on server errors
        .mount(&server)
        .await;

    let request = axum::extract::Request::builder()
        .method(Method::GET)
        .uri("/v1/models")
        .body(axum::body::Body::empty())
        .unwrap();

    let response = handler::proxy_handler(axum::extract::State(app_state.clone()), request)
        .await
        .unwrap();

    // Should return the error response
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn test_concurrent_requests() {
    let (app_state, server, _temp_dir) = create_test_system(3).await;

    // Mock responses for concurrent requests
    Mock::given(method("GET"))
        .and(path("/v1beta/openai/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": []})))
        .expect(10)
        .mount(&server)
        .await;

    // Create multiple concurrent requests
    let mut handles = Vec::new();

    for i in 0..10 {
        let app_state_clone = app_state.clone();
        let handle = tokio::spawn(async move {
            let request = axum::extract::Request::builder()
                .method(Method::GET)
                .uri(format!("/v1/models?req={i}"))
                .body(axum::body::Body::empty())
                .unwrap();

            handler::proxy_handler(axum::extract::State(app_state_clone), request).await
        });
        handles.push(handle);
    }

    // Wait for all requests to complete
    let results = future::join_all(handles).await;

    // All requests should succeed
    for result in results {
        let response = result.unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
