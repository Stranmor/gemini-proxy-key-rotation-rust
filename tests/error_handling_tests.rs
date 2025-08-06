// tests/error_handling_tests.rs

use crate::error::{AppError, Result};
use axum::{
    http::StatusCode,
    response::IntoResponse,
    body::to_bytes,
};
use serde_json::Value;

#[tokio::test]
async fn test_security_violation_error() {
    let error = AppError::Authentication {
        message: "Unauthorized access attempt".to_string(),
    };
    let response = error.into_response();
    
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    
    assert_eq!(json["error"]["type"], "SECURITY_VIOLATION");
    assert_eq!(json["error"]["message"], "Access denied due to security policy");
}

#[tokio::test]
async fn test_rate_limit_exceeded_error() {
    let error = AppError::RateLimit {
        limit: 5,
        window: "5 minutes".to_string(),
    };
    let response = error.into_response();
    
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    
    assert_eq!(json["error_type"], "https://gemini-proxy.dev/errors/rate-limit");
    assert_eq!(json["title"], "Rate Limit Exceeded");
}

#[tokio::test]
async fn test_key_health_check_failed_error() {
    let error = AppError::KeyHealthCheck {
        key_id: "all".to_string(),
        message: "All keys unhealthy".to_string(),
    };
    let response = error.into_response();
    
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    
    assert_eq!(json["error_type"], "https://gemini-proxy.dev/errors/key-management");
    assert_eq!(json["title"], "Key Management Error");
}