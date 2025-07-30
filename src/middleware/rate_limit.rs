// src/middleware/rate_limit.rs

use axum::{
    extract::{ConnectInfo, Request},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct RateLimitEntry {
    pub count: u32,
    pub window_start: Instant,
}

#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub max_requests: u32,
    pub window_duration: Duration,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests: 10, // 10 requests per window
            window_duration: Duration::from_secs(60), // 1 minute window
        }
    }
}

pub type RateLimitStore = Arc<RwLock<HashMap<String, RateLimitEntry>>>;

pub fn create_rate_limit_store() -> RateLimitStore {
    Arc::new(RwLock::new(HashMap::new()))
}

/// Rate limiting middleware for admin endpoints
pub async fn rate_limit_middleware(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let config = RateLimitConfig::default();
    
    // Use IP address as the key for rate limiting
    let client_key = addr.ip().to_string();
    
    // For now, we'll use a simple in-memory store
    // In production, this should be shared across instances via Redis
    static RATE_LIMIT_STORE: std::sync::OnceLock<RateLimitStore> = std::sync::OnceLock::new();
    let store = RATE_LIMIT_STORE.get_or_init(create_rate_limit_store);
    
    let now = Instant::now();
    let mut store_guard = store.write().await;
    
    let entry = store_guard.entry(client_key.clone()).or_insert(RateLimitEntry {
        count: 0,
        window_start: now,
    });
    
    // Reset window if expired
    if now.duration_since(entry.window_start) >= config.window_duration {
        entry.count = 0;
        entry.window_start = now;
    }
    
    // Check rate limit
    if entry.count >= config.max_requests {
        warn!(
            client_ip = %addr.ip(),
            count = entry.count,
            max_requests = config.max_requests,
            "Rate limit exceeded for admin endpoint"
        );
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    
    // Increment counter
    entry.count += 1;
    
    debug!(
        client_ip = %addr.ip(),
        count = entry.count,
        max_requests = config.max_requests,
        "Rate limit check passed"
    );
    
    drop(store_guard);
    
    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Method, Request},
        middleware,
        routing::get,
        Router,
    };
    
    use tower::ServiceExt;

    async fn dummy_handler() -> &'static str {
        "OK"
    }

    #[tokio::test]
    async fn test_rate_limit_allows_requests_within_limit() {
        let app = Router::new()
            .route("/test", get(dummy_handler))
            .layer(middleware::from_fn(rate_limit_middleware));

        // Make several requests within the limit
        for i in 1..=5 {
            let request = Request::builder()
                .method(Method::GET)
                .uri("/test")
                .body(Body::empty())
                .unwrap();

            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "Request {} should succeed", i);
        }
    }

    #[tokio::test]
    async fn test_rate_limit_blocks_excessive_requests() {
        let app = Router::new()
            .route("/test", get(dummy_handler))
            .layer(middleware::from_fn(rate_limit_middleware));

        // Make requests up to the limit
        for i in 1..=10 {
            let request = Request::builder()
                .method(Method::GET)
                .uri("/test")
                .body(Body::empty())
                .unwrap();

            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "Request {} should succeed", i);
        }

        // The next request should be rate limited
        let request = Request::builder()
            .method(Method::GET)
            .uri("/test")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}