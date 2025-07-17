// tests/system_integration_tests.rs

use axum::http::{Method, StatusCode};
use gemini_proxy_key_rotation_rust::{
    cache::ResponseCache,
    config::{AppConfig, KeyGroup, ServerConfig},
    handler,
    state::AppState,
};
use std::{fs::File, sync::Arc, time::Duration};
use tempfile::tempdir;
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};

async fn create_test_system() -> (Arc<AppState>, MockServer, tempfile::TempDir) {
    let server = MockServer::start().await;
    let temp_dir = tempdir().unwrap();
    let config_path = temp_dir.path().join("config.yaml");
    File::create(&config_path).unwrap();
    
    let test_group = KeyGroup {
        name: "test-group".to_string(),
        api_keys: vec!["test-key-1".to_string(), "test-key-2".to_string()],
        target_url: server.uri(),
        proxy_url: None,
    };
    
    let config = AppConfig {
        server: ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 8080,
        },
        groups: vec![test_group],
    };
    
    let cache = Arc::new(ResponseCache::new(Duration::from_secs(300), 100));
    let app_state = Arc::new(AppState::new(&config, &config_path, cache).await.unwrap());
    
    (app_state, server, temp_dir)
}

#[tokio::test]
async fn test_full_system_with_caching() {
    let (app_state, server, _temp_dir) = create_test_system().await;
    
    // Mock successful response
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("x-goog-api-key", "test-key-1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({
                    "object": "list",
                    "data": [{"id": "gemini-1.5-flash", "object": "model"}]
                }))
                .insert_header("content-type", "application/json")
                .insert_header("cache-control", "public, max-age=300")
        )
        .expect(1) // Should only be called once due to caching
        .mount(&server)
        .await;
    
    // First request - should hit the API
    let request1 = axum::extract::Request::builder()
        .method(Method::GET)
        .uri("http://test.com/v1/models")
        .body(axum::body::Body::empty())
        .unwrap();
    
    let response1 = handler::proxy_handler(
        axum::extract::State(app_state.clone()),
        request1,
    ).await.unwrap();
    
    assert_eq!(response1.status(), StatusCode::OK);
    
    // Second request - should hit the cache
    let request2 = axum::extract::Request::builder()
        .method(Method::GET)
        .uri("http://test.com/v1/models")
        .body(axum::body::Body::empty())
        .unwrap();
    
    let response2 = handler::proxy_handler(
        axum::extract::State(app_state.clone()),
        request2,
    ).await.unwrap();
    
    assert_eq!(response2.status(), StatusCode::OK);
    
    // Verify cache statistics
    let cache_stats = app_state.cache.stats().await;
    assert_eq!(cache_stats.total_entries, 1);
    assert_eq!(cache_stats.active_entries, 1);
}

#[tokio::test]
async fn test_key_rotation_with_rate_limiting() {
    let (app_state, server, _temp_dir) = create_test_system().await;
    
    // First key returns 429, second key succeeds
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("x-goog-api-key", "test-key-1"))
        .respond_with(ResponseTemplate::new(429).set_body_string("Rate limit exceeded"))
        .expect(1)
        .mount(&server)
        .await;
    
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("x-goog-api-key", "test-key-2"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({
                    "id": "chatcmpl-123",
                    "object": "chat.completion",
                    "choices": [{"message": {"role": "assistant", "content": "Hello!"}}]
                }))
        )
        .expect(1)
        .mount(&server)
        .await;
    
    let request = axum::extract::Request::builder()
        .method(Method::POST)
        .uri("http://test.com/v1/chat/completions")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(serde_json::json!({
            "model": "gemini-1.5-flash",
            "messages": [{"role": "user", "content": "Hello"}]
        }).to_string()))
        .unwrap();
    
    let response = handler::proxy_handler(
        axum::extract::State(app_state.clone()),
        request,
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    
    // Verify first key is marked as limited
    let key_states = app_state.key_manager.key_states.read().await;
    let key1_state = key_states.get("test-key-1").unwrap();
    assert!(key1_state.is_limited);
}

