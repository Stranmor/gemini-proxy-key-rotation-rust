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
};
use axum::{
    body::{to_bytes, Body},
    response::Response,
};
use secrecy::ExposeSecret;
use std::sync::Arc;
use tracing::{error, info, trace};

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