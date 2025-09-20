// src/handlers/terminal_error.rs

use super::base::{Action, ResponseHandler};
use axum::{
    body::{Body, Bytes},
    http::StatusCode,
    response::Response,
};

pub struct TerminalErrorHandler;

impl ResponseHandler for TerminalErrorHandler {
    fn handle(&self, response: &Response, body_bytes: &Bytes, _api_key: &str) -> Option<Action> {
        let status = response.status();
        // This handler is last in the chain. It catches terminal errors that
        // weren't handled by previous handlers.
        // We explicitly exclude codes that have dedicated handlers:
        // - 400 (handled by InvalidApiKeyHandler for specific cases)
        // - 408, 504 (handled by TimeoutHandler)
        // - 429 (handled by RateLimitHandler)
        // - 500, 502, 503 (handled by ServerErrorHandler)
        if (status.is_client_error()
            && status != StatusCode::BAD_REQUEST
            && status != StatusCode::REQUEST_TIMEOUT
            && status != StatusCode::TOO_MANY_REQUESTS)
            || (status.is_server_error()
                && status != StatusCode::INTERNAL_SERVER_ERROR
                && status != StatusCode::BAD_GATEWAY
                && status != StatusCode::SERVICE_UNAVAILABLE
                && status != StatusCode::GATEWAY_TIMEOUT)
        {
            let mut builder = Response::builder().status(status);
            if let Some(headers) = builder.headers_mut() {
                headers.clone_from(response.headers());
            }
            let resp = builder.body(Body::from(body_bytes.clone())).unwrap();
            // Явно помечаем как терминальный исход
            Some(Action::Terminal(resp))
        } else {
            None
        }
    }
}
