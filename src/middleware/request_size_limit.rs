// src/middleware/request_size_limit.rs

use axum::{
    body::Body,
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use tracing::warn;

const MAX_REQUEST_SIZE: usize = 10 * 1024 * 1024; // 10MB limit

/// Middleware to limit request body size to prevent DoS attacks
pub async fn request_size_limit_middleware(
    request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    // Only check Content-Length for methods that typically have bodies
    let method = request.method();
    if matches!(method, &axum::http::Method::POST | &axum::http::Method::PUT | &axum::http::Method::PATCH) {
        // Check Content-Length header if present
        if let Some(content_length) = request.headers().get("content-length") {
            if let Ok(length_str) = content_length.to_str() {
                if let Ok(length) = length_str.parse::<usize>() {
                    if length > MAX_REQUEST_SIZE {
                        warn!(
                            content_length = length,
                            max_size = MAX_REQUEST_SIZE,
                            method = %method,
                            "Request rejected: body size exceeds limit"
                        );
                        return Err(StatusCode::PAYLOAD_TOO_LARGE);
                    }
                }
            }
        }
    }

    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::Method,
        middleware::from_fn,
        routing::post,
        Router,
    };
    use tower::ServiceExt;

    async fn dummy_handler() -> &'static str {
        "OK"
    }

    #[tokio::test]
    async fn test_request_size_limit_allows_small_requests() {
        let app = Router::new()
            .route("/test", post(dummy_handler))
            .layer(from_fn(request_size_limit_middleware));

        let request = Request::builder()
            .method(Method::POST)
            .uri("/test")
            .header("content-length", "1000")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_request_size_limit_blocks_large_requests() {
        let app = Router::new()
            .route("/test", post(dummy_handler))
            .layer(from_fn(request_size_limit_middleware));

        let request = Request::builder()
            .method(Method::POST)
            .uri("/test")
            .header("content-length", "20971520") // 20MB
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}