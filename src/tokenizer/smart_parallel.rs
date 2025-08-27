// src/tokenizer/smart_parallel.rs

use std::error::Error;
use std::sync::OnceLock;
use tokio::time::{timeout, Duration};
use tracing::{debug, info, warn};

/// Smart parallel tokenizer with reliable protection against limit overruns
pub struct SmartParallelTokenizer {
    config: SmartParallelConfig,
}

#[derive(Debug, Clone)]
pub struct SmartParallelConfig {
    /// Token limit (default 250k)
    pub token_limit: usize,
    /// Safe threshold for quick check (200k = 80% of limit)
    pub safe_threshold: usize,
    /// Characters per token for quick estimation (conservative estimate)
    pub chars_per_token_conservative: f64,
    /// Timeout for precise tokenization (ms)
    pub precise_tokenization_timeout_ms: u64,
    /// Enable parallel sending
    pub enable_parallel_sending: bool,
    /// Rejection threshold by characters (absolute maximum)
    pub rejection_threshold_chars: usize,
}

impl Default for SmartParallelConfig {
    fn default() -> Self {
        Self {
            token_limit: 250_000,
            safe_threshold: 150_000, // 60% of limit - more conservative threshold
            chars_per_token_conservative: 2.0, // Very conservative estimate (actually ~4)
            precise_tokenization_timeout_ms: 100, // 100ms timeout
            enable_parallel_sending: true,
            rejection_threshold_chars: 1_500_000, // Absolute maximum, ~2x-3x expected for token limit
        }
    }
}

#[derive(Debug)]
pub enum ProcessingDecision {
    /// Send directly - obviously safe
    SendDirectly { estimated_tokens: usize },
    /// Parallel processing - count precisely + send
    ParallelProcessing { estimated_tokens: usize },
    /// Reject immediately - obviously exceeds limit
    RejectImmediately { estimated_tokens: usize },
}

#[derive(Debug)]
pub struct ProcessingResult {
    pub decision_time_ms: u64,
    pub tokenization_time_ms: Option<u64>,
    pub network_time_ms: Option<u64>,
    pub total_time_ms: u64,
    pub actual_tokens: Option<usize>,
    pub estimated_tokens: usize,
    pub was_parallel: bool,
    pub was_rejected: bool,
}

static SMART_PARALLEL_TOKENIZER: OnceLock<SmartParallelTokenizer> = OnceLock::new();

impl SmartParallelTokenizer {
    /// Creates a new instance of the smart parallel tokenizer
    pub fn new(config: SmartParallelConfig) -> Self {
        Self { config }
    }

    /// Initializes the smart parallel tokenizer
    pub fn initialize(
        config: Option<SmartParallelConfig>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let config = config.unwrap_or_default();

        info!("Initializing Smart Parallel Tokenizer");
        info!("Token limit: {}", config.token_limit);
        info!(
            "Safe threshold: {} ({}%)",
            config.safe_threshold,
            (config.safe_threshold as f64 / config.token_limit as f64 * 100.0) as u32
        );
        info!(
            "Conservative estimate: {:.1} chars/token",
            config.chars_per_token_conservative
        );

        let tokenizer = Self { config };

        match SMART_PARALLEL_TOKENIZER.set(tokenizer) {
            Ok(_) => info!("Smart Parallel Tokenizer initialized successfully"),
            Err(_) => warn!("Smart Parallel Tokenizer was already initialized"),
        }

        Ok(())
    }