#[tokio::test]
async fn test_admin_endpoints() {
    let (app_state, _server, _temp_dir) = create_test_system().await;
    
    // Test detailed health endpoint
    let health_response = gemini_proxy_key_rotation_rust::admin::detailed_health(
        axum::extract::State(app_state.clone())
    ).await.unwrap();
    
    let health_data = health_response.0;
    assert_eq!(health_data.status, "healthy");
    assert_eq!(health_data.key_status.total_keys, 2);
    assert_eq!(health_data.key_status.active_keys, 2);
    
    // Test cache stats endpoint
    let cache_response = gemini_proxy_key_rotation_rust::admin::get_cache_stats(
        axum::extract::State(app_state.clone())
    ).await.unwrap();
    
    let cache_data = cache_response.0;
    assert_eq!(cache_data.total_entries, 0);
    assert_eq!(cache_data.max_size, 100);
    
    // Test keys list endpoint
    let keys_response = gemini_proxy_key_rotation_rust::admin::list_keys(
        axum::extract::State(app_state.clone()),
        axum::extract::Query(gemini_proxy_key_rotation_rust::admin::ListKeysQuery {
            group: None,
            status: None,
        })
    ).await.unwrap();
    
    let keys_data = keys_response.0;
    assert_eq!(keys_data.len(), 2);
    assert!(keys_data.iter().all(|k| k.status == "active"));
}

#[tokio::test]
async fn test_metrics_collection() {
    let (app_state, server, _temp_dir) = create_test_system().await;
    
    // Mock a successful request
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": []})))
        .mount(&server)
        .await;
    
    // Make a request to generate metrics
    let request = axum::extract::Request::builder()
        .method(Method::GET)
        .uri("http://test.com/v1/models")
        .body(axum::body::Body::empty())
        .unwrap();
    
    let _response = handler::proxy_handler(
        axum::extract::State(app_state.clone()),
        request,
    ).await.unwrap();
    
    // Metrics should be recorded (this is more of a smoke test)
    // In a real scenario, you'd check the actual metrics values
    // but that requires more complex setup with the metrics registry
}

#[tokio::test]
async fn test_error_handling_and_recovery() {
    let (app_state, server, _temp_dir) = create_test_system().await;
    
    // Mock server error followed by success
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("x-goog-api-key", "test-key-1"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .expect(1)
        .mount(&server)
        .await;
    
    let request = axum::extract::Request::builder()
        .method(Method::GET)
        .uri("http://test.com/v1/models")
        .body(axum::body::Body::empty())
        .unwrap();
    
    let response = handler::proxy_handler(
        axum::extract::State(app_state.clone()),
        request,
    ).await.unwrap();
    
    // Should return the error response
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn test_concurrent_requests() {
    let (app_state, server, _temp_dir) = create_test_system().await;
    
    // Mock responses for concurrent requests
    Mock::given(method("GET"))
        .and(path("/v1/models"))
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
                .uri(format!("http://test.com/v1/models?req={}", i))
                .body(axum::body::Body::empty())
                .unwrap();
            
            handler::proxy_handler(
                axum::extract::State(app_state_clone),
                request,
            ).await
        });
        handles.push(handle);
    }
    
    // Wait for all requests to complete
    let results = futures::future::join_all(handles).await;
    
    // All requests should succeed
    for result in results {
        let response = result.unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn test_cache_eviction() {
    let (app_state, server, _temp_dir) = create_test_system().await;

    // 1. Populate the cache
    Mock::given(method("GET"))
        .and(path("/v1/models/cached-model"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"id": "cached-model"}))
                .insert_header("cache-control", "public, max-age=300"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let request = axum::extract::Request::builder()
        .method(Method::GET)
        .uri("http://test.com/v1/models/cached-model")
        .body(axum::body::Body::empty())
        .unwrap();
    
    let cache_key = app_state.cache.generate_key("GET", "/v1/models/cached-model", &[]);

    let response = handler::proxy_handler(
        axum::extract::State(app_state.clone()),
        request,
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // 2. Verify cache entry exists
    assert!(app_state.cache.get(&cache_key).await.is_some());
    let stats = app_state.cache.stats().await;
    assert_eq!(stats.total_entries, 1);

    // 3. Evict the cache entry
    let evict_response = gemini_proxy_key_rotation_rust::admin::evict_cache_entry(
        axum::extract::State(app_state.clone()),
        axum::extract::Path(cache_key.clone()),
    ).await.unwrap();
    assert_eq!(evict_response, StatusCode::NO_CONTENT);

    // 4. Verify cache entry is gone
    assert!(app_state.cache.get(&cache_key).await.is_none());
    let stats_after_evict = app_state.cache.stats().await;
    assert_eq!(stats_after_evict.total_entries, 0);

    // 5. Try to evict again, should result in 404
    let evict_again_response = gemini_proxy_key_rotation_rust::admin::evict_cache_entry(
        axum::extract::State(app_state.clone()),
        axum::extract::Path(cache_key),
    ).await.unwrap();
    assert_eq!(evict_again_response, StatusCode::NOT_FOUND);
}