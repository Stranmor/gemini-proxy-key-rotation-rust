// src/proxy.rs

use crate::{
    circuit_breaker::{CircuitBreaker, CircuitBreakerError},
    error::{AppError, Result},
    key_manager::FlattenedKeyInfo,
};
use secrecy::ExposeSecret;
use axum::{
    body::{Body, Bytes},
    http::{HeaderMap, HeaderValue, Method, header},
    response::Response,
};
use futures_util::TryStreamExt;
use once_cell::sync::Lazy; // Added for efficient static HashSet
use std::collections::HashSet; // Added for HashSet
use std::error::Error;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info, trace, warn};
use url::Url;

// Using a Lazy<HashSet> for O(1) average time complexity lookups.
// This avoids allocating a new lowercase string for every header on every request.
static HOP_BY_HOP_HEADERS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailers",
        "transfer-encoding",
        "upgrade",
        "host",
        "authorization",
        "x-goog-api-key",
    ]
    .into_iter()
    .collect()
});

/// Takes incoming request components and forwards them to the appropriate upstream target.
///
/// Orchestrates the core proxying logic. Assumes this function is called
/// within a tracing span that includes `request_id`.
#[tracing::instrument(
    level = "info",
    skip_all,
    fields(
        http.method = %method,
        target.url = %target_url,
        group.name = %key_info.group_name
    )
)]
pub async fn forward_request(
    client: &reqwest::Client,
    key_info: &FlattenedKeyInfo,
    method: Method,
    target_url: Url,
    headers: HeaderMap,
    body_bytes: Bytes,
    circuit_breaker: Option<Arc<CircuitBreaker>>,
) -> Result<Response> {
    let outgoing_headers = build_forward_headers(&headers, key_info.key.expose_secret())?;

    debug!(
        http.request.body = %String::from_utf8_lossy(&body_bytes),
        "Full request body"
    );

    let outgoing_reqwest_body = reqwest::Body::from(body_bytes);

    info!(
        proxy.url = ?key_info.proxy_url.as_deref(),
        "Forwarding request to target"
    );

    let start_time = Instant::now();
    
    // Execute request through circuit breaker if available
    let target_response_result = if let Some(cb) = circuit_breaker {
        match cb.call(|| async {
            client
                .request(method.clone(), target_url.clone())
                .headers(outgoing_headers.clone())
                .body(outgoing_reqwest_body)
                .send()
                .await
        }).await {
            Ok(response) => Ok(response),
            Err(cb_error) => match cb_error {
                CircuitBreakerError::CircuitOpen => {
                    warn!(target.url = %target_url, "Circuit breaker is open, failing fast");
                    Err(AppError::CircuitBreakerOpen(target_url.to_string()))
                }
                CircuitBreakerError::OperationFailed(req_error) => {
                    Err(AppError::RequestError(req_error.to_string()))
                }
            }
        }
    } else {
        client
            .request(method, target_url.clone())
            .headers(outgoing_headers)
            .body(outgoing_reqwest_body)
            .send()
            .await
            .map_err(|e| AppError::RequestError(e.to_string()))
    };
    
    let elapsed_time = start_time.elapsed();

    // Handle the response from the target, whether success or error
    let target_response = match target_response_result {
        Ok(response) => handle_target_response(Ok(response), elapsed_time, &target_url, key_info)?,
        Err(app_error) => return Err(app_error),
    };

    let response_status = target_response.status();
    let response_headers = build_response_headers(target_response.headers());

    // Process the body differently based on the response status code.
    let axum_response_body =
        if response_status.is_client_error() || response_status.is_server_error() {
            // For 4xx/5xx responses, buffer the body to log it, then forward.
            let body_bytes = read_and_log_error_body(target_response, key_info).await?;
            Body::from(body_bytes)
        } else {
            // For success responses, stream the body directly to the client.
            let captured_response_status = response_status;
            let response_body_stream = target_response.bytes_stream().map_err(move |e| {
                warn!(
                    status = captured_response_status.as_u16(),
                    error = %e,
                    "Error reading upstream response body stream"
                );
                AppError::ResponseBodyError(format!(
                    "Upstream body stream error (status {captured_response_status}): {e}"
                ))
            });
            Body::from_stream(response_body_stream)
        };

    // Build the final response to the original client.
    let mut client_response = Response::builder()
        .status(response_status)
        .body(axum_response_body)
        .map_err(|e| {
            error!(error = %e, "Failed to build final client response");
            AppError::Internal(format!("Failed to construct client response: {e}"))
        })?;

    *client_response.headers_mut() = response_headers;
    Ok(client_response)
}

