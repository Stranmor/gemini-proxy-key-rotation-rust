// src/handlers/terminal_error.rs

use super::base::{Action, ResponseHandler};
use axum::{body::{Body, Bytes}, http::StatusCode, response::Response};

pub struct TerminalErrorHandler;

impl ResponseHandler for TerminalErrorHandler {
    fn handle(&self, response: &Response, body_bytes: &Bytes, _api_key: &str) -> Option<Action> {
        let status = response.status();
        // This handler is last in the chain. It catches terminal server errors (5xx)
        // or any client errors that weren't specifically handled by previous handlers.
        // We explicitly exclude 400 and 429, as they have dedicated logic paths
        // (either specific handlers or the main loop's default behavior).
        if status.is_server_error() || (status.is_client_error() && status != StatusCode::BAD_REQUEST && status != StatusCode::TOO_MANY_REQUESTS) {
            let mut builder = Response::builder().status(status);
            if let Some(headers) = builder.headers_mut() {
                headers.clone_from(response.headers());
            }
            let resp = builder.body(Body::from(body_bytes.clone())).unwrap();
            Some(Action::ReturnToClient(resp))
        } else {
            None
        }
    }
}