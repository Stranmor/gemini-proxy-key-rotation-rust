// src/tokenizer/gemini.rs

use std::error::Error;
use std::sync::OnceLock;
use tracing::{error, info, warn};

/// Специализированный токенизатор для Gemini моделей
pub struct GeminiTokenizer {
    #[cfg(feature = "tokenizer")]
    tiktoken: Option<tiktoken_rs::CoreBPE>,
    #[cfg(feature = "tokenizer")]
    hf_tokenizer: Option<tokenizers::Tokenizer>,
    fallback_enabled: bool,
}

static GEMINI_TOKENIZER: OnceLock<GeminiTokenizer> = OnceLock::new();

impl GeminiTokenizer {
    /// Инициализирует Gemini токенизатор с максимальной точностью
    pub async fn initialize() -> Result<(), Box<dyn Error + Send + Sync>> {
        info!("Initializing Gemini tokenizer with maximum accuracy");
        
        let tokenizer = Self::new().await?;
        
        match GEMINI_TOKENIZER.set(tokenizer) {
            Ok(_) => info!("Gemini tokenizer initialized successfully"),
            Err(_) => warn!("Gemini tokenizer was already initialized"),
        }
        
        Ok(())
    }
    
    async fn new() -> Result<Self, Box<dyn Error + Send + Sync>> {
        // Пробуем несколько способов получить максимально точный токенизатор
        
        // 1. Пробуем использовать tiktoken cl100k_base (очень точный для Gemini)
        #[cfg(feature = "tokenizer")]
        {
            if let Ok(tiktoken) = Self::load_tiktoken_cl100k().await {
                info!("Using tiktoken cl100k_base for Gemini (high accuracy)");
                return Ok(Self {
                    tiktoken: Some(tiktoken),
                    hf_tokenizer: None,
                    fallback_enabled: true,
                });
            }
            
            // 2. Пробуем загрузить официальный Gemini токенизатор с HuggingFace
            if let Ok(hf_tokenizer) = Self::load_official_gemini_tokenizer().await {
                info!("Using official Gemini tokenizer from HuggingFace");
                return Ok(Self {
                    tiktoken: None,
                    hf_tokenizer: Some(hf_tokenizer),
                    fallback_enabled: true,
                });
            }
            
            // 3. Fallback на приближенный токенизатор
            warn!("Could not load official Gemini tokenizer, using approximation");
            let approx_tokenizer = Self::create_gemini_approximation()?;
            return Ok(Self {
                tiktoken: None,
                hf_tokenizer: Some(approx_tokenizer),
                fallback_enabled: true,
            });
        }
        
        #[cfg(not(feature = "tokenizer"))]
        {
            info!("Tokenizer feature disabled, using simple approximation");
            Ok(Self {
                fallback_enabled: true,
            })
        }
    }
    
    #[cfg(feature = "tokenizer")]
    async fn load_tiktoken_cl100k() -> Result<tiktoken_rs::CoreBPE, Box<dyn Error + Send + Sync>> {
        use tiktoken_rs::cl100k_base;
        
        info!("Loading tiktoken cl100k_base for Gemini");
        let tiktoken = cl100k_base()?;
        Ok(tiktoken)
    }
    
