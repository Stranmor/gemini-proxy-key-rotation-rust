// src/handlers/proxy_loop.rs

use crate::{
    error::{AppError, Result},
    handlers::{base::Action, RequestContext},
    key_manager::FlattenedKeyInfo,
    proxy,
    state::AppState,
    tokenizer::gemini_ml_calibrated::count_ml_calibrated_gemini_tokens,
};
use axum::{body::Body, http::StatusCode, response::Response};
use secrecy::ExposeSecret;
use std::sync::Arc;
use tracing::{error, info, trace, warn};

/// Tries a single request with a given key.
async fn try_request_with_key(
    state: &Arc<AppState>,
    req_context: &RequestContext<'_>,
    key_info: &FlattenedKeyInfo,
) -> Result<Response> {
    let url = super::build_target_url(req_context.uri, key_info)?;
    let client = state.get_client(key_info.proxy_url.as_deref()).await?;
    let circuit_breaker = state.get_circuit_breaker(&key_info.target_url).await;

    proxy::forward_request(
        &client,
        key_info,
        req_context.method.clone(),
        url,
        req_context.headers.clone(),
        req_context.body.clone(),
        circuit_breaker,
    )
    .await
}

/// Main loop for handling proxy requests, iterating through available keys.
pub async fn proxy_loop(
    state: &Arc<AppState>,
    req_context: &RequestContext<'_>,
    model: &Option<String>,
    is_streaming: bool,
) -> Result<Response> {
    let max_tokens = {
        let config_guard = state.config.read().await;
        config_guard.server.max_tokens_per_request
    };

    if let Some(max_tokens) = max_tokens {
        if let Ok(body_str) = std::str::from_utf8(req_context.body) {
            if let Ok(json_body) = serde_json::from_str::<serde_json::Value>(body_str) {
                let mut total_tokens = 0;
                if let Some(contents) = json_body.get("contents") {
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
                        match count_ml_calibrated_gemini_tokens(&text_to_tokenize) {
                            Ok(token_count) => {
                                total_tokens += token_count;
                            }
                            Err(e) => {
                                warn!("Token counting failed: {}", e);
                            }
                        }
                    }
                } else if let Some(messages) = json_body.get("messages").and_then(|m| m.as_array())
                {
                    let mut text_to_tokenize = String::new();
                    for message in messages {
                        if let Some(content) = message.get("content").and_then(|c| c.as_str()) {
                            text_to_tokenize.push_str(content);
                            text_to_tokenize.push('\n'); // Add separator between messages
                        }
                    }

                    if !text_to_tokenize.is_empty() {
                        match count_ml_calibrated_gemini_tokens(&text_to_tokenize) {
                            Ok(token_count) => {
                                total_tokens += token_count;
                            }
                            Err(e) => {
                                warn!("Token counting for messages failed: {}", e);
                            }
                        }
                    }
                }

                if total_tokens > 0 && total_tokens as u64 > max_tokens {
                    return Err(AppError::RequestTooLarge {
                        size: total_tokens,
                        max_size: max_tokens as usize,
                    });
                }
            }
        }
    }
    let mut last_response: Option<Response> = None;

    loop {
        let group_name = {
            let config_guard = state.config.read().await;
            model
                .as_deref()
                .and_then(|m| config_guard.get_group_for_model(m))
                .map(|g| g.to_string())
        };

        let key_info = match state
            .key_manager
            .read()
            .await
            .get_next_available_key_info(group_name.as_deref())
            .await?
        {
            Some(info) => info,
            None => break,
        };

        info!(key.preview = %crate::key_manager::KeyManager::preview_key(&key_info.key), "Attempting to use key");

        let response = match try_request_with_key(state, req_context, &key_info).await {
            Ok(r) => r,
            Err(e) => {
                error!(error = ?e, key.preview = %crate::key_manager::KeyManager::preview_key(&key_info.key), "Request failed");
                return Err(e);
            }
        };

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

        let (action, final_response) = state
            .response_processor
            .process(response, &key_info)
            .await?;

        match action {
            Action::ReturnToClient(resp) => return Ok(resp),
            Action::Terminal(resp) => return Ok(resp),
            Action::RetryNextKey => {
                trace!("Retrying with next key");
                state
                    .key_manager
                    .write()
                    .await
                    .handle_api_failure(key_info.key.expose_secret(), false)
                    .await?;
                last_response = Some(final_response);
            }
            Action::BlockKeyAndRetry => {
                trace!("Blocking key and retrying");
                state
                    .key_manager
                    .write()
                    .await
                    .handle_api_failure(key_info.key.expose_secret(), true)
                    .await?;
                last_response = Some(final_response);
            }
            Action::WaitFor(duration) => {
                trace!("Rate limit with wait period received. Marking key and waiting.");
                state
                    .key_manager
                    .write()
                    .await
                    .handle_rate_limit(key_info.key.expose_secret(), duration)
                    .await?;

                info!(?duration, "Rate limit hit. Waiting before retrying.");
                tokio::time::sleep(duration).await;
                last_response = Some(final_response);
            }
        }
    }

    last_response.map_or_else(
        || {
            tracing::warn!("No available API keys for the given group.");
            Err(AppError::NoHealthyKeys)
        },
        |last_res| {
            if last_res.status().is_server_error() {
                let mut new_resp = Response::new(Body::from("All upstream servers failed"));
                *new_resp.status_mut() = StatusCode::BAD_GATEWAY;
                Ok(new_resp)
            } else {
                Ok(last_res)
            }
        },
    )
}
