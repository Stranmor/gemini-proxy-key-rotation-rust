// src/handler.rs

use crate::{
    error::{AppError, Result},
    // Removed FlattenedKeyInfo import, not needed directly here
    proxy, // Import the new proxy module
    state::AppState,
};
use axum::{
    extract::{Request, State},
    http::StatusCode, // Keep StatusCode for checking 429
    response::Response,
    // Removed unused imports: Body, header*, Method, Uri, BoxError, futures_util, reqwest, url
};
use std::sync::Arc;
use tracing::{debug, info, warn, error}; // Keep logging imports, added error

// HOP_BY_HOP_HEADERS and helper functions removed (moved to proxy.rs)

/// The main Axum handler function that processes incoming requests.
///
/// This function acts as the entry point for proxied requests. It performs the following steps:
/// 1. Retrieves the next available API key using `KeyManager` from the shared `AppState`.
/// 2. If no key is available (all are rate-limited), returns a `SERVICE_UNAVAILABLE` error.
/// 3. Calls `proxy::forward_request` to handle the actual request forwarding logic,
///    passing the original request, HTTP client, and selected key information.
/// 4. Checks the response from the upstream service.
/// 5. If the upstream response is `429 Too Many Requests`, it marks the used key as limited
///    via the `KeyManager`.
/// 6. Returns the response (or error) to the client.
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
    req: Request, // Request is now passed to forward_request
) -> Result<Response> {
    info!("---> Received request in handler");

    // 1. Get the next available key info
    // Use clone here to avoid borrowing state across await point if mark_key_as_limited is called
    let key_info = state
        .key_manager
        .get_next_available_key_info()
        .await
        .ok_or_else(|| {
            warn!("Handler: No available API keys.");
            AppError::NoAvailableKeys
        })?;
    let key_preview = format!("{}...", key_info.key.chars().take(4).collect::<String>());
    let group_name = key_info.group_name.clone(); // Clone for logging/marking
    let api_key_to_mark = key_info.key.clone(); // Clone key string for potential marking

    debug!(api_key_preview = %key_preview, group = %group_name, "Handler: Selected key info");

    // 2. Forward the request using the proxy module
    // Pass the base client from AppState and the selected key info.
    // forward_request consumes the original `req`.
    let forward_result = proxy::forward_request(state.client(), &key_info, req).await;

    // 3. Handle the result from forward_request
    match forward_result {
        Ok(response) => {
            // Check for rate limit response *after* successful forwarding
            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                warn!(status = %response.status(), api_key_preview=%key_preview, group=%group_name, "Handler: Target API returned 429. Marking key as limited.");
                // Mark the key as limited using the cloned key string
                state
                    .key_manager
                    .mark_key_as_limited(&api_key_to_mark)
                    .await;
            }
            info!(status = %response.status(), "<--- Handler: Sending response to client");
            Ok(response) // Return the successful response
        }
        Err(err) => {
            // Log the error originating from the proxy module or earlier steps
            error!(error = ?err, "<--- Handler: Error occurred during request forwarding");
            Err(err) // Propagate the error
        }
    }
}

// All helper functions related to request/response building and sending were moved to proxy.rs
