// src/tokenizer/multimodal.rs

use std::error::Error;
use std::sync::OnceLock;
use serde_json::Value;
use tracing::{info, warn, debug};


/// Конфигурация для multimodal токенизации
#[derive(Debug, Clone)]
pub struct MultimodalConfig {
    /// Поправочный коэффициент для безопасности (по умолчанию 1.2)
    pub safety_multiplier: f64,
    /// Максимальный размер изображения для обработки (в байтах)
    pub max_image_size: usize,
    /// Коэффициенты для разных типов изображений
    pub image_coefficients: ImageCoefficients,
    /// Включить детальное логирование
    pub debug_logging: bool,
}

#[derive(Debug, Clone)]
pub struct ImageCoefficients {
    /// Базовый коэффициент для JPEG/PNG (оптимизированы)
    pub jpeg_png_factor: f64,
    /// Коэффициент для WebP (более эффективный)
    pub webp_factor: f64,
    /// Коэффициент для GIF (менее эффективный)
    pub gif_factor: f64,
    /// Коэффициент для неизвестных форматов
    pub unknown_factor: f64,
}

impl Default for MultimodalConfig {
    fn default() -> Self {
        Self {
            safety_multiplier: 1.2,
            max_image_size: 20 * 1024 * 1024, // 20MB
            image_coefficients: ImageCoefficients {
                jpeg_png_factor: 0.85,  // JPEG/PNG более эффективны
                webp_factor: 0.75,      // WebP самый эффективный
                gif_factor: 1.1,        // GIF менее эффективный
                unknown_factor: 1.0,    // Консервативная оценка
            },
            debug_logging: false,
        }
    }
}

/// Результат подсчета токенов для multimodal контента
#[derive(Debug, Clone)]
pub struct TokenCount {
    /// Токены текста
    pub text_tokens: usize,
    /// Токены изображений
    pub image_tokens: usize,
    /// Общее количество токенов (с поправочным коэффициентом)
    pub total_tokens: usize,
    /// Количество обработанных изображений
    pub image_count: usize,
    /// Детали по каждому изображению
    pub image_details: Vec<ImageTokenInfo>,
}

#[derive(Debug, Clone)]
pub struct ImageTokenInfo {
    /// Размер base64 данных
    pub base64_size: usize,
    /// Предполагаемый размер декодированного изображения
    pub decoded_size: usize,
    /// Определенный формат изображения
    pub format: ImageFormat,
    /// Расчетное количество токенов
    pub estimated_tokens: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImageFormat {
    JPEG,
    PNG,
    WebP,
    GIF,
    Unknown,
}

/// Multimodal токенизатор для Gemini
pub struct MultimodalTokenizer {
    config: MultimodalConfig,
}

static MULTIMODAL_TOKENIZER: OnceLock<MultimodalTokenizer> = OnceLock::new();

impl MultimodalTokenizer {
    /// Инициализирует multimodal токенизатор
    pub fn initialize(config: Option<MultimodalConfig>) -> Result<(), Box<dyn Error + Send + Sync>> {
        let config = config.unwrap_or_default();
        
        info!(
            safety_multiplier = config.safety_multiplier,
            max_image_size = config.max_image_size,
            "Initializing multimodal tokenizer"
        );
        
        let tokenizer = Self { config };
        
        match MULTIMODAL_TOKENIZER.set(tokenizer) {
            Ok(_) => info!("Multimodal tokenizer initialized successfully"),
            Err(_) => warn!("Multimodal tokenizer was already initialized"),
        }
        
        Ok(())
    }
    
    /// Подсчитывает токены в multimodal сообщении
    pub fn count_tokens(&self, json_body: &Value) -> Result<TokenCount, Box<dyn Error + Send + Sync>> {
        let mut text_tokens = 0;
        let mut image_tokens = 0;
        let mut image_count = 0;
        let mut image_details = Vec::new();
        
        // Обрабатываем сообщения
        if let Some(messages) = json_body.get("messages").and_then(|m| m.as_array()) {
            for message in messages {
                let (msg_text_tokens, msg_image_tokens, msg_image_count, mut msg_details) = 
                    self.process_message(message)?;
                
                text_tokens += msg_text_tokens;
                image_tokens += msg_image_tokens;
                image_count += msg_image_count;
                image_details.append(&mut msg_details);
            }
        }
        
        // Применяем поправочный коэффициент
        let raw_total = text_tokens + image_tokens;
        let total_tokens = ((raw_total as f64) * self.config.safety_multiplier).ceil() as usize;
        
        if self.config.debug_logging {
            debug!(
                text_tokens,
                image_tokens,
                raw_total,
                total_tokens,
                image_count,
                safety_multiplier = self.config.safety_multiplier,
                "Multimodal token count calculated"
            );
        }
        
        Ok(TokenCount {
            text_tokens,
            image_tokens,
            total_tokens,
            image_count,
            image_details,
        })
    }
    
