// src/handlers/success.rs

use super::base::{Action, ResponseHandler};
use axum::{
    body::{Body, Bytes},
    response::Response,
};

pub struct SuccessHandler;

impl ResponseHandler for SuccessHandler {
    fn handle(&self, response: &Response, body_bytes: &Bytes, _api_key: &str) -> Option<Action> {
        if response.status().is_success() {
            let mut builder = Response::builder().status(response.status());
            builder
                .headers_mut()
                .unwrap()
                .clone_from(response.headers());
            let resp = builder.body(Body::from(body_bytes.clone())).unwrap();
            Some(Action::ReturnToClient(resp))
        } else {
            None
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[tokio::test]
    async fn test_success_handler_returns_action_on_200_ok() {
        let handler = SuccessHandler;
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Body::empty())
            .unwrap();
        let body_bytes = Bytes::new();
        let api_key = "test_key";

        let action = handler.handle(&response, &body_bytes, api_key);

        assert!(matches!(action, Some(Action::ReturnToClient(_))));
    }

    #[tokio::test]
    async fn test_success_handler_returns_none_on_400_bad_request() {
        let handler = SuccessHandler;
        let response = Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::empty())
            .unwrap();
        let body_bytes = Bytes::new();
        let api_key = "test_key";

        let action = handler.handle(&response, &body_bytes, api_key);

        assert!(action.is_none());
    }

    #[tokio::test]
    async fn test_success_handler_returns_none_on_429_too_many_requests() {
        let handler = SuccessHandler;
        let response = Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .body(Body::empty())
            .unwrap();
        let body_bytes = Bytes::new();
        let api_key = "test_key";

        let action = handler.handle(&response, &body_bytes, api_key);

        assert!(action.is_none());
    }

    #[tokio::test]
    async fn test_success_handler_returns_none_on_500_internal_server_error() {
        let handler = SuccessHandler;
        let response = Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::empty())
            .unwrap();
        let body_bytes = Bytes::new();
        let api_key = "test_key";

        let action = handler.handle(&response, &body_bytes, api_key);

        assert!(action.is_none());
    }
}