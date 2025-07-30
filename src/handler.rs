// src/handler.rs
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
use std::sync::Arc;
use tracing::{error, info, instrument, trace, warn};
use url::Url;

#[instrument(name = "health_check", skip_all)]
pub async fn health_check() -> StatusCode {
    StatusCode::OK
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
    url.query_pairs_mut().append_pair("key", &key_info.key);
    Ok(url)
}

struct RequestContext<'a> {
    method: &'a Method,
    uri: &'a Uri,
    headers: &'a HeaderMap,
    body: &'a Bytes,
}

/* ---------- main handler ---------- */

#[instrument(skip_all, fields(uri = %req.uri(), method = %req.method()))]
pub async fn proxy_handler(State(state): State<Arc<AppState>>, req: Request) -> Result<Response> {
    let (mut parts, body) = req.into_parts();
    let mut body_bytes = to_bytes(body, usize::MAX)
        .await
        .map_err(|e| AppError::RequestBodyError(e.to_string()))?;

    // Conditionally inject top_p
    if let Some(top_p) = state.config.read().await.top_p {
        if let Ok(mut json_body) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
            if let Some(obj) = json_body.as_object_mut() {
                obj.insert("top_p".to_string(), serde_json::json!(top_p));
                if let Ok(new_body_bytes) = serde_json::to_vec(&json_body) {
                    let new_len = new_body_bytes.len();
                    body_bytes = Bytes::from(new_body_bytes);
                    parts.headers.insert(
                        "content-length",
                        http::HeaderValue::from_str(&new_len.to_string())
                            .map_err(|_| AppError::InvalidHttpHeader)?,
                    );
                }
            }
        }
    }

    let req_context = RequestContext {
        method: &parts.method,
        uri: &parts.uri,
        headers: &parts.headers,
        body: &body_bytes,
    };

    let model = extract_model_from_request(req_context.uri.path(), &body_bytes);

    info!(
        model = ?model,
        path = %req_context.uri.path(),
        "Processing request with model-specific key management"
    );

    let key_manager = state.key_manager.clone();
    let response_handlers = state.response_handlers.clone();

    let mut last_response: Option<Response> = None;

    loop {
        // DE-NEST LOCKS TO PREVENT DEADLOCKS
        // 1. Get group name from config first
        let group_name = {
            trace!("Attempting to acquire read lock on config...");
            let config_guard = state.config.read().await;
            trace!("Acquired read lock on config.");
            let group = model
                .as_deref()
                .and_then(|m| config_guard.get_group_for_model(m))
                .map(|g| g.name.clone());
            trace!("Releasing read lock on config.");
            group
        };
        // Drop config lock before acquiring key_manager lock

        // 2. Get key from key_manager and handle immediately
        let key_info = {
            trace!("Attempting to acquire read lock on key_manager...");
            let key_manager_guard = key_manager.read().await;
            trace!("Acquired read lock on key_manager.");
            let key_info_result = key_manager_guard
                .get_next_available_key_info(group_name.as_deref())
                .await?;
            trace!("Releasing read lock on key_manager.");
            match key_info_result {
                Some(info) => info,
                None => break, // No keys available for this group, exit loop.
            }
        };
        info!(key = %key_info.key, "Attempting to use key");

        let url = build_target_url(req_context.uri, &key_info)?;

        let client = state.get_client(key_info.proxy_url.as_deref()).await?;

        let response = match proxy::forward_request(
            &client,
            &key_info,
            req_context.method.clone(),
            url,
            req_context.headers.clone(),
            req_context.body.clone(),
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                error!(error = ?e, key = %key_info.key, "Forwarding request failed. Breaking loop.");
                let mut resp = Response::new(Body::from(format!("Proxy error: {e}")));
                *resp.status_mut() = StatusCode::BAD_GATEWAY;
                last_response = Some(resp);
                // Immediately break the loop on a proxy error. The response will be handled outside.
                break;
            }
        };

        let (parts, body) = response.into_parts();
        let response_bytes = to_bytes(body, usize::MAX)
            .await
            .map_err(|e| AppError::BodyReadError(e.to_string()))?;

        let response_for_analysis =
            Response::from_parts(parts.clone(), Body::from(response_bytes.clone()));

        let mut action_to_take = None;
        for handler in response_handlers.iter() {
            if let Some(action) = handler.handle(&response_for_analysis, &response_bytes) {
                action_to_take = Some(action);
                break;
            }
        }

        let final_response = Response::from_parts(parts, Body::from(response_bytes.clone()));

        match action_to_take {
            Some(Action::ReturnToClient(resp)) => return Ok(resp),
            Some(Action::RetryNextKey) => {
                trace!("Retrying with next key");
                key_manager
                    .write()
                    .await
                    .handle_api_failure(&key_info.key, false)
                    .await?;
                last_response = Some(final_response);
            }
            Some(Action::BlockKeyAndRetry) => {
                trace!("Blocking key and retrying");
                key_manager
                    .write()
                    .await
                    .handle_api_failure(&key_info.key, true)
                    .await?;
                last_response = Some(final_response);
            }
            None => {
                // No specific action, so this is the final response.
                // This handles success cases and terminal errors not caught by handlers.
                return Ok(final_response);
            }
        }
    }

    // If the loop completes, it means all keys were tried and failed.
    // Return the last response that was captured.
    if let Some(resp) = last_response {
        warn!("All keys failed, returning last captured error response.");
        Ok(resp)
    } else {
        warn!("No available API keys for the given group.");
        Err(AppError::NoAvailableKeys)
    }
}
