// src/handlers/timeout.rs

use crate::handlers::base::{Action, ResponseHandler};
use axum::{body::Bytes, http::StatusCode, response::Response};
use tracing::{info, warn};

/// Handler for timeout-specific errors with retry logic
pub struct TimeoutHandler;

impl ResponseHandler for TimeoutHandler {
    fn handle(&self, response: &Response, body_bytes: &Bytes, _api_key: &str) -> Option<Action> {
        let status = response.status();
        
        // Handle gateway timeout (504) and request timeout (408)
        if matches!(status, StatusCode::GATEWAY_TIMEOUT | StatusCode::REQUEST_TIMEOUT) {
            warn!(
                status = status.as_u16(),
                "Timeout error detected, will retry with next key"
            );
            
            // For timeout errors, we want to try a different key
            // but not block the current key permanently
            return Some(Action::RetryNextKey);
        }
        
        // Check for timeout indications in server error responses
        if status.is_server_error() {
            let body_text = String::from_utf8_lossy(body_bytes);
            if body_text.contains("timeout") || body_text.contains("timed out") {
                info!(
                    status = status.as_u16(),
                    "Server error with timeout indication, retrying with next key"
                );
                
                return Some(Action::RetryNextKey);
            }
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
    fn test_timeout_handler_gateway_timeout() {
        let handler = TimeoutHandler;
        let (response, body) = create_test_response(StatusCode::GATEWAY_TIMEOUT, "");
        
        let action = handler.handle(&response, &body, "test_key");
        
        assert!(matches!(action, Some(Action::RetryNextKey)));
    }

    #[test]
    fn test_timeout_handler_request_timeout() {
        let handler = TimeoutHandler;
        let (response, body) = create_test_response(StatusCode::REQUEST_TIMEOUT, "");
        
        let action = handler.handle(&response, &body, "test_key");
        
        assert!(matches!(action, Some(Action::RetryNextKey)));
    }

    #[test]
    fn test_timeout_handler_server_error_with_timeout_body() {
        let handler = TimeoutHandler;
        let (response, body) = create_test_response(
            StatusCode::INTERNAL_SERVER_ERROR, 
            "Request timed out while processing"
        );
        
        let action = handler.handle(&response, &body, "test_key");
        
        assert!(matches!(action, Some(Action::RetryNextKey)));
    }

    #[test]
    fn test_timeout_handler_no_action_for_other_errors() {
        let handler = TimeoutHandler;
        let (response, body) = create_test_response(StatusCode::BAD_REQUEST, "Invalid request");
        
        let action = handler.handle(&response, &body, "test_key");
        
        assert!(action.is_none());
    }
}