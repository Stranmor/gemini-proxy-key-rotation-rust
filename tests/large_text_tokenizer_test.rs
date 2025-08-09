// tests/large_text_tokenizer_test.rs

use std::env;
use std::error::Error;
use serde_json::{json, Value};
use reqwest::Client;
use tokio::time::{sleep, Duration};
use tracing::warn;
use gemini_proxy::tokenizer;

/// Тест токенизаторов на больших текстах
#[tokio::test]
async fn test_large_text_tokenization() {
    tracing_subscriber::fmt::init();
    
    let api_key = match env::var("GOOGLE_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("GOOGLE_API_KEY not found, skipping large text test");
            return;
        }
    };
    
    println!("\n📚 LARGE TEXT TOKENIZATION TEST\n");
    
    // Инициализируем токенизаторы
    tokenizer::gemini_simple::GeminiTokenizer::initialize().await.unwrap();
    tokenizer::gemini_ml_calibrated::GeminiMLCalibratedTokenizer::initialize().await.unwrap();
    
    let proxy_tokenizer = tokenizer::ProxyCachedTokenizer::new(api_key.clone())
        .with_fallback(|text| text.split_whitespace().count() + text.len() / 20);
    
    // Большие тексты разных типов
    let large_texts = vec![
        (
            "Technical Documentation",
            generate_technical_doc(),
        ),
        (
            "Code File",
            generate_large_code_file(),
        ),
        (
            "Natural Language",
            generate_natural_language_text(),
        ),
        (
            "Mixed Content",
            generate_mixed_content(),
        ),
        (
            "Unicode Heavy",
            generate_unicode_heavy_text(),
        ),
    ];
    
    let client = Client::new();
    
    println!("{:<20} | {:>8} | {:>8} | {:>8} | {:>8} | {:>8} | {:>8}", 
        "Text Type", "Length", "Google", "Simple", "ML-Cal", "Proxy", "Accuracy");
    println!("{:-<20}-+-{:->8}-+-{:->8}-+-{:->8}-+-{:->8}-+-{:->8}-+-{:->8}", 
        "", "", "", "", "", "", "");
    
    let mut total_tests = 0;
    let mut proxy_perfect = 0;
    let mut simple_good = 0;
    let mut ml_good = 0;
    
    for (name, text) in large_texts {
        let text_length = text.len();
        
        println!("🔍 Testing: {} ({} chars)", name, text_length);
        
        // Google API (эталон)
        let google_count = match get_google_token_count(&client, &api_key, &text).await {
            Ok(count) => count,
            Err(e) => {
                warn!("Google API failed for {}: {}", name, e);
                sleep(Duration::from_millis(2000)).await;
                continue;
            }
        };
        
        // Наши токенизаторы
        let simple_count = tokenizer::count_gemini_tokens(&text).unwrap_or(0);
        let ml_count = tokenizer::count_ml_calibrated_gemini_tokens(&text).unwrap_or(0);
        let proxy_count = proxy_tokenizer.count_tokens(&text).await.unwrap_or(0);
        
        // Вычисляем точность
        let simple_accuracy = calculate_accuracy(simple_count, google_count);
        let ml_accuracy = calculate_accuracy(ml_count, google_count);
        let proxy_accuracy = calculate_accuracy(proxy_count, google_count);
        
        total_tests += 1;
        if proxy_accuracy >= 99.0 { proxy_perfect += 1; }
        if simple_accuracy >= 85.0 { simple_good += 1; }
        if ml_accuracy >= 85.0 { ml_good += 1; }
        
        let best_accuracy = [simple_accuracy, ml_accuracy, proxy_accuracy]
            .iter().fold(0.0f64, |a, &b| a.max(b));
        
        println!("{:<20} | {:>8} | {:>8} | {:>8} | {:>8} | {:>8} | {:>7.1}%", 
            name, text_length, google_count, simple_count, ml_count, proxy_count, best_accuracy);
        
        // Детальный анализ для больших расхождений
        if best_accuracy < 90.0 {
            println!("  ⚠️  Large discrepancy detected!");
            println!("    Simple error: {}", (simple_count as i32 - google_count as i32).abs());
            println!("    ML error: {}", (ml_count as i32 - google_count as i32).abs());
            println!("    Proxy error: {}", (proxy_count as i32 - google_count as i32).abs());
        }
        
        sleep(Duration::from_millis(1000)).await;
    }
    
    // Итоговая статистика
    println!("\n📊 LARGE TEXT RESULTS\n");
    
    let proxy_score = (proxy_perfect as f64 / total_tests as f64) * 100.0;
    let simple_score = (simple_good as f64 / total_tests as f64) * 100.0;
    let ml_score = (ml_good as f64 / total_tests as f64) * 100.0;
    
    println!("🎯 Performance on Large Texts:");
    println!("  Proxy-Cached (>99%):  {:.1}%", proxy_score);
    println!("  Simple (>85%):        {:.1}%", simple_score);
    println!("  ML-Calibrated (>85%): {:.1}%", ml_score);
    
    // Тест производительности на больших текстах
    println!("\n⚡ PERFORMANCE ON LARGE TEXTS\n");
    
    let large_text = generate_very_large_text();
    println!("Testing performance on {} character text", large_text.len());
    
    // Простой токенизатор
    let start = std::time::Instant::now();
    let _ = tokenizer::count_gemini_tokens(&large_text).unwrap();
    let simple_time = start.elapsed();
    
    // ML-калиброванный
    let start = std::time::Instant::now();
    let _ = tokenizer::count_ml_calibrated_gemini_tokens(&large_text).unwrap();
    let ml_time = start.elapsed();
    
    println!("Performance Results:");
    println!("  Simple:        {:>8.2}ms", simple_time.as_millis());
    println!("  ML-Calibrated: {:>8.2}ms", ml_time.as_millis());
    
    // Проверяем что производительность приемлемая даже для больших текстов
    assert!(simple_time.as_millis() < 100, "Simple tokenizer too slow on large text");
    assert!(ml_time.as_millis() < 200, "ML tokenizer too slow on large text");
    
    println!("\n✅ Large text tokenization test completed!");
}

