// src/handler.rs

use crate::{
    error::{AppError, Result},
    proxy, // Import the proxy module
    state::AppState,
};
use axum::{
    body::Bytes,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, Uri},
    response::Response,
};
use chrono::Duration as ChronoDuration;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info, instrument, warn};
use url::Url;

/// Simple health check handler. Returns HTTP 200 OK.
#[instrument(name = "health_check", level = "debug", skip_all)] // Add span for health check
pub async fn health_check() -> StatusCode {
    debug!("Responding to health check"); // Add a debug log
    StatusCode::OK
}

/// The main Axum handler function that processes incoming requests.
/// This function is instrumented by the `trace_requests` middleware.
/// # Arguments
///
/// * `State(state)`: The shared application state (`Arc<AppState>`).
/// * `req`: The incoming `axum::extract::Request`.
///
/// # Returns
///
/// Returns a `Result<Response, AppError>` which Axum converts into an HTTP response.
/// Handles incoming proxy requests, forwarding them to the appropriate upstream service
/// using a rotated API key.
///
/// This function is instrumented by the `trace_requests` middleware.
///
/// # Arguments
/// * `state` - Shared application state containing `KeyManager` and HTTP clients.
/// * `req` - The incoming HTTP request from the client.
///
/// # Errors
/// Returns an `AppError` if:
/// - The request body cannot be read.
/// - No API keys are available.
/// - An error occurs during forwarding the request to the upstream service.
/// - The upstream service returns an error.
/// - The response body from the upstream service cannot be processed.
pub async fn proxy_handler(
    State(state): State<Arc<AppState>>,
    req: Request, // Request is consumed here to extract parts
) -> Result<Response> {
    // Note: Request ID and basic info are already logged by the trace_requests middleware span
    info!("Received request in proxy handler"); // Keep high-level log

    // Extract parts from the original request *before* the loop
    let method = req.method().clone();
    let mut uri = req.uri().clone();
    let headers = req.headers().clone();

    // Special handling for /health/detailed to map it to the models endpoint
    if uri.path() == "/health/detailed" {
        let mut parts = uri.into_parts();
        parts.path_and_query = Some("/v1beta/models".parse().unwrap());
        uri = Uri::from_parts(parts).unwrap();
        info!(new_path = %uri.path(), "Translated /health/detailed to models endpoint");
    }

    // Buffer the body. Handle potential errors during buffering.
    let body_bytes = match axum::body::to_bytes(req.into_body(), usize::MAX).await {
        Ok(bytes) => bytes,
        Err(e) => {
            // Log with structured error field
            error!(error = ?e, error.source = %e, "Failed to buffer request body");
            return Err(AppError::RequestBodyError(format!(
                "Failed to read request body: {e}"
            )));
        }
    };

    let mut last_error: Option<(StatusCode, HeaderMap, Bytes)> = None;
    let mut attempt_count = 0;

    loop {
        attempt_count += 1;
        debug!(attempt = attempt_count, "Looking for next available API key");

        let Some(key_info) = state.key_manager.get_next_available_key_info().await else {
            warn!(
                cause = "no_available_keys",
                attempts = attempt_count,
                "No available API keys remaining after retries."
            );
            return if let Some((status, headers, body)) = last_error {
                let mut response = Response::new(axum::body::Body::from(body));
                *response.status_mut() = status;
                *response.headers_mut() = headers;
                Ok(response)
            } else {
                Err(AppError::NoAvailableKeys)
            };
        };

        let key_preview = format!("{}...", key_info.key.chars().take(4).collect::<String>());
        let group_name = key_info.group_name.clone();
        let api_key_to_mark = key_info.key.clone();

        // --- Internal Retry Loop for 5xx Errors ---
        let mut internal_retry_count = 0;
        let max_internal_retries = 2;
        loop {
            internal_retry_count += 1;
            debug!(
                attempt = attempt_count,
                internal_attempt = internal_retry_count,
                api_key.preview = %key_preview,
                group.name = %group_name,
                "Attempting request with key"
            );

            // --- URL Translation ---
            // This block translates OpenAI-compatible paths to Gemini-specific paths.
            // For example, a request to `/v1/chat/completions` is proxied
            // to `/v1beta/openai/chat/completions` on the target service.
            let translated_path = if let Some(stripped_path) = uri.path().strip_prefix("/v1/") {
                format!("/v1beta/openai/{}", stripped_path)
            } else {
                uri.path().to_string()
            };

            // --- URL Construction ---
            let base_url = Url::parse(&key_info.target_url).map_err(|e| {
                error!(
                    target_base_url = %key_info.target_url,
                    group.name = %key_info.group_name,
                    error = %e,
                    "Failed to parse target_base_url from configuration for group"
                );
                AppError::Internal(format!("Invalid base URL in config: {e}"))
            })?;

            let mut final_target_url = base_url.join(&translated_path).map_err(|e| {
                error!(
                    base_url = %base_url,
                    path = %uri.path(),
                    error = %e,
                    "Failed to join path to base URL"
                );
                AppError::UrlJoinError(e.to_string())
            })?;

            if let Some(query) = uri.query() {
                final_target_url.set_query(Some(query));
            }

            final_target_url
                .query_pairs_mut()
                .append_pair("key", &key_info.key);
                
            debug!(target.url = %final_target_url, "Constructed final target URL for request");
            // --- End URL Construction ---

            let forward_result = proxy::forward_request(
                &state,
                &key_info,
                method.clone(),
                final_target_url, // Pass the constructed URL
                headers.clone(),
                body_bytes.clone(),
            );

            let response = match forward_result.await {
                Ok(resp) => resp,
                Err(err) => {
                    error!(error = ?err, "Request forwarding failed. Not retrying.");
                    return Err(err);
                }
            };

            let status = response.status();
            match status {
                // --- Terminal Success ---
                s if s.is_success() => {
                    info!(status = s.as_u16(), "Request successful.");
                    return Ok(response);
                }
                // --- Terminal Client Errors (404, 504) ---
                StatusCode::NOT_FOUND | StatusCode::GATEWAY_TIMEOUT => {
                    warn!(
                        status = status.as_u16(),
                        "Received terminal client error. Not retrying."
                    );
                    return Ok(response);
                }
                // --- Key Invalid Errors (400, 403) ---
                StatusCode::BAD_REQUEST | StatusCode::FORBIDDEN => {
                    warn!(
                        status = status.as_u16(),
                        "Received key error, marking key as invalid and retrying."
                    );
                    state
                        .key_manager
                        .mark_key_as_invalid(&api_key_to_mark)
                        .await;
                    // Deconstruct the response, buffer the body, and store it.
                    let (parts, body) = response.into_parts();
                    let body_bytes = match axum::body::to_bytes(body, usize::MAX).await {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            error!(error = ?e, "Failed to buffer error response body");
                            // If we can't even buffer the error response, return a generic internal error.
                            return Err(AppError::Internal(
                                "Failed to process error response".to_string(),
                            ));
                        }
                    };
                    last_error = Some((parts.status, parts.headers, body_bytes));
                    break; // Break internal loop to get next key
                }
                // --- Rate Limit Error (429) ---
                StatusCode::TOO_MANY_REQUESTS => {
                    warn!(
                        status = status.as_u16(),
                        "Received 429 Too Many Requests. Marking key as rate-limited and retrying with next key."
                    );
                    state
                        .key_manager
                        .mark_key_as_limited(&api_key_to_mark)
                        .await;
                    // Deconstruct the response, buffer the body, and store it.
                    let (parts, body) = response.into_parts();
                    let body_bytes = match axum::body::to_bytes(body, usize::MAX).await {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            error!(error = ?e, "Failed to buffer error response body");
                            return Err(AppError::Internal(
                                "Failed to process error response".to_string(),
                            ));
                        }
                    };
                    last_error = Some((parts.status, parts.headers, body_bytes));
                    break; // Break internal loop to get next key
                }
                // --- Retriable Server Errors (500, 503) ---
                StatusCode::INTERNAL_SERVER_ERROR | StatusCode::SERVICE_UNAVAILABLE => {
                    warn!(
                        status = status.as_u16(),
                        internal_attempt = internal_retry_count,
                        max_internal_retries,
                        "Received retriable server error."
                    );
                    if internal_retry_count >= max_internal_retries {
                        error!("Internal retries exhausted for key. Marking as temporarily unavailable.");
                        state
                            .key_manager
                            .mark_key_as_temporarily_unavailable(
                                &api_key_to_mark,
                                ChronoDuration::minutes(5),
                            )
                            .await;
                        // If we're breaking due to server errors, only set this as the last
                        // error if we haven't already captured a more specific client error
                        // like 429 (rate-limited) or 403 (invalid key). This ensures
                        // we return the most relevant error to the client upon exhaustion.
                        if last_error.is_none() {
                            let (parts, body) = response.into_parts();
                            let body_bytes = match axum::body::to_bytes(body, usize::MAX).await {
                                Ok(bytes) => bytes,
                                Err(e) => {
                                    error!(error = ?e, "Failed to buffer error response body");
                                    return Err(AppError::Internal(
                                        "Failed to process error response".to_string(),
                                    ));
                                }
                            };
                            last_error = Some((parts.status, parts.headers, body_bytes));
                        }
                        break; // Break internal loop to get next key
                    }
                    sleep(Duration::from_secs(1)).await; // Wait before internal retry
                }
                // --- Other Unexpected Errors ---
                _ => {
                    warn!(
                        status = status.as_u16(),
                        "Received unexpected status code. Returning response to client."
                    );
                    return Ok(response);
                }
            }
        } // End internal retry loop
    } // End main loop
}



// Removed the incorrect build_translated_gemini_url function.
// The logic is now correctly handled inside the proxy_handler.
