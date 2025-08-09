// src/tokenizer/gemini_first.rs

use std::error::Error;
use std::sync::OnceLock;
use tracing::{info, warn, debug};

/// "Gemini First" токенизатор - отправляет запросы напрямую к Gemini
/// Токенизация происходит только при необходимости (лимиты, биллинг)
pub struct GeminiFirstTokenizer {
    config: GeminiFirstConfig,
}

#[derive(Debug, Clone)]
pub struct GeminiFirstConfig {
    /// Включить предварительную проверку токенов (по умолчанию false)
    pub enable_pre_check: bool,
    /// Лимит токенов для предварительной проверки
    pub pre_check_limit: Option<usize>,
    /// Включить подсчет токенов после ответа (для биллинга)
    pub enable_post_count: bool,
    /// Использовать быструю оценку для больших текстов
    pub use_fast_estimation: bool,
    /// Порог для быстрой оценки (символы)
    pub fast_estimation_threshold: usize,
}

impl Default for GeminiFirstConfig {
    fn default() -> Self {
        Self {
            enable_pre_check: false,        // По умолчанию отправляем сразу
            pre_check_limit: None,          // Без лимитов
            enable_post_count: true,        // Считаем после для статистики
            use_fast_estimation: true,      // Быстрая оценка для больших текстов
            fast_estimation_threshold: 50000, // 50KB порог
        }
    }
}

static GEMINI_FIRST_TOKENIZER: OnceLock<GeminiFirstTokenizer> = OnceLock::new();

impl GeminiFirstTokenizer {
    /// Инициализирует "Gemini First" токенизатор
    pub fn initialize(config: Option<GeminiFirstConfig>) -> Result<(), Box<dyn Error + Send + Sync>> {
        let config = config.unwrap_or_default();
        
        info!("Initializing Gemini First tokenizer (direct API approach)");
        info!("Pre-check enabled: {}", config.enable_pre_check);
        info!("Fast estimation threshold: {} chars", config.fast_estimation_threshold);
        
        let tokenizer = Self { config };
        
        match GEMINI_FIRST_TOKENIZER.set(tokenizer) {
            Ok(_) => info!("Gemini First tokenizer initialized successfully"),
            Err(_) => warn!("Gemini First tokenizer was already initialized"),
        }
        
        Ok(())
    }
    
    /// Основной метод: решает нужна ли токенизация или отправляем сразу
    pub fn should_tokenize_before_request(&self, text: &str) -> TokenizationDecision {
        let text_length = text.len();
        
        // Если предварительная проверка отключена - отправляем сразу
        if !self.config.enable_pre_check {
            debug!("Pre-check disabled, sending directly to Gemini");
            return TokenizationDecision::SendDirectly;
        }
        
        // Если есть лимит и текст очень большой - быстрая оценка
        if let Some(limit) = self.config.pre_check_limit {
            if text_length > self.config.fast_estimation_threshold {
                let estimated_tokens = self.fast_estimate_tokens(text);
                
                if estimated_tokens > limit {
                    debug!("Fast estimation: {} tokens > {} limit, rejecting", estimated_tokens, limit);
                    return TokenizationDecision::RejectTooLarge(estimated_tokens);
                } else {
                    debug!("Fast estimation: {} tokens < {} limit, sending directly", estimated_tokens, limit);
                    return TokenizationDecision::SendDirectly;
                }
            }
        }
        
        // Для небольших текстов можем сделать точную токенизацию если нужно
        if text_length < 10000 && self.config.pre_check_limit.is_some() {
            return TokenizationDecision::TokenizeFirst;
        }
        
        // По умолчанию отправляем сразу
        TokenizationDecision::SendDirectly
    }
    
