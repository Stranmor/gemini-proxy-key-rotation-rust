// src/handler.rs
use crate::{
    error::{AppError, Result},
    key_manager::{FlattenedKeyInfo, KeyManager},
    proxy,
    state::AppState,
};
use axum::{
    body::{Body, Bytes, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::Response,
};
use chrono::Duration;
use std::sync::Arc;
use tracing::{error, info, instrument, warn};
use url::Url;

/// Represents the outcome of a single key attempt.
enum RetryOutcome {
    /// The request was successful.
    Success(Response),
    /// The request failed in a way that warrants trying the next available key.
    /// The associated data is the last error response received.
    RetryNextKey((StatusCode, HeaderMap, Bytes)),
    /// The request failed with a terminal error that should be returned to the client immediately.
    Terminal(Response),
}

#[instrument(name = "health_check", skip_all)]
pub async fn health_check() -> StatusCode {
    StatusCode::OK
}

/* ---------- helpers ---------- */

fn translate_path(path: &str) -> String {
    if path == "/health/detailed" {
        return "/v1beta/models".into();
    }
    if let Some(rest) = path.strip_prefix("/v1/") {
        return match rest {
            r if r.starts_with("chat/completions") => format!("/v1beta/openai/{r}"),
            r if r.starts_with("embeddings") || r.starts_with("audio/speech") => {
                format!("/v1beta/{r}")
            }
            r => format!("/v1beta/openai/{r}"),
        };
    }
    path.to_owned()
}

fn build_target_url(original_uri: &Uri, key_info: &FlattenedKeyInfo) -> Result<Url> {
    let mut url = Url::parse(&key_info.target_url)?.join(&translate_path(original_uri.path()))?;
    url.set_query(original_uri.query());
    url.query_pairs_mut().append_pair("key", &key_info.key);
    Ok(url)
}

async fn mutate_key<F>(state: &Arc<AppState>, key: &str, f: F) -> Result<()>
where
    F: FnOnce(&mut KeyManager, &str),
{
    let mut km = state.key_manager.write().await;
    f(&mut km, key);
    km.save_states().await?;
    Ok(())
}

struct RequestContext<'a> {
    method: &'a Method,
    uri: &'a Uri,
    headers: &'a HeaderMap,
    body: &'a Bytes,
}

