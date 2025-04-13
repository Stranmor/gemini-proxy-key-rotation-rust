use crate::state::AppState; // Import FlattenedKeyInfo
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
    BoxError,
};
use futures_util::TryStreamExt;
use reqwest::RequestBuilder; // Import Proxy and explicitly RequestBuilder
use std::sync::Arc;
use tracing::{debug, error, info, instrument, trace, warn};
// Import Url for parsing proxy scheme

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

#[instrument(skip(state, req), fields(method = %req.method(), uri = %req.uri()))]
pub async fn proxy_handler(
    // Note: state is Arc<AppState>
    State(state): State<Arc<AppState>>,
    req: Request,
) -> Response {
    info!("---> Received request");

    // 1. Get the next available key info (key, target_url, proxy_url, group_name)
    let key_info_option = state.get_next_available_key_info().await;

    let key_info = if let Some(info) = key_info_option {
        let key_preview = format!("{}...", info.key.chars().take(4).collect::<String>());
        debug!(api_key_preview = %key_preview, group = %info.group_name, "Selected available API key info");
        info
    } else {
        warn!("No available API keys (all might be rate-limited).");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "All available API keys are currently rate-limited. Please try again later.",
        )
            .into_response();
    };

    // Extract details from the selected key info
    let api_key = key_info.key.clone();
    let target_base_url_str = &key_info.target_url;
    let proxy_url_option = key_info.proxy_url.clone(); // Keep for logging proxy_used
    let group_name = key_info.group_name.clone();
    let request_key_preview = format!("{}...", api_key.chars().take(4).collect::<String>());

    // 2. Construct the Target URL using the base from key_info
    let original_uri = req.uri();
    let path_and_query = original_uri.path_and_query().map_or("/", |pq| pq.as_str());
    let target_url_str = format!(
        "{}{}",
        target_base_url_str.trim_end_matches('/'),
        path_and_query
    );

    let target_url = match target_url_str.parse::<Uri>() {
        Ok(url) => url,
        Err(e) => {
            error!(target_url = %target_url_str, error = %e, "Failed to parse target URL from group config");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal proxy error: Invalid target URL derived from configuration.",
            )
                .into_response();
        }
    };
    debug!(target = %target_url, "Constructed target URL for request");

    // 3. Prepare the outgoing request for reqwest
    let outgoing_method = req.method().clone();
    let request_headers = req.headers().clone();
    let outgoing_headers = build_forward_headers(&request_headers, &api_key, target_url.host());

    let incoming_body = req.into_body();
    let outgoing_body_stream = incoming_body.into_data_stream().map_err(|e| {
        BoxError::from(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Axum body read error: {e}"),
        ))
    });
    let outgoing_reqwest_body = reqwest::Body::wrap_stream(outgoing_body_stream);

    let http_client = state.client();
    // Start building the request
    let outgoing_request_builder: RequestBuilder = http_client
        .request(outgoing_method.clone(), target_url.to_string())
        .headers(outgoing_headers);

    // --- Apply Proxy if configured for the group ---
    // TEMPORARILY COMMENTED OUT TO DEBUG COMPILATION
    /*

    if let Some(proxy_url_str) = &proxy_url_option {
        match Url::parse(proxy_url_str) {
            Ok(parsed_proxy_url) => {
                let scheme = parsed_proxy_url.scheme().to_lowercase();
                let proxy_to_apply: Option<Result<Proxy, reqwest::Error>> = match scheme.as_str() {
                    "http" => Some(Proxy::http(proxy_url_str)),
                    "https" => Some(Proxy::https(proxy_url_str)),
                    "socks5" => Some(Proxy::all(proxy_url_str)), // Use all for socks5
                    _ => {
                        warn!(proxy_url = %proxy_url_str, group = %group_name, scheme = %scheme, "Unsupported proxy scheme. Proceeding without proxy.");
                        None
                    }
                };

                if let Some(proxy_result) = proxy_to_apply {
                    match proxy_result {
                        Ok(proxy) => {
                            // This is the problematic line
                            // outgoing_request_builder = outgoing_request_builder.proxy(proxy);
                            debug!(proxy_url = %proxy_url_str, scheme = %scheme, group = %group_name, "Applying outgoing proxy for group (CODE COMMENTED OUT)");
                        }
                        Err(e) => {
                            warn!(proxy_url = %proxy_url_str, group = %group_name, scheme = %scheme, error = %e, "Failed to create proxy object from URL. Proceeding without proxy.");
                        }
                    }
                }
            }
            Err(e) => {
                 warn!(proxy_url = %proxy_url_str, group = %group_name, error = %e, "Failed to parse proxy URL string. Proceeding without proxy.");
            }
        }
    }
    */
    // --- End Proxy Application ---

    // Add the body
    let final_request_builder = outgoing_request_builder.body(outgoing_reqwest_body);

    // 4. Send the request using the final builder
    // Use proxy_url_option.is_some() to log if proxy *would* have been used
    info!(method = %outgoing_method, url = %target_url, api_key_preview=%request_key_preview, group=%group_name, proxy_configured=proxy_url_option.is_some(), "Forwarding request to target");
    let target_response_result = final_request_builder.send().await;

    // 5. Process the response from the target
    match target_response_result {
        Ok(target_response) => {
            let response_status = target_response.status();
            info!(status = %response_status, "Received response from target");

            // --- Rate Limit Handling ---
            if response_status == StatusCode::TOO_MANY_REQUESTS {
                warn!(status = %response_status, api_key_preview=%request_key_preview, group=%group_name, "Target API returned 429 Too Many Requests. Marking key as limited.");
                // Call the method on AppState to mark the key
                state.mark_key_as_limited(&api_key).await;
            }
            // --- End Rate Limit Handling ---

            let response_headers = build_response_headers(target_response.headers());
            let response_body_stream = target_response
                .bytes_stream()
                .map_err(|e| BoxError::from(format!("Target response stream error: {e}")));
            let axum_response_body = Body::from_stream(response_body_stream);

            let mut client_response = Response::builder()
                .status(response_status)
                .body(axum_response_body)
                .expect("Failed to build response");
            *client_response.headers_mut() = response_headers;

            info!(status = %response_status, "<--- Sending response to client");
            client_response
        }
        Err(e) => {
            error!(error = %e, url = %target_url, group=%group_name, "Request to target API failed");
            let status_code = if e.is_connect() {
                error!(
                    proxy_url = proxy_url_option.as_deref().unwrap_or("None"),
                    "Connection error - check target URL and proxy settings if used."
                );
                StatusCode::BAD_GATEWAY
            } else if e.is_timeout() {
                StatusCode::GATEWAY_TIMEOUT
            } else if e.is_request() {
                StatusCode::BAD_GATEWAY
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            error!(status = %status_code, "<--- Sending error response to client");
            (status_code, format!("Proxy error: {e}")).into_response()
        }
    }
}

// --- Helper functions ---

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
            // Insert the API key header. No clone needed as key_value is consumed.
            headers.insert("x-goog-api-key", key_value);
            debug!("Added x-goog-api-key header");
            let bearer_value_str = format!("Bearer {api_key}"); // Use inline formatting
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
