// tests/handlers_tests.rs

use crate::{
    handlers::{
        base::ResponseHandler,
        success::SuccessHandler,
        rate_limit::RateLimitHandler,
        terminal_error::TerminalErrorHandler,
        invalid_api_key::InvalidApiKeyHandler,
        timeout::TimeoutHandler,
    },
    error::AppError,
};
use axum::{body::{Body, Bytes}, response::Response};
use axum::http::StatusCode;
use reqwest::Response as ReqwestResponse;

// Helper to create mock response
fn create_mock_response(status: StatusCode) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(Body::from("test body"))
        .unwrap()
}

#[tokio::test]
async fn test_success_handler() {
    let handler = SuccessHandler;
    let body_bytes = Bytes::from("test response");
    
    // Test with success response
    let success_response = create_mock_response(StatusCode::OK);
    let result = handler.handle(&success_response, &body_bytes, "test-key");
    assert!(result.is_some());
    
    // Test with error response
    let error_response = create_mock_response(StatusCode::BAD_REQUEST);
    let result = handler.handle(&error_response, &body_bytes, "test-key");
    assert!(result.is_none());
}

#[tokio::test]
async fn test_rate_limit_handler() {
    let handler = RateLimitHandler;
    let body_bytes = Bytes::from("rate limit exceeded");
    
    // Test with 429 response
    let rate_limit_response = create_mock_response(StatusCode::TOO_MANY_REQUESTS);
    let result = handler.handle(&rate_limit_response, &body_bytes, "test-key");
    assert!(result.is_some());
    
    // Test with non-429 response
    let ok_response = create_mock_response(StatusCode::OK);
    let result = handler.handle(&ok_response, &body_bytes, "test-key");
    assert!(result.is_none());
}

#[tokio::test]
async fn test_terminal_error_handler() {
    let handler = TerminalErrorHandler;
    let body_bytes = Bytes::from("bad request");
    
    // Test with terminal error (500) - terminal handler handles server errors
    let server_error_response = create_mock_response(StatusCode::INTERNAL_SERVER_ERROR);
    let result = handler.handle(&server_error_response, &body_bytes, "test-key");
    assert!(result.is_some());
    
    // Test with non-terminal error (400) - excluded by terminal handler
    let bad_request_response = create_mock_response(StatusCode::BAD_REQUEST);
    let result = handler.handle(&bad_request_response, &body_bytes, "test-key");
    assert!(result.is_none());
}

#[tokio::test]
async fn test_invalid_api_key_handler() {
    let handler = InvalidApiKeyHandler;
    let body_bytes = Bytes::from("forbidden");
    
    // Test with 400 response containing API_KEY_INVALID
    let bad_request_response = create_mock_response(StatusCode::BAD_REQUEST);
    let body_bytes = Bytes::from("API_KEY_INVALID error");
    let result = handler.handle(&bad_request_response, &body_bytes, "test-key");
    assert!(result.is_some());
    
    // Test with 400 response without API_KEY_INVALID
    let bad_request_response = create_mock_response(StatusCode::BAD_REQUEST);
    let body_bytes = Bytes::from("other error");
    let result = handler.handle(&bad_request_response, &body_bytes, "test-key");
    assert!(result.is_none());
}

#[tokio::test]
async fn test_timeout_handler() {
    let handler = TimeoutHandler;
    let body_bytes = Bytes::from("timeout");
    
    // Test with timeout response
    let timeout_response = create_mock_response(StatusCode::GATEWAY_TIMEOUT);
    let result = handler.handle(&timeout_response, &body_bytes, "test-key");
    assert!(result.is_some());
    
    // Test with non-timeout response
    let ok_response = create_mock_response(StatusCode::OK);
    let result = handler.handle(&ok_response, &body_bytes, "test-key");
    assert!(result.is_none());
}

#[test]
fn test_handler_response_types() {
    // Test that different handlers handle different response types
    let success = SuccessHandler;
    let rate_limit = RateLimitHandler;
    let terminal = TerminalErrorHandler;
    let invalid_key = InvalidApiKeyHandler;
    let timeout = TimeoutHandler;
    let body_bytes = Bytes::from("test");
    
    // Success handler should handle 2xx
    let ok_response = create_mock_response(StatusCode::OK);
    assert!(success.handle(&ok_response, &body_bytes, "key").is_some());
    
    // Rate limit handler should handle 429
    let rate_limit_response = create_mock_response(StatusCode::TOO_MANY_REQUESTS);
    assert!(rate_limit.handle(&rate_limit_response, &body_bytes, "key").is_some());
    
    // Terminal handler should handle 5xx errors
    let server_error_response = create_mock_response(StatusCode::INTERNAL_SERVER_ERROR);
    assert!(terminal.handle(&server_error_response, &body_bytes, "key").is_some());
    
    // Invalid key handler should handle 400 with API_KEY_INVALID
    let bad_request_response = create_mock_response(StatusCode::BAD_REQUEST);
    let api_key_invalid_bytes = Bytes::from("API_KEY_INVALID error");
    assert!(invalid_key.handle(&bad_request_response, &api_key_invalid_bytes, "key").is_some());
    
    // Timeout handler should handle timeout codes
    let timeout_response = create_mock_response(StatusCode::GATEWAY_TIMEOUT);
    assert!(timeout.handle(&timeout_response, &body_bytes, "key").is_some());
}