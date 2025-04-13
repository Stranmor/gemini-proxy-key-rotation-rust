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

    // Use the provided uri directly
    let path_and_query = uri.path_and_query().map_or("/", |pq| pq.as_str());
    let target_url_str = format!(
        "{}{}",
        target_base_url_str.trim_end_matches('/'),
        path_and_query
    );

    let target_url = target_url_str.parse::<Uri>().map_err(|e| {
        error!(target_url = %target_url_str, error = %e, "Failed to parse target URL from group config");
        AppError::Internal(format!(
            "Invalid target URL derived from configuration: {}",
            e
        ))
    })?;
    debug!(target = %target_url, "Constructed target URL for request");

    // Use the provided method and headers
    let outgoing_method = method;
    let outgoing_headers = build_forward_headers(&headers, api_key, target_url.host());

    // Convert Bytes to Reqwest body
    let outgoing_reqwest_body = reqwest::Body::from(body_bytes);


    info!(method = %outgoing_method, url = %target_url, api_key_preview=%request_key_preview, group=%group_name, proxy_configured=proxy_url_option.is_some(), "Forwarding request to target");

    // Send request using the appropriate client (direct or proxied)
    // send_request_with_optional_proxy now returns Result<reqwest::Response, AppError>
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
    // Capture response_status for use in the error mapping closure
    let captured_response_status = response_status;
    let response_body_stream = target_response.bytes_stream().map_err(move |e| { // Add 'move'
        // Map reqwest stream error to our AppError, logging the status
        warn!(status = %captured_response_status, error = %e, "Error reading upstream response body stream");
        AppError::ResponseBodyError(format!("Upstream body stream error (status {}): {}", captured_response_status, e))
    });
    // Body::from_stream requires Item = Result<Bytes, E> where E: Into<BoxError>
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
///
/// This function checks if a `proxy_url_str` is provided. If so, it attempts
/// to create a *new*, temporary `reqwest::Client` configured with that proxy.
/// If a proxy is not provided, or if building the proxy client fails, it uses
/// the provided `base_client`.
///
/// # Arguments
///
/// * `base_client` - The shared `reqwest::Client` (used if no proxy or proxy setup fails).
/// * `proxy_url_str` - Optional string slice containing the proxy URL.
/// * `method` - The HTTP method for the outgoing request.
/// * `target_url` - The `Uri` for the outgoing request.
/// * `headers` - The `HeaderMap` for the outgoing request.
/// * `body` - The `reqwest::Body` for the outgoing request.
/// * `group_name` - The name of the key group (for logging).
///
/// # Returns
///
/// Returns a `Result<reqwest::Response, AppError>`, converting potential `reqwest::Error`s.
async fn send_request_with_optional_proxy(
    base_client: &Client,
    proxy_url_str: Option<&str>,
    method: Method,
    target_url: Uri,
    headers: HeaderMap,
    body: reqwest::Body,
    group_name: &str,
) -> Result<reqwest::Response> { // Use the crate's Result alias
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
                        // Use base client directly and map error here
                        return base_client
                            .request(method, &target_url_string)
                            .headers(headers)
                            .body(body)
                            .send()
                            .await
                            .map_err(AppError::from); // Convert reqwest::Error here
                    }
                };

                match proxy_obj_result {
                    Ok(proxy) => {
                        debug!(proxy_url = %proxy_str, scheme = %scheme, group = %group_name, "Attempting to build client with proxy");
                        match Client::builder()
                            .proxy(proxy)
                            .connect_timeout(Duration::from_secs(10)) // Added connect timeout
                            .timeout(Duration::from_secs(300))       // Added request timeout
                            .tcp_keepalive(Some(Duration::from_secs(60))) // Added TCP keep-alive for proxy client
                            .build() {
                             Ok(proxy_client) => {
                                debug!("Sending request via proxy client");
                                proxy_client
                                    .request(method, &target_url_string)
                                    .headers(headers)
                                    .body(body)
                                    .send()
                                    .await
                                    .map_err(AppError::from) // Convert reqwest::Error here
                            }
                            Err(e) => {
                                error!(proxy_url = %proxy_str, group = %group_name, error = %e, "Failed to build reqwest client with proxy. Falling back to default client.");
                                // Fallback to base client and map error here
                                base_client
                                    .request(method, &target_url_string)
                                    .headers(headers)
                                    .body(body)
                                    .send()
                                    .await
                                    .map_err(AppError::from) // Convert reqwest::Error here
                            }
                        }
                    }
                    Err(e) => {
                        warn!(proxy_url = %proxy_str, group = %group_name, scheme = %scheme, error = %e, "Failed to create proxy object from URL. Proceeding without proxy.");
                         // Fallback to base client and map error here
                         base_client
                            .request(method, &target_url_string)
                            .headers(headers)
                            .body(body)
                            .send()
                            .await
                            .map_err(AppError::from) // Convert reqwest::Error here
                    }
                }
            }
            Err(e) => {
                 warn!(proxy_url = %proxy_str, group = %group_name, error = %e, "Failed to parse proxy URL string. Proceeding without proxy.");
                 // Fallback to base client and map error here
                base_client
                    .request(method, &target_url_string)
                    .headers(headers)
                    .body(body)
                    .send()
                    .await
                    .map_err(AppError::from) // Convert reqwest::Error here
            }
        }
    } else {
        // Send using shared base client and map error here
        debug!("Sending request via default client (no proxy configured for group)");
        base_client
            .request(method, &target_url_string)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(AppError::from) // Convert reqwest::Error here
    }
    // No final map_err needed here, errors should be AppError by now
}