/// Handles the immediate result of the `reqwest::Client::send` operation.
///
/// Logs success or failure and returns a `reqwest::Response` on success,
/// or an `AppError` on failure.
fn handle_target_response(
    response_result: std::result::Result<reqwest::Response, reqwest::Error>,
    elapsed_time: std::time::Duration,
    target_url: &Url,
    key_info: &FlattenedKeyInfo,
) -> Result<reqwest::Response> {
    let request_key_preview = format!("{}...", key_info.key.expose_secret().chars().take(4).collect::<String>());

    match response_result {
        Ok(resp) => {
            info!(
                http.status_code = resp.status().as_u16(),
                http.response.duration = ?elapsed_time,
                api_key.preview = %request_key_preview,
                "Received response from target"
            );
            Ok(resp)
        }
        Err(e) => {
            let error_kind = if e.is_timeout() {
                "timeout"
            } else if e.is_connect() {
                "connect"
            } else if e.is_redirect() {
                "redirect_policy"
            } else if e.is_request() {
                "request_error"
            } else if e.is_body() || e.is_decode() {
                "body/decode"
            } else if e.is_builder() {
                "builder"
            } else {
                "unknown"
            };
            let underlying_source = e.source().map(ToString::to_string);

            error!(
                error = %e,
                error.kind = error_kind,
                error.source = ?underlying_source,
                http.response.duration = ?elapsed_time,
                target.url = %target_url,
                api_key.preview = %request_key_preview,
                group.name = %key_info.group_name,
                proxy.url = ?key_info.proxy_url.as_deref(),
                "Error sending request to target"
            );
            Err(AppError::Reqwest(e))
        }
    }
}

/// Reads the body of an error response, logs it, and returns the body bytes.
///
/// The body is truncated if it exceeds `MAX_ERROR_BODY_SIZE` to prevent
/// excessive memory usage.
async fn read_and_log_error_body(
    response: reqwest::Response,
    key_info: &FlattenedKeyInfo,
) -> Result<Bytes> {
    const MAX_ERROR_BODY_SIZE: usize = 64 * 1024; // 64KB limit
    let status = response.status();
    let request_key_preview = format!("{}...", key_info.key.expose_secret().chars().take(4).collect::<String>());

    let full_body = response.bytes().await.map_err(|e| {
        error!(status = status.as_u16(), error = %e, "Failed to read error response body");
        AppError::ResponseBodyError(format!(
            "Failed to read error response body (status {status}): {e}"
        ))
    })?;

    let (truncated, body_to_log) = if full_body.len() > MAX_ERROR_BODY_SIZE {
        (true, &full_body[..MAX_ERROR_BODY_SIZE])
    } else {
        (false, &full_body[..])
    };

    let response_body_text = String::from_utf8_lossy(body_to_log);
    let truncated_msg = if truncated { " [TRUNCATED]" } else { "" };

    warn!(
        status = status.as_u16(),
        response_body = %format!("{}{}", response_body_text, truncated_msg),
        body_size = full_body.len(),
        api_key.preview = %request_key_preview,
        group.name = %key_info.group_name,
        "Error response received from target"
    );

    Ok(full_body)
}

/// Creates the `HeaderMap` for the outgoing request to the target service.
#[tracing::instrument(level="debug", skip(original_headers, api_key), fields(header_count = original_headers.len()))]
fn build_forward_headers(original_headers: &HeaderMap, api_key: &str) -> Result<HeaderMap> {
    let mut filtered = HeaderMap::with_capacity(original_headers.len() + 1); // +1 for Authorization
    copy_non_hop_by_hop_headers(original_headers, &mut filtered);
    add_auth_headers(&mut filtered, api_key)?;
    Ok(filtered)
}

/// Creates the `HeaderMap` for the response sent back to the original client.
#[tracing::instrument(level="debug", skip(original_headers), fields(header_count = original_headers.len()))]
fn build_response_headers(original_headers: &HeaderMap) -> HeaderMap {
    let mut filtered = HeaderMap::with_capacity(original_headers.len());
    copy_non_hop_by_hop_headers(original_headers, &mut filtered);
    filtered
}

/// Copies headers from `source` to `dest`, excluding hop-by-hop headers.
fn copy_non_hop_by_hop_headers(source: &HeaderMap, dest: &mut HeaderMap) {
    for (name, value) in source {
        if !HOP_BY_HOP_HEADERS.contains(name.as_str()) {
            dest.insert(name.clone(), value.clone());
            trace!(header.name=%name, header.action="forward", "Forwarding header");
        } else {
            trace!(header.name=%name, header.action="skip", "Skipping hop-by-hop or auth header");
        }
    }
}