/// Генерирует техническую документацию
fn generate_technical_doc() -> String {
    r#"
# Gemini API Token Counting Documentation

## Overview

The Gemini API uses tokens as the fundamental unit for processing text input and generating responses. Understanding token counting is crucial for:

1. **Cost Estimation**: Billing is based on token consumption
2. **Rate Limiting**: API limits are enforced per token
3. **Context Management**: Models have token-based context windows
4. **Performance Optimization**: Token count affects processing time

## Token Calculation Methods

### Method 1: Local Tokenization

```python
from vertexai.preview import tokenization

model_name = "gemini-1.5-flash-001"
tokenizer = tokenization.get_tokenizer_for_model(model_name)
result = tokenizer.count_tokens("Hello World!")
print(f"Tokens: {result.total_tokens}")
```

### Method 2: API-based Counting

```bash
curl -X POST \
  "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:countTokens?key=$API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "contents": [{
      "parts": [{"text": "Hello World!"}]
    }]
  }'
```

## Best Practices

1. **Cache Results**: Token counts for identical text remain constant
2. **Batch Processing**: Group multiple texts for efficiency
3. **Monitor Usage**: Track token consumption for cost control
4. **Optimize Prompts**: Reduce unnecessary tokens in system prompts

## Model-Specific Considerations

- **Gemini 1.0 Pro**: 32,760 input tokens maximum
- **Gemini 1.5 Pro**: 2,097,152 input tokens maximum
- **Gemini 2.0 Flash**: 1,048,576 input tokens maximum

## Error Handling

Always implement proper error handling for token counting operations:

```rust
match tokenizer.count_tokens(text) {
    Ok(count) => println!("Token count: {}", count),
    Err(e) => eprintln!("Tokenization failed: {}", e),
}
```

## Performance Metrics

Typical tokenization performance:
- Small texts (<1KB): <1ms
- Medium texts (1-10KB): 1-5ms  
- Large texts (10-100KB): 5-50ms
- Very large texts (>100KB): 50-500ms

## Troubleshooting

Common issues and solutions:

1. **Inconsistent Counts**: Ensure using same model version
2. **Performance Issues**: Consider caching for repeated texts
3. **API Errors**: Implement retry logic with exponential backoff
4. **Memory Usage**: Monitor cache size for long-running applications
"#.to_string()
}