/// Creates the `HeaderMap` for the outgoing request to the target service.
///
/// Copies non-hop-by-hop headers from the original request, adds authentication headers
/// (`x-goog-api-key`, `Authorization: Bearer`), and sets the correct `Host` header.
fn build_forward_headers(
    original_headers: &HeaderMap,
    api_key: &str,
    target_host: Option<&str>,
) -> HeaderMap {
    let mut filtered = HeaderMap::with_capacity(original_headers.len() + 3);
    copy_non_hop_by_hop_headers(original_headers, &mut filtered, true);
    add_auth_headers(&mut filtered, api_key);
    add_host_header(&mut filtered, target_host);
    filtered
}

/// Creates the `HeaderMap` for the response sent back to the original client.
///
/// Copies non-hop-by-hop headers from the upstream service's response.
fn build_response_headers(original_headers: &HeaderMap) -> HeaderMap {
    let mut filtered = HeaderMap::with_capacity(original_headers.len());
    copy_non_hop_by_hop_headers(original_headers, &mut filtered, false);
    filtered
}

/// Copies headers from `source` to `dest`, excluding hop-by-hop headers defined in `HOP_BY_HOP_HEADERS`.
/// Also skips auth headers if `is_request` is true, as they are added separately.
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

/// Adds the necessary authentication headers (`x-goog-api-key`, `Authorization: Bearer`) to the outgoing request headers.
#[allow(clippy::cognitive_complexity)] // TODO: Consider refactoring this function for lower complexity
fn add_auth_headers(headers: &mut HeaderMap, api_key: &str) {
     match HeaderValue::from_str(api_key) {
        Ok(key_value) => {
            headers.insert("x-goog-api-key", key_value);
            debug!("Added x-goog-api-key header");
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

/// Sets the `Host` header on the outgoing request headers based on the target URL's host.
fn add_host_header(headers: &mut HeaderMap, target_host: Option<&str>) {
     if let Some(host) = target_host {
        match HeaderValue::from_str(host) {
            Ok(host_value) => {
                headers.insert(header::HOST, host_value);
                debug!(host=%host, "Set HOST header");
            }
            Err(e) => {
                warn!(host=%host, error=%e, "Failed to create HeaderValue for HOST header (invalid characters?)");
            }
        }
    } else {
        warn!("Target URL has no host, cannot set HOST header");
    }
}