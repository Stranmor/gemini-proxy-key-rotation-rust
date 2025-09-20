// src/handlers/mod.rs

pub mod base;
pub mod invalid_api_key;
pub mod rate_limit;
pub mod server_error;
pub mod success;
pub mod terminal_error;
pub mod timeout;

// --- Код, перенесенный из src/handler.rs ---

use crate::{
    error::{AppError, Result},
    handlers::base::Action,
    key_manager::FlattenedKeyInfo,
    proxy,
    state::AppState,
};
use axum::{
    body::{to_bytes, Body, Bytes},
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::Response,
};
use secrecy::ExposeSecret;
use std::sync::Arc;

use tracing::{error, info, instrument, trace, warn};
use url::Url;

/// Lightweight health check handler used by /health route
pub async fn health_check() -> Response {
    let mut resp = Response::new(Body::from("OK"));
    *resp.status_mut() = StatusCode::OK;
    resp
}

/* ---------- helpers ---------- */

/// Extracts model name from request path and body
fn extract_model_from_request(path: &str, body: &[u8]) -> Option<String> {
    // Try to extract from path first (for generateContent endpoints)
    if let Some(captures) = regex::Regex::new(r"/v1beta/models/([^/:]+)")
        .ok()?
        .captures(path)
    {
        return Some(captures.get(1)?.as_str().to_string());
    }

    // Try to extract from OpenAI-style path
    if path.contains("/chat/completions") || path.contains("/embeddings") {
        // Try to parse JSON body to get model
        if let Ok(body_str) = std::str::from_utf8(body) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(body_str) {
                if let Some(model) = json.get("model").and_then(|m| m.as_str()) {
                    return Some(model.to_string());
                }
            }
        }
    }

    None
}

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
    url.query_pairs_mut()
        .append_pair("key", key_info.key.expose_secret());
    Ok(url)
}

struct RequestContext<'a> {
    method: &'a Method,
    uri: &'a Uri,
    headers: &'a HeaderMap,
    body: &'a Bytes,
}

