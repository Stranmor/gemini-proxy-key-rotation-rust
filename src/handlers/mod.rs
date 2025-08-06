// src/handlers/mod.rs

pub mod base;
pub mod invalid_api_key;
pub mod rate_limit;
pub mod success;
pub mod terminal_error;
pub mod timeout;

// --- Код, перенесенный из src/handler.rs ---

use crate::{
    error::{AppError, Result},
    handlers::base::Action,
    key_manager::{FlattenedKeyInfo, KeyManagerTrait},
    proxy,
    state::AppState,
};
use secrecy::ExposeSecret;
use axum::{
    body::{Body, Bytes, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::Response,
};
use std::sync::Arc;

use tracing::{debug, error, info, instrument, trace, warn};
use url::Url;

const TOKEN_LIMIT: usize = 250_000;

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
    url.query_pairs_mut().append_pair("key", key_info.key.expose_secret());
    Ok(url)
}

struct RequestContext<'a> {
    method: &'a Method,
    uri: &'a Uri,
    headers: &'a HeaderMap,
    body: &'a Bytes,
}

fn validate_token_count(json_body: &serde_json::Value) -> Result<()> {
    let total_text: String = json_body
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|messages| {
            messages
                .iter()
                .filter_map(|message| message.get("content").and_then(|c| c.as_str()))
                .collect::<Vec<&str>>()
                .join("\n")
        })
        .unwrap_or_default();

    if total_text.is_empty() {
        return Ok(());
    }

    // Only count tokens if tokenizer is initialized
    #[cfg(feature = "tokenizer")]
    if let Some(tokenizer) = crate::tokenizer::TOKENIZER.get() {
        let token_count = tokenizer
            .encode(total_text.as_str(), false)
            .map(|encoding| encoding.len())
            .unwrap_or(0);

        info!(token_count, "Calculated request token count");

        if token_count > TOKEN_LIMIT {
            warn!(
                token_count,
                limit = TOKEN_LIMIT,
                "Request exceeds token limit. Rejecting request."
            );
            return Err(AppError::RequestTooLarge {
                size: token_count,
                max_size: TOKEN_LIMIT,
            });
        }
    } else {
        debug!("Tokenizer not initialized, skipping token count check");
    }

    Ok(())
}


fn process_request_body(body_bytes: Bytes, top_p: Option<f64>) -> Result<(Bytes, HeaderMap)> {
    let mut json_body_opt: Option<serde_json::Value> = serde_json::from_slice(&body_bytes).ok();
    let mut headers = HeaderMap::new();

    if let Some(json_body) = &json_body_opt {
        validate_token_count(json_body)?;
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
                        http::HeaderValue::from_str(&new_len.to_string())
                            .map_err(|e| AppError::InvalidRequest {
                                message: format!("Invalid header value created internally: {}", e),
                            })?,
                    );
                    return Ok((body_bytes, headers));
                }
                Err(e) => {
                    error!(error = ?e, "Failed to re-serialize JSON body after modification.");
                }
            }
        }
    }

    Ok((body_bytes, headers))
}

async fn try_request_with_key(
    state: &Arc<AppState>,
    req_context: &RequestContext<'_>,
    key_info: &FlattenedKeyInfo,
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

    let (parts, body) = response.into_parts();
    let response_bytes = to_bytes(body, usize::MAX)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Response::from_parts(parts, Body::from(response_bytes)))
}

/* ---------- main handler ---------- */

