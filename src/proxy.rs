// src/proxy.rs

use crate::{
    error::{AppError, Result},
    key_manager::FlattenedKeyInfo,
    state::AppState, // Import AppState
};
use axum::{
    body::{Body, Bytes},
    http::{header, HeaderMap, HeaderValue, Method}, // Removed unused StatusCode
    response::Response,
};

use futures_util::TryStreamExt;
use std::error::Error; // Import Error trait for source()
use std::time::Instant; // Added Instant
use tracing::{debug, error, info, trace, warn};
use url::Url; // Keep Url

// Hop-by-hop headers that should not be forwarded
const HOP_BY_HOP_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
    "host",           // Explicitly include host as it's often added by clients
    "authorization",  // Original authorization should be replaced
    "x-goog-api-key", // Original key (if any) should be replaced
];
/// Takes incoming request components and forwards them to the appropriate upstream target.
///
/// Orchestrates the core proxying logic. Rate limit handling (429) is delegated to the calling handler.
/// Assumes this function is called within a tracing span that includes `request_id`.
///
/// # Errors
///
/// This function will return an error if:
/// - The base URL from configuration is invalid.
/// - The final target URL cannot be constructed.
/// - The HTTP client for the required proxy cannot be retrieved.
/// - The request to the target fails (e.g., network error, timeout).
/// - The response body stream from the target has an error.
/// - The final response to the client cannot be constructed.
pub async fn forward_request(
    state: &AppState,
    key_info: &FlattenedKeyInfo,
    method: Method,
    target_url: Url,
    headers: HeaderMap,
    body_bytes: Bytes,
) -> Result<Response> {
    let api_key = &key_info.key;
    let proxy_url_option = key_info.proxy_url.as_deref();
    let group_name = &key_info.group_name;
    let request_key_preview = format!("{}...", api_key.chars().take(4).collect::<String>());

    let final_target_url = target_url;
    let outgoing_method = method;
    let mut outgoing_headers = build_forward_headers(&headers, api_key)?;

    // --- Body Modification ---
    let body_to_send =
        if let Some(top_p_value) = state.config.server.top_p {
            // Attempt to modify the body only if top_p is configured.
            // The logic is wrapped in a closure to easily return the original bytes on any failure.
            let modified_body_bytes = (|| {
                if let Ok(mut json_body) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
                    if let Some(obj) = json_body.as_object_mut() {
                        obj.insert("top_p".to_string(), serde_json::json!(top_p_value));
                        if let Ok(modified_bytes) = serde_json::to_vec(&json_body) {
                            info!(top_p = top_p_value, "Successfully injected top_p into request body");
                            return Bytes::from(modified_bytes);
                        } else {
                            warn!("Failed to re-serialize JSON body after injecting top_p, forwarding original body.");
                        }
                    } else {
                        debug!("Request body is valid JSON but not an object, cannot inject top_p.");
                    }
                } else {
                    debug!("Request body is not valid JSON, cannot inject top_p.");
                }
                body_bytes.clone() // Return original on any failure
            })();

            // If the body was modified, update the Content-Length header.
            if modified_body_bytes.len() != body_bytes.len() {
                let new_length = modified_body_bytes.len();
                debug!(old_len = body_bytes.len(), new_len = new_length, "Updating Content-Length due to body modification");
                outgoing_headers.insert(header::CONTENT_LENGTH, HeaderValue::from(new_length));
            }

            modified_body_bytes
        } else {
            body_bytes
        };

    let outgoing_reqwest_body = reqwest::Body::from(body_to_send);
    // --- End Body Modification ---
    // --- Get Client ---
    let http_client = state.get_client(proxy_url_option)?; // Error handled within
                                                           // ---

    // Log before sending the request
    info!(
        http.method = %outgoing_method,
        target.url = %final_target_url,
        http.headers = ?outgoing_headers,
        api_key.preview = %request_key_preview,
        group.name = %group_name,
        proxy.url = ?proxy_url_option, // Use debug formatting for Option<&str>
        "Forwarding raw request to target"
    );

    // --- Send request ---
    let start_time = Instant::now();

    let target_response_result = http_client
        .request(outgoing_method.clone(), final_target_url.clone()) // Clone Url for request
        .headers(outgoing_headers)
        .body(outgoing_reqwest_body)
        .send()
        .await;

    let elapsed_time = start_time.elapsed(); // Calculate duration immediately after await

    let target_response = match target_response_result {
        Ok(resp) => {
            let status = resp.status();
            // Structured success log
            info!(
                // Use standard semantic convention fields where possible
                http.status_code = status.as_u16(),
                http.response.duration = ?elapsed_time, // Use standard field name if available in log aggregator
                target.url = %final_target_url,
                api_key.preview = %request_key_preview,
                group.name = %group_name,
                proxy.url = ?proxy_url_option,
                "Received response from target"
            );
            resp // Return the response
        }
        Err(e) => {
            // Structured error log, trying to extract more detail from reqwest::Error
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
            // Use the imported Error trait to call source()
            let underlying_source = e.source().map(ToString::to_string); // Get underlying error if available

            error!(
                error = %e, // Display format for top-level error
                error.kind = error_kind,
                error.source = ?underlying_source, // Debug format for underlying source
                http.response.duration = ?elapsed_time,
                target.url = %final_target_url, // Log target URL on error too
                api_key.preview = %request_key_preview,
                group.name = %group_name,
                proxy.url = ?proxy_url_option,
                "Error received while sending request to target"
            );
            // Return the error wrapped in AppError
            return Err(AppError::Reqwest(e));
        }
    };
    // --- End Send Request ---

    let response_status = target_response.status();
    let response_headers = build_response_headers(target_response.headers());

    // Stream response body back
    let captured_response_status = response_status; // Capture status for closure
    let response_body_stream = target_response.bytes_stream().map_err(move |e| {
        // Log error during stream reading
        warn!(
            status = captured_response_status.as_u16(),
            error = %e,
            "Error reading upstream response body stream"
        );
        AppError::ResponseBodyError(format!(
            "Upstream body stream error (status {captured_response_status}): {e}"
        ))
    });
    let axum_response_body = Body::from_stream(response_body_stream);

    // Build final response to client
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

