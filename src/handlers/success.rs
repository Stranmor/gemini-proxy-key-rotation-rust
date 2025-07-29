// src/handlers/success.rs

use super::base::{Action, ResponseHandler};
use axum::{body::{Body, Bytes}, response::Response};

pub struct SuccessHandler;

impl ResponseHandler for SuccessHandler {
    fn handle(&self, response: &Response, body_bytes: &Bytes) -> Option<Action> {
        if response.status().is_success() {
            let mut builder = Response::builder().status(response.status());
            builder.headers_mut().unwrap().clone_from(response.headers());
            let resp = builder.body(Body::from(body_bytes.clone())).unwrap();
            Some(Action::ReturnToClient(resp))
        } else {
            None
        }
    }
}