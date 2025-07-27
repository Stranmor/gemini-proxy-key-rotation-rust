// src/handler.rs
use crate::{
    error::{AppError, Result},
    key_manager::{KeyInfo, KeyManager},
    proxy, state::AppState,
};
use axum::{
    body::{to_bytes, Body, Bytes},
    extract::{Request, State},
    http::{response::Parts, HeaderMap, Method, StatusCode, Uri},
    response::Response,
};
use chrono::Duration;
use std::sync::Arc;
use tracing::{debug, error, info, instrument, warn};
use url::Url;

#[instrument(name = "health_check", skip_all)]
pub async fn health_check() -> StatusCode {
    StatusCode::OK
}

/* ---------- helpers ---------- */

// ... (translate_path и mutate_key остаются без изменений)
/// Возвращает переведённый путь для апстрима.
fn translate_path(path: &str) -> String {
    match path {
        "/health/detailed" => "/v1beta/models".into(),
        p if let Some(rest) = p.strip_prefix("/v1/") => match rest {
            r if r.starts_with("chat/completions") => format!("/v1beta/openai/{r}"),
            r if r.starts_with("embeddings") || r.starts_with("audio/speech") => {
                format!("/v1beta/{r}")
            }
            r => format!("/v1beta/openai/{r}"),
        },
        _ => path.to_owned(),
    }
}

/// Собирает финальный URL, добавляя ключ.
fn build_target_url(original_uri: &Uri, key_info: &KeyInfo) -> Result<Url> {
    let mut url = Url::parse(&key_info.target_url)?.join(&translate_path(original_uri.path()))?;
    url.set_query(original_uri.query());
    url.query_pairs_mut().append_pair("key", &key_info.key);
    Ok(url)
}

/// Универсальный helper: меняем состояние ключа и сохраняем.
async fn mutate_key<F>(state: &Arc<AppState>, key: &str, f: F) -> Result<()>
where
    F: FnOnce(&mut KeyManager, &str),
{
    let mut km = state.key_manager.write().await;
    f(&mut km, key);
    km.save_states().await?;
    Ok(())
}


/// Контекст входящего запроса, чтобы не передавать кучу аргументов.
struct RequestContext<'a> {
    method: &'a Method,
    uri: &'a Uri,
    headers: &'a HeaderMap,
    body: &'a Bytes,
}

/// Повторяем запрос на одном ключе, пока не исчерпаем `internal_retries`.
async fn retry_with_key(
    state: &Arc<AppState>,
    key_info: &KeyInfo,
    req_context: &RequestContext<'_>,
    internal_retries: u32,
    last_error: &mut Option<(StatusCode, HeaderMap, Bytes)>,
) -> Result<Option<Response>> {
    for attempt in 1..=internal_retries + 1 {
        let url = build_target_url(req_context.uri, key_info)?; // <--- ИСПРАВЛЕНИЕ БАГА
        
        let response_result = proxy::forward_request(
            state,
            key_info,
            req_context.method.clone(),
            url,
            req_context.headers.clone(),
            req_context.body.clone(),
        )
        .await;

        let response = match response_result {
            Ok(r) => r,
            Err(e) => {
                error!(error = ?e, key = %key_info.key, "Forwarding request failed");
                let block_duration = Duration::minutes(state.config.read().await.temporary_block_minutes);
                mutate_key(state, &key_info.key, |km, k| {
                    km.mark_key_as_temporarily_unavailable(k, block_duration)
                })
                .await?;
                return Ok(None); // Ключ ушёл в блок, пробуем следующий
            }
        };

        let status = response.status();
        let (parts, body) = response.into_parts();
        let bytes = to_bytes(body, usize::MAX).await.map_err(|e| AppError::BodyReadError(e.to_string()))?;

        match status {
            s if s.is_success() => {
                info!(key = %key_info.key, "Request successful");
                return Ok(Some(Response::from_parts(parts, Body::from(bytes))));
            }
            StatusCode::NOT_FOUND | StatusCode::GATEWAY_TIMEOUT => {
                warn!(%status, key = %key_info.key, "Received terminal error, not retrying with another key.");
                return Ok(Some(Response::from_parts(parts, Body::from(bytes))));
            }
            StatusCode::BAD_REQUEST if String::from_utf8_lossy(&bytes).contains("API_KEY_INVALID") => {
                warn!(key = %key_info.key, "Marking key as invalid");
                mutate_key(state, &key_info.key, |km, k| km.mark_key_as_invalid(k)).await?;
                return Ok(None); // Ключ невалиден, пробуем следующий
            }
            s if s.is_client_error() => {
                warn!(%s, key = %key_info.key, "Client error, marking key as limited");
                mutate_key(state, &key_info.key, |km, k| km.mark_key_as_limited(k)).await?;
                *last_error = Some((s, parts.headers, bytes));
                return Ok(None); // Ошибка клиента, пробуем следующий ключ
            }
            s if s.is_server_error() => {
                warn!(%s, attempt, key = %key_info.key, "Server error, will retry");
                *last_error = Some((s, parts.headers, bytes.clone())); // Сохраняем последнюю ошибку

                if attempt > internal_retries {
                    error!(key=%key_info.key, "Internal retries exhausted. Marking key as temporarily unavailable.");
                    let block_duration = Duration::minutes(state.config.read().await.temporary_block_minutes);
                    mutate_key(state, &key_info.key, |km, k| {
                        km.mark_key_as_temporarily_unavailable(k, block_duration)
                    })
                    .await?;
                    return Ok(None); // Попытки на этом ключе кончились, пробуем следующий
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue; // Повторяем с тем же ключом
            }
            _ => {
                warn!(%status, "Received unexpected status code, returning as is.");
                return Ok(Some(Response::from_parts(parts, Body::from(bytes))));
            }
        }
    }

    // Этот код недостижим, так как цикл всегда завершается через return или continue.
    // Если вы хотите избежать unreachable!(), можно просто вернуть Ok(None) здесь.
    Ok(None)
}

/* ---------- main handler ---------- */

#[instrument(skip(state, req), fields(uri = %req.uri(), method = %req.method()))]
pub async fn proxy_handler(
    State(state): State<Arc<AppState>>,
    req: Request,
) -> Result<Response> {
    let (parts, body) = req.into_parts();
    let body_bytes = to_bytes(body, usize::MAX)
        .await
        .map_err(|e| AppError::RequestBodyError(e.to_string()))?;

    let req_context = RequestContext {
        method: &parts.method,
        uri: &parts.uri,
        headers: &parts.headers,
        body: &body_bytes,
    };
    
    // Получаем конфиг один раз
    let (internal_retries, temporary_block_minutes) = {
        let config = state.config.read().await;
        (config.internal_retries, config.temporary_block_minutes)
    };

    let mut last_error: Option<(StatusCode, HeaderMap, Bytes)> = None;

    loop {
        let key_info = state.key_manager.read().await.get_next_available_key_info();

        let Some(info) = key_info else {
            warn!("No available API keys remaining.");
            return if let Some((status, headers, body)) = last_error {
                let mut resp = Response::new(Body::from(body));
                *resp.status_mut() = status;
                *resp.headers_mut() = headers;
                Ok(resp)
            } else {
                Err(AppError::NoAvailableKeys)
            };
        };

        let result = retry_with_key(
            &state,
            &info,
            &req_context,
            internal_retries,
            &mut last_error,
        )
        .await?;

        if let Some(resp) = result {
            return Ok(resp);
        }
    }
}