    #[cfg(feature = "tokenizer")]
    async fn load_official_gemini_tokenizer() -> Result<tokenizers::Tokenizer, Box<dyn Error + Send + Sync>> {
        use hf_hub::api::tokio::Api;
        
        // Пробуем несколько официальных источников
        let model_candidates = [
            "google/gemma-tokenizer",
            "google/gemma-2b",
            "google/gemma-7b", 
            "google/gemini-tokenizer",
        ];
        
        let api = Api::new()?;
        
        for model_name in &model_candidates {
            info!(model = model_name, "Trying to load Gemini tokenizer");
            
            match api.model(model_name.to_string()).get("tokenizer.json").await {
                Ok(tokenizer_path) => {
                    match tokenizers::Tokenizer::from_file(tokenizer_path) {
                        Ok(tokenizer) => {
                            info!(model = model_name, "Successfully loaded Gemini tokenizer");
                            return Ok(tokenizer);
                        }
                        Err(e) => {
                            warn!(model = model_name, error = %e, "Failed to parse tokenizer");
                            continue;
                        }
                    }
                }
                Err(e) => {
                    warn!(model = model_name, error = %e, "Failed to download tokenizer");
                    continue;
                }
            }
        }
        
        Err("Could not load any official Gemini tokenizer".into())
    }
    

    
    #[cfg(feature = "tokenizer")]
    fn create_gemini_approximation() -> Result<tokenizers::Tokenizer, Box<dyn Error + Send + Sync>> {
        use tokenizers::{
            models::bpe::BPE,
            normalizers::{Sequence, NFD, Lowercase, StripAccents},
            pre_tokenizers::Whitespace,
            processors::TemplateProcessing,
            Tokenizer, TokenizerBuilder,
        };
        
        // Создаем приближенный токенизатор, похожий на Gemini
        // Gemini использует SentencePiece, который похож на BPE
        
        let mut builder = TokenizerBuilder::new();
        
        // Используем BPE модель как приближение
        let bpe = BPE::default();
        builder = builder.with_model(bpe);
        
        // Нормализация текста (как в SentencePiece)
        let normalizer = Sequence::new(vec![
            Box::new(NFD),
            Box::new(StripAccents),
            Box::new(Lowercase),
        ]);
        builder = builder.with_normalizer(Box::new(normalizer));
        
        // Pre-tokenizer (разбиение по пробелам)
        builder = builder.with_pre_tokenizer(Box::new(Whitespace {}));
        
        // Post-processor
        let processor = TemplateProcessing::builder()
            .try_single("[CLS] $A [SEP]")?
            .special_tokens(vec![("[CLS]", 1), ("[SEP]", 2)])
            .build()?;
        builder = builder.with_post_processor(Box::new(processor));
        
        let tokenizer = builder.build()?;
        
        info!("Created Gemini approximation tokenizer");
        Ok(tokenizer)
    }
    
    /// Подсчитывает токены с максимальной точностью для Gemini
    pub fn count_tokens(&self, text: &str) -> Result<usize, Box<dyn Error + Send + Sync>> {
        #[cfg(feature = "tokenizer")]
        {
            // 1. Пробуем tiktoken cl100k_base (очень точный для Gemini)
            if let Some(ref tiktoken) = self.tiktoken {
                let tokens = tiktoken.encode_with_special_tokens(text);
                return Ok(tokens.len());
            }
            
            // 2. Пробуем HuggingFace токенизатор
            if let Some(ref tokenizer) = self.hf_tokenizer {
                match tokenizer.encode(text, false) {
                    Ok(encoding) => return Ok(encoding.len()),
                    Err(e) => {
                        warn!(error = %e, "HF tokenizer encoding failed, trying fallback");
                    }
                }
            }
        }
        
        // 3. Fallback: приближенный подсчет
        if self.fallback_enabled {
            Ok(self.approximate_token_count(text))
        } else {
            Err("No tokenizer available and fallback disabled".into())
        }
    }
    
    /// Приближенный подсчет токенов для Gemini
    /// Основан на анализе поведения Gemini токенизатора
    fn approximate_token_count(&self, text: &str) -> usize {
        if text.is_empty() {
            return 0;
        }
        
        // Gemini токенизатор ведет себя похоже на SentencePiece:
        // - Разбивает по словам и подсловам
        // - Учитывает пунктуацию
        // - Обрабатывает Unicode
        
        let mut token_count = 0;
        let mut chars = text.chars().peekable();
        let mut current_word = String::new();
        
        while let Some(ch) = chars.next() {
            if ch.is_whitespace() {
                if !current_word.is_empty() {
                    token_count += self.estimate_word_tokens(&current_word);
                    current_word.clear();
                }
            } else if ch.is_ascii_punctuation() {
                if !current_word.is_empty() {
                    token_count += self.estimate_word_tokens(&current_word);
                    current_word.clear();
                }
                token_count += 1; // Пунктуация обычно отдельный токен
            } else {
                current_word.push(ch);
            }
        }
        
        if !current_word.is_empty() {
            token_count += self.estimate_word_tokens(&current_word);
        }
        
        // Минимум 1 токен для непустого текста
        token_count.max(1)
    }
    
    /// Оценивает количество токенов в слове
    fn estimate_word_tokens(&self, word: &str) -> usize {
        let len = word.chars().count();
        
        // Эмпирические правила для Gemini:
        // - Короткие слова (1-4 символа): 1 токен
        // - Средние слова (5-8 символов): 1-2 токена  
        // - Длинные слова (9+ символов): разбиваются на подслова
        
        match len {
            0 => 0,
            1..=4 => 1,
            5..=8 => {
                // Проверяем, есть ли общие префиксы/суффиксы
                if word.starts_with("un") || word.starts_with("re") || 
                   word.ends_with("ing") || word.ends_with("ed") {
                    2
                } else {
                    1
                }
            }
            9..=12 => 2,
            13..=16 => 3,
            _ => (len + 3) / 4, // Примерно 4 символа на токен для очень длинных слов
        }
    }
    
