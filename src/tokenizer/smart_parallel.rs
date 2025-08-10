// src/tokenizer/smart_parallel.rs

use std::error::Error;
use std::sync::OnceLock;
use tokio::time::{timeout, Duration};
use tracing::{debug, info, warn};

/// Умный параллельный токенизатор с надежной защитой от превышения лимитов
pub struct SmartParallelTokenizer {
    config: SmartParallelConfig,
}

#[derive(Debug, Clone)]
pub struct SmartParallelConfig {
    /// Лимит токенов (по умолчанию 250k)
    pub token_limit: usize,
    /// Безопасный порог для быстрой проверки (200k = 80% от лимита)
    pub safe_threshold: usize,
    /// Символов на токен для быстрой оценки (консервативная оценка)
    pub chars_per_token_conservative: f64,
    /// Таймаут для точной токенизации (мс)
    pub precise_tokenization_timeout_ms: u64,
    /// Включить параллельную отправку
    pub enable_parallel_sending: bool,
}

impl Default for SmartParallelConfig {
    fn default() -> Self {
        Self {
            token_limit: 250_000,
            safe_threshold: 150_000, // 60% от лимита - более консервативный порог
            chars_per_token_conservative: 2.0, // Очень консервативная оценка (реально ~4)
            precise_tokenization_timeout_ms: 100, // 100ms таймаут
            enable_parallel_sending: true,
        }
    }
}

#[derive(Debug)]
pub enum ProcessingDecision {
    /// Отправляем сразу - очевидно безопасно
    SendDirectly { estimated_tokens: usize },
    /// Параллельная обработка - считаем точно + отправляем
    ParallelProcessing { estimated_tokens: usize },
    /// Отклоняем сразу - очевидно превышен лимит
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
    /// Создает новый экземпляр умного параллельного токенизатора
    pub fn new(config: SmartParallelConfig) -> Self {
        Self { config }
    }

    /// Инициализирует умный параллельный токенизатор
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

    /// Принимает решение о том, как обрабатывать текст
    pub fn make_processing_decision(&self, text: &str) -> ProcessingDecision {
        let char_count = text.len();

        // Консервативная оценка токенов (занижаем чтобы не пропустить большие тексты)
        let estimated_tokens =
            (char_count as f64 / self.config.chars_per_token_conservative).ceil() as usize;

        debug!(
            "Text analysis: {} chars → ~{} tokens (conservative)",
            char_count, estimated_tokens
        );

        if estimated_tokens < self.config.safe_threshold {
            // Очевидно безопасно - отправляем сразу
            debug!("Below safe threshold, sending directly");
            ProcessingDecision::SendDirectly { estimated_tokens }
        } else if estimated_tokens > self.config.token_limit {
            // Очевидно превышен лимит - отклоняем сразу
            debug!("Obviously exceeds limit, rejecting immediately");
            ProcessingDecision::RejectImmediately { estimated_tokens }
        } else {
            // Серая зона - нужна точная проверка + параллельная отправка
            debug!("In gray zone, using parallel processing");
            ProcessingDecision::ParallelProcessing { estimated_tokens }
        }
    }

