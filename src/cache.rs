// src/cache.rs

use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Represents a cached response with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResponse {
    pub data: Vec<u8>,
    pub headers: HashMap<String, String>,
    pub status: u16,
    pub timestamp_millis: u128, // Unix timestamp for serialization
    pub ttl_millis: u128,
}

impl CachedResponse {
    /// # Panics
    ///
    /// This function will panic if the system time is before the UNIX epoch.
    #[must_use]
    pub fn new(data: Vec<u8>, headers: &HeaderMap, status: StatusCode, ttl: Duration) -> Self {
        let headers_map = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        Self {
            data,
            headers: headers_map,
            status: status.as_u16(),
            timestamp_millis: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
            ttl_millis: ttl.as_millis(),
        }
    }

    /// # Panics
    ///
    /// This function will panic if the system time is before the UNIX epoch.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        now > self.timestamp_millis + self.ttl_millis
    }

    #[must_use]
    pub fn to_header_map(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (k, v) in &self.headers {
            if let (Ok(name), Ok(value)) = (k.parse::<axum::http::HeaderName>(), v.parse::<axum::http::HeaderValue>()) {
                headers.insert(name, value);
            }
        }
        headers
    }
}

/// In-memory response cache with TTL support
#[derive(Debug)]
pub struct ResponseCache {
    cache: Arc<RwLock<HashMap<String, CachedResponse>>>,
    default_ttl: Duration,
    max_size: usize,
}

impl ResponseCache {
    #[must_use]
    pub fn new(default_ttl: Duration, max_size: usize) -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            default_ttl,
            max_size,
        }
    }

    /// Generate cache key from request components
    #[must_use]
    pub fn generate_key(&self, method: &str, path: &str, body: &[u8]) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        method.hash(&mut hasher);
        path.hash(&mut hasher);
        body.hash(&mut hasher);
        format!("{}:{:x}", method, hasher.finish())
    }

    /// Get cached response if exists and not expired
    pub async fn get(&self, key: &str) -> Option<CachedResponse> {
        let cache = self.cache.read().await;
        if let Some(cached) = cache.get(key) {
            if !cached.is_expired() {
                debug!(cache_key = %key, "Cache hit");
                let response = Some(cached.clone());
                drop(cache);
                return response;
            }
            debug!(cache_key = %key, "Cache entry expired");
        }
        None
    }

    /// Store response in cache
    pub async fn put(
        &self,
        key: String,
        data: Vec<u8>,
        headers: HeaderMap,
        status: StatusCode,
        ttl: Option<Duration>,
    ) {
        let ttl = ttl.unwrap_or(self.default_ttl);
        let cached = CachedResponse::new(data, &headers, status, ttl);

        let mut cache = self.cache.write().await;
        
        // Evict expired entries and enforce size limit
        if cache.len() >= self.max_size {
            self.evict_expired(&mut cache);
            
            // If still at capacity, remove oldest entries
            if cache.len() >= self.max_size {
                let oldest_key = cache
                    .iter()
                    .min_by_key(|(_, v)| v.timestamp_millis)
                    .map(|(k, _)| k.clone());
                
                if let Some(key_to_remove) = oldest_key {
                    cache.remove(&key_to_remove);
                    debug!(removed_key = %key_to_remove, "Evicted oldest cache entry");
                }
            }
        }

        cache.insert(key.clone(), cached);
        drop(cache);
        debug!(cache_key = %key, ttl_seconds = ttl.as_secs(), "Cached response");
    }

    /// Remove expired entries from cache
    #[allow(clippy::unused_self)]
    fn evict_expired(&self, cache: &mut HashMap<String, CachedResponse>) {
        let expired_keys: Vec<String> = cache
            .iter()
            .filter(|(_, v)| v.is_expired())
            .map(|(k, _)| k.clone())
            .collect();

        for key in expired_keys {
            cache.remove(&key);
            debug!(cache_key = %key, "Evicted expired cache entry");
        }
    }

    /// Get cache statistics
    pub async fn stats(&self) -> CacheStats {
        let cache = self.cache.read().await;
        let total_entries = cache.len();
        let expired_entries = cache.values().filter(|v| v.is_expired()).count();
        drop(cache);
        
        CacheStats {
            total_entries,
            expired_entries,
            active_entries: total_entries - expired_entries,
            max_size: self.max_size,
            default_ttl_millis: self.default_ttl.as_millis().try_into().unwrap_or(u64::MAX),
        }
    }

    /// Clear all cache entries
    pub async fn clear(&self) {
        let mut cache = self.cache.write().await;
        let count = cache.len();
        cache.clear();
        drop(cache);
        info!(cleared_entries = count, "Cache cleared");
    }

    /// Remove a specific entry from the cache
    pub async fn remove(&self, key: &str) -> bool {
        let mut cache = self.cache.write().await;
        cache.remove(key).is_some()
    }

    /// Determine if response should be cached based on status and headers
    #[must_use]
    pub fn should_cache(&self, status: StatusCode, headers: &HeaderMap) -> bool {
        // Don't cache error responses except for specific cases
        if status.is_server_error() || status.is_client_error() {
            return false;
        }

        // Don't cache if explicitly told not to
        if let Some(cache_control) = headers.get("cache-control") {
            if let Ok(value) = cache_control.to_str() {
                if value.contains("no-cache") || value.contains("no-store") {
                    return false;
                }
            }
        }

        // Cache successful responses
        status.is_success()
    }
}

#[derive(Debug, Serialize)]
pub struct CacheStats {
    pub total_entries: usize,
    pub expired_entries: usize,
    pub active_entries: usize,
    pub max_size: usize,
    pub default_ttl_millis: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, StatusCode};
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn test_cache_put_and_get() {
        let cache = ResponseCache::new(Duration::from_secs(60), 100);
        let key = "test_key".to_string();
        let data = b"test response".to_vec();
        let mut headers = HeaderMap::new();
        headers.insert("content-type", HeaderValue::from_static("application/json"));

        cache.put(key.clone(), data.clone(), headers, StatusCode::OK, None).await;
        
        let cached = cache.get(&key).await.unwrap();
        assert_eq!(cached.data, data);
        assert_eq!(cached.status, 200);
    }

    #[tokio::test]
    async fn test_cache_expiration() {
        let cache = ResponseCache::new(Duration::from_millis(100), 100);
        let key = "expire_test".to_string();
        let data = b"test".to_vec();
        let headers = HeaderMap::new();

        cache.put(key.clone(), data, headers, StatusCode::OK, Some(Duration::from_millis(50))).await;
        
        // Should be available immediately
        assert!(cache.get(&key).await.is_some());
        
        // Wait for expiration
        sleep(Duration::from_millis(60)).await;
        
        // Should be expired
        assert!(cache.get(&key).await.is_none());
    }

    #[tokio::test]
    async fn test_should_cache() {
        let cache = ResponseCache::new(Duration::from_secs(60), 100);
        let mut headers = HeaderMap::new();

        // Should cache successful responses
        assert!(cache.should_cache(StatusCode::OK, &headers));
        
        // Should not cache error responses
        assert!(!cache.should_cache(StatusCode::INTERNAL_SERVER_ERROR, &headers));
        assert!(!cache.should_cache(StatusCode::BAD_REQUEST, &headers));
        
        // Should not cache when explicitly told not to
        headers.insert("cache-control", HeaderValue::from_static("no-cache"));
        assert!(!cache.should_cache(StatusCode::OK, &headers));
    }
}