/// Adds the necessary `Authorization: Bearer` header.
#[tracing::instrument(level = "debug", skip(headers, api_key))]
fn add_auth_headers(headers: &mut HeaderMap, api_key: &str) -> Result<()> {
    let auth_value_str = format!("Bearer {api_key}");
    match HeaderValue::from_str(&auth_value_str) {
        Ok(auth_value) => {
            headers.insert(header::AUTHORIZATION, auth_value);
            trace!(header.name = "Authorization", "Added Bearer token");
            Ok(())
        }
        Err(e) => {
            error!(error = %e, "Failed to create Authorization header value from API key");
            Err(AppError::Internal(
                "Failed to construct Authorization header".to_string(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderName, HeaderValue, header};

    #[test]
    fn test_build_forward_headers_basic() {
        let mut original_headers = HeaderMap::new();
        original_headers.insert("content-type", HeaderValue::from_static("application/json"));
        original_headers.insert("x-custom-header", HeaderValue::from_static("value1"));
        original_headers.insert("host", HeaderValue::from_static("original.host.com"));
        original_headers.insert("connection", HeaderValue::from_static("keep-alive"));
        original_headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer old_token"),
        );
        original_headers.insert("x-goog-api-key", HeaderValue::from_static("old_key"));

        let result_headers = build_forward_headers(&original_headers, "test_key").unwrap();

        assert_eq!(
            result_headers.get("content-type").unwrap(),
            "application/json"
        );
        assert_eq!(result_headers.get("x-custom-header").unwrap(), "value1");
        assert!(result_headers.get("host").is_none());
        assert!(result_headers.get("connection").is_none());
        assert!(result_headers.get("x-goog-api-key").is_none());
        let auth_header = result_headers.get(header::AUTHORIZATION).unwrap();
        assert_eq!(auth_header, "Bearer test_key");
        assert_eq!(result_headers.len(), 3); // content-type, x-custom-header, authorization
    }

    // This is a new test to verify that invalid characters in a key are handled correctly.
    #[test]
    fn test_add_auth_headers_invalid_chars_in_key() {
        let mut headers = HeaderMap::new();
        let invalid_key = "key-with-\n-invalid-chars";
        let result = add_auth_headers(&mut headers, invalid_key);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Internal(msg) => {
                assert_eq!(msg, "Failed to construct Authorization header");
            }
            _ => panic!("Expected AppError::Internal"),
        }
    }

    #[test]
    fn test_build_response_headers_filters_hop_by_hop() {
        let mut upstream_headers = HeaderMap::new();
        upstream_headers.insert("content-type", HeaderValue::from_static("text/plain"));
        upstream_headers.insert("x-upstream-specific", HeaderValue::from_static("value2"));
        upstream_headers.insert("transfer-encoding", HeaderValue::from_static("chunked"));
        upstream_headers.insert("connection", HeaderValue::from_static("close"));
        upstream_headers.insert(
            HeaderName::from_static("keep-alive"),
            HeaderValue::from_static("timeout=15"),
        );

        let result_headers = build_response_headers(&upstream_headers);

        assert_eq!(result_headers.get("content-type").unwrap(), "text/plain");
        assert_eq!(result_headers.get("x-upstream-specific").unwrap(), "value2");
        assert!(result_headers.get("transfer-encoding").is_none());
        assert!(result_headers.get("connection").is_none());
        assert!(result_headers.get("keep-alive").is_none());
        assert_eq!(result_headers.len(), 2);
    }

    #[test]
    fn test_copy_non_hop_by_hop_headers() {
        let mut source = HeaderMap::new();
        source.insert("content-type", HeaderValue::from_static("application/json"));
        source.insert("host", HeaderValue::from_static("example.com")); // Hop-by-hop
        source.insert("authorization", HeaderValue::from_static("Bearer old")); // Hop-by-hop
        source.insert("x-custom", HeaderValue::from_static("custom"));

        let mut dest = HeaderMap::new();
        copy_non_hop_by_hop_headers(&source, &mut dest);

        assert!(dest.contains_key("content-type"));
        assert!(dest.contains_key("x-custom"));
        assert!(!dest.contains_key("host"));
        assert!(!dest.contains_key("authorization"));
        assert_eq!(dest.len(), 2);
    }
}
