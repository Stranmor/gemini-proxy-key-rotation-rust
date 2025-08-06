// src/security/token_manager.rs

use rand::{thread_rng, Rng};
use secrecy::{ExposeSecret, Secret};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Менеджер токенов с поддержкой ротации
pub struct TokenManager {
    tokens: Arc<RwLock<HashMap<String, TokenInfo>>>,
    master_token: Secret<String>,
    token_lifetime: Duration,
}

#[derive(Debug, Clone)]
struct TokenInfo {
    created_at: Instant,
    last_used: Instant,
    usage_count: u64,
    client_ip: Option<String>,
}

impl TokenManager {
    pub fn new(master_token: String) -> Self {
        Self {
            tokens: Arc::new(RwLock::new(HashMap::new())),
            master_token: Secret::new(master_token),
            token_lifetime: Duration::from_secs(24 * 3600), // 24 часа
        }
    }

    /// Генерирует новый временный токен
    pub async fn generate_session_token(&self, client_ip: Option<String>) -> String {
        let token = self.generate_secure_token();
        let now = Instant::now();

        let token_info = TokenInfo {
            created_at: now,
            last_used: now,
            usage_count: 0,
            client_ip: client_ip.clone(),
        };

        {
            let mut tokens = self.tokens.write().await;
            tokens.insert(token.clone(), token_info);
        }

        // Очищаем старые токены
        self.cleanup_expired_tokens().await;

        info!(
            client_ip = ?client_ip.as_ref(),
            "Generated new session token"
        );

        token
    }

    /// Проверяет валидность токена
    pub async fn validate_token(&self, token: &str, client_ip: Option<String>) -> bool {
        // Проверяем мастер-токен
        if token == self.master_token.expose_secret() {
            return true;
        }

        // Проверяем сессионные токены
        let mut tokens = self.tokens.write().await;

        if let Some(token_info) = tokens.get_mut(token) {
            let now = Instant::now();

            // Проверяем срок действия
            if now.duration_since(token_info.created_at) > self.token_lifetime {
                tokens.remove(token);
                warn!("Expired session token removed");
                return false;
            }

            // Проверяем IP (если задан)
            if let (Some(stored_ip), Some(current_ip)) = (&token_info.client_ip, &client_ip) {
                if stored_ip != current_ip {
                    warn!(
                        stored_ip = %stored_ip,
                        current_ip = %current_ip,
                        "Token used from different IP address"
                    );
                    return false;
                }
            }

            // Обновляем статистику использования
            token_info.last_used = now;
            token_info.usage_count += 1;

            return true;
        }

        false
    }

    /// Отзывает токен
    pub async fn revoke_token(&self, token: &str) {
        let mut tokens = self.tokens.write().await;
        if tokens.remove(token).is_some() {
            info!("Session token revoked");
        }
    }

    /// Отзывает все сессионные токены
    pub async fn revoke_all_sessions(&self) {
        let mut tokens = self.tokens.write().await;
        let count = tokens.len();
        tokens.clear();
        info!(revoked_count = count, "All session tokens revoked");
    }

    /// Получает статистику токенов
    pub async fn get_token_stats(&self) -> TokenStats {
        let tokens = self.tokens.read().await;
        let now = Instant::now();

        let active_sessions = tokens.len();
        let total_usage: u64 = tokens.values().map(|t| t.usage_count).sum();

        let recent_activity = tokens
            .values()
            .filter(|t| now.duration_since(t.last_used) < Duration::from_secs(3600))
            .count();

        TokenStats {
            active_sessions,
            total_usage,
            recent_activity,
        }
    }

    async fn cleanup_expired_tokens(&self) {
        let mut tokens = self.tokens.write().await;
        let now = Instant::now();
        let initial_count = tokens.len();

        tokens.retain(|_, token_info| {
            now.duration_since(token_info.created_at) <= self.token_lifetime
        });

        let removed_count = initial_count - tokens.len();
        if removed_count > 0 {
            info!(removed_count, "Cleaned up expired session tokens");
        }
    }

    fn generate_secure_token(&self) -> String {
        let mut rng = thread_rng();
        let token: String = (0..32)
            .map(|_| {
                let chars = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
                chars[rng.gen_range(0..chars.len())] as char
            })
            .collect();

        format!("st_{token}") // session token prefix
    }
}

#[derive(Debug, Clone)]
pub struct TokenStats {
    pub active_sessions: usize,
    pub total_usage: u64,
    pub recent_activity: usize,
}