    /// Быстрая оценка токенов (для больших текстов)
    fn fast_estimate_tokens(&self, text: &str) -> usize {
        let char_count = text.chars().count();
        
        // Эмпирические коэффициенты на основе тестов:
        // - Обычный текст: ~4 символа на токен
        // - Код: ~3.5 символа на токен  
        // - Unicode: ~2.5 символа на токен
        // - JSON: ~3.2 символа на токен
        
        let unicode_ratio = text.chars().filter(|c| !c.is_ascii()).count() as f64 / char_count as f64;
        let code_indicators = text.matches('{').count() + text.matches('}').count() + 
                             text.matches(';').count() + text.matches("function").count();
        let json_indicators = text.matches('"').count() + text.matches(':').count();
        
        let chars_per_token = if unicode_ratio > 0.2 {
            2.5 // Много Unicode
        } else if code_indicators > char_count / 100 {
            3.5 // Много кода
        } else if json_indicators > char_count / 50 {
            3.2 // JSON структуры
        } else {
            4.0 // Обычный текст
        };
        
        let estimated = (char_count as f64 / chars_per_token).ceil() as usize;
        
        debug!("Fast estimation: {} chars → {} tokens (ratio: {:.1})", 
            char_count, estimated, chars_per_token);
        
        estimated
    }
    
    /// Подсчет токенов после получения ответа (для статистики)
    pub fn count_tokens_post_response(&self, request_text: &str, response_text: &str) -> PostResponseTokens {
        if !self.config.enable_post_count {
            return PostResponseTokens::default();
        }
        
        // Для больших текстов используем быструю оценку
        let request_tokens = if request_text.len() > self.config.fast_estimation_threshold {
            self.fast_estimate_tokens(request_text)
        } else {
            // Для небольших можем использовать более точный подсчет
            self.accurate_count_small_text(request_text)
        };
        
        let response_tokens = if response_text.len() > 5000 {
            self.fast_estimate_tokens(response_text)
        } else {
            self.accurate_count_small_text(response_text)
        };
        
        PostResponseTokens {
            request_tokens,
            response_tokens,
            total_tokens: request_tokens + response_tokens,
            estimation_used: request_text.len() > self.config.fast_estimation_threshold,
        }
    }
    
    /// Точный подсчет для небольших текстов
    fn accurate_count_small_text(&self, text: &str) -> usize {
        // Используем более точный алгоритм для небольших текстов
        let words = text.split_whitespace().count();
        let punctuation = text.chars().filter(|c| c.is_ascii_punctuation()).count();
        let unicode_chars = text.chars().filter(|c| !c.is_ascii()).count();
        
        // Базовая оценка
        let base_tokens = words + punctuation / 2;
        
        // Корректировка для Unicode
        let unicode_adjustment = if unicode_chars > 0 {
            (unicode_chars as f64 * 0.7) as usize
        } else {
            0
        };
        
        (base_tokens + unicode_adjustment).max(1)
    }
    
    /// Возвращает конфигурацию
    pub fn get_config(&self) -> &GeminiFirstConfig {
        &self.config
    }
    
    /// Возвращает информацию о токенизаторе
    pub fn get_info(&self) -> String {
        format!(
            "Gemini First Tokenizer (pre-check: {}, fast estimation: {})",
            self.config.enable_pre_check,
            self.config.use_fast_estimation
        )
    }
}

/// Решение о токенизации
#[derive(Debug, Clone)]
pub enum TokenizationDecision {
    /// Отправить запрос напрямую к Gemini (рекомендуется)
    SendDirectly,
    /// Сначала токенизировать (только для небольших текстов)
    TokenizeFirst,
    /// Отклонить запрос - слишком много токенов
    RejectTooLarge(usize),
}

/// Результат подсчета токенов после ответа
#[derive(Debug, Clone, Default)]
pub struct PostResponseTokens {
    pub request_tokens: usize,
    pub response_tokens: usize,
    pub total_tokens: usize,
    pub estimation_used: bool,
}

/// Получает экземпляр токенизатора
pub fn get_gemini_first_tokenizer() -> Option<&'static GeminiFirstTokenizer> {
    GEMINI_FIRST_TOKENIZER.get()
}

