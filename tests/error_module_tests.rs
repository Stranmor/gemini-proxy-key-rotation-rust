// tests/error_module_tests.rs

use axum::{
    http::StatusCode,
    response::IntoResponse,
};
use gemini_proxy::error::{AppError, ErrorContext};

#[test]
fn test_error_context_creation() {
    let context = ErrorContext::new("test_operation")
        .with_metadata("key1", "value1")
        .with_metadata("key2", "value2");
    
    assert_eq!(context.operation, "test_operation");
    assert_eq!(context.metadata.get("key1"), Some(&"value1".to_string()));
    assert_eq!(context.metadata.get("key2"), Some(&"value2".to_string()));
}

#[test]
fn test_error_context_with_request_id() {
    let request_id = "test-request-id";
    let context = ErrorContext::new("test_operation")
        .with_request_id(request_id);
    
    assert_eq!(context.request_id, request_id);
}

#[test]
fn test_error_context_with_metadata() {
    let context = ErrorContext::new("macro_test")
        .with_metadata("test_key", "test_value");
    
    assert_eq!(context.operation, "macro_test");
    assert_eq!(context.metadata.get("test_key"), Some(&"test_value".to_string()));
}

#[test]
fn test_app_error_authentication() {
    let error = AppError::Authentication {
        message: "Invalid credentials".to_string(),
    };
    
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[test]
fn test_app_error_rate_limit() {
    let error = AppError::RateLimit {
        limit: 100,
        window: "1 minute".to_string(),
    };
    
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[test]
fn test_app_error_validation() {
    let error = AppError::Validation {
        field: "email".to_string(),
        message: "Invalid email format".to_string(),
    };
    
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn test_app_error_invalid_api_key() {
    let error = AppError::InvalidApiKey {
        key_id: "test-key-123".to_string(),
    };
    
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[test]
fn test_app_error_internal() {
    let error = AppError::Internal {
        message: "Database connection failed".to_string(),
    };
    
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn test_app_error_upstream_unavailable() {
    let error = AppError::UpstreamUnavailable {
        service: "gemini-api".to_string(),
    };
    
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
}

#[test]
fn test_app_error_request_too_large() {
    let error = AppError::RequestTooLarge {
        size: 1000000,
        max_size: 500000,
    };
    
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn test_app_error_key_health_check() {
    let error = AppError::KeyHealthCheck {
        key_id: "key-123".to_string(),
        message: "Key is unhealthy".to_string(),
    };
    
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[test]
fn test_app_error_no_healthy_keys() {
    let error = AppError::NoHealthyKeys;
    
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[test]
fn test_app_error_invalid_request() {
    let error = AppError::InvalidRequest {
        message: "Missing required field".to_string(),
    };
    
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn test_app_error_tokenizer_init() {
    let error = AppError::TokenizerInit {
        message: "Failed to initialize tokenizer".to_string(),
    };
    
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn test_app_error_config_validation() {
    let error = AppError::config_validation("Invalid port number", Some("port"));
    
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn test_app_error_internal_helper() {
    let error = AppError::internal("Database connection failed");
    
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn test_app_error_validation_helper() {
    let error = AppError::validation("email", "Invalid email format");
    
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn test_app_error_display() {
    let error = AppError::Authentication {
        message: "Invalid token".to_string(),
    };
    
    let display_string = format!("{}", error);
    assert!(display_string.contains("Authentication failed"));
    assert!(display_string.contains("Invalid token"));
}

#[test]
fn test_app_error_debug() {
    let error = AppError::Internal {
        message: "Database error".to_string(),
    };
    
    let debug_string = format!("{:?}", error);
    assert!(debug_string.contains("Internal"));
    assert!(debug_string.contains("Database error"));
}

#[test]
fn test_app_error_from_std_error() {
    let std_error = std::io::Error::new(std::io::ErrorKind::Other, "IO error");
    let app_error: AppError = std_error.into();
    
    // Проверяем, что ошибка преобразована
    let error_string = format!("{}", app_error);
    assert!(error_string.contains("IO error"));
}

#[test]
fn test_app_error_status_codes() {
    // Тестируем различные статус коды
    let auth_error = AppError::Authentication { message: "Invalid token".to_string() };
    assert_eq!(auth_error.status_code(), StatusCode::FORBIDDEN);
    
    let validation_error = AppError::Validation { 
        field: "email".to_string(), 
        message: "Required field".to_string() 
    };
    assert_eq!(validation_error.status_code(), StatusCode::BAD_REQUEST);
    
    let internal_error = AppError::Internal { message: "Server error".to_string() };
    assert_eq!(internal_error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}