/// Генерирует большой файл кода
fn generate_large_code_file() -> String {
    r#"
// Large Rust code file for tokenization testing
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::time::{sleep, Duration};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenizerConfig {
    pub model_name: String,
    pub max_tokens: usize,
    pub temperature: f64,
    pub top_p: f64,
    pub cache_enabled: bool,
    pub cache_size_mb: usize,
    pub timeout_secs: u64,
}

impl Default for TokenizerConfig {
    fn default() -> Self {
        Self {
            model_name: "gemini-2.0-flash".to_string(),
            max_tokens: 1048576,
            temperature: 0.7,
            top_p: 0.9,
            cache_enabled: true,
            cache_size_mb: 100,
            timeout_secs: 30,
        }
    }
}

pub struct AdvancedTokenizer {
    config: TokenizerConfig,
    cache: Arc<Mutex<HashMap<String, usize>>>,
    stats: Arc<Mutex<TokenizerStats>>,
}

#[derive(Debug, Default)]
struct TokenizerStats {
    total_requests: u64,
    cache_hits: u64,
    cache_misses: u64,
    total_tokens_processed: u64,
    average_processing_time_ms: f64,
    errors: u64,
}

impl AdvancedTokenizer {
    pub fn new(config: TokenizerConfig) -> Self {
        Self {
            config,
            cache: Arc::new(Mutex::new(HashMap::new())),
            stats: Arc::new(Mutex::new(TokenizerStats::default())),
        }
    }
    
    pub async fn count_tokens(&self, text: &str) -> Result<usize, TokenizerError> {
        let start_time = std::time::Instant::now();
        
        // Update stats
        {
            let mut stats = self.stats.lock().unwrap();
            stats.total_requests += 1;
        }
        
        // Check cache first
        if self.config.cache_enabled {
            let cache_key = self.generate_cache_key(text);
            
            if let Ok(cache) = self.cache.lock() {
                if let Some(&cached_count) = cache.get(&cache_key) {
                    let mut stats = self.stats.lock().unwrap();
                    stats.cache_hits += 1;
                    return Ok(cached_count);
                }
            }
        }
        
        // Perform actual tokenization
        let token_count = self.perform_tokenization(text).await?;
        
        // Update cache
        if self.config.cache_enabled {
            if let Ok(mut cache) = self.cache.lock() {
                let cache_key = self.generate_cache_key(text);
                cache.insert(cache_key, token_count);
                
                // Cleanup cache if too large
                if cache.len() > 10000 {
                    cache.clear();
                }
            }
        }
        
        // Update stats
        {
            let mut stats = self.stats.lock().unwrap();
            stats.cache_misses += 1;
            stats.total_tokens_processed += token_count as u64;
            
            let processing_time = start_time.elapsed().as_millis() as f64;
            stats.average_processing_time_ms = 
                (stats.average_processing_time_ms * (stats.total_requests - 1) as f64 + processing_time) 
                / stats.total_requests as f64;
        }
        
        Ok(token_count)
    }
    
    async fn perform_tokenization(&self, text: &str) -> Result<usize, TokenizerError> {
        // Simulate different tokenization strategies
        match self.config.model_name.as_str() {
            "gemini-1.0-pro" => self.tokenize_v1(text).await,
            "gemini-1.5-pro" | "gemini-1.5-flash" => self.tokenize_v15(text).await,
            "gemini-2.0-flash" => self.tokenize_v2(text).await,
            _ => Err(TokenizerError::UnsupportedModel(self.config.model_name.clone())),
        }
    }
    
    async fn tokenize_v1(&self, text: &str) -> Result<usize, TokenizerError> {
        // Gemini 1.0 tokenization logic
        let base_count = text.split_whitespace().count();
        let punctuation_count = text.chars().filter(|c| c.is_ascii_punctuation()).count();
        Ok(base_count + punctuation_count / 2)
    }
    
    async fn tokenize_v15(&self, text: &str) -> Result<usize, TokenizerError> {
        // Gemini 1.5 tokenization logic (more sophisticated)
        let words = text.split_whitespace().collect::<Vec<_>>();
        let mut token_count = 0;
        
        for word in words {
            token_count += match word.len() {
                0 => 0,
                1..=4 => 1,
                5..=8 => if word.contains('-') || word.contains('_') { 2 } else { 1 },
                9..=12 => 2,
                _ => (word.len() + 3) / 4,
            };
        }
        
        // Add tokens for punctuation and special characters
        let special_chars = text.chars().filter(|c| !c.is_alphanumeric() && !c.is_whitespace()).count();
        token_count += special_chars / 2;
        
        Ok(token_count)
    }
    
    async fn tokenize_v2(&self, text: &str) -> Result<usize, TokenizerError> {
        // Gemini 2.0 tokenization logic (most advanced)
        let mut token_count = 0;
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        
        while i < chars.len() {
            if chars[i].is_whitespace() {
                i += 1;
                continue;
            }
            
            // Extract word or token
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            
            let token = chars[start..i].iter().collect::<String>();
            token_count += self.estimate_token_count_v2(&token);
        }
        
        Ok(token_count)
    }
    
    fn estimate_token_count_v2(&self, token: &str) -> usize {
        // Advanced token estimation for Gemini 2.0
        let len = token.chars().count();
        let has_numbers = token.chars().any(|c| c.is_ascii_digit());
        let has_special = token.chars().any(|c| c.is_ascii_punctuation());
        let has_unicode = token.chars().any(|c| !c.is_ascii());
        
        let base_tokens = match len {
            0 => 0,
            1..=3 => 1,
            4..=6 => if has_special || has_numbers { 2 } else { 1 },
            7..=10 => if has_unicode { 2 } else { 1 + (len - 6) / 3 },
            _ => (len + 2) / 3,
        };
        
        base_tokens.max(1)
    }
    
    fn generate_cache_key(&self, text: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        self.config.model_name.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
    
    pub fn get_stats(&self) -> TokenizerStats {
        self.stats.lock().unwrap().clone()
    }
    
    pub fn clear_cache(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.clear();
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TokenizerError {
    #[error("Unsupported model: {0}")]
    UnsupportedModel(String),
    
    #[error("Tokenization failed: {0}")]
    TokenizationFailed(String),
    
    #[error("Cache error: {0}")]
    CacheError(String),
    
    #[error("Configuration error: {0}")]
    ConfigError(String),
}

// Helper functions and utilities
pub mod utils {
    use super::*;
    
    pub fn estimate_tokens_simple(text: &str) -> usize {
        // Simple estimation: ~4 characters per token
        (text.len() + 3) / 4
    }
    
    pub fn estimate_tokens_by_words(text: &str) -> usize {
        // Word-based estimation
        let words = text.split_whitespace().count();
        let punctuation = text.chars().filter(|c| c.is_ascii_punctuation()).count();
        words + punctuation / 2
    }
    
    pub fn analyze_text_complexity(text: &str) -> TextComplexity {
        let total_chars = text.len();
        let words = text.split_whitespace().count();
        let sentences = text.matches('.').count() + text.matches('!').count() + text.matches('?').count();
        let unicode_chars = text.chars().filter(|c| !c.is_ascii()).count();
        let numbers = text.chars().filter(|c| c.is_ascii_digit()).count();
        let punctuation = text.chars().filter(|c| c.is_ascii_punctuation()).count();
        
        TextComplexity {
            total_chars,
            words,
            sentences,
            unicode_chars,
            numbers,
            punctuation,
            avg_word_length: if words > 0 { total_chars as f64 / words as f64 } else { 0.0 },
            complexity_score: calculate_complexity_score(total_chars, words, unicode_chars, punctuation),
        }
    }
    
    fn calculate_complexity_score(chars: usize, words: usize, unicode: usize, punct: usize) -> f64 {
        let base_score = chars as f64 / 100.0;
        let word_factor = if words > 0 { chars as f64 / words as f64 } else { 1.0 };
        let unicode_factor = 1.0 + (unicode as f64 / chars as f64) * 0.5;
        let punct_factor = 1.0 + (punct as f64 / chars as f64) * 0.3;
        
        base_score * word_factor * unicode_factor * punct_factor
    }
}

#[derive(Debug, Clone)]
pub struct TextComplexity {
    pub total_chars: usize,
    pub words: usize,
    pub sentences: usize,
    pub unicode_chars: usize,
    pub numbers: usize,
    pub punctuation: usize,
    pub avg_word_length: f64,
    pub complexity_score: f64,
}

// Integration tests
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_advanced_tokenizer() {
        let config = TokenizerConfig::default();
        let tokenizer = AdvancedTokenizer::new(config);
        
        let test_text = "Hello, world! This is a test of the advanced tokenizer.";
        let count = tokenizer.count_tokens(test_text).await.unwrap();
        
        assert!(count > 0);
        assert!(count < test_text.len());
        
        let stats = tokenizer.get_stats();
        assert_eq!(stats.total_requests, 1);
        assert_eq!(stats.cache_misses, 1);
    }
    
    #[test]
    fn test_text_complexity_analysis() {
        let text = "Hello, world! 世界 🌍 How are you today?";
        let complexity = utils::analyze_text_complexity(text);
        
        assert!(complexity.unicode_chars > 0);
        assert!(complexity.punctuation > 0);
        assert!(complexity.complexity_score > 0.0);
    }
}
"#.to_string()
}

/// Генерирует естественный язык
fn generate_natural_language_text() -> String {
    r#"
The Evolution of Artificial Intelligence and Its Impact on Modern Society

Artificial Intelligence (AI) has emerged as one of the most transformative technologies of the 21st century, fundamentally reshaping how we interact with digital systems, process information, and solve complex problems. From its humble beginnings in the 1950s as a theoretical concept explored by pioneers like Alan Turing and John McCarthy, AI has evolved into a sophisticated field encompassing machine learning, deep learning, natural language processing, computer vision, and robotics.

The journey of AI development can be traced through several distinct phases, each marked by significant breakthroughs and paradigm shifts. The early symbolic AI era focused on rule-based systems and expert knowledge representation, leading to the development of expert systems that could mimic human decision-making in specific domains. However, these systems were limited by their inability to learn from experience and adapt to new situations.

The introduction of machine learning algorithms in the 1980s and 1990s marked a crucial turning point, enabling systems to improve their performance through exposure to data rather than explicit programming. This shift from rule-based to data-driven approaches opened new possibilities for pattern recognition, predictive modeling, and automated decision-making across various industries.

The deep learning revolution of the 2010s, powered by advances in neural network architectures and the availability of massive datasets, has accelerated AI capabilities exponentially. Convolutional neural networks have revolutionized computer vision, enabling applications ranging from medical image analysis to autonomous vehicle navigation. Recurrent neural networks and transformer architectures have transformed natural language processing, making possible sophisticated language models that can generate human-like text, translate between languages, and engage in meaningful conversations.

Today's AI systems demonstrate remarkable capabilities across diverse domains. In healthcare, AI assists in diagnostic imaging, drug discovery, and personalized treatment recommendations. Financial institutions leverage AI for fraud detection, algorithmic trading, and risk assessment. Manufacturing companies employ AI-powered robotics for quality control, predictive maintenance, and supply chain optimization. Entertainment platforms use recommendation algorithms to personalize content delivery, while search engines employ sophisticated ranking algorithms to provide relevant results to billions of users daily.

The integration of AI into everyday life has been particularly evident in the proliferation of virtual assistants, smart home devices, and mobile applications that understand natural language commands and adapt to user preferences. These consumer-facing applications have made AI more accessible and familiar to the general public, demonstrating the technology's potential to enhance productivity and convenience.

However, the rapid advancement of AI also raises important ethical, social, and economic considerations. Concerns about job displacement due to automation have sparked debates about the future of work and the need for reskilling programs. Issues related to algorithmic bias, privacy protection, and the concentration of AI capabilities in the hands of a few large corporations have prompted calls for regulatory frameworks and ethical guidelines.

The development of increasingly powerful AI systems has also reignited discussions about artificial general intelligence (AGI) and its potential implications for humanity. While current AI systems excel in narrow domains, the prospect of machines achieving human-level intelligence across all cognitive tasks remains a subject of intense research and speculation.

As we look toward the future, several trends are likely to shape the continued evolution of AI. Edge computing will enable more efficient and privacy-preserving AI applications by processing data locally rather than in centralized cloud servers. Federated learning approaches will allow AI models to be trained across distributed datasets without compromising data privacy. Explainable AI techniques will make machine learning models more interpretable and trustworthy, addressing concerns about black-box decision-making.

The convergence of AI with other emerging technologies such as quantum computing, biotechnology, and nanotechnology promises to unlock new frontiers of innovation. Quantum machine learning algorithms could solve optimization problems that are intractable for classical computers, while AI-driven drug discovery platforms could accelerate the development of life-saving medications.

Education and workforce development will play crucial roles in ensuring that society can adapt to and benefit from AI advancements. Universities and training institutions are already incorporating AI and data science curricula to prepare the next generation of professionals. Lifelong learning programs will become increasingly important as the pace of technological change continues to accelerate.

International cooperation and governance frameworks will be essential for addressing the global challenges and opportunities presented by AI. Collaborative efforts to establish ethical standards, safety protocols, and beneficial AI development practices will help ensure that the technology serves the common good while minimizing potential risks.

In conclusion, artificial intelligence represents both a remarkable achievement of human ingenuity and a powerful tool for addressing some of our most pressing challenges. As we continue to push the boundaries of what machines can accomplish, it is crucial that we remain mindful of the broader implications and work collectively to harness AI's potential for the benefit of all humanity. The future of AI is not predetermined but will be shaped by the choices we make today regarding research priorities, ethical considerations, and societal values.
"#.to_string()
}

/// Генерирует смешанный контент
fn generate_mixed_content() -> String {
    r#"
# API Integration Guide: Gemini Tokenization Service

## Overview 概要 Обзор

This document provides comprehensive guidance for integrating with the Gemini tokenization service. The service supports multiple languages including English, 中文, Русский, Español, Français, Deutsch, 日本語, and العربية.

## Quick Start 快速开始

```bash
# Install dependencies
npm install @google/generative-ai
pip install google-generativeai
cargo add tokio reqwest serde_json

# Set environment variables
export GOOGLE_API_KEY="your-api-key-here"
export TOKENIZER_ENDPOINT="https://api.gemini.google.com/v1/tokenize"
```

### JavaScript Example

```javascript
import { GoogleGenerativeAI } from "@google/generative-ai";

const genAI = new GoogleGenerativeAI(process.env.GOOGLE_API_KEY);

async function countTokens(text) {
  try {
    const model = genAI.getGenerativeModel({ model: "gemini-pro" });
    const result = await model.countTokens(text);
    return result.totalTokens;
  } catch (error) {
    console.error("Tokenization failed:", error);
    throw error;
  }
}

// Usage examples
const examples = [
  "Hello, world! 🌍",
  "Mathematical equation: E = mc²",
  "Code snippet: const x = (a, b) => a + b;",
  "Unicode text: 你好世界 مرحبا بالعالم Здравствуй мир"
];

for (const text of examples) {
  const tokens = await countTokens(text);
  console.log(`"${text}" → ${tokens} tokens`);
}
```

### Python Example

```python
import google.generativeai as genai
import os
from typing import List, Dict, Any

# Configure the API
genai.configure(api_key=os.environ['GOOGLE_API_KEY'])

class TokenCounter:
    def __init__(self, model_name: str = "gemini-pro"):
        self.model = genai.GenerativeModel(model_name)
    
    def count_tokens(self, text: str) -> int:
        """Count tokens in the given text."""
        try:
            response = self.model.count_tokens(text)
            return response.total_tokens
        except Exception as e:
            print(f"Error counting tokens: {e}")
            return 0
    
    def batch_count(self, texts: List[str]) -> Dict[str, int]:
        """Count tokens for multiple texts."""
        results = {}
        for i, text in enumerate(texts):
            try:
                count = self.count_tokens(text)
                results[f"text_{i}"] = count
            except Exception as e:
                print(f"Error processing text {i}: {e}")
                results[f"text_{i}"] = 0
        return results

# Usage
counter = TokenCounter()

test_cases = [
    "Simple English text",
    "Text with émojis 😀🎉🚀 and spëcial characters",
    "Code: def fibonacci(n): return n if n <= 1 else fibonacci(n-1) + fibonacci(n-2)",
    "Mixed languages: Hello 你好 Привет مرحبا Bonjour Hola こんにちは",
    "JSON: {\"name\": \"John\", \"age\": 30, \"skills\": [\"Python\", \"JavaScript\", \"Rust\"]}",
    "Mathematical notation: ∑(i=1 to n) i² = n(n+1)(2n+1)/6"
]

for text in test_cases:
    tokens = counter.count_tokens(text)
    print(f"Tokens: {tokens:3d} | Text: {text[:50]}{'...' if len(text) > 50 else ''}")
```

### Rust Example

```rust
use reqwest::Client;
use serde_json::{json, Value};
use std::error::Error;
use tokio;

#[derive(Debug)]
struct TokenizerClient {
    client: Client,
    api_key: String,
    base_url: String,
}

impl TokenizerClient {
    fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
        }
    }
    
    async fn count_tokens(&self, text: &str, model: &str) -> Result<usize, Box<dyn Error>> {
        let url = format!("{}/models/{}:countTokens?key={}", 
            self.base_url, model, self.api_key);
        
        let payload = json!({
            "contents": [{
                "parts": [{"text": text}]
            }]
        });
        
        let response = self.client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;
        
        let result: Value = response.json().await?;
        let token_count = result["totalTokens"]
            .as_u64()
            .ok_or("Invalid response format")?;
        
        Ok(token_count as usize)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let api_key = std::env::var("GOOGLE_API_KEY")
        .expect("GOOGLE_API_KEY environment variable not set");
    
    let client = TokenizerClient::new(api_key);
    
    let test_texts = vec![
        "Hello, Rust! 🦀",
        "Complex text with 中文, العربية, and Русский",
        "fn main() { println!(\"Hello, world!\"); }",
        "Data: [1, 2, 3, 4, 5] → sum = 15, avg = 3.0",
    ];
    
    for text in test_texts {
        match client.count_tokens(text, "gemini-pro").await {
            Ok(count) => println!("✅ {} tokens: {}", count, text),
            Err(e) => println!("❌ Error: {} for text: {}", e, text),
        }
    }
    
    Ok(())
}
```

## Performance Benchmarks 性能基准

| Text Type | Length | Tokens | Ratio | Processing Time |
|-----------|--------|--------|-------|-----------------|
| English | 1,000 chars | ~250 tokens | 4.0:1 | 15ms |
| Chinese 中文 | 1,000 chars | ~500 tokens | 2.0:1 | 18ms |
| Code | 1,000 chars | ~300 tokens | 3.3:1 | 12ms |
| Mixed | 1,000 chars | ~280 tokens | 3.6:1 | 20ms |
| JSON | 1,000 chars | ~320 tokens | 3.1:1 | 14ms |

## Error Handling 错误处理

Common error scenarios and solutions:

### Rate Limiting (429)
```javascript
async function countTokensWithRetry(text, maxRetries = 3) {
  for (let i = 0; i < maxRetries; i++) {
    try {
      return await countTokens(text);
    } catch (error) {
      if (error.status === 429 && i < maxRetries - 1) {
        await new Promise(resolve => setTimeout(resolve, Math.pow(2, i) * 1000));
        continue;
      }
      throw error;
    }
  }
}
```

### Invalid Input (400)
```python
def validate_input(text: str) -> bool:
    if not text or not isinstance(text, str):
        return False
    if len(text) > 1_000_000:  # 1MB limit
        return False
    return True

def safe_count_tokens(text: str) -> int:
    if not validate_input(text):
        raise ValueError("Invalid input text")
    return counter.count_tokens(text)
```

## Best Practices 最佳实践

1. **Caching**: Cache token counts for repeated texts
2. **Batching**: Process multiple texts in batches when possible
3. **Monitoring**: Track API usage and performance metrics
4. **Fallback**: Implement local estimation for offline scenarios

## Troubleshooting 故障排除

### Common Issues:

- **Inconsistent counts**: Ensure same model version
- **Timeout errors**: Increase timeout for large texts
- **Memory issues**: Process large texts in chunks
- **Unicode problems**: Use proper encoding (UTF-8)

### Debug Mode:

```bash
export DEBUG_TOKENIZER=true
export LOG_LEVEL=debug
```

## Support 支持

For technical support:
- 📧 Email: support@example.com
- 💬 Discord: https://discord.gg/example
- 📚 Documentation: https://docs.example.com
- 🐛 Issues: https://github.com/example/tokenizer/issues

---

© 2024 Tokenization Service. All rights reserved.
Licensed under MIT License.
"#.to_string()
}

/// Генерирует текст с большим количеством Unicode
fn generate_unicode_heavy_text() -> String {
    r#"
🌍 多语言文本处理系统 Многоязычная система обработки текста نظام معالجة النصوص متعدد اللغات

## 概述 Overview Обзор نظرة عامة

这个系统支持多种语言和字符集的处理，包括但不限于：
This system supports processing of multiple languages and character sets, including but not limited to:
Эта система поддерживает обработку множества языков и наборов символов, включая, но не ограничиваясь:
يدعم هذا النظام معالجة لغات ومجموعات أحرف متعددة، بما في ذلك على سبيل المثال لا الحصر:

### 支持的语言 Supported Languages Поддерживаемые языки اللغات المدعومة

1. **中文 Chinese 中国话**
   - 简体中文：你好世界！这是一个测试文本。
   - 繁體中文：你好世界！這是一個測試文本。
   - 古文：子曰：「學而時習之，不亦說乎？」

2. **English**
   - Standard: Hello world! This is a test text.
   - Technical: API endpoints, JSON parsing, HTTP requests
   - Colloquial: Hey there! What's up? How's it going?

3. **Русский Russian**
   - Стандартный: Привет мир! Это тестовый текст.
   - Технический: API интерфейсы, парсинг JSON, HTTP запросы
   - Разговорный: Привет! Как дела? Что нового?

4. **العربية Arabic**
   - معياري: مرحبا بالعالم! هذا نص تجريبي.
   - تقني: واجهات برمجة التطبيقات، تحليل JSON، طلبات HTTP
   - عامي: أهلاً! كيف الحال؟ إيش الأخبار؟

5. **日本語 Japanese**
   - ひらがな：こんにちは せかい！これは てすと の ぶんしょう です。
   - カタカナ：コンニチハ セカイ！コレハ テスト ノ ブンショウ デス。
   - 漢字：今日は世界！これは試験の文章です。
   - 混合：Hello世界！これはtestのテキストです。

6. **한국어 Korean**
   - 한글: 안녕하세요 세계! 이것은 테스트 텍스트입니다.
   - 한자: 安寧하세요 世界! 이것은 試驗 텍스트입니다.

7. **Español Spanish**
   - Estándar: ¡Hola mundo! Este es un texto de prueba.
   - Técnico: APIs, análisis JSON, peticiones HTTP
   - Coloquial: ¡Hola! ¿Qué tal? ¿Cómo va todo?

8. **Français French**
   - Standard: Bonjour le monde! Ceci est un texte de test.
   - Technique: APIs, analyse JSON, requêtes HTTP
   - Familier: Salut! Ça va? Quoi de neuf?

9. **Deutsch German**
   - Standard: Hallo Welt! Dies ist ein Testtext.
   - Technisch: APIs, JSON-Parsing, HTTP-Anfragen
   - Umgangssprachlich: Hallo! Wie geht's? Was gibt's Neues?

### 特殊字符和符号 Special Characters and Symbols Специальные символы والرموز الخاصة

#### 数学符号 Mathematical Symbols Математические символы الرموز الرياضية
- 基本运算：+ - × ÷ = ≠ ≈ ≤ ≥ ± ∓
- 高级数学：∑ ∏ ∫ ∮ ∂ ∇ ∞ ∅ ∈ ∉ ⊂ ⊃ ∪ ∩
- 希腊字母：α β γ δ ε ζ η θ ι κ λ μ ν ξ ο π ρ σ τ υ φ χ ψ ω
- 上标下标：x² y³ H₂O CO₂ E=mc² a₁ + a₂ = a₃

#### 货币符号 Currency Symbols Валютные символы رموز العملات
- 常用货币：$ € £ ¥ ₹ ₽ ₩ ₪ ₦ ₡ ₨ ₫ ₱ ₵ ₴ ₸ ₼ ₾
- 加密货币：₿ Ξ Ł Ð ⟐

#### 表情符号 Emojis Эмодзи الرموز التعبيرية
- 面部表情：😀 😃 😄 😁 😆 😅 😂 🤣 😊 😇 🙂 🙃 😉 😌 😍 🥰 😘 😗 😙 😚 😋 😛 😝 😜 🤪 🤨 🧐 🤓 😎 🤩 🥳
- 手势：👍 👎 👌 🤌 🤏 ✌️ 🤞 🤟 🤘 🤙 👈 👉 👆 🖕 👇 ☝️ 👋 🤚 🖐️ ✋ 🖖 👏 🙌 🤲 🤝 🙏
- 动物：🐶 🐱 🐭 🐹 🐰 🦊 🐻 🐼 🐨 🐯 🦁 🐮 🐷 🐸 🐵 🙈 🙉 🙊 🐒 🐔 🐧 🐦 🐤 🐣 🐥 🦆 🦅 🦉 🦇 🐺 🐗
- 食物：🍎 🍊 🍋 🍌 🍉 🍇 🍓 🫐 🍈 🍒 🍑 🥭 🍍 🥥 🥝 🍅 🍆 🥑 🥦 🥬 🥒 🌶️ 🫑 🌽 🥕 🫒 🧄 🧅 🥔 🍠 🥐 🥯 🍞 🥖 🥨 🧀 🥚 🍳 🧈 🥞 🧇 🥓 🥩 🍗 🍖 🦴 🌭 🍔 🍟 🍕

#### 技术符号 Technical Symbols Технические символы الرموز التقنية
- 编程：{ } [ ] ( ) < > / \ | & ^ ~ ` @ # % * + - = _ : ; " ' ? ! . ,
- 网络：@ # $ % ^ & * ( ) - _ + = { } [ ] | \ : ; " ' < > , . ? / ~
- 箭头：← → ↑ ↓ ↔ ↕ ↖ ↗ ↘ ↙ ⇐ ⇒ ⇑ ⇓ ⇔ ⇕ ⇖ ⇗ ⇘ ⇙ ➡️ ⬅️ ⬆️ ⬇️

### 测试用例 Test Cases Тестовые случаи حالات الاختبار

1. **混合语言文本 Mixed Language Text**
   Hello世界! Привет мир! مرحبا بالعالم! Bonjour le monde! ¡Hola mundo! こんにちは世界！안녕하세요 세계！

2. **技术文档 Technical Documentation**
   ```javascript
   const API_URL = 'https://api.example.com/v1/tokenize';
   const response = await fetch(API_URL, {
     method: 'POST',
     headers: { 'Content-Type': 'application/json' },
     body: JSON.stringify({ text: '你好世界！Hello world! مرحبا بالعالم!' })
   });
   ```

3. **数学公式 Mathematical Formulas**
   E = mc² | F = ma | a² + b² = c² | ∫₀^∞ e^(-x²) dx = √π/2 | ∑ᵢ₌₁ⁿ i = n(n+1)/2

4. **表情符号测试 Emoji Test**
   今天天气很好！☀️🌤️⛅🌦️🌧️⛈️🌩️🌨️❄️☃️⛄🌬️💨🌪️🌫️🌊💧💦☔⚡🔥💥❄️

5. **特殊格式 Special Formatting**
   **粗体 Bold жирный عريض** *斜体 Italic курсив مائل* ~~删除线 Strikethrough зачеркнутый يتوسطه خط~~
   `代码 Code код كود` [链接 Link ссылка رابط](https://example.com)

### 性能测试数据 Performance Test Data Данные тестирования производительности بيانات اختبار الأداء

| 语言 Language | 字符数 Chars | 预期令牌 Expected Tokens | 实际令牌 Actual Tokens | 准确率 Accuracy |
|---------------|--------------|--------------------------|------------------------|-----------------|
| 中文 Chinese | 1000 | 500 | 485 | 97.0% |
| English | 1000 | 250 | 248 | 99.2% |
| Русский | 1000 | 200 | 195 | 97.5% |
| العربية | 1000 | 180 | 175 | 97.2% |
| 日本語 | 1000 | 400 | 390 | 97.5% |
| 한국어 | 1000 | 300 | 295 | 98.3% |
| Mixed 混合 | 1000 | 280 | 275 | 98.2% |

这个测试文档包含了各种语言、字符集、符号和格式，用于全面测试令牌化系统的准确性和性能。
This test document contains various languages, character sets, symbols, and formats for comprehensive testing of tokenization system accuracy and performance.
Этот тестовый документ содержит различные языки, наборы символов, символы и форматы для всестороннего тестирования точности и производительности системы токенизации.
تحتوي هذه الوثيقة الاختبارية على لغات وأحرف ورموز وتنسيقات مختلفة لاختبار شامل لدقة وأداء نظام الترميز.
"#.to_string()
}

/// Генерирует очень большой текст для тестирования производительности
fn generate_very_large_text() -> String {
    let base_text = generate_technical_doc();
    let mut large_text = String::new();
    
    // Повторяем базовый текст 10 раз для создания большого документа
    for i in 0..10 {
        large_text.push_str(&format!("\n\n=== SECTION {} ===\n\n", i + 1));
        large_text.push_str(&base_text);
    }
    
    large_text
}

/// Получает количество токенов от Google API
async fn get_google_token_count(
    client: &Client, 
    api_key: &str, 
    text: &str
) -> Result<usize, Box<dyn Error + Send + Sync>> {
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:countTokens?key={}", 
        api_key
    );
    
    let request_body = json!({
        "contents": [{
            "parts": [{
                "text": text
            }]
        }]
    });
    
    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&request_body)
        .timeout(Duration::from_secs(30)) // Увеличенный таймаут для больших текстов
        .send()
        .await?;
    
    if !response.status().is_success() {
        return Err(format!("Google API error: {}", response.status()).into());
    }
    
    let response_json: Value = response.json().await?;
    
    let total_tokens = response_json
        .get("totalTokens")
        .and_then(|t| t.as_u64())
        .ok_or("Missing totalTokens in response")?;
    
    Ok(total_tokens as usize)
}

/// Вычисляет точность в процентах
fn calculate_accuracy(our_count: usize, google_count: usize) -> f64 {
    if google_count == 0 {
        return if our_count == 0 { 100.0 } else { 0.0 };
    }
    
    let diff = (our_count as i32 - google_count as i32).abs() as f64;
    let accuracy = (1.0 - (diff / google_count as f64)) * 100.0;
    accuracy.max(0.0)
}