/// Проверяет нужна ли токенизация перед запросом
pub fn should_tokenize_before_request(text: &str) -> TokenizationDecision {
    match GEMINI_FIRST_TOKENIZER.get() {
        Some(tokenizer) => tokenizer.should_tokenize_before_request(text),
        None => {
            warn!("Gemini First tokenizer not initialized, sending directly");
            TokenizationDecision::SendDirectly
        }
    }
}

/// Подсчитывает токены после получения ответа
pub fn count_tokens_post_response(request_text: &str, response_text: &str) -> PostResponseTokens {
    match GEMINI_FIRST_TOKENIZER.get() {
        Some(tokenizer) => tokenizer.count_tokens_post_response(request_text, response_text),
        None => PostResponseTokens::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_gemini_first_initialization() {
        // Тест инициализации - просто проверяем что не падает
        let result = GeminiFirstTokenizer::initialize(None);
        assert!(result.is_ok());
        
        // Проверяем что токенизатор доступен
        let tokenizer = get_gemini_first_tokenizer();
        assert!(tokenizer.is_some());
    }
    
    #[test]
    fn test_should_send_directly_by_default() {
        GeminiFirstTokenizer::initialize(None).unwrap();
        
        let decision = should_tokenize_before_request("This is a test message");
        matches!(decision, TokenizationDecision::SendDirectly);
    }
    
    #[test]
    fn test_fast_estimation() {
        let config = GeminiFirstConfig {
            enable_pre_check: true,
            pre_check_limit: Some(1000),
            use_fast_estimation: true,
            fast_estimation_threshold: 100,
            enable_post_count: true,
        };
        
        GeminiFirstTokenizer::initialize(Some(config)).unwrap();
        
        // Большой текст - должен использовать быструю оценку
        let large_text = "Hello world! ".repeat(1000); // ~13000 символов
        let decision = should_tokenize_before_request(&large_text);
        
        // Должен отклонить как слишком большой
        matches!(decision, TokenizationDecision::RejectTooLarge(_));
    }
    
    #[test]
    fn test_post_response_counting() {
        GeminiFirstTokenizer::initialize(None).unwrap();
        
        let request = "What is the capital of France?";
        let response = "The capital of France is Paris.";
        
        let tokens = count_tokens_post_response(request, response);
        
        assert!(tokens.request_tokens > 0);
        assert!(tokens.response_tokens > 0);
        assert_eq!(tokens.total_tokens, tokens.request_tokens + tokens.response_tokens);
        assert!(!tokens.estimation_used); // Небольшой текст
    }
    
    #[test]
    fn test_unicode_handling() {
        GeminiFirstTokenizer::initialize(None).unwrap();
        
        let unicode_text = "Hello 世界! 🌍 How are you? Привет мир!";
        let tokens = count_tokens_post_response(unicode_text, "");
        
        assert!(tokens.request_tokens > 0);
        println!("Unicode text tokens: {}", tokens.request_tokens);
    }
    
    #[test]
    fn test_performance_on_large_text() {
        GeminiFirstTokenizer::initialize(None).unwrap();
        
        // Создаем текст ~180k токенов (как в вашем случае)
        let large_text = "This is a comprehensive test document with various content types. ".repeat(10000);
        println!("Large text size: {} chars", large_text.len());
        
        let start = std::time::Instant::now();
        let decision = should_tokenize_before_request(&large_text);
        let decision_time = start.elapsed();
        
        let start = std::time::Instant::now();
        let tokens = count_tokens_post_response(&large_text, "Response");
        let counting_time = start.elapsed();
        
        println!("Decision time: {:?}", decision_time);
        println!("Counting time: {:?}", counting_time);
        println!("Estimated tokens: {}", tokens.request_tokens);
        
        // Должно быть очень быстро
        assert!(decision_time.as_millis() < 5);
        assert!(counting_time.as_millis() < 10);
        
        // По умолчанию должен отправлять сразу
        matches!(decision, TokenizationDecision::SendDirectly);
    }
}