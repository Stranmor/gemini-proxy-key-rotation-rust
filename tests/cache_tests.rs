// tests/cache_tests.rs

use gemini_proxy_key_rotation_rust::cache::{ResponseCache, CachedResponse};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_cache_basic_operations() {
    let cache = ResponseCache::new(Duration::from_secs(60), 100);
    
    // Test cache miss
    assert!(cache.get("nonexistent").await.is_none());
    
    // Test cache put and get
    let data = b"test response data".to_vec();
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    
    cache.put(
        "test_key".to_string(),
        data.clone(),
        headers.clone(),
        StatusCode::OK,
        None,
    ).await;
    
    let cached = cache.get("test_key").await.unwrap();
    assert_eq!(cached.data, data);
    assert_eq!(cached.status, 200);
    assert!(cached.headers.contains_key("content-type"));
}

#[tokio::test]
async fn test_cache_expiration() {
    let cache = ResponseCache::new(Duration::from_millis(100), 100);
    
    let data = b"test data".to_vec();
    let headers = HeaderMap::new();
    
    // Put with short TTL
    cache.put(
        "expire_test".to_string(),
        data,
        headers,
        StatusCode::OK,
        Some(Duration::from_millis(50)),
    ).await;
    
    // Should be available immediately
    assert!(cache.get("expire_test").await.is_some());
    
    // Wait for expiration
    sleep(Duration::from_millis(60)).await;
    
    // Should be expired
    assert!(cache.get("expire_test").await.is_none());
}

#[tokio::test]
async fn test_cache_should_cache_logic() {
    let cache = ResponseCache::new(Duration::from_secs(60), 100);
    let mut headers = HeaderMap::new();
    
    // Should cache successful responses
    assert!(cache.should_cache(StatusCode::OK, &headers));
    assert!(cache.should_cache(StatusCode::CREATED, &headers));
    
    // Should not cache error responses
    assert!(!cache.should_cache(StatusCode::INTERNAL_SERVER_ERROR, &headers));
    assert!(!cache.should_cache(StatusCode::BAD_REQUEST, &headers));
    assert!(!cache.should_cache(StatusCode::NOT_FOUND, &headers));
    
    // Should not cache when explicitly told not to
    headers.insert("cache-control", HeaderValue::from_static("no-cache"));
    assert!(!cache.should_cache(StatusCode::OK, &headers));
    
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    assert!(!cache.should_cache(StatusCode::OK, &headers));
}

#[tokio::test]
async fn test_cache_size_limit() {
    let cache = ResponseCache::new(Duration::from_secs(60), 2); // Small cache
    
    let data = b"test".to_vec();
    let headers = HeaderMap::new();
    
    // Fill cache to capacity
    cache.put("key1".to_string(), data.clone(), headers.clone(), StatusCode::OK, None).await;
    sleep(Duration::from_millis(5)).await; // Ensure key2 is newer
    cache.put("key2".to_string(), data.clone(), headers.clone(), StatusCode::OK, None).await;
    
    // Both should be present
    assert!(cache.get("key1").await.is_some());
    assert!(cache.get("key2").await.is_some());
    
    // Add third item, should evict oldest (key1)
    sleep(Duration::from_millis(5)).await; // Ensure key3 is newer
    cache.put("key3".to_string(), data, headers, StatusCode::OK, None).await;
    
    // key1 should be evicted, key2 and key3 should remain
    assert!(cache.get("key1").await.is_none());
    assert!(cache.get("key2").await.is_some());
    assert!(cache.get("key3").await.is_some());
}

#[tokio::test]
async fn test_cache_stats() {
    let cache = ResponseCache::new(Duration::from_secs(60), 100);
    
    let initial_stats = cache.stats().await;
    assert_eq!(initial_stats.total_entries, 0);
    assert_eq!(initial_stats.active_entries, 0);
    
    // Add some entries
    let data = b"test".to_vec();
    let headers = HeaderMap::new();
    
    cache.put("key1".to_string(), data.clone(), headers.clone(), StatusCode::OK, None).await;
    cache.put("key2".to_string(), data, headers, StatusCode::OK, Some(Duration::from_millis(1))).await;
    
    // Wait for one to expire
    sleep(Duration::from_millis(10)).await;
    
    let stats = cache.stats().await;
    assert_eq!(stats.total_entries, 2);
    assert_eq!(stats.active_entries, 1);
    assert_eq!(stats.expired_entries, 1);
}

#[tokio::test]
async fn test_cache_clear() {
    let cache = ResponseCache::new(Duration::from_secs(60), 100);
    
    let data = b"test".to_vec();
    let headers = HeaderMap::new();
    
    // Add some entries
    cache.put("key1".to_string(), data.clone(), headers.clone(), StatusCode::OK, None).await;
    cache.put("key2".to_string(), data, headers, StatusCode::OK, None).await;
    
    let stats_before = cache.stats().await;
    assert_eq!(stats_before.total_entries, 2);
    
    // Clear cache
    cache.clear().await;
    
    let stats_after = cache.stats().await;
    assert_eq!(stats_after.total_entries, 0);
    
    // Entries should be gone
    assert!(cache.get("key1").await.is_none());
    assert!(cache.get("key2").await.is_none());
}

#[test]
fn test_cached_response_serialization() {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    headers.insert("x-custom", HeaderValue::from_static("test-value"));
    
    let cached = CachedResponse::new(
        b"test data".to_vec(),
        &headers,
        StatusCode::OK,
        Duration::from_secs(300),
    );
    
    // Test serialization
    let json = serde_json::to_string(&cached).unwrap();
    let deserialized: CachedResponse = serde_json::from_str(&json).unwrap();
    
    assert_eq!(cached.data, deserialized.data);
    assert_eq!(cached.status, deserialized.status);
    assert_eq!(cached.ttl_millis, deserialized.ttl_millis);
    
    // Test header map conversion
    let header_map = cached.to_header_map();
    assert!(header_map.contains_key("content-type"));
    assert!(header_map.contains_key("x-custom"));
}