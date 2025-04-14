// src/proxy.rs

 use crate::{
     error::{AppError, Result},
     key_manager::FlattenedKeyInfo,
     state::AppState, // Import AppState
 };
 use axum::{
     body::{Body, Bytes},
     http::{header, HeaderMap, HeaderValue, Method, Uri},
     response::Response,
 };
 use futures_util::TryStreamExt;
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
     "host", // Explicitly include host as it's often added by clients
     "authorization", // Original authorization should be replaced
     "x-goog-api-key", // Original key (if any) should be replaced
 ];


 /// Takes incoming request components and forwards them to the appropriate upstream target using cached clients.
 ///
 /// Orchestrates the core proxying logic:
 /// - Determines the target URL using base URL and request path/query via `Url::join`.
 /// - Builds outgoing request headers (filtering hop-by-hop, adding auth).
 /// - Retrieves the appropriate pre-configured `reqwest::Client` (direct or proxied) from `AppState`.
 /// - Sends the request using the retrieved client.
 /// - Processes and streams the upstream response back.
 ///
 /// Rate limit handling (429) is delegated to the calling handler.
 ///
 /// # Arguments
 ///
 /// * `state` - A reference to the shared `Arc<AppState>`.
 /// * `key_info` - A reference to the `FlattenedKeyInfo` for the selected API key.
 /// * `method` - The HTTP `Method` of the original request.
 /// * `uri` - The `Uri` of the original request.
 /// * `headers` - The `HeaderMap` from the original request.
 /// * `body_bytes` - The buffered body (`Bytes`) of the original request.
 ///
 /// # Returns
 ///
 /// Returns a `Result<Response, AppError>`.
 pub async fn forward_request(
     state: &AppState,
     key_info: &FlattenedKeyInfo,
     method: Method,
     uri: Uri,
     headers: HeaderMap,
     body_bytes: Bytes,
 ) -> Result<Response> {
     let api_key = &key_info.key;
     let target_base_url_str = &key_info.target_url;
     let proxy_url_option = key_info.proxy_url.as_deref();
     let group_name = &key_info.group_name;
     let request_key_preview = format!("{}...", api_key.chars().take(4).collect::<String>());

     // --- Corrected URL Construction Logic (v13 - Clean re-apply of relative join) ---
     let base_url = Url::parse(target_base_url_str).map_err(|e| {
         error!(target_base_url = %target_base_url_str, error = %e, "Failed to parse target_base_url from config");
         AppError::Internal(format!("Invalid base URL in config: {}", e))
     })?;
     // +++ Add logging to verify parsed base_url +++
     debug!(parsed_base_url = %base_url, "Parsed base URL from config");
     // +++ End of added logging +++


     // Get the full path and query from the original request URI. Use "" if none.
     let original_path_and_query = uri.path_and_query().map_or("", |pq| pq.as_str());

     // Trim the leading '/' to make the path relative for join. If path was "/", this becomes "".
     let relative_path_and_query = original_path_and_query.trim_start_matches('/');

     // Use url::Url::join. This should append relative_path_and_query to the base_url's path correctly.
     let final_target_url = base_url.join(relative_path_and_query).map_err(|e| {
          error!(base = %base_url, relative_path = %relative_path_and_query, error = %e, "Failed to join base URL and relative path");
          AppError::Internal(format!("URL construction error: {}", e))
     })?;
     // --- End of Corrected URL Construction Logic ---

     debug!(target = %final_target_url, "Constructed target URL for request");

     let outgoing_method = method;
     let outgoing_headers = build_forward_headers(&headers, api_key)?;
     let outgoing_reqwest_body = reqwest::Body::from(body_bytes);

     // --- Get the appropriate client from AppState ---
     let http_client = state.get_client(proxy_url_option)?;
     // ---

     info!(method = %outgoing_method, url = %final_target_url, api_key_preview=%request_key_preview, group=%group_name, proxy_used=?proxy_url_option, "Forwarding request to target");

     // --- Send request using the retrieved client ---
     let start_time = std::time::Instant::now();
     // Use info level for initiation as it's a key step. Keep group/proxy info.
     tracing::info!(
         target_url = %final_target_url,
         api_key_preview = %request_key_preview,
         group = %group_name,
         proxy_used = ?proxy_url_option,
         "Initiating request to target via proxy"
     );

     let target_response_result = http_client
         .request(outgoing_method, final_target_url.clone()) // Clone Url for request
         .headers(outgoing_headers)
         .body(outgoing_reqwest_body)
         .send()
         .await;

     let elapsed_time = start_time.elapsed(); // Calculate duration immediately after await

     let target_response = match target_response_result {
         Ok(resp) => {
             let status = resp.status();
             // Log success with duration
             tracing::info!(
                 status = %status,
                 duration = ?elapsed_time,
                 target_url = %final_target_url,
                 api_key_preview = %request_key_preview,
                 group = %group_name,
                 proxy_used = ?proxy_url_option,
                 "Received successful response from target"
             );
             resp // Return the response
         }
         Err(e) => {
             // Log error with duration
             error!(
                 error = %e,
                 duration = ?elapsed_time,
                 target_url = %final_target_url,
                 api_key_preview = %request_key_preview,
                 group = %group_name,
                 proxy_used = ?proxy_url_option,
                 "Received error from target"
             );
             // Return the error wrapped in AppError
             return Err(AppError::Reqwest(e));
         }
     };
     // ---

     let response_status = target_response.status();
     // info!(status = %response_status, "Received response from target"); // Removed: Now logged within the match arms with duration

     let response_headers = build_response_headers(target_response.headers());

     // Stream response body back
     let captured_response_status = response_status;
     let response_body_stream = target_response.bytes_stream().map_err(move |e| {
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


 /// Creates the `HeaderMap` for the outgoing request to the target service.
 /// Now returns a Result to handle potential errors from add_auth_headers.
 fn build_forward_headers(
     original_headers: &HeaderMap,
     api_key: &str,
 ) -> Result<HeaderMap> {
     let mut filtered = HeaderMap::with_capacity(original_headers.len() + 3); // +3 for potential additions
     copy_non_hop_by_hop_headers(original_headers, &mut filtered, true);
     add_auth_headers(&mut filtered, api_key)?;
     Ok(filtered)
 }

 /// Creates the `HeaderMap` for the response sent back to the original client.
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
             trace!(header=%name, "Skipping hop-by-hop or auth header");
         } else {
             dest.insert(name.clone(), value.clone());
             trace!(header=%name, "Forwarding {} header", if is_request {"request"} else {"response"});
         }
     }
 }

 /// Adds the necessary authentication headers (`x-goog-api-key` and `Authorization: Bearer`).
 /// Returns a Result to indicate potential failures.
 #[allow(clippy::cognitive_complexity)]
 fn add_auth_headers(headers: &mut HeaderMap, api_key: &str) -> Result<()> {
      let key_value = HeaderValue::from_str(api_key).map_err(|e| {
          error!(error=%e, "Failed to create HeaderValue for x-goog-api-key (invalid characters in key?)");
          AppError::Internal(format!("Invalid API key format for header: {}", e))
      })?;

     // Use insert to potentially overwrite existing headers if present after filtering logic refinement
     headers.insert("x-goog-api-key", key_value.clone());
     debug!("Added/updated x-goog-api-key header");

     let bearer_value_str = format!("Bearer {}", api_key);
     let bearer_value = HeaderValue::from_str(&bearer_value_str).map_err(|e| {
         error!(error=%e, "Failed to create HeaderValue for Authorization header");
         AppError::Internal(format!("Failed to create Bearer token header: {}", e))
     })?;

     headers.insert(header::AUTHORIZATION, bearer_value);
     debug!("Added/updated Authorization: Bearer header");

     Ok(())
 }


 #[cfg(test)]
 mod tests {
     use super::*;
     use axum::http::{header, HeaderName, HeaderValue}; // Correct imports

     #[test]
     fn test_build_forward_headers_basic() {
         let mut original_headers = HeaderMap::new();
         original_headers.insert("content-type", HeaderValue::from_static("application/json"));
         original_headers.insert("x-custom-header", HeaderValue::from_static("value1"));
         original_headers.insert("host", HeaderValue::from_static("original.host.com")); // Hop-by-hop
         original_headers.insert("connection", HeaderValue::from_static("keep-alive")); // Hop-by-hop
         original_headers.insert("authorization", HeaderValue::from_static("Bearer old_token")); // Auth, should be removed
         original_headers.insert("x-goog-api-key", HeaderValue::from_static("old_key")); // Auth, should be removed

         let api_key = "new_test_key";
         let result_headers = build_forward_headers(&original_headers, api_key).unwrap();

         // Check standard headers are present
         assert_eq!(result_headers.get("content-type").unwrap(), "application/json");
         assert_eq!(result_headers.get("x-custom-header").unwrap(), "value1");

         // Check hop-by-hop are absent
         assert!(result_headers.get("host").is_none());
         assert!(result_headers.get("connection").is_none());

         // Check auth headers are added/overwritten
         assert_eq!(result_headers.get("x-goog-api-key").unwrap(), api_key);
         assert_eq!(
             result_headers.get(header::AUTHORIZATION).unwrap().to_str().unwrap(),
             format!("Bearer {}", api_key)
         );

         // Check original auth headers are absent
         assert!(result_headers.iter().all(|(k, v)| v != "Bearer old_token" || k == header::AUTHORIZATION));
         assert!(result_headers.iter().all(|(k, v)| v != "old_key" || k == "x-goog-api-key"));
     }

     #[test]
     fn test_build_forward_headers_invalid_key_chars() {
         let original_headers = HeaderMap::new();
         let api_key_with_newline = "key\nwith\ninvalid\nchars";

         let result = build_forward_headers(&original_headers, api_key_with_newline);
         assert!(result.is_err());
         assert!(matches!(result, Err(AppError::Internal(_))));
         if let Err(AppError::Internal(msg)) = result {
             assert!(msg.contains("Invalid API key format for header"));
         }
     }

     #[test]
     fn test_build_response_headers_filters_hop_by_hop() {
         let mut upstream_headers = HeaderMap::new();
         upstream_headers.insert("content-type", HeaderValue::from_static("text/plain"));
         upstream_headers.insert("x-upstream-specific", HeaderValue::from_static("value2"));
         upstream_headers.insert("transfer-encoding", HeaderValue::from_static("chunked")); // Hop-by-hop
         upstream_headers.insert("connection", HeaderValue::from_static("close")); // Hop-by-hop
         upstream_headers.insert(HeaderName::from_static("keep-alive"), HeaderValue::from_static("timeout=15")); // Hop-by-hop (case insensitive check needed)

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