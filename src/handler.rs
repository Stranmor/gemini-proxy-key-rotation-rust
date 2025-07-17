// src/handler.rs

use crate::{
    error::{AppError, Result},
    key_manager::FlattenedKeyInfo,
    proxy, // Import the proxy module
    state::AppState,
};
use axum::{
    extract::{Request, State},
    http::{StatusCode, Uri},
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
    let uri = req.uri().clone();
    let headers = req.headers().clone();

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

    // --- Cache Check ---
    let cache_key = state
        .cache
        .generate_key(method.as_str(), uri.path(), uri.query(), &body_bytes);
    if let Some(cached) = state.cache.get(&cache_key).await {
        info!("Cache hit. Returning cached response.");
        let mut response = Response::builder()
            .status(StatusCode::from_u16(cached.status).unwrap_or(StatusCode::OK))
            .body(axum::body::Body::from(cached.data.clone()))
            .unwrap(); // Safe unwrap
        *response.headers_mut() = cached.to_header_map();
        return Ok(response);
    }
    info!("Cache miss. Proceeding to forward request.");

    let mut last_error: Option<(StatusCode, axum::http::HeaderMap, axum::body::Bytes)> = None;
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
                let mut response = Response::builder()
                    .status(status)
                    .body(axum::body::Body::from(body))
                    .unwrap();
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
            let final_target_url =
                match build_translated_gemini_url(&key_info, &uri).await {
                    Ok(url) => url,
                    Err(e) => {
                        error!("Failed to build translated Gemini URL: {}. This is a non-retriable client error.", e);
                        return Err(e);
                    }
                };
            // --- End URL Translation ---

            let forward_result = proxy::forward_request(
                &state,
                &key_info,
                method.clone(),
                final_target_url, // Pass the translated URL
                headers.clone(),
                body_bytes.clone(),
            )
            .await;

            let response = match forward_result {
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
                    // --- Cache Put ---
                    if state.cache.should_cache(s, response.headers()) {
                        let (parts, body) = response.into_parts();
                        let body_bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
                        state
                            .cache
                            .put(
                                cache_key.clone(),
                                body_bytes.to_vec(),
                                parts.headers.clone(),
                                parts.status,
                                None,
                            )
                            .await;
                        // Reconstruct the response
                        return Ok(Response::from_parts(parts, axum::body::Body::from(body_bytes)));
                    }
                    return Ok(response);
                }
                // --- Terminal Client Errors (400, 404, 504) ---
                StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::GATEWAY_TIMEOUT => {
                    warn!(
                        status = status.as_u16(),
                        "Received terminal client error. Not retrying."
                    );
                    return Ok(response);
                }
                // --- Key Invalid Error (403) ---
                StatusCode::FORBIDDEN => {
                    warn!(
                        status = status.as_u16(),
                        "Received 403 Forbidden. Marking key as invalid and retrying with next key."
                    );
                    state
                        .key_manager
                        .mark_key_as_invalid(&api_key_to_mark)
                        .await;
                    // Buffer the body before storing the response
                    let (parts, body) = response.into_parts();
                    let error_body_bytes = axum::body::to_bytes(body, usize::MAX).await.map_err(|e| AppError::RequestBodyError(format!("Failed to buffer error response body: {e}")))?;
                    last_error = Some((parts.status, parts.headers, error_body_bytes));
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
                    // Buffer the body before storing the response
                    let (parts, body) = response.into_parts();
                    let error_body_bytes = axum::body::to_bytes(body, usize::MAX).await.map_err(|e| AppError::RequestBodyError(format!("Failed to buffer error response body: {e}")))?;
                    last_error = Some((parts.status, parts.headers, error_body_bytes));
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
                        // Buffer the body before storing the response
                        let (parts, body) = response.into_parts();
                        let error_body_bytes = axum::body::to_bytes(body, usize::MAX).await.map_err(|e| AppError::RequestBodyError(format!("Failed to buffer error response body: {e}")))?;
                        last_error = Some((parts.status, parts.headers, error_body_bytes));
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



/// Builds the final Gemini URL, including the API key.
/// This function encapsulates the logic that was previously inside `proxy::forward_request`.
async fn build_translated_gemini_url(
    key_info: &FlattenedKeyInfo,
    original_uri: &Uri,
) -> Result<Url> {
    let target_base_url_str = &key_info.target_url;
    let base_url = Url::parse(target_base_url_str).map_err(|e| {
        error!(
            target_base_url = %target_base_url_str,
            group.name = %key_info.group_name,
            error = %e,
            "Failed to parse target_base_url from configuration for group"
        );
        AppError::Internal(format!("Invalid base URL in config: {e}"))
    })?;

    let path = original_uri.path();
    let mut final_target_url = base_url.join(path).map_err(|e| {
        error!(
            base_url = %base_url,
            path_to_join = %path,
            error = %e,
            "Failed to join base URL with request path"
        );
        AppError::UrlJoinError(format!("Failed to join URL path: {e}"))
    })?;

    if let Some(query) = original_uri.query() {
        final_target_url.set_query(Some(query));
    }

    final_target_url
        .query_pairs_mut()
        .append_pair("key", &key_info.key);
        
    debug!(target.url = %final_target_url, "Constructed final target URL with key for request");

    Ok(final_target_url)
}
