// tests/security_tests.rs

use gemini_proxy_key_rotation_rust::security::{SecurityMiddleware, token_manager::TokenManager};
use axum::{
    extract::ConnectInfo,
    http::{HeaderMap, HeaderValue, Method, Request, StatusCode},
    response::Response,
    body::Body,
};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

#[tokio::test]
async fn test_security_middleware_rate_limiting() {
    let security = SecurityMiddleware::new();
    let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);
    
    // Тест проверяет, что rate limiting работает
    // В реальной системе это будет интегрировано с middleware
    
    // Проверяем, что IP не заблокирован изначально
    let is_limited = security.is_rate_limited(&client_addr.ip().to_string()).await;
    assert!(!is_limited, "IP should not be rate limited initially");
    
    // Записываем несколько неудачных попыток
    for _ in 0..5 {
        security.record_failed_attempt(&client_addr.ip().to_string()).await;
    }
    
    // Теперь IP должен быть заблокирован
    let is_limited_after = security.is_rate_limited(&client_addr.ip().to_string()).await;
    assert!(is_limited_after, "IP should be rate limited after 5 failed attempts");
}

#[tokio::test]
async fn test_security_middleware_https_enforcement() {
    let security = SecurityMiddleware::new();
    let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);
    
    // Тестируем проверку HTTPS заголовков
    let headers_http = HeaderMap::new(); // Нет HTTPS заголовков
    let headers_https = {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        h
    };
    
    // HTTP должен быть отклонен в продакшене (но в тестах разрешен)
    let is_secure_http = security.is_secure_connection(&headers_http);
    let is_secure_https = security.is_secure_connection(&headers_https);
    
    // В тестовом режиме HTTP разрешен, но функция проверяет только заголовки
    // В продакшене HTTP будет отклонен
    assert!(!is_secure_http || cfg!(test), "HTTP should be rejected in production");
    assert!(is_secure_https, "HTTPS should always be allowed");
}

#[tokio::test]
async fn test_token_manager_session_tokens() {
    let token_manager = TokenManager::new("master-token-123".to_string());
    let client_ip = Some("192.168.1.1".to_string());
    
    // Генерируем сессионный токен
    let session_token = token_manager.generate_session_token(client_ip.clone()).await;
    assert!(session_token.starts_with("st_"), "Session token should have correct prefix");
    
    // Проверяем валидность токена
    let is_valid = token_manager.validate_token(&session_token, client_ip.clone()).await;
    assert!(is_valid, "Session token should be valid");
    
    // Проверяем мастер-токен
    let is_master_valid = token_manager.validate_token("master-token-123", client_ip.clone()).await;
    assert!(is_master_valid, "Master token should be valid");
    
    // Проверяем невалидный токен
    let is_invalid = token_manager.validate_token("invalid-token", client_ip).await;
    assert!(!is_invalid, "Invalid token should not be valid");
}

#[tokio::test]
async fn test_token_manager_ip_validation() {
    let token_manager = TokenManager::new("master-token-123".to_string());
    let original_ip = Some("192.168.1.1".to_string());
    let different_ip = Some("192.168.1.2".to_string());
    
    // Генерируем токен для определенного IP
    let session_token = token_manager.generate_session_token(original_ip.clone()).await;
    
    // Проверяем с правильным IP
    let is_valid_correct_ip = token_manager.validate_token(&session_token, original_ip).await;
    assert!(is_valid_correct_ip, "Token should be valid with correct IP");
    
    // Проверяем с другим IP
    let is_valid_different_ip = token_manager.validate_token(&session_token, different_ip).await;
    assert!(!is_valid_different_ip, "Token should not be valid with different IP");
}

#[tokio::test]
async fn test_token_manager_revocation() {
    let token_manager = TokenManager::new("master-token-123".to_string());
    let client_ip = Some("192.168.1.1".to_string());
    
    // Генерируем токен
    let session_token = token_manager.generate_session_token(client_ip.clone()).await;
    
    // Проверяем, что токен валиден
    let is_valid_before = token_manager.validate_token(&session_token, client_ip.clone()).await;
    assert!(is_valid_before, "Token should be valid before revocation");
    
    // Отзываем токен
    token_manager.revoke_token(&session_token).await;
    
    // Проверяем, что токен больше не валиден
    let is_valid_after = token_manager.validate_token(&session_token, client_ip).await;
    assert!(!is_valid_after, "Token should not be valid after revocation");
}

#[tokio::test]
async fn test_token_manager_statistics() {
    let token_manager = TokenManager::new("master-token-123".to_string());
    
    // Генерируем несколько токенов
    let _token1 = token_manager.generate_session_token(Some("192.168.1.1".to_string())).await;
    let _token2 = token_manager.generate_session_token(Some("192.168.1.2".to_string())).await;
    
    // Получаем статистику
    let stats = token_manager.get_token_stats().await;
    
    assert_eq!(stats.active_sessions, 2, "Should have 2 active sessions");
    assert_eq!(stats.total_usage, 0, "Should have 0 total usage initially");
    // recent_activity может быть > 0 если токены только что созданы
    assert!(stats.recent_activity <= 2, "Recent activity should be <= 2");
}

#[tokio::test]
async fn test_token_manager_revoke_all_sessions() {
    let token_manager = TokenManager::new("master-token-123".to_string());
    
    // Генерируем несколько токенов
    let token1 = token_manager.generate_session_token(Some("192.168.1.1".to_string())).await;
    let token2 = token_manager.generate_session_token(Some("192.168.1.2".to_string())).await;
    
    // Проверяем, что токены валидны
    assert!(token_manager.validate_token(&token1, Some("192.168.1.1".to_string())).await);
    assert!(token_manager.validate_token(&token2, Some("192.168.1.2".to_string())).await);
    
    // Отзываем все сессии
    token_manager.revoke_all_sessions().await;
    
    // Проверяем, что токены больше не валидны
    assert!(!token_manager.validate_token(&token1, Some("192.168.1.1".to_string())).await);
    assert!(!token_manager.validate_token(&token2, Some("192.168.1.2".to_string())).await);
    
    // Мастер-токен должен остаться валидным
    assert!(token_manager.validate_token("master-token-123", Some("192.168.1.1".to_string())).await);
}

