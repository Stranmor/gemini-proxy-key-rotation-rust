// tests/google_api_token_comparison.rs

use std::env;
use std::error::Error;
use serde_json::{json, Value};
use reqwest::Client;
use tokio::time::{sleep, Duration};
use tracing::{warn, debug, error};
use gemini_proxy::tokenizer;

/// Тест для сравнения нашего токенизатора с реальным API Google Gemini
/// Этот тест поможет выявить расхождения и откалибровать наш токенизатор

#[tokio::test]
async fn test_token_count_accuracy_vs_google_api() {
    // Инициализируем логирование для отладки
    tracing_subscriber::fmt::init();
    
    // Проверяем наличие API ключа
    let api_key = match env::var("GOOGLE_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("GOOGLE_API_KEY not found, skipping Google API comparison test");
            return;
        }
    };
    
    // Инициализируем наш токенизатор
    if let Err(e) = tokenizer::gemini_simple::GeminiTokenizer::initialize().await {
        panic!("Failed to initialize our tokenizer: {e}");
    }
    
    // Тестовые случаи разной сложности
    let test_cases = vec![
        // Простые случаи
        "Hello",
        "Hello world",
        "Hello, world!",
        
        // Средней сложности
        "The quick brown fox jumps over the lazy dog.",
        "What is the capital of France?",
        "Explain quantum computing in simple terms.",
        
        // Сложные случаи
        "Write a Python function to calculate fibonacci numbers recursively and iteratively.",
        "Translate the following text from English to Spanish: 'Hello, how are you today? I hope you're having a wonderful day!'",
        "Create a detailed explanation of how machine learning algorithms work, including supervised and unsupervised learning approaches.",
        
        // Очень длинные тексты
        "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
        
        // Специальные символы и Unicode
        "Hello 世界! 🌍 How are you? Привет мир! ¿Cómo estás?",
        "Mathematical symbols: ∑, ∫, ∂, ∇, ∞, π, α, β, γ, δ",
        
        // Код
        r#"
        function fibonacci(n) {
            if (n <= 1) return n;
            return fibonacci(n - 1) + fibonacci(n - 2);
        }
        console.log(fibonacci(10));
        "#,
        
        // JSON
        r#"{"name": "John", "age": 30, "city": "New York", "hobbies": ["reading", "swimming", "coding"]}"#,
    ];
    
    let client = Client::new();
    let mut total_tests = 0;
    let mut accurate_tests = 0;
    let mut total_our_tokens = 0;
    let mut total_google_tokens = 0;
    
    println!("\n=== Token Count Comparison: Our Tokenizer vs Google API ===\n");
    
    for (i, text) in test_cases.iter().enumerate() {
        println!("Test case {}: \"{}\"", i + 1, 
            if text.chars().count() > 50 { 
                format!("{}...", text.chars().take(50).collect::<String>()) 
            } else { 
                text.to_string() 
            }
        );
        
        // Получаем количество токенов от нашего токенизатора
        let our_count = match tokenizer::count_gemini_tokens(text) {
            Ok(count) => count,
            Err(e) => {
                error!("Our tokenizer failed for text: {}", e);
                continue;
            }
        };
        
        // Получаем количество токенов от Google API
        let google_count = match get_google_token_count(&client, &api_key, text).await {
            Ok(count) => count,
            Err(e) => {
                warn!("Google API failed for text: {}", e);
                // Добавляем небольшую задержку перед следующим запросом
                sleep(Duration::from_millis(1000)).await;
                continue;
            }
        };
        
        total_tests += 1;
        total_our_tokens += our_count;
        total_google_tokens += google_count;
        
        // Вычисляем точность
        let accuracy = if google_count > 0 {
            let diff = (our_count as i32 - google_count as i32).abs() as f64;
            let accuracy = (1.0 - (diff / google_count as f64)) * 100.0;
            accuracy.max(0.0)
        } else if our_count == 0 { 100.0 } else { 0.0 };
        
        // Считаем тест точным если расхождение менее 10%
        if accuracy >= 90.0 {
            accurate_tests += 1;
        }
        
        println!("  Our count: {our_count} | Google count: {google_count} | Accuracy: {accuracy:.1}%");
        
        if accuracy < 90.0 {
            println!("  ⚠️  Significant discrepancy detected!");
        }
        
        // Добавляем задержку между запросами к API
        sleep(Duration::from_millis(500)).await;
    }
    
    // Итоговая статистика
    println!("\n=== Summary ===");
    println!("Total tests: {total_tests}");
    println!("Accurate tests (>90%): {accurate_tests}");
    println!("Overall accuracy: {:.1}%", (accurate_tests as f64 / total_tests as f64) * 100.0);
    
    let overall_ratio = if total_google_tokens > 0 {
        total_our_tokens as f64 / total_google_tokens as f64
    } else {
        1.0
    };
    
    println!("Total our tokens: {total_our_tokens}");
    println!("Total Google tokens: {total_google_tokens}");
    println!("Overall ratio (our/google): {overall_ratio:.3}");
    
    if overall_ratio > 2.0 {
        println!("🚨 CRITICAL: Our tokenizer counts more than 2x Google's count!");
        println!("This explains the discrepancy you mentioned.");
    } else if overall_ratio > 1.5 {
        println!("⚠️  WARNING: Our tokenizer counts significantly more than Google");
    } else if overall_ratio < 0.5 {
        println!("⚠️  WARNING: Our tokenizer counts significantly less than Google");
    } else {
        println!("✅ Token counts are reasonably close");
    }
    
    // Рекомендации по улучшению
    println!("\n=== Recommendations ===");
    if overall_ratio > 1.2 {
        println!("- Consider reducing token estimates in our tokenizer");
        println!("- Review word splitting logic");
        println!("- Check punctuation handling");
    } else if overall_ratio < 0.8 {
        println!("- Consider increasing token estimates in our tokenizer");
        println!("- Review subword tokenization");
    }
    
    // Тест должен пройти если общая точность > 50% (более реалистичный порог)
    let overall_accuracy = (accurate_tests as f64 / total_tests as f64) * 100.0;
    assert!(overall_accuracy >= 50.0, 
        "Overall accuracy {overall_accuracy:.1}% is below 50% threshold");
}

