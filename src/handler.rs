use crate::state::ProxyState;
use actix_web::{web, HttpRequest, HttpResponse, Error as ActixError};
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, HeaderName};
use reqwest::Url;
use tracing::{error, info, instrument, debug};

const HOP_BY_HOP_HEADERS: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
];

#[instrument(skip(state, req, payload), fields(proxy_name = %state.config.name, target_url = %state.config.target_url, path = %req.uri().path()))]
pub async fn proxy_handler(
    state: web::Data<ProxyState>,
    req: HttpRequest,
    mut payload: web::Payload,
) -> Result<HttpResponse, ActixError> {
    let api_key = match state.get_next_api_key() {
        Some(key) => key,
        None => {
            error!("No API keys available for proxy {}", state.config.name);
            return Ok(HttpResponse::InternalServerError().body("Proxy configuration error: No API keys"));
        }
    };
    let key_preview = api_key.chars().take(4).collect::<String>() + "...";
    info!(api_key_preview = %key_preview, "Selected API key");

    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");

    let mut target_url = match Url::parse(&state.config.target_url) {
        Ok(mut url) => {
            url.set_path(path_and_query);
            url
        },
        Err(e) => {
            error!("Invalid target_url {} in config {}: {}", state.config.target_url, state.config.name, e);
            return Ok(HttpResponse::InternalServerError().body("Proxy configuration error: Invalid target URL"));
        }
    };

    target_url
        .query_pairs_mut()
        .append_pair(&state.config.key_parameter_name, &api_key);

    debug!(target = %target_url, "Constructed target URL");

    let mut outgoing_headers = HeaderMap::new();
    for (header_name, header_value) in req.headers().iter() {
        let name_str = header_name.as_str().to_lowercase();
        if !HOP_BY_HOP_HEADERS.contains(&name_str.as_str()) && name_str != "host" {
            // Преобразуем actix header в строку, потом создаем reqwest header
            if let (Ok(hname), Ok(hval)) = (
                HeaderName::from_bytes(header_name.as_str().as_bytes()),
                HeaderValue::from_bytes(header_value.as_bytes()),
            ) {
                outgoing_headers.insert(hname, hval);
                debug!(header = %header_name, value = ?header_value.to_str().unwrap_or("<non-utf8>"), "Forwarding header");
            } else {
                debug!(header = %header_name, "Skipping invalid header");
            }
        } else {
            debug!(header = %header_name, "Skipping hop-by-hop or host header");
        }
    }

    if let Some(peer_addr) = req.peer_addr() {
        let forwarded_for_val = peer_addr.ip().to_string();
        if let Ok(forwarded_for) = HeaderValue::from_str(&forwarded_for_val) {
            outgoing_headers.insert(reqwest::header::FORWARDED, forwarded_for);
        }
    }

    let mut body_bytes = web::BytesMut::new();
    while let Some(chunk) = payload.next().await {
        body_bytes.extend_from_slice(&chunk?);
    }
    let outgoing_body = reqwest::Body::from(body_bytes.freeze());

    let client = state.client();
    let method = reqwest::Method::from_bytes(req.method().as_str().as_bytes())
        .unwrap_or(reqwest::Method::GET);

    info!(method = %method, url = %target_url, "Sending request to target");

    let target_request = client
        .request(method.clone(), target_url)
        .headers(outgoing_headers)
        .body(outgoing_body);

    let target_response = match target_request.send().await {
        Ok(resp) => resp,
        Err(e) => {
            error!("Request to target API failed: {}", e);
            if e.is_timeout() {
                return Ok(HttpResponse::GatewayTimeout().body(format!("Target API timed out: {}", e)));
            } else if e.is_connect() || e.is_request() {
                return Ok(HttpResponse::BadGateway().body(format!("Failed to connect or send request to target API: {}", e)));
            } else {
                return Ok(HttpResponse::InternalServerError().body(format!("Error during request to target API: {}", e)));
            }
        }
    };

    info!(status = %target_response.status(), "Received response from target");

    let mut client_response = HttpResponse::build(
        actix_web::http::StatusCode::from_u16(target_response.status().as_u16())
            .unwrap_or(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR),
    );

    for (name, value) in target_response.headers().iter() {
        let name_str = name.as_str().to_lowercase();
        if !HOP_BY_HOP_HEADERS.contains(&name_str.as_str()) {
            // конвертируем reqwest header в actix header
            if let (Ok(hname), Ok(hval)) = (
                actix_web::http::header::HeaderName::from_bytes(name.as_str().as_bytes()),
                actix_web::http::header::HeaderValue::from_bytes(value.as_bytes()),
            ) {
                client_response.insert_header((hname, hval));
                debug!(header = %name, value = ?value.to_str().unwrap_or("<non-utf8>"), "Forwarding response header");
            } else {
                debug!(header = %name, "Skipping invalid response header");
            }
        } else {
            debug!(header = %name, "Skipping hop-by-hop response header");
        }
    }

    Ok(client_response.streaming(target_response.bytes_stream()))
}