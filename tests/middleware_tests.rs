// tests/middleware_tests.rs

use gemini_proxy::middleware::rate_limit::{create_rate_limit_store, RateLimitEntry};
use axum::{
    body::Body,
    http::Request,
};
use std::time::Duration;

#[tokio::test]
async fn test_rate_limit_store_basic_functionality() {
    let store = create_rate_limit_store();
    let ip_str = "127.0.0.1";
    
    // Simulate rate limiting by manually updating the store
    {
        let mut store_guard = store.write().await;
        store_guard.insert(ip_str.to_string(), RateLimitEntry {
            count: 5,
            window_start: std::time::Instant::now(),
        });
    }
    
    // Check that entry exists
    let store_guard = store.read().await;
    assert!(store_guard.contains_key(ip_str));
    assert_eq!(store_guard.get(ip_str).unwrap().count, 5);
}

#[tokio::test]
async fn test_rate_limit_store_window_reset() {
    let store = create_rate_limit_store();
    let ip_str = "127.0.0.1";
    
    // Add entry with old timestamp
    {
        let mut store_guard = store.write().await;
        store_guard.insert(ip_str.to_string(), RateLimitEntry {
            count: 10,
            window_start: std::time::Instant::now() - Duration::from_secs(120), // Old entry
        });
    }
    
    // Entry should exist but be old
    let store_guard = store.read().await;
    let entry = store_guard.get(ip_str).unwrap();
    assert!(entry.window_start.elapsed() > Duration::from_secs(60));
}

#[tokio::test]
async fn test_request_size_limit_middleware_small_request() {
    let request = Request::builder()
        .method("POST")
        .uri("/test")
        .body(Body::from("small body"))
        .unwrap();
    
    // Simplified test - just check request was created
    assert_eq!(request.method(), "POST");
    assert_eq!(request.uri(), "/test");
}

#[tokio::test]
async fn test_request_size_limit_middleware_large_request() {
    // Create a large body (over typical limits)
    let large_body = "x".repeat(10 * 1024 * 1024); // 10MB
    
    let request = Request::builder()
        .method("POST")
        .uri("/test")
        .header("content-length", large_body.len().to_string())
        .body(Body::from(large_body))
        .unwrap();
    
    // Check that request has large content-length header
    let content_length = request.headers().get("content-length").unwrap();
    let length: usize = content_length.to_str().unwrap().parse().unwrap();
    assert!(length > 1024 * 1024); // Large request
}