/// Creates the `HeaderMap` for the outgoing request to the target service.
/// Now returns a Result to handle potential errors from add_auth_headers.
#[tracing::instrument(level="debug", skip(original_headers, api_key), fields(header_count = original_headers.len()))]
fn build_forward_headers(original_headers: &HeaderMap, api_key: &str) -> Result<HeaderMap> {
    let mut filtered = HeaderMap::with_capacity(original_headers.len() + 3);
    copy_non_hop_by_hop_headers(original_headers, &mut filtered, true);
    add_auth_headers(&mut filtered, api_key)?;
    Ok(filtered)
}


/// Creates the `HeaderMap` for the response sent back to the original client.
#[tracing::instrument(level="debug", skip(original_headers), fields(header_count = original_headers.len()))]
fn build_response_headers(original_headers: &HeaderMap) -> HeaderMap {
    let mut filtered = HeaderMap::with_capacity(original_headers.len());
    copy_non_hop_by_hop_headers(original_headers, &mut filtered, false);
    filtered
}

/// Copies headers from `source` to `dest`, excluding hop-by-hop headers.
fn copy_non_hop_by_hop_headers(source: &HeaderMap, dest: &mut HeaderMap, is_request: bool) {
    for (name, value) in source {
        let name_str = name.as_str().to_lowercase();
        // Check against the HOP_BY_HOP_HEADERS list using lowercase comparison
        if HOP_BY_HOP_HEADERS.contains(&name_str.as_str()) {
            trace!(header.name=%name, header.action="skip", context=if is_request {"request"} else {"response"}, "Skipping hop-by-hop or auth header");
        } else {
            dest.insert(name.clone(), value.clone());
            trace!(header.name=%name, header.action="forward", context=if is_request {"request"} else {"response"}, "Forwarding header");
        }
    }
}