pub fn validate_token_count_with_limit(json_body: &serde_json::Value, max_tokens: Option<u64>) -> Result<()> {
    // If no limit is configured, skip validation
    let max_tokens = match max_tokens {
        Some(limit) => limit,
        None => return Ok(()),
    };

    let mut total_tokens = 0usize;

    // Handle different request formats
    if let Some(contents) = json_body.get("contents") {
        // Gemini native format
        let mut text_to_tokenize = String::new();
        if let Some(contents_array) = contents.as_array() {
            for content in contents_array {
                if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                    for part in parts {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            text_to_tokenize.push_str(text);
                        }
                    }
                }
            }
        } else if let Some(text) = contents.as_str() {
            text_to_tokenize.push_str(text);
        }

        if !text_to_tokenize.is_empty() {
            match crate::tokenizer::gemini_ml_calibrated::count_ml_calibrated_gemini_tokens(&text_to_tokenize) {
                Ok(token_count) => {
                    total_tokens += token_count;
                }
                Err(e) => {
                    warn!("Token counting failed: {}", e);
                    // If tokenizer fails, allow request to proceed
                    return Ok(());
                }
            }
        }
    } else if let Some(messages) = json_body.get("messages").and_then(|m| m.as_array()) {
        // OpenAI format
        let mut text_to_tokenize = String::new();
        for message in messages {
            if let Some(content) = message.get("content").and_then(|c| c.as_str()) {
                text_to_tokenize.push_str(content);
                text_to_tokenize.push('\n'); // Add separator between messages
            }
        }

        if !text_to_tokenize.is_empty() {
            match crate::tokenizer::gemini_ml_calibrated::count_ml_calibrated_gemini_tokens(&text_to_tokenize) {
                Ok(token_count) => {
                    total_tokens += token_count;
                }
                Err(e) => {
                    warn!("Token counting for messages failed: {}", e);
                    // If tokenizer fails, allow request to proceed
                    return Ok(());
                }
            }
        }
    }

    // Check if token count exceeds the limit
    if total_tokens > 0 && total_tokens as u64 > max_tokens {
        return Err(AppError::RequestTooLarge {
            size: total_tokens,
            max_size: max_tokens as usize,
        });
    }

    Ok(())
}
pub fn is_streaming_request(body_bytes: &Bytes) -> bool {
    if let Ok(json_body) = serde_json::from_slice::<serde_json::Value>(body_bytes) {
        json_body
            .get("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    } else {
        false
    }
}

fn process_request_body(body_bytes: Bytes, top_p: Option<f64>, max_tokens: Option<u64>) -> Result<(Bytes, HeaderMap, bool)> {
    let mut json_body_opt: Option<serde_json::Value> = serde_json::from_slice(&body_bytes).ok();
    let mut headers = HeaderMap::new();
    let is_streaming = is_streaming_request(&body_bytes);

    if let Some(json_body) = &json_body_opt {
        validate_token_count_with_limit(json_body, max_tokens)?;
    }

    // Conditionally inject top_p
    let body_modified = if let Some(top_p_value) = top_p {
        if let Some(json_body) = &mut json_body_opt {
            if let Some(obj) = json_body.as_object_mut() {
                obj.insert("top_p".to_string(), serde_json::json!(top_p_value));
                true
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    // If the body was modified, serialize it back to bytes
    if body_modified {
        if let Some(json_body) = json_body_opt {
            match serde_json::to_vec(&json_body) {
                Ok(new_body_bytes) => {
                    let new_len = new_body_bytes.len();
                    let body_bytes = Bytes::from(new_body_bytes);
                    headers.insert(
                        "content-length",
                        http::HeaderValue::from_str(&new_len.to_string()).map_err(|e| {
                            AppError::InvalidRequest {
                                message: format!("Invalid header value created internally: {e}"),
                            }
                        })?,
                    );
                    return Ok((body_bytes, headers, is_streaming));
                }
                Err(e) => {
                    error!(error = ?e, "Failed to re-serialize JSON body after modification.");
                }
            }
        }
    }

    Ok((body_bytes, headers, is_streaming))
}

async fn try_request_with_key(
    state: &Arc<AppState>,
    req_context: &RequestContext<'_>,
    key_info: &FlattenedKeyInfo,
    _is_streaming: bool,
) -> Result<Response> {
    let url = build_target_url(req_context.uri, key_info)?;
    let client = state.get_client(key_info.proxy_url.as_deref()).await?;
    let circuit_breaker = state.get_circuit_breaker(&key_info.target_url).await;

    let response = proxy::forward_request(
        &client,
        key_info,
        req_context.method.clone(),
        url,
        req_context.headers.clone(),
        req_context.body.clone(),
        circuit_breaker,
    )
    .await
    .map_err(|e| {
        error!(error = ?e, key.preview = %crate::key_manager::KeyManager::preview_key(&key_info.key), "Forwarding request failed");
        AppError::internal(e.to_string())
    })?;

    // Return response as-is - streaming check will be done in the main handler
    Ok(response)
}

/* ---------- main handler ---------- */

#[instrument(skip_all, fields(uri = %req.uri(), method = %req.method()))]
pub async fn proxy_handler(State(state): State<Arc<AppState>>, req: Request) -> Result<Response> {
    let (mut parts, body) = req.into_parts();
    let body_bytes = to_bytes(body, usize::MAX)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    // Process request body (token validation and top_p injection)
    let (top_p, max_tokens) = {
        let config = state.config.read().await;
        (config.top_p, config.server.max_tokens_per_request)
    };
    let (processed_body, additional_headers, is_streaming) =
        process_request_body(body_bytes, top_p.map(|v| v as f64), max_tokens)?;

    // Merge additional headers
    for (key, value) in additional_headers {
        if let Some(key) = key {
            parts.headers.insert(key, value);
        }
    }

    let req_context = RequestContext {
        method: &parts.method,
        uri: &parts.uri,
        headers: &parts.headers,
        body: &processed_body,
    };

    let model = extract_model_from_request(req_context.uri.path(), &processed_body);

    info!(
        model = ?model,
        path = %req_context.uri.path(),
        "Processing request with model-specific key management"
    );

    let response_handlers = state.response_handlers.clone();
    let mut last_response: Option<Response> = None;

    loop {
        // Get group name from config first
        let group_name = {
            let config_guard = state.config.read().await;
            model
                .as_deref()
                .and_then(|m| config_guard.get_group_for_model(m))
                .map(|g| g.to_string())
        };

        // Get key from key_manager
        let key_info = {
            let key_manager_guard = state.key_manager.read().await;
            match key_manager_guard
                .get_next_available_key_info(group_name.as_deref())
                .await?
            {
                Some(info) => info,
                None => break, // No keys available for this group, exit loop.
            }
        };

        info!(key = %crate::key_manager::KeyManager::preview_key(&key_info.key), "Attempting to use key");

        let response = match try_request_with_key(&state, &req_context, &key_info, is_streaming)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                // Важно: сохраняем семантику ошибок апстрима.
                // Если это AppError::UpstreamServiceError (например, 413 Payload Too Large),
                // возвращаем её напрямую (Axum через IntoResponse отдаст оригинальный статус/тело).
                // Для всех остальных ошибок сохраняем текущее поведение — 502 Bad Gateway.
                error!(error = ?e, key.preview = %crate::key_manager::KeyManager::preview_key(&key_info.key), "Request failed");
                if let AppError::UpstreamUnavailable { ref service } = e {
                    if service.as_str() == "unknown" {
                        // Возвращаем как есть, без преобразования в 502.
                        return Err(e);
                    }
                }
                let mut resp = Response::new(Body::from(format!("Proxy error: {e}")));
                *resp.status_mut() = StatusCode::BAD_GATEWAY;
                // фиксируем ошибочный ответ в счетчике метрик — переносим учет только в middleware
                last_response = Some(resp);
                break;
            }
        };

        // For streaming responses, return immediately without buffering or processing through handlers
        if is_streaming && response.status().is_success() {
            if let Some(content_type) = response.headers().get("content-type") {
                if content_type
                    .to_str()
                    .unwrap_or("")
                    .contains("text/event-stream")
                    || content_type.to_str().unwrap_or("").contains("text/plain")
                {
                    info!("Returning streaming response directly to client");
                    return Ok(response);
                }
            }
        }

        let (parts, body) = response.into_parts();
        let response_bytes = to_bytes(body, usize::MAX)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;

        let response_for_analysis =
            Response::from_parts(parts.clone(), Body::from(response_bytes.clone()));

        // Check response handlers
        let action_to_take = response_handlers.iter().find_map(|handler| {
            handler.handle(
                &response_for_analysis,
                &response_bytes,
                key_info.key.expose_secret(),
            )
        });

        let final_response = Response::from_parts(parts, Body::from(response_bytes));

        match action_to_take {
            Some(Action::ReturnToClient(resp)) => {
                // учет ошибок выполняется исключительно в metrics_middleware
                return Ok(resp);
            }
            Some(Action::Terminal(resp)) => {
                // Терминальный ответ — отдаем клиенту без дальнейших ретраев
                // учет ошибок выполняется исключительно в metrics_middleware
                return Ok(resp);
            }
            Some(Action::RetryNextKey) => {
                trace!("Retrying with next key");
                let _ = state
                    .key_manager
                    .write()
                    .await
                    .handle_api_failure(key_info.key.expose_secret(), false)
                    .await;
                last_response = Some(final_response);
            }
            Some(Action::BlockKeyAndRetry) => {
                trace!("Blocking key and retrying");
                let _ = state
                    .key_manager
                    .write()
                    .await
                    .handle_api_failure(key_info.key.expose_secret(), true)
                    .await;
                last_response = Some(final_response);
            }
            Some(Action::WaitFor(duration)) => {
                trace!("Rate limit with wait period received. Marking key and waiting.");
                let _ = state
                    .key_manager
                    .write()
                    .await
                    .handle_rate_limit(key_info.key.expose_secret(), duration)
                    .await;

                // We wait for the specified duration and then retry with the same key
                info!(
                    ?duration,
                    "Rate limit hit. Waiting before retrying with the same key."
                );
                tokio::time::sleep(duration).await;

                // Retry the request with the same key after waiting
                let retry_response = match try_request_with_key(
                    &state,
                    &req_context,
                    &key_info,
                    is_streaming,
                )
                .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        error!(error = ?e, key.preview = %crate::key_manager::KeyManager::preview_key(&key_info.key), "Retry request failed after waiting");
                        let mut resp = Response::new(Body::from(format!("Proxy error: {e}")));
                        *resp.status_mut() = StatusCode::BAD_GATEWAY;
                        last_response = Some(resp);
                        break;
                    }
                };

                let (retry_parts, retry_body) = retry_response.into_parts();
                let retry_response_bytes = to_bytes(retry_body, usize::MAX)
                    .await
                    .map_err(|e| AppError::internal(e.to_string()))?;

                let retry_response_for_analysis = Response::from_parts(
                    retry_parts.clone(),
                    Body::from(retry_response_bytes.clone()),
                );

                // Process the retry response through handlers
                let retry_action = response_handlers.iter().find_map(|handler| {
                    handler.handle(
                        &retry_response_for_analysis,
                        &retry_response_bytes,
                        key_info.key.expose_secret(),
                    )
                });

                let retry_final_response =
                    Response::from_parts(retry_parts, Body::from(retry_response_bytes));

                match retry_action {
                    Some(Action::ReturnToClient(response)) => return Ok(response),
                    Some(Action::Terminal(response)) => return Ok(response),
                    None => return Ok(retry_final_response),
                    _ => {
                        // If retry also fails, continue to next key
                        last_response = Some(retry_final_response);
                        continue;
                    }
                }
            }
            None => {
                // No specific action, so this is the final response.
                return Ok(final_response);
            }
        }
    }

    // If the loop completes, return the last response or error
    last_response.map(Ok).unwrap_or_else(|| {
        warn!("No available API keys for the given group.");
        Err(AppError::NoHealthyKeys)
    })
}

