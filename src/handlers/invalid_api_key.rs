// src/handlers/invalid_api_key.rs

use super::base::{Action, ResponseHandler};
use axum::{body::Bytes, http::StatusCode, response::Response};

pub struct InvalidApiKeyHandler;

impl ResponseHandler for InvalidApiKeyHandler {
    fn handle(&self, response: &Response, body_bytes: &Bytes, _api_key: &str) -> Option<Action> {
        if response.status() == StatusCode::BAD_REQUEST {
            if let Ok(body_str) = std::str::from_utf8(body_bytes) {
                if body_str.contains("API_KEY_INVALID") {
                    return Some(Action::BlockKeyAndRetry);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::response::Response;

    #[test]
    fn handle_invalid_api_key_error_returns_block_and_retry() {
        let handler = InvalidApiKeyHandler;
        let body = "Request contains an invalid API key. [detail: API_KEY_INVALID]";
        let response = Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(axum::body::Body::from(body))
            .unwrap();

        let (parts, body) = response.into_parts();
        let body_bytes =
            futures::executor::block_on(axum::body::to_bytes(body, usize::MAX)).unwrap();
        let response = Response::from_parts(parts, axum::body::Body::empty()); // Reconstruct response without body for handle

        let action = handler.handle(&response, &body_bytes, "test_key");
        assert!(matches!(action, Some(Action::BlockKeyAndRetry)));
    }

    #[test]
    fn handle_bad_request_without_invalid_key_returns_none() {
        let handler = InvalidApiKeyHandler;
        let body = "Some other bad request error";
        let response = Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(axum::body::Body::from(body))
            .unwrap();

        let (parts, body) = response.into_parts();
        let body_bytes =
            futures::executor::block_on(axum::body::to_bytes(body, usize::MAX)).unwrap();
        let response = Response::from_parts(parts, axum::body::Body::empty());

        let action = handler.handle(&response, &body_bytes, "test_key");
        assert!(action.is_none());
    }

    #[test]
    fn handle_non_bad_request_status_returns_none() {
        let handler = InvalidApiKeyHandler;
        let body = "Everything is fine";
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(axum::body::Body::from(body))
            .unwrap();

        let (parts, body) = response.into_parts();
        let body_bytes =
            futures::executor::block_on(axum::body::to_bytes(body, usize::MAX)).unwrap();
        let response = Response::from_parts(parts, axum::body::Body::empty());

        let action = handler.handle(&response, &body_bytes, "test_key");
        assert!(action.is_none());
    }
}