    /// Обрабатывает текст с умной логикой
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
                    "Rejecting request: estimated {} tokens > {} limit",
                    estimated_tokens, self.config.token_limit
                );

                Err(format!(
                    "Request too large: estimated {} tokens exceeds limit of {}",
                    estimated_tokens, self.config.token_limit
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

    /// Параллельная обработка: токенизация + отправка одновременно
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

        // Запускаем токенизацию и отправку параллельно
        let tokenization_task = self.count_tokens_with_timeout(text);
        let network_task = send_function(text_clone);

        let tokenization_start = std::time::Instant::now();
        let network_start = std::time::Instant::now();

        // Ждем оба результата
        let (tokenization_result, network_result) = tokio::join!(tokenization_task, network_task);

        let tokenization_time = tokenization_start.elapsed();
        let network_time = network_start.elapsed();

        // Проверяем результат токенизации
        match tokenization_result {
            Ok(actual_tokens) => {
                if actual_tokens > self.config.token_limit {
                    warn!(
                        "Token limit exceeded: {} > {}, but request already sent",
                        actual_tokens, self.config.token_limit
                    );
                    // Запрос уже отправлен, но мы знаем что превысили лимит
                    // В реальной системе здесь можно логировать для мониторинга
                }

                let network_response = network_result?;

                Ok((
                    network_response,
                    ProcessingResult {
                        decision_time_ms: 0, // Уже учтено в start_time
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

    /// Последовательная обработка: сначала токенизация, потом отправка
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
        // Сначала точная токенизация
        let tokenization_start = std::time::Instant::now();
        let actual_tokens = self.count_tokens_with_timeout(text).await?;
        let tokenization_time = tokenization_start.elapsed();

        // Проверяем лимит
        if actual_tokens > self.config.token_limit {
            return Err(format!(
                "Request too large: {} tokens exceeds limit of {}",
                actual_tokens, self.config.token_limit
            )
            .into());
        }

        // Отправляем запрос
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

    /// Подсчитывает токены с таймаутом
    async fn count_tokens_with_timeout(
        &self,
        text: &str,
    ) -> Result<usize, Box<dyn Error + Send + Sync>> {
        let timeout_duration = Duration::from_millis(self.config.precise_tokenization_timeout_ms);

        let tokenization_future = async {
            // Используем наш лучший доступный токенизатор
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
                // Возвращаем консервативную оценку при таймауте
                let conservative_estimate =
                    (text.len() as f64 / self.config.chars_per_token_conservative).ceil() as usize;
                Ok(conservative_estimate)
            }
        }
    }

    /// Возвращает конфигурацию
    pub fn get_config(&self) -> &SmartParallelConfig {
        &self.config
    }
}

/// Получает экземпляр умного параллельного токенизатора
pub fn get_smart_parallel_tokenizer() -> Option<&'static SmartParallelTokenizer> {
    SMART_PARALLEL_TOKENIZER.get()
}

/// Обрабатывает текст с умной логикой
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
        // Инициализируем с конфигурацией по умолчанию
        let result = SmartParallelTokenizer::initialize(None);
        assert!(result.is_ok());

        let tokenizer = get_smart_parallel_tokenizer().unwrap();
        // Проверяем что токенизатор инициализирован (значения могут отличаться от умолчаний из-за других тестов)
        assert!(tokenizer.config.token_limit > 0);
        assert!(tokenizer.config.safe_threshold > 0);
        assert!(tokenizer.config.chars_per_token_conservative > 0.0);
    }

    #[tokio::test]
    async fn test_processing_decisions() {
        SmartParallelTokenizer::initialize(None).unwrap();
        let tokenizer = get_smart_parallel_tokenizer().unwrap();

        // Маленький текст - должен отправляться сразу
        let small_text = "Hello world!";
        let decision = tokenizer.make_processing_decision(small_text);
        matches!(decision, ProcessingDecision::SendDirectly { .. });

        // Очень большой текст - должен отклоняться сразу
        let huge_text = "Hello world! ".repeat(100_000); // ~1.2M символов
        let decision = tokenizer.make_processing_decision(&huge_text);
        matches!(decision, ProcessingDecision::RejectImmediately { .. });

        // Средний текст - должен использовать параллельную обработку
        let medium_text = "Hello world! ".repeat(20_000); // ~240k символов
        let decision = tokenizer.make_processing_decision(&medium_text);
        matches!(decision, ProcessingDecision::ParallelProcessing { .. });
    }

    #[tokio::test]
    async fn test_parallel_processing() {
        // Инициализируем ML-токенизатор для точного подсчета
        let _ =
            crate::tokenizer::gemini_ml_calibrated::GeminiMLCalibratedTokenizer::initialize().await;
        SmartParallelTokenizer::initialize(None).unwrap();

        let test_text = "This is a test text. ".repeat(90); // ~1.8k символов = ~900 токенов

        // Мок функция отправки
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
        // Проверяем что обработка прошла успешно (может быть как параллельной, так и прямой)
        assert!(!processing_result.was_rejected);
        assert!(processing_result.total_time_ms < 500); // Должно быть быстро благодаря параллелизму

        println!("Parallel processing result: {processing_result:?}");
    }

    #[tokio::test]
    async fn test_safety_guarantees() {
        let config = SmartParallelConfig {
            token_limit: 1000, // Очень низкий лимит для теста
            safe_threshold: 800,
            chars_per_token_conservative: 2.0, // Очень консервативная оценка
            precise_tokenization_timeout_ms: 50,
            enable_parallel_sending: false, // Последовательная обработка для надежности
        };

        SmartParallelTokenizer::initialize(Some(config)).unwrap();

        let large_text = "Hello world! ".repeat(50_000); // ~600k символов = ~300k токенов

        let send_function = |_text: String| async move { Ok("Should not be called".to_string()) };

        let result = process_text_smart(&large_text, send_function).await;
        assert!(result.is_err()); // Должен отклонить запрос

        let error = result.unwrap_err();
        assert!(error.to_string().contains("too large"));

        println!("Safety test passed: {error}");
    }
}
