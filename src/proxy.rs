// src/proxy.rs

use crate::{
    error::{AppError, Result},
    key_manager::FlattenedKeyInfo, // Import FlattenedKeyInfo from key_manager

};
use axum::{
    body::Body,
    extract::Request, // Use original Request type
    http::{header, HeaderMap, HeaderValue, Method, Uri},
    response::Response,
    BoxError,
};
use futures_util::TryStreamExt;
use reqwest::{Client, Proxy};
use tracing::{debug, error, info, trace, warn}; // Removed instrument as it's on handler
use url::Url;

// --- Constants and Helpers moved from handler.rs ---

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

/// Forwards the incoming request to the target URL specified in KeyInfo,
/// handling proxy settings and header manipulation.
/// Returns the processed response from the target or an AppError.
pub async fn forward_request(
    // Take base client and key info by reference
    http_client: &Client, // Pass the base client directly
    key_info: &FlattenedKeyInfo,
    req: Request, // Consume the original request
) -> Result<Response> {
    // Extract details from the selected key info
    let api_key = &key_info.key; // Borrow key
    let target_base_url_str = &key_info.target_url;
    let proxy_url_option = key_info.proxy_url.as_deref(); // Borrow proxy URL string option
    let group_name = &key_info.group_name; // Borrow group name
    let request_key_preview = format!("{}...", api_key.chars().take(4).collect::<String>());

    // 2. Construct the Target URL
    let original_uri = req.uri();
    let path_and_query = original_uri.path_and_query().map_or("/", |pq| pq.as_str());
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

    // 3. Prepare the outgoing request parts
    let outgoing_method = req.method().clone();
    let request_headers = req.headers().clone();
    let outgoing_headers = build_forward_headers(&request_headers, api_key, target_url.host());

    // Convert Axum body to Reqwest body using wrap_stream
    let outgoing_body_stream = req.into_body().into_data_stream().map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Axum body read error: {}", e),
        )
    });
    let outgoing_reqwest_body = reqwest::Body::wrap_stream(outgoing_body_stream);

    // 4. Build and send the request using the helper function
    info!(method = %outgoing_method, url = %target_url, api_key_preview=%request_key_preview, group=%group_name, proxy_configured=proxy_url_option.is_some(), "Forwarding request to target");

    let target_response = send_request_with_optional_proxy(
        http_client, // Pass the client reference
        proxy_url_option,
        outgoing_method,
        target_url.clone(),
        outgoing_headers,
        outgoing_reqwest_body,
        group_name,
    )
    .await?; // Propagate AppError::Reqwest if sending fails

    // 5. Process the successful response from the target
    let response_status = target_response.status();
    info!(status = %response_status, "Received response from target");

    // NOTE: Rate limit handling (checking status 429 and marking key)
    // is moved to the caller (proxy_handler) because it needs access
    // to the KeyManager state. This function just forwards the response.

    let response_headers = build_response_headers(target_response.headers());

    // Stream response body
    let response_body_stream = target_response
        .bytes_stream()
        .map_err(|e| BoxError::from(format!("Target response stream error: {e}")));
    let axum_response_body = Body::from_stream(response_body_stream);

    // Build the final response to the client
    let mut client_response = Response::builder()
        .status(response_status)
        .body(axum_response_body)
        .map_err(|e| {
            error!("Failed to build response: {}", e);
            AppError::Internal(format!("Failed to construct client response: {}", e))
        })?;

    *client_response.headers_mut() = response_headers;

    // Don't log "<--- Sending response" here, let the handler do it.
    Ok(client_response)
}

// --- Helper Functions (moved from handler.rs) ---

/// Sends the request using the appropriate client (proxy or default).
async fn send_request_with_optional_proxy(
    base_client: &Client, // Use the base client passed in
    proxy_url_str: Option<&str>,
    method: Method,
    target_url: Uri,
    headers: HeaderMap,
    body: reqwest::Body,
    group_name: &str,
) -> std::result::Result<reqwest::Response, AppError> { // Return standard Result with AppError
    let target_url_string = target_url.to_string();

    let response_result = if let Some(proxy_str) = proxy_url_str {
        match Url::parse(proxy_str) {
            Ok(parsed_proxy_url) => {
                let scheme = parsed_proxy_url.scheme().to_lowercase();
                let proxy_obj_result = match scheme.as_str() {
                    "http" => Proxy::http(proxy_str),
                    "https" => Proxy::https(proxy_str),
                    "socks5" => Proxy::all(proxy_str),
                    _ => {
                        warn!(proxy_url = %proxy_str, group = %group_name, scheme = %scheme, "Unsupported proxy scheme. Proceeding without proxy.");
                        // Use base client directly
                        return base_client
                            .request(method, &target_url_string)
                            .headers(headers)
                            .body(body)
                            .send()
                            .await
                            .map_err(AppError::from); // Convert reqwest::Error to AppError
                    }
                };

                match proxy_obj_result {
                    Ok(proxy) => {
                        debug!(proxy_url = %proxy_str, scheme = %scheme, group = %group_name, "Attempting to build client with proxy");
                        // Build a *new* client with this proxy for this request only
                        match Client::builder().proxy(proxy).build() {
                             Ok(proxy_client) => {
                                debug!("Sending request via proxy client");
                                proxy_client
                                    .request(method, &target_url_string)
                                    .headers(headers)
                                    .body(body)
                                    .send()
                                    .await
                            }
                            Err(e) => {
                                error!(proxy_url = %proxy_str, group = %group_name, error = %e, "Failed to build reqwest client with proxy. Falling back to default client.");
                                // Fallback to base client
                                base_client
                                    .request(method, &target_url_string)
                                    .headers(headers)
                                    .body(body)
                                    .send()
                                    .await
                            }
                        }
                    }
                    Err(e) => {
                        warn!(proxy_url = %proxy_str, group = %group_name, scheme = %scheme, error = %e, "Failed to create proxy object from URL. Proceeding without proxy.");
                         // Fallback to base client
                         base_client
                            .request(method, &target_url_string)
                            .headers(headers)
                            .body(body)
                            .send()
                            .await
                    }
                }
            }
            Err(e) => {
                 warn!(proxy_url = %proxy_str, group = %group_name, error = %e, "Failed to parse proxy URL string. Proceeding without proxy.");
                 // Fallback to base client
                base_client
                    .request(method, &target_url_string)
                    .headers(headers)
                    .body(body)
                    .send()
                    .await
            }
        }
    } else {
        // Send using shared base client
        debug!("Sending request via default client (no proxy configured for group)");
        base_client
            .request(method, &target_url_string)
            .headers(headers)
            .body(body)
            .send()
            .await
    };

    // Convert Result<reqwest::Response, reqwest::Error> to Result<reqwest::Response, AppError>
    response_result.map_err(AppError::from)
}

// --- Header manipulation functions (unchanged from handler.rs) ---

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

fn build_response_headers(original_headers: &HeaderMap) -> HeaderMap {
    let mut filtered = HeaderMap::with_capacity(original_headers.len());
    copy_non_hop_by_hop_headers(original_headers, &mut filtered, false);
    filtered
}

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