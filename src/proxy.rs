// src/proxy.rs

use crate::{
    error::{AppError, Result}, // Use the crate's Result alias
    key_manager::FlattenedKeyInfo, // Import FlattenedKeyInfo from key_manager
};
use axum::{
    body::{Body, Bytes}, // Use Bytes
    http::{header, HeaderMap, HeaderValue, Method, Uri}, // Removed Request, added Method, Uri, HeaderMap
    response::Response, // Keep Response type
};
use futures_util::TryStreamExt;
use reqwest::{Client, Proxy};
use std::time::Duration; // Added Duration for timeout settings
use tracing::{debug, error, info, trace, warn};
use url::Url;


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
    "host",
    // Keep auth headers here, they are replaced by add_auth_headers
    "authorization",
    "x-goog-api-key",
];


/// Takes incoming request components and forwards them to the appropriate upstream target.
///
/// This function orchestrates the core proxying logic:
/// - Determines the target URL based on the incoming request path and the key's group info.
/// - Builds the outgoing request headers, filtering hop-by-hop headers and adding authentication.
/// - Creates a `reqwest::Body` from the provided `Bytes`.
/// - Calls `send_request_with_optional_proxy` to actually send the request using the correct HTTP client (direct or proxied).
/// - Processes the upstream response, filtering hop-by-hop headers.
/// - Streams the upstream response body back to the client.
///
/// Rate limit handling (checking for 429) is delegated to the calling handler (`proxy_handler`).
///
/// # Arguments
///
/// * `http_client` - A reference to the shared `reqwest::Client` instance.
/// * `key_info` - A reference to the `FlattenedKeyInfo` for the selected API key.
/// * `method` - The HTTP `Method` of the original request.
/// * `uri` - The `Uri` of the original request.
/// * `headers` - The `HeaderMap` from the original request.
/// * `body_bytes` - The buffered body (`Bytes`) of the original request.
///
/// # Returns
///
/// Returns a `Result<Response, AppError>` containing the response to be sent to the client, or an error.
pub async fn forward_request(
    http_client: &Client,
    key_info: &FlattenedKeyInfo,
    method: Method, // Changed from req: Request
    uri: Uri,       // Changed from req: Request
    headers: HeaderMap, // Changed from req: Request
    body_bytes: Bytes, // Changed from req: Request
) -> Result<Response> { // Use crate::error::Result alias
    let api_key = &key_info.key;
    let target_base_url_str = &key_info.target_url;
    let proxy_url_option = key_info.proxy_url.as_deref();
    let group_name = &key_info.group_name;
    let request_key_preview = format!("{}...", api_key.chars().take(4).collect::<String>());

    // --- Corrected URL Construction Logic (v3) ---
    let original_path_and_query = uri.path_and_query().map_or("/", |pq| pq.as_str());

    // Extract path part and query part
    let (original_path, query) = match original_path_and_query.find('?') {
        Some(index) => original_path_and_query.split_at(index),
        None => (original_path_and_query, ""),
    };

    // Strip known prefixes (/v1, /v1beta) from the path *before* joining
    let path_to_join: &str = original_path
        .strip_prefix("/v1beta")
        .or_else(|| original_path.strip_prefix("/v1"))
        .unwrap_or(original_path); // Keep original if no prefix matches

    // Ensure the path segment starts with a '/' and is a String
    let final_path_segment: String = if path_to_join.is_empty() {
        "/".to_string() // Ensure this branch returns String
    } else if !path_to_join.starts_with('/') {
        format!("/{}", path_to_join) // Returns String
    } else {
        path_to_join.to_string() // Returns String
    };

    // Construct the final target URL string carefully
    let base_trimmed = target_base_url_str.trim_end_matches('/');
    // Remove leading slash from path segment if base already has one (or will get one)
    let path_segment_trimmed = final_path_segment.trim_start_matches('/');

    let target_url_str = format!("{}/{}", base_trimmed, path_segment_trimmed);

    // Append the original query string if it exists
    let final_target_url_str = if query.is_empty() {
        target_url_str
    } else {
        format!("{}{}", target_url_str, query)
    };
    // --- End of Corrected URL Construction Logic (v3) ---


    let target_url = final_target_url_str.parse::<Uri>().map_err(|e| {
        error!(target_url = %final_target_url_str, error = %e, "Failed to parse target URL");
        AppError::Internal(format!(
            "Invalid target URL derived: {}",
            e
        ))
    })?;
    debug!(target = %target_url, "Constructed target URL for request");

    // Use the provided method and headers
    let outgoing_method = method;
    let outgoing_headers = build_forward_headers(&headers, api_key);

    // Convert Bytes to Reqwest body
    let outgoing_reqwest_body = reqwest::Body::from(body_bytes);


    info!(method = %outgoing_method, url = %target_url, api_key_preview=%request_key_preview, group=%group_name, proxy_configured=proxy_url_option.is_some(), "Forwarding request to target");

    // Send request using the appropriate client (direct or proxied)
    let target_response = send_request_with_optional_proxy(
        http_client,
        proxy_url_option,
        outgoing_method, // Use the cloned method
        target_url.clone(), // Clone here as it's used later
        outgoing_headers, // Use the built headers
        outgoing_reqwest_body, // Use the reqwest body
        group_name,
    )
    .await?; // Propagate AppError directly if sending/proxy setup fails

    let response_status = target_response.status();
    info!(status = %response_status, "Received response from target");

    let response_headers = build_response_headers(target_response.headers());

    // Stream response body back, mapping potential stream errors
    let captured_response_status = response_status;
    let response_body_stream = target_response.bytes_stream().map_err(move |e| { // Add 'move'
        warn!(status = %captured_response_status, error = %e, "Error reading upstream response body stream");
        AppError::ResponseBodyError(format!("Upstream body stream error (status {}): {}", captured_response_status, e))
    });
    let axum_response_body = Body::from_stream(response_body_stream);


    let mut client_response = Response::builder()
        .status(response_status)
        .body(axum_response_body)
        .map_err(|e| {
            error!("Failed to build response: {}", e);
            AppError::Internal(format!("Failed to construct client response: {}", e))
        })?;

    *client_response.headers_mut() = response_headers;

    Ok(client_response)
}