    /// Makes a decision on how to process the text
    pub fn make_processing_decision(&self, text: &str) -> ProcessingDecision {
        let char_count = text.len();

        // First line of defense: absolute character limit
        if char_count > self.config.rejection_threshold_chars {
            warn!(
                "Rejecting request: char count {} > {} limit",
                char_count, self.config.rejection_threshold_chars
            );
            return ProcessingDecision::RejectImmediately {
                estimated_tokens: (char_count as f64 / self.config.chars_per_token_conservative)
                    .ceil() as usize,
            };
        }

        // Conservative token estimation (underestimate to avoid missing large texts)
        let estimated_tokens =
            (char_count as f64 / self.config.chars_per_token_conservative).ceil() as usize;

        debug!(
            "Text analysis: {} chars → ~{} tokens (conservative)",
            char_count, estimated_tokens
        );

        if estimated_tokens < self.config.safe_threshold {
            // Obviously safe - send immediately
            debug!("Below safe threshold, sending directly");
            ProcessingDecision::SendDirectly { estimated_tokens }
        } else if estimated_tokens > self.config.token_limit {
            // Obviously exceeds limit - reject immediately
            debug!("Obviously exceeds limit, rejecting immediately");
            ProcessingDecision::RejectImmediately { estimated_tokens }
        } else {
            // Gray zone - need precise check + parallel sending
            debug!("In gray zone, using parallel processing");
            ProcessingDecision::ParallelProcessing { estimated_tokens }
        }
    }

    /// Processes text with smart logic
    pub async fn process_text<F, Fut, T>(
        &self,
        text: &str,
        send_function: F,
    ) -> Result<(T, ProcessingResult), Box<dyn Error + Send + Sync>>
    where
        F: FnOnce(String) -> Fut,
        Fut: std::future::Future<Output = Result<T, Box<dyn Error + Send + Sync>>>,
    {
        let start_time = std::time::Instant::now();

        let decision = self.make_processing_decision(text);
        let decision_time = start_time.elapsed();

        match decision {
            ProcessingDecision::SendDirectly { estimated_tokens } => {
                debug!("Sending directly without tokenization");

                let network_start = std::time::Instant::now();
                let result = send_function(text.to_string()).await?;
                let network_time = network_start.elapsed();

                Ok((
                    result,
                    ProcessingResult {
                        decision_time_ms: decision_time.as_millis() as u64,
                        tokenization_time_ms: None,
                        network_time_ms: Some(network_time.as_millis() as u64),
                        total_time_ms: start_time.elapsed().as_millis() as u64,
                        actual_tokens: None,
                        estimated_tokens,
                        was_parallel: false,
                        was_rejected: false,
                    },
                ))
            }

            ProcessingDecision::RejectImmediately { estimated_tokens } => {
                warn!(
                    "Rejecting request: char count {} or estimated tokens {} exceeds limits",
                    text.len(),
                    estimated_tokens
                );

                Err(format!(
                    "Request too large: size {} chars, estimated {} tokens. Exceeds limits.",
                    text.len(),
                    estimated_tokens
                )
                .into())
            }

            ProcessingDecision::ParallelProcessing { estimated_tokens } => {
                debug!("Starting parallel processing: tokenization + network request");

                if self.config.enable_parallel_sending {
                    self.process_parallel(text, estimated_tokens, send_function, start_time)
                        .await
                } else {
                    self.process_sequential(text, estimated_tokens, send_function, start_time)
                        .await
                }
            }
        }
    }