    /// Обрабатывает одно сообщение
    fn process_message(&self, message: &Value) -> Result<(usize, usize, usize, Vec<ImageTokenInfo>), Box<dyn Error + Send + Sync>> {
        let mut text_tokens = 0;
        let mut image_tokens = 0;
        let mut image_count = 0;
        let mut image_details = Vec::new();
        
        // Обрабатываем content
        if let Some(content) = message.get("content") {
            match content {
                // Простой текстовый контент
                Value::String(text) => {
                    text_tokens += self.count_text_tokens(text)?;
                }
                // Массив контента (multimodal)
                Value::Array(content_parts) => {
                    for part in content_parts {
                        if let Some(part_type) = part.get("type").and_then(|t| t.as_str()) {
                            match part_type {
                                "text" => {
                                    if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                        text_tokens += self.count_text_tokens(text)?;
                                    }
                                }
                                "image_url" => {
                                    if let Some(image_url) = part.get("image_url") {
                                        let (tokens, info) = self.count_image_tokens(image_url)?;
                                        image_tokens += tokens;
                                        image_count += 1;
                                        image_details.push(info);
                                    }
                                }
                                _ => {
                                    warn!(part_type, "Unknown content part type");
                                }
                            }
                        }
                    }
                }
                _ => {
                    // Fallback: считаем как текст
                    let text = content.to_string();
                    text_tokens += self.count_text_tokens(&text)?;
                }
            }
        }
        
        Ok((text_tokens, image_tokens, image_count, image_details))
    }
    
    /// Подсчитывает токены в тексте
    fn count_text_tokens(&self, text: &str) -> Result<usize, Box<dyn Error + Send + Sync>> {
        // Используем наш Gemini токенизатор
        crate::tokenizer::count_gemini_tokens(text)
    }
    
    /// Подсчитывает токены для изображения
    fn count_image_tokens(&self, image_url: &Value) -> Result<(usize, ImageTokenInfo), Box<dyn Error + Send + Sync>> {
        let url = image_url.get("url")
            .and_then(|u| u.as_str())
            .ok_or("Missing image URL")?;
        
        // Проверяем, что это base64 изображение
        if !url.starts_with("data:image/") {
            return Err("Only base64 images are supported for token counting".into());
        }
        
        // Извлекаем base64 данные
        let base64_data = url.split(',').nth(1)
            .ok_or("Invalid base64 image format")?;
        
        let base64_size = base64_data.len();
        
        // Проверяем размер
        if base64_size > self.config.max_image_size {
            return Err(format!("Image too large: {} bytes (max: {})", 
                base64_size, self.config.max_image_size).into());
        }
        
        // Определяем формат изображения
        let format = self.detect_image_format(url);
        
        // Оцениваем размер декодированного изображения
        let decoded_size = (base64_size * 3) / 4; // Приблизительно
        
        // Рассчитываем токены с улучшенной эвристикой
        let estimated_tokens = self.calculate_image_tokens(decoded_size, &format);
        
        let info = ImageTokenInfo {
            base64_size,
            decoded_size,
            format,
            estimated_tokens,
        };
        
        if self.config.debug_logging {
            debug!(
                base64_size,
                decoded_size,
                estimated_tokens,
                format = ?info.format,
                "Image token count calculated"
            );
        }
        
        Ok((estimated_tokens, info))
    }
    
    /// Определяет формат изображения по data URL
    fn detect_image_format(&self, data_url: &str) -> ImageFormat {
        if data_url.starts_with("data:image/jpeg") || data_url.starts_with("data:image/jpg") {
            ImageFormat::JPEG
        } else if data_url.starts_with("data:image/png") {
            ImageFormat::PNG
        } else if data_url.starts_with("data:image/webp") {
            ImageFormat::WebP
        } else if data_url.starts_with("data:image/gif") {
            ImageFormat::GIF
        } else {
            ImageFormat::Unknown
        }
    }
    