/// Helper function to send the constructed `reqwest` request.
/// (Function remains the same as before)
async fn send_request_with_optional_proxy(
    base_client: &Client,
    proxy_url_str: Option<&str>,
    method: Method,
    target_url: Uri,
    headers: HeaderMap,
    body: reqwest::Body,
    group_name: &str,
) -> Result<reqwest::Response> {
    let target_url_string = target_url.to_string();

    if let Some(proxy_str) = proxy_url_str {
        match Url::parse(proxy_str) {
            Ok(parsed_proxy_url) => {
                let scheme = parsed_proxy_url.scheme().to_lowercase();
                let proxy_obj_result = match scheme.as_str() {
                    "http" => Proxy::http(proxy_str),
                    "https" => Proxy::https(proxy_str),
                    "socks5" => Proxy::all(proxy_str),
                    _ => {
                        warn!(proxy_url = %proxy_str, group = %group_name, scheme = %scheme, "Unsupported proxy scheme. Proceeding without proxy.");
                        return base_client
                            .request(method, &target_url_string)
                            .headers(headers)
                            .body(body)
                            .send()
                            .await
                            .map_err(AppError::from);
                    }
                };

                match proxy_obj_result {
                    Ok(proxy) => {
                        debug!(proxy_url = %proxy_str, scheme = %scheme, group = %group_name, "Attempting to build client with proxy");
                        match Client::builder()
                            .proxy(proxy)
                            .connect_timeout(Duration::from_secs(10))
                            .timeout(Duration::from_secs(300))
                            .tcp_keepalive(Some(Duration::from_secs(60)))
                            .build() {
                             Ok(proxy_client) => {
                                debug!("Sending request via proxy client");
                                proxy_client
                                    .request(method, &target_url_string)
                                    .headers(headers)
                                    .body(body)
                                    .send()
                                    .await
                                    .map_err(AppError::from)
                            }
                            Err(e) => {
                                error!(proxy_url = %proxy_str, group = %group_name, error = %e, "Failed to build reqwest client with proxy. Falling back to default client.");
                                base_client
                                    .request(method, &target_url_string)
                                    .headers(headers)
                                    .body(body)
                                    .send()
                                    .await
                                    .map_err(AppError::from)
                            }
                        }
                    }
                    Err(e) => {
                        warn!(proxy_url = %proxy_str, group = %group_name, scheme = %scheme, error = %e, "Failed to create proxy object from URL. Proceeding without proxy.");
                         base_client
                            .request(method, &target_url_string)
                            .headers(headers)
                            .body(body)
                            .send()
                            .await
                            .map_err(AppError::from)
                    }
                }
            }
            Err(e) => {
                 warn!(proxy_url = %proxy_str, group = %group_name, error = %e, "Failed to parse proxy URL string. Proceeding without proxy.");
                base_client
                    .request(method, &target_url_string)
                    .headers(headers)
                    .body(body)
                    .send()
                    .await
                    .map_err(AppError::from)
            }
        }
    } else {
        debug!("Sending request via default client (no proxy configured for group)");
        base_client
            .request(method, &target_url_string)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(AppError::from)
    }
}


