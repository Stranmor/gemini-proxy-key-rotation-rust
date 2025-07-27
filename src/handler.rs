// src/handler.rs

use crate::{
    error::{AppError, Result},
    proxy,
    state::AppState,
};
use axum::{
    body::{to_bytes, Bytes},
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use std::sync::Arc;
use tracing::{debug, error, info, instrument, warn};
use url::Url;

/// Enum to control the flow of the retry loop in the proxy handler.
enum NextAction {
    /// Return the response to the client immediately.
    ReturnResponse(Response),
    /// Break the inner loop (internal retries) and try the next key.
    BreakLoop,
    /// Continue the inner loop to retry with the same key.
    ContinueLoop,
}

/// Simple health check handler. Returns HTTP 200 OK.
#[instrument(name = "health_check", level = "debug", skip_all)]
pub async fn health_check() -> StatusCode {
    debug!("Responding to health check");
    StatusCode::OK
}


/// Helper to apply an action to a key and save the state.
/// This function centralizes the logic of acquiring a write lock,
/// performing an action, and then saving the state if the action was successful.
async fn apply_key_action<F>(
    state: &Arc<AppState>,
    api_key: &str,
    action: F,
) -> Result<NextAction>
where
    F: FnOnce(&mut crate::key_manager::KeyManager, &str) -> bool,
{
    let mut km = state.key_manager.write().await;
    if action(&mut km, api_key) {
        km.save_states().await?;
    }
    Ok(NextAction::BreakLoop)
}

/// Handles the response from the upstream service and determines the next action.
#[instrument(skip_all, fields(status = response.status().as_u16()))]
async fn handle_upstream_response(
    response: Response,
    state: &Arc<AppState>,
    api_key_to_mark: &str,
    internal_retry_count: u32,
    last_error: &mut Option<(StatusCode, HeaderMap, Bytes)>,
) -> Result<NextAction> {
    let status = response.status();

    match status {
        s if s.is_success() => {
            info!("Request successful.");
            Ok(NextAction::ReturnResponse(response))
        }
        StatusCode::NOT_FOUND | StatusCode::GATEWAY_TIMEOUT => {
            warn!("Received terminal client error, not retrying.");
            Ok(NextAction::ReturnResponse(response))
        }
        s if s.is_client_error() => {
            let (parts, body) = response.into_parts();
            let body_bytes = to_bytes(body, usize::MAX).await?;
            // By calling .to_string(), we create a new owned String, which breaks the borrow on body_bytes.
            let body_str = String::from_utf8_lossy(&body_bytes).to_string();

            warn!(
                "Upstream API returned 4xx error. Status: {}. Body: {}. Marking key and retrying.",
                s, body_str
            );
            *last_error = Some((parts.status, parts.headers, body_bytes));

            // If the body indicates an invalid API key, we treat it as a permanent failure for that key.
            if s == StatusCode::BAD_REQUEST && body_str.contains("API_KEY_INVALID") {
                warn!("API key is invalid. Marking as Invalid.");
                apply_key_action(state, api_key_to_mark, |km, key| {
                    km.mark_key_as_invalid(key)
                })
                .await
            } else {
                // For other 4xx errors (like 429 or other 400s), mark the key as limited and retry.
                warn!("API key is rate-limited or request is malformed. Marking as Limited.");
                apply_key_action(state, api_key_to_mark, |km, key| {
                    km.mark_key_as_limited(key)
                })
                .await
            }
        }
        s if s.is_server_error() => {
            let (parts, body) = response.into_parts();
            let body_bytes = to_bytes(body, usize::MAX).await?;
            *last_error = Some((parts.status, parts.headers, body_bytes));

            warn!("Received retriable server error {}.", s);
            let config = state.config.read().await;
            if internal_retry_count >= config.internal_retries {
                error!("Internal retries exhausted. Marking key as temporarily unavailable.");
                let temporary_block_minutes = config.temporary_block_minutes;
                apply_key_action(state, api_key_to_mark, |km, key| {
                    km.mark_key_as_temporarily_unavailable(
                        key,
                        chrono::Duration::minutes(temporary_block_minutes),
                    )
                })
                .await
            } else {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                Ok(NextAction::ContinueLoop)
            }
        }
        _ => {
            warn!("Received unexpected status code {}. Not retrying.", status);
            Ok(NextAction::ReturnResponse(response))
        }
    }
}

/// The main Axum handler for proxying requests.
#[instrument(name="proxy_handler", skip(state, req), fields(uri = %req.uri(), method = %req.method()))]
pub async fn proxy_handler(
    State(state): State<Arc<AppState>>,
    req: Request,
) -> Result<Response> {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let headers = req.headers().clone();
    let body_bytes = match to_bytes(req.into_body(), usize::MAX).await {
        Ok(bytes) => bytes,
        Err(e) => {
            error!(error = ?e, "Failed to buffer request body");
            return Err(AppError::RequestBodyError(e.to_string()));
        }
    };

    let mut last_error: Option<(StatusCode, HeaderMap, Bytes)> = None;
    let mut attempt_count = 0;

    loop {
        attempt_count += 1;
        debug!(attempt = attempt_count, "Looking for next available API key");

        let key_info = state.key_manager.read().await.get_next_available_key_info();

        let Some(key_info) = key_info else {
            warn!(attempts = attempt_count, "No available API keys remaining.");
            return if let Some((status, headers, body)) = last_error {
                let mut response = Response::new(axum::body::Body::from(body));
                *response.status_mut() = status;
                *response.headers_mut() = headers;
                Ok(response)
            } else {
                Err(AppError::NoAvailableKeys)
            };
        };

        let api_key_to_mark = key_info.key.clone();
        let mut internal_retry_count = 0;

        loop {
            internal_retry_count += 1;
            debug!(attempt = attempt_count, internal_attempt = internal_retry_count, "Attempting request with key");

            let translated_path = if uri.path() == "/health/detailed" {
                "/v1beta/models".to_string()
            } else if let Some(stripped) = uri.path().strip_prefix("/v1/") {
                match stripped {
                    s if s.starts_with("chat/completions") => format!("/v1beta/openai/{s}"),
                    s if s.starts_with("embeddings") => format!("/v1beta/{s}"),
                    s if s.starts_with("audio/speech") => format!("/v1beta/{s}"),
                    _ => format!("/v1beta/openai/{stripped}"),
                }
            } else {
                uri.path().to_string()
            };

            let base_url = Url::parse(&key_info.target_url)?;
            let mut final_target_url = base_url.join(&translated_path)?;
            final_target_url.set_query(uri.query());
            final_target_url.query_pairs_mut().append_pair("key", &key_info.key);

            let response_result = proxy::forward_request(
                &state, &key_info, method.clone(), final_target_url, headers.clone(), body_bytes.clone(),
            ).await;

            let response = match response_result {
                Ok(res) => res,
                Err(e) => {
                    error!(error = ?e, "Failed to forward request");
                    // Treat request forwarding errors as temporary unavailability of the key/service
                    let mut km = state.key_manager.write().await;
                    let temporary_block_minutes = state.config.read().await.temporary_block_minutes;
                    if km.mark_key_as_temporarily_unavailable(
                        &api_key_to_mark,
                        chrono::Duration::minutes(temporary_block_minutes),
                    ) {
                        km.save_states().await?;
                    }
                    break;
                }
            };

            match handle_upstream_response(response, &state, &api_key_to_mark, internal_retry_count, &mut last_error).await? {
                NextAction::ReturnResponse(r) => return Ok(r),
                NextAction::BreakLoop => break,
                NextAction::ContinueLoop => {
                    // Clear last_error on successful internal retry
                    last_error.take();
                    continue;
                }
            }
        }
    }
}
