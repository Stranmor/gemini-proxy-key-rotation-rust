// tests/streaming_fix_test.rs

use axum::body::Bytes;
use gemini_proxy::handlers::is_streaming_request;
use serde_json::json;

#[test]
fn test_streaming_detection_works() {
    // Test that streaming detection works correctly
    let streaming_body = json!({
        "model": "gemini-1.5-flash",
        "messages": [{"role": "user", "content": "Hello"}],
        "stream": true
    });

    let body_bytes = Bytes::from(serde_json::to_vec(&streaming_body).unwrap());
    assert!(
        is_streaming_request(&body_bytes),
        "Should detect streaming request"
    );

    let non_streaming_body = json!({
        "model": "gemini-1.5-flash",
        "messages": [{"role": "user", "content": "Hello"}],
        "stream": false
    });

    let body_bytes = Bytes::from(serde_json::to_vec(&non_streaming_body).unwrap());
    assert!(
        !is_streaming_request(&body_bytes),
        "Should detect non-streaming request"
    );

    let default_body = json!({
        "model": "gemini-1.5-flash",
        "messages": [{"role": "user", "content": "Hello"}]
    });

    let body_bytes = Bytes::from(serde_json::to_vec(&default_body).unwrap());
    assert!(
        !is_streaming_request(&body_bytes),
        "Should default to non-streaming"
    );
}