#[instrument(skip_all, fields(uri = %req.uri(), method = %req.method()))]
pub async fn proxy_handler(State(state): State<Arc<AppState>>, req: Request) -> Result<Response> {
    let (mut parts, body) = req.into_parts();
    let body_bytes = to_bytes(body, usize::MAX)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    // Process request body (token validation and top_p injection)
    let top_p = state.config.read().await.top_p;
    let (processed_body, additional_headers) =
        process_request_body(body_bytes, top_p.map(|v| v as f64))?;

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

        let response = match try_request_with_key(&state, &req_context, &key_info).await {
            Ok(r) => r,
            Err(e) => {
                // Важно: сохраняем семантику ошибок апстрима.
                // Если это AppError::UpstreamServiceError (например, 413 Payload Too Large),
                // возвращаем её напрямую (Axum через IntoResponse отдаст оригинальный статус/тело).
                // Для всех остальных ошибок сохраняем текущее поведение — 502 Bad Gateway.
                error!(error = ?e, key.preview = %crate::key_manager::KeyManager::preview_key(&key_info.key), "Request failed");
                if let AppError::UpstreamUnavailable { ref service } = e {
                    if *service == "unknown".to_string() {
                        // Возвращаем как есть, без преобразования в 502.
                        return Err(e);
                    }
                }
                let mut resp = Response::new(Body::from(format!("Proxy error: {e}")));
                *resp.status_mut() = StatusCode::BAD_GATEWAY;
                last_response = Some(resp);
                break;
            }
        };

        let (parts, body) = response.into_parts();
        let response_bytes = to_bytes(body, usize::MAX)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;

        let response_for_analysis =
            Response::from_parts(parts.clone(), Body::from(response_bytes.clone()));

        // Check response handlers
        let action_to_take = response_handlers.iter().find_map(|handler| {
            handler.handle(&response_for_analysis, &response_bytes, key_info.key.expose_secret())
        });

        let final_response = Response::from_parts(parts, Body::from(response_bytes));

        match action_to_take {
            Some(Action::ReturnToClient(resp)) => return Ok(resp),
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
    use tokenizers::Tokenizer;

    // Helper to set a minimal tokenizer into the global OnceLock for tests.
    fn install_minimal_tokenizer() {
        if crate::tokenizer::TOKENIZER.get().is_none() {
            // Валидный минимальный токенизатор: WordLevel + Whitespace, decoder отключен (null)
            // Такой конфиг стабильно парсится библиотекой tokenizers и даёт разбиение по пробелам.
            let simple_tokenizer_json = r#"
           {
             "version":"1.0",
             "truncation":null,
             "padding":null,
             "added_tokens":[],
             "normalizer": null,
             "pre_tokenizer": { "type": "Whitespace" },
             "post_processor": null,
             "decoder": null,
             "model": { "type": "WordLevel", "vocab": {"a":0, "b":1, "c":2, "[UNK]":3}, "unk_token":"[UNK]" }
           }"#;

            let tk = Tokenizer::from_bytes(simple_tokenizer_json.as_bytes())
                .expect("Failed to construct minimal tokenizer (WordLevel + Whitespace)");
            let _ = crate::tokenizer::TOKENIZER.set(tk);
        }
    }

    #[test]
    fn validate_does_not_block_without_tokenizer() {
        // Ensure tokenizer is absent: If previously set by other tests in suite,
        // OnceLock can't be reset, so this test is only reliable when run in isolation first.
        // We still verify that with empty messages the function is Ok.
        let body = json!({
            "messages": [{"role": "user", "content": ""}]
        });
        let res = validate_token_count(&body);
        assert!(
            res.is_ok(),
            "Should not error without tokenizer on empty content"
        );
    }

    #[test]
    fn validate_blocks_when_over_limit_with_tokenizer() {
        install_minimal_tokenizer();

        // Build a huge text exceeding TOKEN_LIMIT by creating many words.
        // Each word should count roughly as a token with Whitespace pre-tokenizer.
        let big_text = "a ".repeat(TOKEN_LIMIT + 10);
        let body = json!({
            "messages": [
                {"role":"user", "content": big_text}
            ]
        });

        let err =
            validate_token_count(&body).expect_err("Expected UpstreamServiceError for over-limit");
        if let AppError::UpstreamUnavailable { ref service } = err {
            if *service == "unknown".to_string() {
                // Возвращаем как есть, без преобразования в 502.
                // This part of the code is unreachable in the test, as the error is returned.
            }
        }
        // The test expects a panic if the error is not UpstreamUnavailable.
        // The previous code was trying to match on `err` again, which is incorrect.
        // The `expect_err` already gives us the error, so we just need to assert its type.
        // Since we replaced UpstreamServiceError with UpstreamUnavailable, we assert that.
        assert!(matches!(err, AppError::RequestTooLarge { .. }));
    }
}