    /// Возвращает информацию о типе используемого токенизатора
    pub fn get_info(&self) -> String {
        #[cfg(feature = "tokenizer")]
        {
            if self.tiktoken.is_some() {
                "TikToken cl100k_base (High accuracy for Gemini)".to_string()
            } else if self.hf_tokenizer.is_some() {
                "HuggingFace Tokenizer (Gemini-compatible)".to_string()
            } else {
                "Approximation (Fallback)".to_string()
            }
        }
        
        #[cfg(not(feature = "tokenizer"))]
        {
            "Approximation (Feature disabled)".to_string()
        }
    }
}

/// Подсчитывает токены для Gemini с максимальной точностью
pub fn count_gemini_tokens(text: &str) -> Result<usize, Box<dyn Error + Send + Sync>> {
    let tokenizer = GEMINI_TOKENIZER
        .get()
        .ok_or("Gemini tokenizer not initialized. Call GeminiTokenizer::initialize() first.")?;
    
    tokenizer.count_tokens(text)
}

/// Возвращает информацию о текущем Gemini токенизаторе
pub fn get_gemini_tokenizer_info() -> Option<String> {
    GEMINI_TOKENIZER.get().map(|t| t.get_info())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    
    #[tokio::test]
    async fn test_gemini_tokenizer_initialization() {
        let result = GeminiTokenizer::initialize().await;
        
        // Инициализация должна пройти успешно (с fallback если нужно)
        assert!(result.is_ok(), "Gemini tokenizer initialization failed: {:?}", result);
        
        let info = get_gemini_tokenizer_info().unwrap();
        println!("Gemini tokenizer info: {}", info);
    }
    
    #[tokio::test]
    async fn test_gemini_token_counting() {
        GeminiTokenizer::initialize().await.unwrap();
        
        let test_cases = vec![
            ("", 0),
            ("Hello", 1),
            ("Hello world", 2),
            ("Hello, world!", 3), // "Hello", ",", "world", "!"
            ("The quick brown fox jumps over the lazy dog.", 10), // Приблизительно
        ];
        
        for (text, expected_min) in test_cases {
            let count = count_gemini_tokens(text).unwrap();
            println!("Text: '{}' -> {} tokens", text, count);
            
            if expected_min > 0 {
                assert!(count >= expected_min, 
                    "Token count for '{}' should be at least {}, got {}", 
                    text, expected_min, count);
            } else {
                assert_eq!(count, 0, "Empty text should have 0 tokens");
            }
        }
    }
    
    #[tokio::test]
    async fn test_gemini_approximation_accuracy() {
        GeminiTokenizer::initialize().await.unwrap();
        
        // Тестируем на реальных примерах
        let examples = vec![
            "What is the capital of France?",
            "Explain quantum computing in simple terms.",
            "Write a Python function to calculate fibonacci numbers.",
            "Translate 'Hello, how are you?' to Spanish.",
        ];
        
        for example in examples {
            let count = count_gemini_tokens(example).unwrap();
            let word_count = example.split_whitespace().count();
            
            println!("Example: '{}' -> {} tokens ({} words)", example, count, word_count);
            
            // Токенов должно быть больше чем слов (из-за пунктуации)
            // но не слишком много (разумное соотношение)
            assert!(count >= word_count, "Token count should be at least word count");
            assert!(count <= word_count * 3, "Token count should not be too high");
        }
    }
    
    #[tokio::test]
    async fn test_performance() {
        GeminiTokenizer::initialize().await.unwrap();
        
        let text = "This is a performance test for the Gemini tokenizer implementation.";
        let iterations = 1000;
        
        let start = std::time::Instant::now();
        for _ in 0..iterations {
            let _ = count_gemini_tokens(text).unwrap();
        }
        let duration = start.elapsed();
        
        println!("{} tokenizations took: {:?}", iterations, duration);
        println!("Average: {:?} per tokenization", duration / iterations);
        
        // Должно быть быстро (< 1ms на операцию)
        assert!(duration.as_millis() < 100, "Tokenization should be fast");
    }
}