/// Creates the `HeaderMap` for the outgoing request to the target service.
/// (Function remains the same as before)
fn build_forward_headers(
    original_headers: &HeaderMap,
    api_key: &str,
) -> HeaderMap {
    let mut filtered = HeaderMap::with_capacity(original_headers.len() + 3);
    // The Host header is handled by reqwest now
    copy_non_hop_by_hop_headers(original_headers, &mut filtered, true);
    add_auth_headers(&mut filtered, api_key);
    filtered
}

/// Creates the `HeaderMap` for the response sent back to the original client.
/// (Function remains the same as before)
fn build_response_headers(original_headers: &HeaderMap) -> HeaderMap {
    let mut filtered = HeaderMap::with_capacity(original_headers.len());
    copy_non_hop_by_hop_headers(original_headers, &mut filtered, false);
    filtered
}

/// Copies headers from `source` to `dest`, excluding hop-by-hop headers defined in `HOP_BY_HOP_HEADERS`.
/// (Function remains the same as before)
fn copy_non_hop_by_hop_headers(source: &HeaderMap, dest: &mut HeaderMap, is_request: bool) {
    for (name, value) in source {
        let name_str = name.as_str().to_lowercase();
        if HOP_BY_HOP_HEADERS.contains(&name_str.as_str()) {
            trace!(header=%name, "Skipping hop-by-hop or auth header (will be replaced/handled)");
        } else {
            dest.insert(name.clone(), value.clone());
            trace!(header=%name, "Forwarding {} header", if is_request {"request"} else {"response"});
        }
    }
}

/// Adds the necessary authentication headers (`x-goog-api-key` and `Authorization: Bearer`) to the outgoing request headers.
/// (Function remains the same as before - adding both headers)
#[allow(clippy::cognitive_complexity)]
fn add_auth_headers(headers: &mut HeaderMap, api_key: &str) {
     match HeaderValue::from_str(api_key) {
        Ok(key_value) => {
            // Add x-goog-api-key
            headers.insert("x-goog-api-key", key_value.clone()); // Clone key_value for bearer
            debug!("Added x-goog-api-key header");

            // Add Authorization: Bearer <key>
            let bearer_value_str = format!("Bearer {api_key}");
            match HeaderValue::from_str(&bearer_value_str) {
                Ok(bearer_value) => {
                    headers.insert(header::AUTHORIZATION, bearer_value);
                    debug!("Added Authorization: Bearer header");
                }
                Err(e) => {
                    warn!(error=%e, "Failed to create HeaderValue for Authorization header (invalid characters?)");
                }
            }
        }
        Err(e) => {
            warn!(error=%e, "Failed to create HeaderValue for x-goog-api-key (invalid characters in key?)");
        }
    }
}