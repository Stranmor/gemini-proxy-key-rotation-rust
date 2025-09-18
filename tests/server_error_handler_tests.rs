// tests/server_error_handler_tests.rs

use axum::{body::Bytes, http::StatusCode, response::Response};
use gemini_proxy::handlers::{
    base::{Action, ResponseHandler},
    server_error::ServerErrorHandler,
};

#[test]
fn test_server_error_handler_handles_gemini_500_error() {
    let handler = ServerErrorHandler;

    // Simulate the exact error from the log
    let gemini_error_body = r#"{"error": {"code": 500,"message": "An internal error has occurred. Please retry or report in https://developers.generativeai.google/guide/troubleshooting","status": "INTERNAL"}}"#;

    let response = Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .body(axum::body::Body::empty())
        .unwrap();

    let body_bytes = Bytes::from(gemini_error_body);

    let action = handler.handle(&response, &body_bytes, "test_key");

    // Should return RetryNextKey to switch to the next API key
    assert!(matches!(action, Some(Action::RetryNextKey)));
}

#[test]
fn test_server_error_handler_handles_all_server_errors() {
    let handler = ServerErrorHandler;

    let test_cases = vec![
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "500 Internal Server Error",
        ),
        (StatusCode::BAD_GATEWAY, "502 Bad Gateway"),
        (StatusCode::SERVICE_UNAVAILABLE, "503 Service Unavailable"),
        (StatusCode::GATEWAY_TIMEOUT, "504 Gateway Timeout"),
    ];

    for (status_code, error_message) in test_cases {
        let response = Response::builder()
            .status(status_code)
            .body(axum::body::Body::empty())
            .unwrap();

        let body_bytes = Bytes::from(error_message);

        let action = handler.handle(&response, &body_bytes, "test_key");

        assert!(
            matches!(action, Some(Action::RetryNextKey)),
            "Status code {} should trigger RetryNextKey",
            status_code.as_u16()
        );
    }
}

#[test]
fn test_server_error_handler_ignores_client_errors() {
    let handler = ServerErrorHandler;

    let test_cases = vec![
        (StatusCode::BAD_REQUEST, "400 Bad Request"),
        (StatusCode::UNAUTHORIZED, "401 Unauthorized"),
        (StatusCode::FORBIDDEN, "403 Forbidden"),
        (StatusCode::NOT_FOUND, "404 Not Found"),
        (StatusCode::TOO_MANY_REQUESTS, "429 Too Many Requests"),
    ];

    for (status_code, error_message) in test_cases {
        let response = Response::builder()
            .status(status_code)
            .body(axum::body::Body::empty())
            .unwrap();

        let body_bytes = Bytes::from(error_message);

        let action = handler.handle(&response, &body_bytes, "test_key");

        assert!(
            action.is_none(),
            "Status code {} should not be handled by ServerErrorHandler",
            status_code.as_u16()
        );
    }
}

#[test]
fn test_server_error_handler_ignores_success_responses() {
    let handler = ServerErrorHandler;

    let test_cases = vec![
        (StatusCode::OK, "200 OK"),
        (StatusCode::CREATED, "201 Created"),
        (StatusCode::ACCEPTED, "202 Accepted"),
    ];

    for (status_code, response_body) in test_cases {
        let response = Response::builder()
            .status(status_code)
            .body(axum::body::Body::empty())
            .unwrap();

        let body_bytes = Bytes::from(response_body);

        let action = handler.handle(&response, &body_bytes, "test_key");

        assert!(
            action.is_none(),
            "Status code {} should not be handled by ServerErrorHandler",
            status_code.as_u16()
        );
    }
}