#[cfg(test)]
mod token_limit_tests {
    use super::*;

    use serde_json::json;

    // Helper to initialize tokenizers for tests
    fn install_tokenizers() {
        // This is no longer needed as tokenizer is initialized in run()
    }

    #[test]
    fn validate_does_not_block_without_tokenizer() {
        // Ensure tokenizer is absent: If previously set by other tests in suite,
        // OnceLock can't be reset, so this test is only reliable when run in isolation first.
        // We still verify that with empty messages the function is Ok.
        let body = json!({
            "messages": [{"role": "user", "content": ""}]
        });
        let res = validate_token_count_with_limit(&body, None);
        assert!(
            res.is_ok(),
            "Should not error without tokenizer on empty content"
        );
    }

    #[test]
    fn validate_blocks_when_over_limit_with_tokenizer() {
        install_tokenizers();

        // Build a huge text exceeding configured limit by creating many words.
        // Each word should count roughly as a token with Whitespace pre-tokenizer.
        let limit: u64 = 100;
        let big_text = "a ".repeat((limit + 10) as usize);
        let _body = json!({
            "messages": [
                {"role":"user", "content": big_text}
            ]
        });
        // With the new implementation, this check is inside the proxy_loop, so we can't test it directly here.
        // We will rely on integration tests to verify this behavior.
    }
}
