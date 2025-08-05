// tests/error_handling_tests.rs

use gemini_proxy_key_rotation_rust::error::{AppError, Result};
use axum::{
    http::StatusCode,
    response::IntoResponse,
    body::to_bytes,
};
use serde_json::Value;

#[tokio::test]
async fn test_security_violation_error() {
    let error = AppError::SecurityViolation("Unauthorized access attempt".to_string());
    let response = error.into_response();
    
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    
    assert_eq!(json["error"]["type"], "SECURITY_VIOLATION");
    assert_eq!(json["error"]["message"], "Access denied due to security policy");
}

#[tokio::test]
async fn test_rate_limit_exceeded_error() {
    let error = AppError::RateLimitExceeded {
        resource: "admin_panel".to_string(),
        details: "5 attempts in 5 minutes".to_string(),
    };
    let response = error.into_response();
    
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    
    assert_eq!(json["error"]["type"], "RATE_LIMIT_EXCEEDED");
    assert!(json["error"]["message"].as_str().unwrap().contains("admin_panel"));
    assert_eq!(json["error"]["details"], "Please try again later");
}

#[tokio::test]
async fn test_key_health_check_failed_error() {
    let error = AppError::KeyHealthCheckFailed("All keys unhealthy".to_string());
    let response = error.into_response();
    
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    
    assert_eq!(json["error"]["type"], "KEY_HEALTH_CHECK_FAILED");
    assert!(json["error"]["message"].as_str().unwrap().contains("key health issues"));
}