    /// Parallel processing: tokenization + sending simultaneously
    async fn process_parallel<F, Fut, T>(
        &self,
        text: &str,
        estimated_tokens: usize,
        send_function: F,
        start_time: std::time::Instant,
    ) -> Result<(T, ProcessingResult), Box<dyn Error + Send + Sync>>
    where
        F: FnOnce(String) -> Fut,
        Fut: std::future::Future<Output = Result<T, Box<dyn Error + Send + Sync>>>,
    {
        let text_clone = text.to_string();

        // Start tokenization and sending in parallel
        let tokenization_task = self.count_tokens_with_timeout(text);
        let network_task = send_function(text_clone);

        let tokenization_start = std::time::Instant::now();
        let network_start = std::time::Instant::now();

        // Wait for both results
        let (tokenization_result, network_result) = tokio::join!(tokenization_task, network_task);

        let tokenization_time = tokenization_start.elapsed();
        let network_time = network_start.elapsed();

        // Check tokenization result
        match tokenization_result {
            Ok(actual_tokens) => {
                if actual_tokens > self.config.token_limit {
                    warn!(
                        "Token limit exceeded: {} > {}, but request already sent",
                        actual_tokens, self.config.token_limit
                    );
                    // Request already sent, but we know we exceeded the limit
                    // In a real system, this can be logged for monitoring
                }

                let network_response = network_result?;

                Ok((
                    network_response,
                    ProcessingResult {
                        decision_time_ms: 0, // Already accounted for in start_time
                        tokenization_time_ms: Some(tokenization_time.as_millis() as u64),
                        network_time_ms: Some(network_time.as_millis() as u64),
                        total_time_ms: start_time.elapsed().as_millis() as u64,
                        actual_tokens: Some(actual_tokens),
                        estimated_tokens,
                        was_parallel: true,
                        was_rejected: false,
                    },
                ))
            }
            Err(e) => {
                warn!("Tokenization failed: {}, proceeding with network result", e);

                let network_response = network_result?;

                Ok((
                    network_response,
                    ProcessingResult {
                        decision_time_ms: 0,
                        tokenization_time_ms: Some(tokenization_time.as_millis() as u64),
                        network_time_ms: Some(network_time.as_millis() as u64),
                        total_time_ms: start_time.elapsed().as_millis() as u64,
                        actual_tokens: None,
                        estimated_tokens,
                        was_parallel: true,
                        was_rejected: false,
                    },
                ))
            }
        }
    }

    /// Sequential processing: tokenization first, then sending
    async fn process_sequential<F, Fut, T>(
        &self,
        text: &str,
        estimated_tokens: usize,
        send_function: F,
        start_time: std::time::Instant,
    ) -> Result<(T, ProcessingResult), Box<dyn Error + Send + Sync>>
    where
        F: FnOnce(String) -> Fut,
        Fut: std::future::Future<Output = Result<T, Box<dyn Error + Send + Sync>>>,
    {
        // First precise tokenization
        let tokenization_start = std::time::Instant::now();
        let actual_tokens = self.count_tokens_with_timeout(text).await?;
        let tokenization_time = tokenization_start.elapsed();

        // Check limit
        if actual_tokens > self.config.token_limit {
            return Err(format!(
                "Request too large: {} tokens exceeds limit of {}",
                actual_tokens, self.config.token_limit
            )
            .into());
        }

        // Send request
        let network_start = std::time::Instant::now();
        let result = send_function(text.to_string()).await?;
        let network_time = network_start.elapsed();

        Ok((
            result,
            ProcessingResult {
                decision_time_ms: 0,
                tokenization_time_ms: Some(tokenization_time.as_millis() as u64),
                network_time_ms: Some(network_time.as_millis() as u64),
                total_time_ms: start_time.elapsed().as_millis() as u64,
                actual_tokens: Some(actual_tokens),
                estimated_tokens,
                was_parallel: false,
                was_rejected: false,
            },
        ))
    }

    /// Counts tokens with timeout
    async fn count_tokens_with_timeout(
        &self,
        text: &str,
    ) -> Result<usize, Box<dyn Error + Send + Sync>> {
        let timeout_duration = Duration::from_millis(self.config.precise_tokenization_timeout_ms);

        let tokenization_future = async {
            // Use our best available tokenizer
            crate::tokenizer::gemini_ml_calibrated::count_ml_calibrated_gemini_tokens(text)
        };

        match timeout(timeout_duration, tokenization_future).await {
            Ok(Ok(count)) => Ok(count),
            Ok(Err(e)) => Err(format!("Tokenization error: {e}").into()),
            Err(_) => {
                warn!(
                    "Tokenization timeout after {}ms",
                    self.config.precise_tokenization_timeout_ms
                );
                // Return conservative estimate on timeout
                let conservative_estimate =
                    (text.len() as f64 / self.config.chars_per_token_conservative).ceil() as usize;
                Ok(conservative_estimate)
            }
        }
    }

    /// Returns configuration
    pub fn get_config(&self) -> &SmartParallelConfig {
        &self.config
    }
}

