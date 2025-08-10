// src/handlers/base.rs

use axum::response::Response;
use std::time::Duration;

/// Defines the next action to be taken by the main request loop.
#[derive(Debug)]
pub enum Action {
    /// The key is valid, but the quota is exhausted. Try the next key.
    RetryNextKey,
    /// The key is invalid and should be permanently blocked. Then, try the next key.
    BlockKeyAndRetry,
    /// The response is final and should be returned to the client immediately.
    ReturnToClient(Response),
    /// A rate limit with a specific wait duration was encountered.
    WaitFor(Duration),
    /// Terminal (non-retryable) response that should be returned to the client as-is.
    Terminal(Response),
}

impl PartialEq for Action {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Action::RetryNextKey, Action::RetryNextKey) => true,
            (Action::BlockKeyAndRetry, Action::BlockKeyAndRetry) => true,
            (Action::WaitFor(d1), Action::WaitFor(d2)) => d1 == d2,
            // For responses, we can't directly compare them.
            // In tests, we usually care about the variant, not the content.
            // This implementation considers them not equal, which is fine for current tests.
            _ => false,
        }
    }
}

/// A trait for handling responses from the upstream service.
/// Each implementation is responsible for a specific case (e.g., success, rate limit).
pub trait ResponseHandler: Send + Sync {
    /// Examines the response and decides on the next action.
    ///
    /// # Arguments
    /// * `response` - The response from the upstream service.
    ///
    /// # Returns
    /// * `Some(Action)` if this handler can process the response.
    /// * `None` if this handler cannot process the response, allowing the next handler in the chain to try.
    fn handle(
        &self,
        response: &Response,
        body_bytes: &axum::body::Bytes,
        api_key: &str,
    ) -> Option<Action>;
}
