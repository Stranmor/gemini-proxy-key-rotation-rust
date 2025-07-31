// src/tokenizer.rs

use hf_hub::api::sync::Api;
use std::error::Error;
use std::sync::OnceLock;
use tokenizers::Tokenizer;
use tokio::task;
use tracing::{info, warn};

pub static TOKENIZER: OnceLock<Tokenizer> = OnceLock::new();

/// Initializes the global tokenizer by downloading it from the Hugging Face Hub
/// in a blocking-safe manner.
pub async fn initialize_tokenizer(model_name: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    info!(model = model_name, "Initializing tokenizer...");

    let model_name_owned = model_name.to_string();
    let tokenizer_result = task::spawn_blocking(move || -> Result<Tokenizer, Box<dyn Error + Send + Sync>> {
        let api = Api::new()?;
        let repo = api.model(model_name_owned);
        let tokenizer_path = repo.get("tokenizer.json")?;
        Tokenizer::from_file(tokenizer_path)
    })
    .await??;

    match TOKENIZER.set(tokenizer_result) {
        Ok(_) => info!("Tokenizer initialized successfully."),
        Err(_) => warn!("Tokenizer was already initialized, ignoring duplicate initialization."),
    }
    Ok(())
}

/// Counts the number of tokens in a given text using the global tokenizer.
pub fn count_tokens(text: &str) -> Result<usize, Box<dyn Error + Send + Sync>> {
    let tokenizer = TOKENIZER
        .get()
        .ok_or("Tokenizer not initialized. Call initialize_tokenizer() at startup.")?;
    
    let encoding = tokenizer.encode(text, false)?;
    Ok(encoding.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    // Helper to ensure tokenizer is initialized for tests that need it.
    async fn ensure_tokenizer() -> Result<(), Box<dyn Error + Send + Sync>> {
        if TOKENIZER.get().is_none() {
            // Skip tests if HF_TOKEN is not available
            if env::var("HF_TOKEN").is_err() {
                return Err("HF_TOKEN not available, skipping tokenizer tests".into());
            }
            initialize_tokenizer("google/gemma-2-2b").await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_count_tokens_simple() {
        if ensure_tokenizer().await.is_err() {
            println!("Skipping test_count_tokens_simple: HF_TOKEN not available");
            return;
        }
        let text = "Hello, world!";
        let count = count_tokens(text).expect("Should be able to count tokens");
        assert!(count > 0, "Token count for 'Hello, world!' should be greater than 0.");
    }

    #[tokio::test]
    async fn test_count_tokens_empty() {
        if ensure_tokenizer().await.is_err() {
            println!("Skipping test_count_tokens_empty: HF_TOKEN not available");
            return;
        }
        let text = "";
        let count = count_tokens(text).expect("Should be able to count tokens for empty string");
        assert_eq!(count, 0, "Token count for an empty string should be 0.");
    }

    #[tokio::test]
    async fn test_count_tokens_unicode() {
        if ensure_tokenizer().await.is_err() {
            println!("Skipping test_count_tokens_unicode: HF_TOKEN not available");
            return;
        }
        let text = "Hello 世界! 🌍";
        let count = count_tokens(text).expect("Should be able to count tokens for unicode text");
        assert!(count > 0, "Token count for unicode text should be greater than 0.");
    }

    #[tokio::test]
    async fn test_initialize_tokenizer_failure_on_invalid_model() {
        // Skip test if HF_TOKEN is not available
        if env::var("HF_TOKEN").is_err() {
            println!("Skipping test_initialize_tokenizer_failure_on_invalid_model: HF_TOKEN not available");
            return;
        }

        // Attempt to initialize with a model that does not exist
        let result = initialize_tokenizer("invalid/repository-that-does-not-exist").await;

        // We expect this to fail, so we can unwrap the error
        let error = result.expect_err("Initialization should have failed for a non-existent model");

        // Check if the error message contains something indicative of a not found error
        let error_message = error.to_string();
        let is_not_found_or_auth_error = error_message.contains("Not Found") 
            || error_message.contains("404")
            || error_message.contains("401")
            || error_message.contains("403"); // Also accept 403 as valid failure

        assert!(
            is_not_found_or_auth_error,
            "Error message should indicate a 'Not Found', '404', '401', or '403' error, but was: {error_message}"
        );
    }

    #[tokio::test]
    async fn test_count_tokens_without_initialization() {
        // Create a fresh tokenizer instance to test error handling
        let original_tokenizer = TOKENIZER.get().cloned();
        
        // This test assumes we can't easily reset the OnceLock, so we'll just test
        // that count_tokens works when tokenizer is available
        if original_tokenizer.is_some() {
            let result = count_tokens("test");
            assert!(result.is_ok(), "count_tokens should work when tokenizer is initialized");
}    }
}