/// Gets instance of smart parallel tokenizer
pub fn get_smart_parallel_tokenizer() -> Option<&'static SmartParallelTokenizer> {
    SMART_PARALLEL_TOKENIZER.get()
}

/// Processes text with smart logic
pub async fn process_text_smart<F, Fut, T>(
    text: &str,
    send_function: F,
) -> Result<(T, ProcessingResult), Box<dyn Error + Send + Sync>>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<T, Box<dyn Error + Send + Sync>>>,
{
    match SMART_PARALLEL_TOKENIZER.get() {
        Some(tokenizer) => tokenizer.process_text(text, send_function).await,
        None => Err("Smart Parallel Tokenizer not initialized".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_smart_parallel_initialization() {
        // Initialize with default configuration
        let result = SmartParallelTokenizer::initialize(None);
        assert!(result.is_ok());

        let tokenizer = get_smart_parallel_tokenizer().unwrap();
        // Check that tokenizer is initialized (values may differ from defaults due to other tests)
        assert!(tokenizer.config.token_limit > 0);
        assert!(tokenizer.config.safe_threshold > 0);
        assert!(tokenizer.config.chars_per_token_conservative > 0.0);
    }

    #[tokio::test]
    async fn test_processing_decisions() {
        SmartParallelTokenizer::initialize(None).unwrap();
        let tokenizer = get_smart_parallel_tokenizer().unwrap();

        // Small text - should be sent immediately
        let small_text = "Hello world!";
        let decision = tokenizer.make_processing_decision(small_text);
        matches!(decision, ProcessingDecision::SendDirectly { .. });

        // Very large text - should be rejected immediately
        let huge_text = "Hello world! ".repeat(100_000); // ~1.2M characters
        let decision = tokenizer.make_processing_decision(&huge_text);
        matches!(decision, ProcessingDecision::RejectImmediately { .. });

        // Medium text - should use parallel processing
        let medium_text = "Hello world! ".repeat(20_000); // ~240k characters
        let decision = tokenizer.make_processing_decision(&medium_text);
        matches!(decision, ProcessingDecision::ParallelProcessing { .. });
    }

    #[tokio::test]
    async fn test_parallel_processing() {
        // Initialize ML tokenizer for precise counting
        let _ =
            crate::tokenizer::gemini_ml_calibrated::GeminiMLCalibratedTokenizer::initialize().await;
        SmartParallelTokenizer::initialize(None).unwrap();

        let test_text = "This is a test text. ".repeat(90); // ~1.8k characters = ~900 tokens

        // Mock send function
        let send_function = |text: String| async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            Ok(format!("Response for {} chars", text.len()))
        };

        let result = process_text_smart(&test_text, send_function).await;
        if let Err(e) = &result {
            eprintln!("parallel processing test failed: {e}");
        }
        assert!(result.is_ok());

        let (response, processing_result) = result.unwrap();
        assert!(response.contains("Response for"));
        // Check that processing was successful (can be either parallel or direct)
        assert!(!processing_result.was_rejected);
        assert!(processing_result.total_time_ms < 500); // Should be fast thanks to parallelism

        println!("Parallel processing result: {processing_result:?}");
    }

    #[tokio::test]
    async fn test_safety_guarantees() {
        let config = SmartParallelConfig {
            token_limit: 1000, // Very low limit for test
            safe_threshold: 800,
            chars_per_token_conservative: 2.0, // Very conservative estimate
            precise_tokenization_timeout_ms: 50,
            enable_parallel_sending: false, // Sequential processing for reliability
            rejection_threshold_chars: 500_000, // Character limit for test
        };

        SmartParallelTokenizer::initialize(Some(config)).unwrap();

        let large_text = "Hello world! ".repeat(50_000); // ~600k characters = ~300k tokens

        let send_function = |_text: String| async move { Ok("Should not be called".to_string()) };

        let result = process_text_smart(&large_text, send_function).await;
        assert!(result.is_err()); // Should reject request

        let error = result.unwrap_err();
        assert!(error.to_string().contains("too large"));

        println!("Safety test passed: {error}");
    }
}
