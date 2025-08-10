// tests/calibrated_tokenizer_test.rs

use std::env;
use std::error::Error;
use serde_json::{json, Value};
use reqwest::Client;
use tokio::time::{sleep, Duration};
use tracing::{warn, error, debug};
use gemini_proxy::tokenizer;

/// Тест калиброванного токенизатора против Google API
#[tokio::test]
async fn test_calibrated_tokenizer_accuracy() {
    // Инициализируем логирование
    tracing_subscriber::fmt::init();
    
    // Проверяем наличие API ключа
    let api_key = match env::var("GOOGLE_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("GOOGLE_API_KEY not found, skipping calibrated tokenizer test");
            return;
        }
    };
    
    // Инициализируем калиброванный токенизатор
    if let Err(e) = tokenizer::gemini_calibrated::GeminiCalibratedTokenizer::initialize().await {
        panic!("Failed to initialize calibrated tokenizer: {e}");
    }
    
    // Тестовые случаи на основе предыдущих результатов
    let test_cases = [
        // Простые случаи (должны быть точными)
        "Hello",
        "Hello world", 
        "Hello, world!",
        "The quick brown fox jumps over the lazy dog.",
        "What is the capital of France?",
        
        // Проблемные случаи (требуют калибровки)
        "Explain quantum computing in simple terms.",
        "Hello 世界! 🌍 How are you? Привет мир! ¿Cómo estás?",
        "Mathematical symbols: ∑, ∫, ∂, ∇, ∞, π, α, β, γ, δ",
        
        // Код (требует увеличения оценки)
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
    
    println!("\n=== Calibrated Tokenizer vs Google API ===\n");
    
    for (i, text) in test_cases.iter().enumerate() {
        println!("Test case {}: \"{}\"", i + 1, 
            if text.chars().count() > 50 { 
                format!("{}...", text.chars().take(50).collect::<String>()) 
            } else { 
                text.to_string() 
            }
        );
        
        // Получаем количество токенов от калиброванного токенизатора
        let our_count = match tokenizer::count_calibrated_gemini_tokens(text) {
            Ok(count) => count,
            Err(e) => {
                error!("Calibrated tokenizer failed for text: {}", e);
                continue;
            }
        };
        
        // Получаем количество токенов от Google API
        let google_count = match get_google_token_count(&client, &api_key, text).await {
            Ok(count) => count,
            Err(e) => {
                warn!("Google API failed for text: {}", e);
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
        
        println!("  Calibrated: {our_count} | Google: {google_count} | Accuracy: {accuracy:.1}%");
        
        if accuracy < 90.0 {
            println!("  ⚠️  Still needs improvement");
        } else {
            println!("  ✅ Good accuracy");
        }
        
        // Добавляем задержку между запросами к API
        sleep(Duration::from_millis(500)).await;
    }
    
    // Итоговая статистика
    println!("\n=== Calibrated Tokenizer Summary ===");
    println!("Total tests: {total_tests}");
    println!("Accurate tests (>90%): {accurate_tests}");
    println!("Overall accuracy: {:.1}%", (accurate_tests as f64 / total_tests as f64) * 100.0);
    
    let overall_ratio = if total_google_tokens > 0 {
        total_our_tokens as f64 / total_google_tokens as f64
    } else {
        1.0
    };
    
    println!("Total calibrated tokens: {total_our_tokens}");
    println!("Total Google tokens: {total_google_tokens}");
    println!("Overall ratio (calibrated/google): {overall_ratio:.3}");
    
    if overall_ratio > 1.2 {
        println!("⚠️  Still overestimating tokens");
    } else if overall_ratio < 0.8 {
        println!("⚠️  Still underestimating tokens");
    } else {
        println!("✅ Token ratio is well calibrated");
    }
    
    // Проверяем улучшение
    let overall_accuracy = (accurate_tests as f64 / total_tests as f64) * 100.0;
    println!("\nCalibration effectiveness:");
    if overall_accuracy >= 85.0 {
        println!("🎉 Excellent calibration! Accuracy: {overall_accuracy:.1}%");
    } else if overall_accuracy >= 75.0 {
        println!("✅ Good calibration! Accuracy: {overall_accuracy:.1}%");
    } else {
        println!("⚠️  Calibration needs more work. Accuracy: {overall_accuracy:.1}%");
    }
    
    // Тест проходит если точность > 75% (более мягкий порог для калиброванного токенизатора)
    assert!(overall_accuracy >= 75.0, 
        "Calibrated tokenizer accuracy {overall_accuracy:.1}% is below 75% threshold");
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

/// Сравнение производительности калиброванного и обычного токенизаторов
#[tokio::test]
async fn test_calibrated_vs_simple_performance() {
    // Инициализируем оба токенизатора
    tokenizer::gemini_simple::GeminiTokenizer::initialize().await.unwrap();
    tokenizer::gemini_calibrated::GeminiCalibratedTokenizer::initialize().await.unwrap();
    
    let test_text = "This is a performance comparison test between simple and calibrated Gemini tokenizers with various Unicode symbols: 世界 🌍 and mathematical notation: ∑∫∂∇∞π";
    let iterations = 1000;
    
    // Тест простого токенизатора
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let _ = tokenizer::count_gemini_tokens(test_text).unwrap();
    }
    let simple_duration = start.elapsed();
    
    // Тест калиброванного токенизатора
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let _ = tokenizer::count_calibrated_gemini_tokens(test_text).unwrap();
    }
    let calibrated_duration = start.elapsed();
    
    println!("\n=== Performance Comparison ===");
    println!("Simple tokenizer: {} iterations in {:?} (avg: {:?})", 
        iterations, simple_duration, simple_duration / iterations);
    println!("Calibrated tokenizer: {} iterations in {:?} (avg: {:?})", 
        iterations, calibrated_duration, calibrated_duration / iterations);
    
    let overhead = calibrated_duration.as_nanos() as f64 / simple_duration.as_nanos() as f64;
    println!("Calibration overhead: {overhead:.2}x");
    
    // Калиброванный токенизатор не должен быть более чем в 3 раза медленнее
    assert!(overhead < 3.0, "Calibrated tokenizer is too slow: {overhead:.2}x overhead");
    
    // Получаем результаты для сравнения точности
    let simple_count = tokenizer::count_gemini_tokens(test_text).unwrap();
    let calibrated_count = tokenizer::count_calibrated_gemini_tokens(test_text).unwrap();
    
    println!("\nToken counts for test text:");
    println!("Simple: {simple_count} tokens");
    println!("Calibrated: {calibrated_count} tokens");
    println!("Text: \"{test_text}\"");
}