async fn retry_with_key(
    state: &Arc<AppState>,
    key_info: &FlattenedKeyInfo,
    req_context: &RequestContext<'_>,
    internal_retries: u32,
) -> Result<RetryOutcome> {
    for attempt in 1..=internal_retries + 1 {
        let url = build_target_url(req_context.uri, key_info)?;

        let response_result = proxy::forward_request(
            state,
            key_info,
            req_context.method.clone(),
            url,
            req_context.headers.clone(),
            req_context.body.clone(),
        )
        .await;

        let response = match response_result {
            Ok(r) => r,
            Err(e) => {
                error!(error = ?e, key = %key_info.key, "Forwarding request failed");
                let block_duration =
                    Duration::minutes(state.config.read().await.temporary_block_minutes);
                mutate_key(state, &key_info.key, |km, k| {
                    km.mark_key_as_temporarily_unavailable(k, block_duration);
                })
                .await?;
                // This is a network-level error with our proxy or the target.
                // It's a retryable offense (try next key).
                // We don't have a response to store, so we fabricate a 502 error.
                let body = Bytes::from(format!("Proxy error: {e}"));
                return Ok(RetryOutcome::RetryNextKey((
                    StatusCode::BAD_GATEWAY,
                    HeaderMap::new(),
                    body,
                )));
            }
        };

        let status = response.status();
        let (parts, body) = response.into_parts();
        let bytes = to_bytes(body, usize::MAX)
            .await
            .map_err(|e| AppError::BodyReadError(e.to_string()))?;

        match status {
            s if s.is_success() => {
                info!(key = %key_info.key, "Request successful");
                return Ok(RetryOutcome::Success(Response::from_parts(
                    parts,
                    Body::from(bytes),
                )));
            }
            StatusCode::NOT_FOUND | StatusCode::GATEWAY_TIMEOUT => {
                warn!(%status, key = %key_info.key, "Received terminal error, not retrying with another key.");
                return Ok(RetryOutcome::Terminal(Response::from_parts(
                    parts,
                    Body::from(bytes),
                )));
            }
            StatusCode::TOO_MANY_REQUESTS => {
                warn!(key = %key_info.key, "Received 429 Too Many Requests. Marking key as limited and trying next.");
                mutate_key(state, &key_info.key, |km, k| {
                    km.mark_key_as_limited(k);
                })
                .await?;
                return Ok(RetryOutcome::RetryNextKey((status, parts.headers, bytes)));
            }
            s if s == StatusCode::BAD_REQUEST => {
                if let Ok(body_str) = std::str::from_utf8(&bytes) {
                    if body_str.contains("API_KEY_INVALID") {
                        warn!(key = %key_info.key, "Marking key as invalid due to API_KEY_INVALID reason in body.");
                        mutate_key(state, &key_info.key, |km, k| {
                            km.mark_key_as_invalid(k);
                        })
                        .await?;
                        return Ok(RetryOutcome::RetryNextKey((s, parts.headers, bytes)));
                    }
                }
                warn!(%s, key = %key_info.key, "Received 400 Bad Request without API_KEY_INVALID. Returning error to client immediately.");
                return Ok(RetryOutcome::Terminal(Response::from_parts(
                    parts,
                    Body::from(bytes),
                )));
            }
            s if s.is_client_error() => {
                warn!(%s, key = %key_info.key, "Received a terminal client error. Returning error to client immediately.");
                return Ok(RetryOutcome::Terminal(Response::from_parts(
                    parts,
                    Body::from(bytes),
                )));
            }
            s if s.is_server_error() => {
                warn!(%s, attempt, key = %key_info.key, "Server error, will retry");
                if attempt > internal_retries {
                    error!(key=%key_info.key, "Internal retries exhausted. Marking key as temporarily unavailable.");
                    let block_duration =
                        Duration::minutes(state.config.read().await.temporary_block_minutes);
                    mutate_key(state, &key_info.key, |km, k| {
                        km.mark_key_as_temporarily_unavailable(k, block_duration);
                    })
                    .await?;
                    return Ok(RetryOutcome::RetryNextKey((s, parts.headers, bytes)));
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
            _ => {
                warn!(%status, "Received unexpected status code, returning as is.");
                return Ok(RetryOutcome::Terminal(Response::from_parts(
                    parts,
                    Body::from(bytes),
                )));
            }
        }
    }
    // This is only reached if the internal retry loop for server errors finishes
    // without returning. We need to return the last error encountered.
    // This part of the logic is complex, for now we assume the loop always returns.
    // A robust implementation would handle this case explicitly.
    Err(AppError::InternalRetryExhausted)
}

/* ---------- main handler ---------- */

#[instrument(skip(state, req), fields(uri = %req.uri(), method = %req.method()))]
pub async fn proxy_handler(State(state): State<Arc<AppState>>, req: Request) -> Result<Response> {
    let (parts, body) = req.into_parts();
    let body_bytes = to_bytes(body, usize::MAX)
        .await
        .map_err(|e| AppError::RequestBodyError(e.to_string()))?;

    let req_context = RequestContext {
        method: &parts.method,
        uri: &parts.uri,
        headers: &parts.headers,
        body: &body_bytes,
    };

    let internal_retries = {
        let config = state.config.read().await;
        config.internal_retries
    };

    let mut last_error: Option<(StatusCode, HeaderMap, Bytes)> = None;

    loop {
        let key_info = state.key_manager.read().await.get_next_available_key_info();

        let key_info = match key_info {
            Some(ki) => ki,
            None => break, // No more keys, break the loop
        };

        let result = retry_with_key(&state, &key_info, &req_context, internal_retries).await?;

        match result {
            RetryOutcome::Success(resp) => return Ok(resp),
            RetryOutcome::Terminal(resp) => return Ok(resp),
            RetryOutcome::RetryNextKey(err) => {
                last_error = Some(err);
                continue;
            }
        }
    }

    // After the loop, if we've broken out due to no keys
    warn!("No available API keys remaining.");
    if let Some((status, headers, body)) = last_error {
        let mut resp = Response::new(Body::from(body));
        *resp.status_mut() = status;
        *resp.headers_mut() = headers;
        Ok(resp)
    } else {
        // This case should ideally not be hit if there was at least one key attempt that failed
        Err(AppError::NoAvailableKeys)
    }
}
