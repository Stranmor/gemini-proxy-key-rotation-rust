// src/security/mod.rs

use crate::error::{AppError, Result};
use axum::{
    extract::{ConnectInfo, Request},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{warn, error};

pub mod token_manager;

/// Security middleware для защиты админ-панели
pub struct SecurityMiddleware {
    rate_limiter: Arc<RwLock<HashMap<String, RateLimitEntry>>>,
    max_attempts: u32,
    window_duration: Duration,
}

#[derive(Debug, Clone)]
struct RateLimitEntry {
    attempts: u32,
    window_start: Instant,
    blocked_until: Option<Instant>,
}

impl SecurityMiddleware {
    pub fn new() -> Self {
        Self {
            rate_limiter: Arc::new(RwLock::new(HashMap::new())),
            max_attempts: 5, // 5 попыток
            window_duration: Duration::from_secs(300), // за 5 минут
        }
    }

    /// Middleware для защиты от брутфорса админ-панели
    pub async fn admin_protection(
        &self,
        ConnectInfo(addr): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
        request: Request,
        next: Next,
    ) -> Result<Response> {
        let client_ip = addr.ip().to_string();
        
        // Проверяем rate limit
        if self.is_rate_limited(&client_ip).await {
            warn!(
                client_ip = %client_ip,
                "Admin panel access blocked due to rate limiting"
            );
            return Err(AppError::Authentication { message: "Unauthorized".to_string() });
        }

        // Проверяем HTTPS в продакшене
        if !self.is_secure_connection(&headers) {
            warn!(
                client_ip = %client_ip,
                "Admin panel access attempted over insecure connection"
            );
            return Err(AppError::Authentication { message: "Unauthorized".to_string() });
        }

        let response = next.run(request).await;
        
        // Если аутентификация не удалась, записываем попытку
        if response.status() == StatusCode::UNAUTHORIZED {
            self.record_failed_attempt(&client_ip).await;
        }

        Ok(response)
    }

    pub async fn is_rate_limited(&self, client_ip: &str) -> bool {
        let mut limiter = self.rate_limiter.write().await;
        let now = Instant::now();
        
        let entry = limiter.entry(client_ip.to_string()).or_insert(RateLimitEntry {
            attempts: 0,
            window_start: now,
            blocked_until: None,
        });

        // Проверяем блокировку
        if let Some(blocked_until) = entry.blocked_until {
            if now < blocked_until {
                return true;
            } else {
                // Сбрасываем блокировку
                entry.blocked_until = None;
                entry.attempts = 0;
                entry.window_start = now;
            }
        }

        // Проверяем окно времени
        if now.duration_since(entry.window_start) > self.window_duration {
            entry.attempts = 0;
            entry.window_start = now;
        }

        false
    }

    pub async fn record_failed_attempt(&self, client_ip: &str) {
        let mut limiter = self.rate_limiter.write().await;
        let now = Instant::now();
        
        let entry = limiter.entry(client_ip.to_string()).or_insert(RateLimitEntry {
            attempts: 0,
            window_start: now,
            blocked_until: None,
        });

        entry.attempts += 1;

        if entry.attempts >= self.max_attempts {
            // Блокируем на 1 час
            entry.blocked_until = Some(now + Duration::from_secs(3600));
            error!(
                client_ip = %client_ip,
                attempts = entry.attempts,
                "Client blocked due to excessive failed admin login attempts"
            );
        }
    }

    pub fn is_secure_connection(&self, headers: &HeaderMap) -> bool {
        // В тестовом режиме разрешаем HTTP
        if cfg!(test) {
            return true;
        }

        // Проверяем заголовки HTTPS
        headers.get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .map(|v| v == "https")
            .unwrap_or(false)
        || headers.get("x-forwarded-ssl")
            .and_then(|v| v.to_str().ok())
            .map(|v| v == "on")
            .unwrap_or(false)
    }
}

impl Default for SecurityMiddleware {
    fn default() -> Self {
        Self::new()
    }
}