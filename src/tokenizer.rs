// src/tokenizer.rs

use hf_hub::api::sync::Api; // Reverted to sync API
use std::error::Error;
use tokenizers::Tokenizer;
use tokio::sync::OnceCell;
use tokio::task;
use tracing::info;

pub static TOKENIZER: OnceCell<Tokenizer> = OnceCell::const_new();

/// Initializes the global tokenizer by downloading it from the Hugging Face Hub
/// in a blocking-safe manner.
pub async fn initialize_tokenizer(model_name: &str) -> Result<(), Box<dyn Error>> {
    info!(model = model_name, "Initializing tokenizer...");

    let model_name_owned = model_name.to_string();
    let tokenizer_result = task::spawn_blocking(move || {
        let api = Api::new().map_err(|e| e.to_string())?;
        let repo = api.model(model_name_owned);
        let tokenizer_path = repo.get("tokenizer.json").map_err(|e| e.to_string())?;
        Tokenizer::from_file(tokenizer_path).map_err(|e| e.to_string())
    })
    .await?;

    let tokenizer = tokenizer_result.map_err(|e: String| -> Box<dyn Error> { e.into() })?;

    // We don't care if it's already set, so we ignore the error.
    let _ = TOKENIZER.set(tokenizer);

    info!("Tokenizer initialized successfully.");
    Ok(())
}


/// Counts the number of tokens in a given text using the global tokenizer.
pub fn count_tokens(text: &str) -> usize {
    TOKENIZER
        .get()
        .expect("Tokenizer not initialized. Call initialize_tokenizer() at startup.")
        .encode(text, false)
        .map(|encoding| encoding.len())
        .unwrap_or(0)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    // Helper to ensure tokenizer is initialized for tests that need it.
    async fn ensure_tokenizer() -> Result<(), Box<dyn Error>> {
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
        let count = count_tokens(text);
        assert!(count > 0, "Token count for 'Hello, world!' should be greater than 0.");
    }

    #[tokio::test]
    async fn test_count_tokens_empty() {
        if ensure_tokenizer().await.is_err() {
            println!("Skipping test_count_tokens_empty: HF_TOKEN not available");
            return;
        }
        let text = "";
        let count = count_tokens(text);
        assert_eq!(count, 0, "Token count for an empty string should be 0.");
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
            || error_message.contains("401"); // Also accept 401 as valid failure

        assert!(
            is_not_found_or_auth_error,
            "Error message should indicate a 'Not Found', '404', or '401' error, but was: {}",
            error_message
        );
    }
}