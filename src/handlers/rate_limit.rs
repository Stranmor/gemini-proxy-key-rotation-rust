// src/handlers/rate_limit.rs

use super::base::{Action, ResponseHandler};
use axum::{body::Bytes, http::StatusCode, response::Response};

pub struct RateLimitHandler;

impl ResponseHandler for RateLimitHandler {
    fn handle(&self, response: &Response, _body_bytes: &Bytes) -> Option<Action> {
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            Some(Action::RetryNextKey)
        } else {
            None
        }
    }
}