/// Adds the necessary authentication headers (`x-goog-api-key` and `Authorization: Bearer`).
/// Returns a Result to indicate potential failures.
#[tracing::instrument(level="debug")] // Removed skip attribute
// Now takes api_key to add the Bearer token
fn add_auth_headers(headers: &mut HeaderMap, api_key: &str) -> Result<()> {
    let auth_value_str = format!("Bearer {}", api_key);
    match HeaderValue::from_str(&auth_value_str) {
        Ok(auth_value) => {
            headers.insert(header::AUTHORIZATION, auth_value);
            trace!(header.name="Authorization", header.action="add", "Added Bearer token");
            Ok(())
        }
        Err(e) => {
            error!(error = %e, "Failed to create Authorization header value");
            Err(AppError::Internal(
                "Failed to construct Authorization header".to_string(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{header, HeaderName, HeaderValue,};
     // Import Error trait for source()

    #[test]
    fn test_build_forward_headers_basic() {
        let mut original_headers = HeaderMap::new();
        original_headers.insert("content-type", HeaderValue::from_static("application/json"));
        original_headers.insert("x-custom-header", HeaderValue::from_static("value1"));
        original_headers.insert("host", HeaderValue::from_static("original.host.com")); // Hop-by-hop
        original_headers.insert("connection", HeaderValue::from_static("keep-alive")); // Hop-by-hop
        original_headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer old_token"),
        ); // Auth, should be removed
        original_headers.insert("x-goog-api-key", HeaderValue::from_static("old_key")); // Auth, should be removed

        // api_key is no longer passed as it's handled in URL construction
        let result_headers = build_forward_headers(&original_headers, "test_key").unwrap();

        // Check standard headers are present
        assert_eq!(
            result_headers.get("content-type").unwrap(),
            "application/json"
        );
        assert_eq!(result_headers.get("x-custom-header").unwrap(), "value1");

        // Check hop-by-hop are absent
        assert!(result_headers.get("host").is_none());
        assert!(result_headers.get("connection").is_none());

        // Check that x-goog-api-key is absent, but the new Authorization header is present.
        assert!(result_headers.get("x-goog-api-key").is_none());
        let auth_header = result_headers.get(header::AUTHORIZATION).unwrap();
        assert_eq!(auth_header, "Bearer test_key");
    }

    #[test]
    fn test_build_forward_headers_removes_auth_headers() {
        let mut original_headers = HeaderMap::new();
        original_headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer client_token"),
        );
        original_headers.insert("x-goog-api-key", HeaderValue::from_static("client_key"));
        original_headers.insert("x-custom-header", HeaderValue::from_static("custom_value"));

        let result_headers = build_forward_headers(&original_headers, "some_key").unwrap();

        assert!(result_headers.get("x-goog-api-key").is_none());
        assert_eq!(
            result_headers.get("x-custom-header").unwrap(),
            "custom_value"
        );
        assert_eq!(result_headers.len(), 2);
    }

    // Removed test: test_build_forward_headers_invalid_key_chars

    #[test]
    fn test_build_response_headers_filters_hop_by_hop() {











        let mut upstream_headers = HeaderMap::new();
        upstream_headers.insert("content-type", HeaderValue::from_static("text/plain"));
        upstream_headers.insert("x-upstream-specific", HeaderValue::from_static("value2"));
        upstream_headers.insert("transfer-encoding", HeaderValue::from_static("chunked")); // Hop-by-hop
        upstream_headers.insert("connection", HeaderValue::from_static("close")); // Hop-by-hop
        upstream_headers.insert(
            HeaderName::from_static("keep-alive"),
            HeaderValue::from_static("timeout=15"),
        ); // Hop-by-hop (case insensitive check needed)

        let result_headers = build_response_headers(&upstream_headers);

        // Check standard headers are present
        assert_eq!(result_headers.get("content-type").unwrap(), "text/plain");
        assert_eq!(result_headers.get("x-upstream-specific").unwrap(), "value2");

        // Check hop-by-hop are absent
        assert!(result_headers.get("transfer-encoding").is_none());
        assert!(result_headers.get("connection").is_none());
        assert!(result_headers.get("keep-alive").is_none());
    }

    #[test]
    fn test_copy_non_hop_by_hop_headers_request() {
        let mut source = HeaderMap::new();
        source.insert("content-type", HeaderValue::from_static("application/json"));
        source.insert("host", HeaderValue::from_static("example.com")); // Hop-by-hop
        source.insert("authorization", HeaderValue::from_static("Bearer old")); // Hop-by-hop (for request)
        source.insert("x-custom", HeaderValue::from_static("custom"));

        let mut dest = HeaderMap::new();
        copy_non_hop_by_hop_headers(&source, &mut dest, true); // is_request = true

        assert!(dest.contains_key("content-type"));
        assert!(dest.contains_key("x-custom"));
        assert!(!dest.contains_key("host"));
        assert!(!dest.contains_key("authorization")); // Should be filtered for request
        assert_eq!(dest.len(), 2);
    }

    #[test]
    fn test_copy_non_hop_by_hop_headers_response() {
        let mut source = HeaderMap::new();
        source.insert("content-type", HeaderValue::from_static("application/json"));
        source.insert("transfer-encoding", HeaderValue::from_static("chunked")); // Hop-by-hop
        source.insert("connection", HeaderValue::from_static("close")); // Hop-by-hop
        source.insert("x-custom", HeaderValue::from_static("custom"));

        let mut dest = HeaderMap::new();
        copy_non_hop_by_hop_headers(&source, &mut dest, false); // is_request = false

        assert!(dest.contains_key("content-type"));
        assert!(dest.contains_key("x-custom"));
        assert!(!dest.contains_key("transfer-encoding"));
        assert!(!dest.contains_key("connection"));
        assert_eq!(dest.len(), 2);
    }
}
