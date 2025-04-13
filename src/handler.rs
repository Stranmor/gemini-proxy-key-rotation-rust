// src/handler.rs

use crate::{
    error::{AppError, Result},
    proxy, // Import the proxy module
    state::AppState,
};
use axum::{
    body::Bytes, // Import Bytes
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, Uri}, // Import components needed for request reconstruction
    response::Response,
};
use std::sync::Arc;
use tracing::{debug, error, info, warn}; // Keep logging imports, added error

/// The main Axum handler function that processes incoming requests.
///
/// This function acts as the entry point for proxied requests. It performs the following steps:
/// 1. Buffers the incoming request body.
/// 2. Enters a loop to try available API keys:
///    a. Retrieves the next available API key using `KeyManager`.
///    b. If no key is available:
///       - If a previous attempt resulted in a 429, returns that last 429 response.
///       - Otherwise, returns a `SERVICE_UNAVAILABLE` error.
///    c. Calls `proxy::forward_request` to handle the actual request forwarding logic,
///       passing the request components (method, uri, headers, body) and key info.
///    d. Checks the response from the upstream service.
///    e. If the upstream response is `429 Too Many Requests`:
///       - Marks the used key as limited via the `KeyManager`.
///       - Stores the 429 response.
///       - Continues the loop to try the next key.
///    f. If the upstream response is successful or another error:
///       - Returns the response or error to the client, exiting the loop.
///
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
    info!("---> Received request in handler");

    // Extract parts from the original request *before* the loop
    let method = req.method().clone();
    let uri = req.uri().clone();
    let headers = req.headers().clone();

    // Buffer the body. Handle potential errors during buffering.
    let body_bytes = match axum::body::to_bytes(req.into_body(), usize::MAX).await {
        Ok(bytes) => bytes,
        Err(e) => {
            error!("Failed to buffer request body: {}", e);
            // Use the error type from axum if it implements Into<AppError>,
            // otherwise wrap it. Assuming BodyError -> RequestBodyError mapping.
             // Check if axum::Error can be directly converted or needs mapping
             // For now, let's use a generic RequestBodyError variant
            return Err(AppError::RequestBodyError(format!(
                "Failed to read request body: {}",
                e
            )));

        }
    };

    let mut last_429_response: Option<Response> = None;

    loop {
        // 1. Get the next available key info
        let key_info = match state.key_manager.get_next_available_key_info().await {
            Some(ki) => ki,
            None => {
                warn!("Handler: No available API keys remaining after retries.");
                // If we previously got a 429, return that. Otherwise, return NoAvailableKeys.
                return match last_429_response {
                    Some(response) => {
                         info!("<--- Handler: Exhausted all keys, returning last 429 response.");
                         Ok(response)
                    },
                    None => Err(AppError::NoAvailableKeys),
                };
            }
        };

        let key_preview = format!("{}...", key_info.key.chars().take(4).collect::<String>());
        let group_name = key_info.group_name.clone(); // Clone for logging/marking
        let api_key_to_mark = key_info.key.clone(); // Clone key string for potential marking

        debug!(api_key_preview = %key_preview, group = %group_name, "Handler: Trying key");

        // 2. Forward the request using the proxy module
        // Pass cloned components and key info.
        // Note: We clone body_bytes for each attempt. Consider Arc<Bytes> if performance critical.
        let forward_result = proxy::forward_request(
            state.client(),
            &key_info,
            method.clone(),
            uri.clone(), // Clone Uri
            headers.clone(), // Clone HeaderMap
            body_bytes.clone(), // Clone Bytes for the attempt
        )
        .await;

        // 3. Handle the result from forward_request
        match forward_result {
            Ok(response) => {
                // Check for rate limit response *after* successful forwarding
                if response.status() == StatusCode::TOO_MANY_REQUESTS {
                    warn!(status = %response.status(), api_key_preview=%key_preview, group=%group_name, "Handler: Target API returned 429. Marking key as limited and retrying.");
                    // Mark the key as limited using the cloned key string
                    state
                        .key_manager
                        .mark_key_as_limited(&api_key_to_mark)
                        .await;
                    last_429_response = Some(response); // Store the 429 response
                    continue; // Try the next key
                } else {
                    // Success or other non-429 status
                    info!(status = %response.status(), "<--- Handler: Sending successful response to client");
                    return Ok(response); // Return the successful response
                }
            }
            Err(err) => {
                // Log the error originating from the proxy module or earlier steps
                error!(error = ?err, "<--- Handler: Error occurred during request forwarding");
                // Don't retry on other errors (like Bad Gateway, connection refused etc.)
                return Err(err); // Propagate the error
            }
        }
    } // end loop
}
