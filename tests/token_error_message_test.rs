// tests/token_error_message_test.rs

use axum::response::IntoResponse;
use gemini_proxy::error::AppError;
use serde_json::Value;

#[tokio::test]
async fn test_request_too_large_error_message_contains_tokens() {
    // Создаем ошибку RequestTooLarge
    let error = AppError::RequestTooLarge {
        size: 300000,
        max_size: 250000,
    };

    // Проверяем, что сообщение об ошибке содержит "tokens"
    let error_message = format!("{}", error);
    assert!(
        error_message.contains("tokens"),
        "Error message should contain 'tokens', got: {}",
        error_message
    );
    assert!(
        !error_message.contains("bytes"),
        "Error message should not contain 'bytes', got: {}",
        error_message
    );

    // Проверяем JSON ответ
    let response = error.into_response();
    let (parts, body) = response.into_parts();
    
    // Проверяем статус код
    assert_eq!(parts.status, 400);

    // Извлекаем JSON из тела ответа
    let body_bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();

    // Проверяем поля JSON ответа
    assert_eq!(json["type"], "https://gemini-proxy.dev/errors/validation");
    assert_eq!(json["title"], "Validation Error");
    assert_eq!(json["status"], 400);
    
    // Проверяем, что detail содержит правильное сообщение
    let detail = json["detail"].as_str().unwrap();
    assert!(
        detail.contains("tokens"),
        "Detail should contain 'tokens', got: {}",
        detail
    );
    assert!(
        !detail.contains("bytes"),
        "Detail should not contain 'bytes', got: {}",
        detail
    );
}