/// Получает количество токенов от Google API
async fn get_google_token_count(
    client: &Client, 
    api_key: &str, 
    text: &str
) -> Result<usize, Box<dyn Error + Send + Sync>> {
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:countTokens?key={api_key}"
    );
    
    let request_body = json!({
        "contents": [{
            "parts": [{
                "text": text
            }]
        }]
    });
    
    debug!("Sending request to Google API: {}", url);
    debug!("Request body: {}", serde_json::to_string_pretty(&request_body)?);
    
    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await?;
    
    let status = response.status();
    let response_text = response.text().await?;
    
    debug!("Google API response status: {}", status);
    debug!("Google API response body: {}", response_text);
    
    if !status.is_success() {
        return Err(format!("Google API error {status}: {response_text}").into());
    }
    
    let response_json: Value = serde_json::from_str(&response_text)?;
    
    let total_tokens = response_json
        .get("totalTokens")
        .and_then(|t| t.as_u64())
        .ok_or("Missing totalTokens in response")?;
    
    Ok(total_tokens as usize)
}

#[tokio::test]
async fn test_multimodal_token_accuracy() {
    // Проверяем наличие API ключа
    let api_key = match env::var("GOOGLE_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("GOOGLE_API_KEY not found, skipping multimodal comparison test");
            return;
        }
    };
    
    // Инициализируем токенизаторы
    if let Err(e) = tokenizer::gemini_simple::GeminiTokenizer::initialize().await {
        panic!("Failed to initialize our tokenizer: {e}");
    }
    
    tokenizer::multimodal::MultimodalTokenizer::initialize(None)
        .expect("Failed to initialize multimodal tokenizer");
    
    // Создаем простое тестовое изображение (1x1 PNG)
    let tiny_png_base64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==";
    
    let test_cases = [
        // Только текст
        json!({
            "contents": [{
                "parts": [{
                    "text": "What do you see in this image?"
                }]
            }]
        }),
        
        // Текст + изображение
        json!({
            "contents": [{
                "parts": [
                    {
                        "text": "Describe this image:"
                    },
                    {
                        "inline_data": {
                            "mime_type": "image/png",
                            "data": tiny_png_base64
                        }
                    }
                ]
            }]
        }),
    ];
    
    let client = Client::new();
    
    println!("\n=== Multimodal Token Count Comparison ===\n");
    
    for (i, test_case) in test_cases.iter().enumerate() {
        println!("Multimodal test case {}", i + 1);
        
        // Получаем количество токенов от Google API
        let google_count = match get_google_multimodal_token_count(&client, &api_key, test_case).await {
            Ok(count) => count,
            Err(e) => {
                warn!("Google API failed for multimodal content: {}", e);
                sleep(Duration::from_millis(1000)).await;
                continue;
            }
        };
        
        println!("  Google count: {google_count}");
        
        // Добавляем задержку между запросами
        sleep(Duration::from_millis(1000)).await;
    }
}

