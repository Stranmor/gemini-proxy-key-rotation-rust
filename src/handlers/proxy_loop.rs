// src/handlers/proxy_loop.rs

use crate::{
    error::{AppError, Result},
    handlers::{
        base::{Action, ResponseHandler},
        RequestContext,
    },
    key_manager::{FlattenedKeyInfo, KeyManagerTrait},
    proxy,
    state::AppState,
    tokenizer::count_ml_calibrated_gemini_tokens,
};
use axum::{
    body::{to_bytes, Body},
    response::Response,
};
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

    let request_body_str =
        String::from_utf8(req_context.body.to_vec()).unwrap_or_else(|_| "".to_string());

    let send_request = |body_str: String| async move {
        proxy::forward_request(
            &client,
            key_info,
            req_context.method.clone(),
            url.clone(),
            req_context.headers.clone(),
            body_str.into(),
            circuit_breaker.clone(),
        )
        .await
    };

    match crate::tokenizer::process_text_smart(&request_body_str, send_request).await {
        Ok((response, _processing_result)) => Ok(response),
        Err(e) => {
            let app_error = AppError::internal(e.to_string());
            Ok(app_error.into_response())
        }
    }
}

/// Analyzes the response and determines the next action.
async fn analyze_response(
    response: Response,
    response_handlers: &[Arc<dyn ResponseHandler>],
    key_info: &FlattenedKeyInfo,
) -> Result<(Action, Response)> {
    let (parts, body) = response.into_parts();
    let response_bytes = to_bytes(body, usize::MAX)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let response_for_analysis =
        Response::from_parts(parts.clone(), Body::from(response_bytes.clone()));

    let action_to_take = response_handlers
        .iter()
        .find_map(|handler| {
            handler.handle(
                &response_for_analysis,
                &response_bytes,
                key_info.key.expose_secret(),
            )
        });

    if let Some(action) = action_to_take {
        let final_response = Response::from_parts(parts, Body::from(response_bytes));
        Ok((action, final_response))
    } else {
        let final_response = Response::from_parts(parts.clone(), Body::from(response_bytes.clone()));
        Ok((
            Action::ReturnToClient(Response::from_parts(parts, Body::from(response_bytes))),
            final_response,
        ))
    }
}

/// Main loop for handling proxy requests, iterating through available keys.
pub async fn proxy_loop(
    state: &Arc<AppState>,
    req_context: &RequestContext<'_>,
    model: &Option<String>,
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

        let (action, final_response) =
            analyze_response(response, &state.response_handlers, &key_info).await?;

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

    last_response.map(Ok).unwrap_or_else(|| {
        tracing::warn!("No available API keys for the given group.");
        Err(AppError::NoHealthyKeys)
    })
}