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