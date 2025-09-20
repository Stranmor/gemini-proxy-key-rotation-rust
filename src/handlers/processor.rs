// src/handlers/processor.rs

use crate::{
    error::Result,
    handlers::base::{Action, ResponseHandler},
    key_manager::FlattenedKeyInfo,
};
use axum::{
    body::{to_bytes, Body},
    response::Response,
};
use secrecy::ExposeSecret;
use std::sync::Arc;

/// Processes a response through a chain of handlers.
pub struct ResponseProcessor {
    handlers: Arc<Vec<Box<dyn ResponseHandler>>>,
}

impl ResponseProcessor {
    /// Creates a new `ResponseProcessor` with a given chain of handlers.
    pub fn new(handlers: Vec<Box<dyn ResponseHandler>>) -> Self {
        Self {
            handlers: Arc::new(handlers),
        }
    }

    /// Analyzes the response and determines the next action.
    pub async fn process(
        &self,
        response: Response,
        key_info: &FlattenedKeyInfo,
    ) -> Result<(Action, Response)> {
        let (parts, body) = response.into_parts();
        let response_bytes = to_bytes(body, usize::MAX).await?;

        let response_for_analysis =
            Response::from_parts(parts.clone(), Body::from(response_bytes.clone()));

        let action_to_take = self.handlers.iter().find_map(|handler| {
            handler.handle(
                &response_for_analysis,
                &response_bytes,
                key_info.key.expose_secret(),
            )
        });

        if let Some(action) = action_to_take {
            let final_response = Response::from_parts(parts, Body::from(response_bytes));
            Ok((action, final_response))
        } else {
            let final_response =
                Response::from_parts(parts.clone(), Body::from(response_bytes.clone()));
            Ok((
                Action::ReturnToClient(Response::from_parts(parts, Body::from(response_bytes))),
                final_response,
            ))
        }
    }
}
