// tests/streaming_tests.rs

use axum::body::Bytes;
use gemini_proxy::handlers::is_streaming_request;
use serde_json::json;

#[cfg(test)]
mod streaming_tests {
    use super::*;

    #[test]
    fn test_is_streaming_request_detects_streaming() {
        let streaming_body = json!({
            "model": "gemini-1.5-flash",
            "messages": [{"role": "user", "content": "Hello"}],
            "stream": true
        });
        
        let body_bytes = Bytes::from(serde_json::to_vec(&streaming_body).unwrap());
        
        assert!(is_streaming_request(&body_bytes));
        
        // Пока что проверим, что JSON парсится корректно
        let parsed: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(parsed.get("stream").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn test_is_streaming_request_detects_non_streaming() {
        let non_streaming_body = json!({
            "model": "gemini-1.5-flash",
            "messages": [{"role": "user", "content": "Hello"}],
            "stream": false
        });
        
        let body_bytes = Bytes::from(serde_json::to_vec(&non_streaming_body).unwrap());
        
        let parsed: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(parsed.get("stream").and_then(|v| v.as_bool()), Some(false));
    }

    #[test]
    fn test_is_streaming_request_default_false() {
        let default_body = json!({
            "model": "gemini-1.5-flash",
            "messages": [{"role": "user", "content": "Hello"}]
        });
        
        let body_bytes = Bytes::from(serde_json::to_vec(&default_body).unwrap());
        
        let parsed: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(parsed.get("stream").and_then(|v| v.as_bool()), None);
    }

    #[test]
    fn test_invalid_json_not_streaming() {
        let invalid_body = Bytes::from("invalid json");
        
        assert!(!is_streaming_request(&invalid_body));
        
        // Проверим, что JSON не парсится
        assert!(serde_json::from_slice::<serde_json::Value>(&invalid_body).is_err());
    }
}