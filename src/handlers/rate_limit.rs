// src/handlers/rate_limit.rs

use super::base::{Action, ResponseHandler};
use axum::{body::Bytes, http::StatusCode, response::Response};
use tracing::warn;

pub struct RateLimitHandler;

impl ResponseHandler for RateLimitHandler {
    fn handle(&self, response: &Response, body_bytes: &Bytes) -> Option<Action> {
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            // Log the response body to understand the exact reason for rate limiting
            let response_body = String::from_utf8_lossy(body_bytes);
            warn!(
                status = 429,
                response_body = %response_body,
                "Received 429 Too Many Requests. Marking key as rate-limited and retrying with next key."
            );
            Some(Action::RetryNextKey)
        } else {
            None
        }
    }
}
