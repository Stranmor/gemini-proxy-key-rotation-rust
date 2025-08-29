// src/handlers/server_error.rs

use super::base::{Action, ResponseHandler};
use axum::{body::Bytes, http::StatusCode, response::Response};
use tracing::warn;

/// Handler for server errors that should trigger key rotation
pub struct ServerErrorHandler;

impl ResponseHandler for ServerErrorHandler {
    fn handle(&self, response: &Response, body_bytes: &Bytes, _api_key: &str) -> Option<Action> {
        let status = response.status();
        
        // Handle specific server errors that indicate temporary issues
        // and should trigger key rotation instead of returning error to client
        if matches!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR |  // 500
            StatusCode::BAD_GATEWAY |            // 502
            StatusCode::SERVICE_UNAVAILABLE |    // 503
            StatusCode::GATEWAY_TIMEOUT          // 504 (also handled by TimeoutHandler, but as backup)
        ) {
            let body_text = String::from_utf8_lossy(body_bytes);
            
            warn!(
                status = status.as_u16(),
                response_body = %body_text,
                "Server error detected, will retry with next key"
            );

            // For server errors, we want to try a different key
            // These are typically temporary issues on the provider side
            return Some(Action::RetryNextKey);
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Bytes, http::StatusCode, response::Response};

    fn create_test_response(status: StatusCode, body: &str) -> (Response, Bytes) {
        let response = Response::builder()
            .status(status)
            .body(axum::body::Body::empty())
            .unwrap();
        let body_bytes = Bytes::from(body.to_string());
        (response, body_bytes)
    }

    #[test]
    fn test_server_error_handler_500() {
        let handler = ServerErrorHandler;
        let (response, body) = create_test_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error": {"code": 500,"message": "An internal error has occurred. Please retry or report in https://developers.generativeai.google/guide/troubleshooting","status": "INTERNAL"}}"#
        );

        let action = handler.handle(&response, &body, "test_key");
        assert!(matches!(action, Some(Action::RetryNextKey)));
    }

    #[test]
    fn test_server_error_handler_502() {
        let handler = ServerErrorHandler;
        let (response, body) = create_test_response(StatusCode::BAD_GATEWAY, "Bad Gateway");

        let action = handler.handle(&response, &body, "test_key");
        assert!(matches!(action, Some(Action::RetryNextKey)));
    }

    #[test]
    fn test_server_error_handler_503() {
        let handler = ServerErrorHandler;
        let (response, body) = create_test_response(StatusCode::SERVICE_UNAVAILABLE, "Service Unavailable");

        let action = handler.handle(&response, &body, "test_key");
        assert!(matches!(action, Some(Action::RetryNextKey)));
    }

    #[test]
    fn test_server_error_handler_504() {
        let handler = ServerErrorHandler;
        let (response, body) = create_test_response(StatusCode::GATEWAY_TIMEOUT, "Gateway Timeout");

        let action = handler.handle(&response, &body, "test_key");
        assert!(matches!(action, Some(Action::RetryNextKey)));
    }

    #[test]
    fn test_server_error_handler_no_action_for_client_errors() {
        let handler = ServerErrorHandler;
        let (response, body) = create_test_response(StatusCode::BAD_REQUEST, "Bad Request");

        let action = handler.handle(&response, &body, "test_key");
        assert!(action.is_none());
    }

    #[test]
    fn test_server_error_handler_no_action_for_success() {
        let handler = ServerErrorHandler;
        let (response, body) = create_test_response(StatusCode::OK, "Success");

        let action = handler.handle(&response, &body, "test_key");
        assert!(action.is_none());
    }
}