    /// Рассчитывает токены для изображения с улучшенной эвристикой
    fn calculate_image_tokens(&self, decoded_size: usize, format: &ImageFormat) -> usize {
        // Базовая формула: более сложная чем простой sqrt
        // Учитывает, что токены растут не линейно с размером
        
        let base_tokens = if decoded_size < 1024 * 1024 {
            // Маленькие изображения (< 1MB): более эффективная упаковка
            ((decoded_size as f64).sqrt() * 0.8).ceil() as usize
        } else if decoded_size < 5 * 1024 * 1024 {
            // Средние изображения (1-5MB): стандартная формула
            ((decoded_size as f64).sqrt()).ceil() as usize
        } else {
            // Большие изображения (> 5MB): менее эффективная упаковка
            ((decoded_size as f64).sqrt() * 1.2).ceil() as usize
        };
        
        // Применяем коэффициент для формата
        let format_factor = match format {
            ImageFormat::JPEG => self.config.image_coefficients.jpeg_png_factor,
            ImageFormat::PNG => self.config.image_coefficients.jpeg_png_factor,
            ImageFormat::WebP => self.config.image_coefficients.webp_factor,
            ImageFormat::GIF => self.config.image_coefficients.gif_factor,
            ImageFormat::Unknown => self.config.image_coefficients.unknown_factor,
        };
        
        let adjusted_tokens = (base_tokens as f64 * format_factor).ceil() as usize;
        
        // Минимум 10 токенов для любого изображения
        adjusted_tokens.max(10)
    }
}

/// Подсчитывает токены в multimodal сообщении
pub fn count_multimodal_tokens(json_body: &Value) -> Result<TokenCount, Box<dyn Error + Send + Sync>> {
    let tokenizer = MULTIMODAL_TOKENIZER
        .get()
        .ok_or("Multimodal tokenizer not initialized. Call MultimodalTokenizer::initialize() first.")?;
    
    tokenizer.count_tokens(json_body)
}

/// Возвращает конфигурацию multimodal токенизатора
pub fn get_multimodal_config() -> Option<&'static MultimodalConfig> {
    MULTIMODAL_TOKENIZER.get().map(|t| &t.config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    
    #[tokio::test]
    async fn test_multimodal_tokenizer_initialization() {
        let config = MultimodalConfig {
            debug_logging: true,
            ..Default::default()
        };
        
        let result = MultimodalTokenizer::initialize(Some(config));
        assert!(result.is_ok(), "Multimodal tokenizer initialization failed: {:?}", result);
    }
    
    #[tokio::test]
    async fn test_text_only_message() {
        // Инициализируем оба токенизатора
        crate::tokenizer::GeminiTokenizer::initialize().await.unwrap();
        MultimodalTokenizer::initialize(None).unwrap();
        
        let message = json!({
            "messages": [
                {
                    "role": "user",
                    "content": "Hello, how are you today?"
                }
            ]
        });
        
        let result = count_multimodal_tokens(&message).unwrap();
        
        assert!(result.text_tokens > 0);
        assert_eq!(result.image_tokens, 0);
        assert_eq!(result.image_count, 0);
        assert!(result.total_tokens >= result.text_tokens);
        
        println!("Text-only result: {:?}", result);
    }
    
    #[tokio::test]
    async fn test_multimodal_message() {
        // Инициализируем оба токенизатора
        crate::tokenizer::GeminiTokenizer::initialize().await.unwrap();
        MultimodalTokenizer::initialize(None).unwrap();
        
        // Создаем простое тестовое изображение (1x1 PNG)
        let tiny_png_base64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==";
        
        let message = json!({
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "What do you see in this image?"
                        },
                        {
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:image/png;base64,{}", tiny_png_base64)
                            }
                        }
                    ]
                }
            ]
        });
        
        let result = count_multimodal_tokens(&message).unwrap();
        
        assert!(result.text_tokens > 0);
        assert!(result.image_tokens > 0);
        assert_eq!(result.image_count, 1);
        assert!(result.total_tokens > result.text_tokens + result.image_tokens);
        assert_eq!(result.image_details.len(), 1);
        assert_eq!(result.image_details[0].format, ImageFormat::PNG);
        
        println!("Multimodal result: {:?}", result);
    }
    
    #[tokio::test]
    async fn test_performance() {
        // Инициализируем оба токенизатора
        crate::tokenizer::GeminiTokenizer::initialize().await.unwrap();
        MultimodalTokenizer::initialize(None).unwrap();
        
        let message = json!({
            "messages": [
                {
                    "role": "user",
                    "content": "This is a performance test for multimodal tokenization."
                }
            ]
        });
        
        let iterations = 1000;
        let start = std::time::Instant::now();
        
        for _ in 0..iterations {
            let _ = count_multimodal_tokens(&message).unwrap();
        }
        
        let duration = start.elapsed();
        println!("{} multimodal tokenizations took: {:?}", iterations, duration);
        println!("Average: {:?} per tokenization", duration / iterations);
        
        // Должно быть быстро
        assert!(duration.as_millis() < 200, "Multimodal tokenization should be fast");
    }
}