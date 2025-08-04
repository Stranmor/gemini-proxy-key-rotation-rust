// src/middleware/admin_auth.rs

use crate::{error::AppError, state::AppState};
use axum::{
    body::Body,
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;
use tower_cookies::Cookies;
use tracing::{info, warn};

/// Constant-time string comparison to prevent timing attacks
fn secure_compare(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    
    let mut result = 0u8;
    for (byte_a, byte_b) in a.bytes().zip(b.bytes()) {
        result |= byte_a ^ byte_b;
    }
    result == 0
}

const ADMIN_TOKEN_COOKIE: &str = "admin_token";

/// Middleware for admin authentication.
/// Checks if the admin token cookie matches the configured admin token.
pub async fn admin_auth_middleware(
    State(state): State<Arc<AppState>>,
    cookies: Cookies,
    req: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    let config = state.config.read().await;
    let expected_token = config.server.admin_token.as_deref();

    match expected_token {
        Some(expected) if !expected.is_empty() => {
            let cookie_token = cookies
                .get(ADMIN_TOKEN_COOKIE)
                .map(|cookie| cookie.value().to_string());

            match cookie_token {
                Some(token) if secure_compare(&token, expected) => {
                    info!("Admin authentication successful");
                    Ok(next.run(req).await)
                }
                _ => {
                    warn!("Admin authentication failed: invalid or missing token");
                    Err(AppError::Unauthorized)
                }
            }
        }
        _ => {
            warn!("Admin authentication failed: no admin token configured");
            Err(AppError::Unauthorized)
        }
    }
}
