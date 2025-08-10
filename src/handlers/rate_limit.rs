// src/handlers/rate_limit.rs

use super::base::{Action, ResponseHandler};
use axum::{body::Bytes, http::StatusCode, response::Response};
use std::time::Duration;
use tracing::warn;

pub struct RateLimitHandler;

impl ResponseHandler for RateLimitHandler {
    fn handle(&self, response: &Response, body_bytes: &Bytes, _api_key: &str) -> Option<Action> {
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            let response_body = String::from_utf8_lossy(body_bytes);
            warn!(
                status = 429,
                response_body = %response_body,
                "Received 429 Too Many Requests."
            );

            if let Some(retry_after) = response.headers().get("retry-after") {
                if let Ok(retry_after_str) = retry_after.to_str() {
                    if let Ok(seconds) = retry_after_str.parse::<u64>() {
                        warn!("Rate limit requires waiting for {} seconds.", seconds);
                        return Some(Action::WaitFor(Duration::from_secs(seconds)));
                    }
                    // Handle HTTP-date format if necessary in the future
                }
            }

            warn!("No valid 'Retry-After' header found. Retrying with the next key immediately.");
            Some(Action::RetryNextKey)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header::RETRY_AFTER;
    use axum::http::HeaderMap;

    #[test]
    fn test_handle_429_with_retry_after_seconds() {
        let handler = RateLimitHandler;
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, "60".parse().unwrap());
        let mut response = Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .body(axum::body::Body::empty())
            .unwrap();
        *response.headers_mut() = headers;
        let body = Bytes::new();
        let action = handler.handle(&response, &body, "test-key");

        assert_eq!(action, Some(Action::WaitFor(Duration::from_secs(60))));
    }

    #[test]
    fn test_handle_429_without_retry_after() {
        let handler = RateLimitHandler;
        let response = Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .body(axum::body::Body::empty())
            .unwrap();
        let body = Bytes::new();
        let action = handler.handle(&response, &body, "test-key");

        assert_eq!(action, Some(Action::RetryNextKey));
    }

    #[test]
    fn test_handle_429_with_invalid_retry_after() {
        let handler = RateLimitHandler;
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, "invalid-value".parse().unwrap());
        let mut response = Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .body(axum::body::Body::empty())
            .unwrap();
        *response.headers_mut() = headers;
        let body = Bytes::new();
        let action = handler.handle(&response, &body, "test-key");

        assert_eq!(action, Some(Action::RetryNextKey));
    }

    #[test]
    fn test_handle_other_status_codes() {
        let handler = RateLimitHandler;
        let response_ok = Response::builder()
            .status(StatusCode::OK)
            .body(axum::body::Body::empty())
            .unwrap();
        let response_bad_req = Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(axum::body::Body::empty())
            .unwrap();
        let body = Bytes::new();

        assert_eq!(handler.handle(&response_ok, &body, "test-key"), None);
        assert_eq!(
            handler.handle(&response_bad_req, &body, "test-key"),
            None
        );
    }

}