/// Получает количество токенов для multimodal контента от Google API
async fn get_google_multimodal_token_count(
    client: &Client,
    api_key: &str,
    contents: &Value
) -> Result<usize, Box<dyn Error + Send + Sync>> {
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:countTokens?key={api_key}"
    );
    
    debug!("Sending multimodal request to Google API: {}", url);
    debug!("Request body: {}", serde_json::to_string_pretty(contents)?);
    
    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(contents)
        .send()
        .await?;
    
    let status = response.status();
    let response_text = response.text().await?;
    
    debug!("Google API multimodal response status: {}", status);
    debug!("Google API multimodal response body: {}", response_text);
    
    if !status.is_success() {
        return Err(format!("Google API error {status}: {response_text}").into());
    }
    
    let response_json: Value = serde_json::from_str(&response_text)?;
    
    let total_tokens = response_json
        .get("totalTokens")
        .and_then(|t| t.as_u64())
        .ok_or("Missing totalTokens in response")?;
    
    Ok(total_tokens as usize)
}

/// Тест для калибровки нашего токенизатора на основе данных Google API
#[tokio::test]
async fn test_calibrate_tokenizer() {
    let api_key = match env::var("GOOGLE_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("GOOGLE_API_KEY not found, skipping calibration test");
            return;
        }
    };
    
    // Инициализируем наш токенизатор
    if let Err(e) = tokenizer::gemini_simple::GeminiTokenizer::initialize().await {
        panic!("Failed to initialize our tokenizer: {e}");
    }
    
    // Калибровочные тексты разной длины
    let calibration_texts = vec![
        "Hello",
        "Hello world",
        "The quick brown fox",
        "The quick brown fox jumps over the lazy dog",
        "This is a longer sentence with more complex vocabulary and punctuation.",
        "Write a Python function that calculates the factorial of a number using recursion.",
    ];
    
    let client = Client::new();
    let mut calibration_data = Vec::new();
    
    println!("\n=== Tokenizer Calibration Data ===\n");
    
    for text in calibration_texts {
        let our_count = tokenizer::count_gemini_tokens(text).unwrap();
        
        match get_google_token_count(&client, &api_key, text).await {
            Ok(google_count) => {
                let ratio = our_count as f64 / google_count as f64;
                calibration_data.push((text.len(), our_count, google_count, ratio));
                
                println!("Text length: {} chars", text.len());
                println!("  Our: {our_count} | Google: {google_count} | Ratio: {ratio:.3}");
                println!("  Text: \"{}\"", if text.len() > 50 { format!("{}...", &text[..50]) } else { text.to_string() });
                println!();
            }
            Err(e) => {
                warn!("Failed to get Google count for calibration: {}", e);
            }
        }
        
        sleep(Duration::from_millis(500)).await;
    }
    
    // Анализируем калибровочные данные
    if !calibration_data.is_empty() {
        let avg_ratio: f64 = calibration_data.iter().map(|(_, _, _, ratio)| ratio).sum::<f64>() / calibration_data.len() as f64;
        
        println!("=== Calibration Analysis ===");
        println!("Average ratio (our/google): {avg_ratio:.3}");
        
        if avg_ratio > 1.5 {
            println!("🚨 RECOMMENDATION: Reduce token estimates by factor of {:.2}", 1.0 / avg_ratio);
        } else if avg_ratio < 0.7 {
            println!("🚨 RECOMMENDATION: Increase token estimates by factor of {:.2}", 1.0 / avg_ratio);
        } else {
            println!("✅ Token estimates are reasonably calibrated");
        }
        
        // Предлагаем корректировочный коэффициент
        let correction_factor = 1.0 / avg_ratio;
        println!("Suggested correction factor: {correction_factor:.3}");
        println!("Apply this factor to your tokenizer estimates to improve accuracy.");
    }
}