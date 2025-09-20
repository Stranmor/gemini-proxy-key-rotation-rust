// tests/token_validation_test.rs

use gemini_proxy::error::AppError;
use serde_json::json;

#[tokio::test]
async fn test_token_validation_with_limit() {
    // Initialize tokenizer
    gemini_proxy::tokenizer::gemini_ml_calibrated::GeminiMLCalibratedTokenizer::initialize()
        .await
        .expect("Failed to initialize tokenizer");

    // Test with a request that should exceed a very low token limit
    let large_content = "This is a test message that should exceed the token limit when repeated many times. ".repeat(100);
    let request_body = json!({
        "messages": [
            {
                "role": "user",
                "content": large_content
            }
        ]
    });

    // Test with very low limit (should fail)
    let result = gemini_proxy::handlers::validate_token_count_with_limit(&request_body, Some(10));
    match result {
        Err(AppError::RequestTooLarge { size, max_size }) => {
            assert!(size > max_size);
            assert_eq!(max_size, 10);
            println!("Successfully rejected request with {} tokens (limit: {})", size, max_size);
        }
        _ => panic!("Expected RequestTooLarge error, got: {:?}", result),
    }

    // Test with reasonable limit (should pass)
    let result = gemini_proxy::handlers::validate_token_count_with_limit(&request_body, Some(10000));
    assert!(result.is_ok(), "Request should pass with reasonable limit");

    // Test with no limit (should pass)
    let result = gemini_proxy::handlers::validate_token_count_with_limit(&request_body, None);
    assert!(result.is_ok(), "Request should pass with no limit");
}

#[tokio::test]
async fn test_token_validation_gemini_format() {
    // Initialize tokenizer
    gemini_proxy::tokenizer::gemini_ml_calibrated::GeminiMLCalibratedTokenizer::initialize()
        .await
        .expect("Failed to initialize tokenizer");

    // Test Gemini native format
    let request_body = json!({
        "contents": [
            {
                "parts": [
                    {
                        "text": "This is a test message in Gemini format. ".repeat(50)
                    }
                ]
            }
        ]
    });

    // Test with low limit (should fail)
    let result = gemini_proxy::handlers::validate_token_count_with_limit(&request_body, Some(10));
    match result {
        Err(AppError::RequestTooLarge { size, max_size }) => {
            assert!(size > max_size);
            assert_eq!(max_size, 10);
            println!("Successfully rejected Gemini format request with {} tokens (limit: {})", size, max_size);
        }
        _ => panic!("Expected RequestTooLarge error for Gemini format, got: {:?}", result),
    }
}

#[test]
fn test_token_validation_without_tokenizer() {
    // Test that validation doesn't crash when tokenizer is not initialized
    let request_body = json!({
        "messages": [
            {
                "role": "user",
                "content": "Test message"
            }
        ]
    });

    // Should not panic and should allow request to proceed
    let result = gemini_proxy::handlers::validate_token_count_with_limit(&request_body, Some(10));
    assert!(result.is_ok(), "Should not fail when tokenizer is not initialized");
}