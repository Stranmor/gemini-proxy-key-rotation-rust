// src/handler.rs

use crate::{
    error::{AppError, Result},
    proxy, // Import the proxy module
    state::AppState,
};
use axum::{
    extract::{Request, State},
    http::StatusCode,
    response::Response,
};
use std::sync::Arc;
use tracing::{debug, error, info, instrument, warn}; // Added instrument

/// Simple health check handler. Returns HTTP 200 OK.
#[instrument(name = "health_check", level = "debug", skip_all)] // Add span for health check
pub async fn health_check() -> StatusCode {
    debug!("Responding to health check"); // Add a debug log
    StatusCode::OK
}

/// The main Axum handler function that processes incoming requests.
/// This function is instrumented by the trace_requests middleware.
/// # Arguments
///
/// * `State(state)`: The shared application state (`Arc<AppState>`).
/// * `req`: The incoming `axum::extract::Request`.
///
/// # Returns
///
/// Returns a `Result<Response, AppError>` which Axum converts into an HTTP response.
pub async fn proxy_handler(
    State(state): State<Arc<AppState>>,
    req: Request, // Request is consumed here to extract parts
) -> Result<Response> {
    // Note: Request ID and basic info are already logged by the trace_requests middleware span
    info!("Received request in proxy handler"); // Keep high-level log

    // Extract parts from the original request *before* the loop
    let method = req.method().clone();
    let uri = req.uri().clone();
    let headers = req.headers().clone();

    // Buffer the body. Handle potential errors during buffering.
    let body_bytes = match axum::body::to_bytes(req.into_body(), usize::MAX).await {
        Ok(bytes) => bytes,
        Err(e) => {
            // Log with structured error field
            error!(error = ?e, error.source = %e, "Failed to buffer request body");
            return Err(AppError::RequestBodyError(format!(
                "Failed to read request body: {}",
                e
            )));
        }
    };

    let mut last_429_response: Option<Response> = None;
    let mut attempt_count = 0; // Track attempt count for logging

    loop {
        attempt_count += 1;
        debug!(
            attempt = attempt_count,
            "Looking for next available API key"
        );

        // 1. Get the next available key info
        let key_info = match state.key_manager.get_next_available_key_info().await {
            Some(ki) => ki,
            None => {
                // Log with structured field indicating cause
                warn!(
                    cause = "no_available_keys",
                    attempts = attempt_count,
                    last_429_present = last_429_response.is_some(),
                    "No available API keys remaining after retries."
                );
                // If we previously got a 429, return that. Otherwise, return NoAvailableKeys.
                return match last_429_response {
                    Some(response) => {
                        info!(
                            status = response.status().as_u16(),
                            attempts = attempt_count,
                            "Exhausted all keys, returning last 429 response."
                        );
                        Ok(response)
                    }
                    None => Err(AppError::NoAvailableKeys),
                };
            }
        };

        let key_preview = format!("{}...", key_info.key.chars().take(4).collect::<String>());
        let group_name = key_info.group_name.clone(); // Clone for logging/marking
        let api_key_to_mark = key_info.key.clone(); // Clone key string for potential marking

        debug!(
            attempt = attempt_count,
            api_key.preview = %key_preview,
            group.name = %group_name,
            "Attempting request with key"
        );

        // 2. Forward the request using the proxy module
        // Note: We clone body_bytes for each attempt. Consider Arc<Bytes> if performance critical.
        let forward_result = proxy::forward_request(
            &state, // Pass reference to the AppState (Arc derefs to AppState)
            &key_info,
            method.clone(),
            uri.clone(),        // Clone Uri
            headers.clone(),    // Clone HeaderMap
            body_bytes.clone(), // Clone Bytes for the attempt
        )
        .await;

        // 3. Handle the result from forward_request
        match forward_result {
            Ok(response) => {
                let response_status = response.status();
                // Check for rate limit response *after* successful forwarding
                if response_status == StatusCode::TOO_MANY_REQUESTS {
                    warn!(
                        attempt = attempt_count,
                        status = response_status.as_u16(), // Use u16 for status
                        api_key.preview=%key_preview,
                        group.name=%group_name,
                        "Target API returned 429. Marking key as limited and retrying."
                    );
                    // Mark the key as limited using the cloned key string
                    state
                        .key_manager
                        .mark_key_as_limited(&api_key_to_mark)
                        .await;
                    last_429_response = Some(response); // Store the 429 response
                    continue; // Try the next key
                } else {
                    // Success or other non-429 status
                    info!(
                        attempt = attempt_count,
                        status = response_status.as_u16(),
                        api_key.preview=%key_preview,
                        group.name=%group_name,
                        "Sending successful response to client"
                    );
                    return Ok(response); // Return the successful response
                }
            }
            Err(err) => {
                // Log the error originating from the proxy module or earlier steps
                error!(
                   attempt = attempt_count,
                   error = ?err, // Debug format for AppError
                    api_key.preview=%key_preview,
                    group.name=%group_name,
                   "Error occurred during request forwarding attempt. Returning error to client."
                );
                // Don't retry on other errors (like Bad Gateway, connection refused etc.)
                return Err(err); // Propagate the error
            }
        }
    } // end loop
}
