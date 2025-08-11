// tests/ml_calibrated_tokenizer_test.rs

use gemini_proxy::tokenizer;
use reqwest::Client;
use serde_json::{json, Value};
use std::env;
use std::error::Error;
use tokio::time::{sleep, Duration};
use tracing::{debug, error, warn};

/// Финальный тест ML-калиброванного токенизатора против Google API
#[tokio::test]
async fn test_ml_calibrated_tokenizer_accuracy() {
    // Инициализируем логирование
    tracing_subscriber::fmt::init();

    // Проверяем наличие API ключа
    let api_key = match env::var("GOOGLE_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("GOOGLE_API_KEY not found, skipping ML-calibrated tokenizer test");
            return;
        }
    };

    // Инициализируем ML-калиброванный токенизатор
    if let Err(e) = tokenizer::gemini_ml_calibrated::GeminiMLCalibratedTokenizer::initialize().await
    {
        panic!("Failed to initialize ML-calibrated tokenizer: {e}");
    }

    // Тестовые случаи на основе предыдущих результатов
    let test_cases = vec![
        // Простые случаи (должны быть точными)
        "Hello",
        "Hello world",
        "Hello, world!",
        "The quick brown fox jumps over the lazy dog.",
        "What is the capital of France?",

        // Проблемные случаи (требуют ML-калибровки)
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

        // Дополнительные сложные случаи
        "Create a detailed explanation of how machine learning algorithms work, including supervised and unsupervised learning approaches.",
        "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.",
        "Write a Python function to calculate fibonacci numbers recursively and iteratively.",
        "Translate the following text from English to Spanish: 'Hello, how are you today? I hope you're having a wonderful day!'",
    ];

    let client = Client::new();
    let mut total_tests = 0;
    let mut accurate_tests = 0;
    let mut very_accurate_tests = 0; // Точность > 95%
    let mut total_our_tokens = 0;
    let mut total_google_tokens = 0;
    let mut total_absolute_error = 0;

    println!("\n=== ML-Calibrated Tokenizer vs Google API ===\n");

    for (i, text) in test_cases.iter().enumerate() {
        println!(
            "Test case {}: \"{}\"",
            i + 1,
            if text.chars().count() > 50 {
                format!("{}...", text.chars().take(50).collect::<String>())
            } else {
                text.to_string()
            }
        );

        // Получаем количество токенов от ML-калиброванного токенизатора
        let our_count =
            match tokenizer::gemini_ml_calibrated::count_ml_calibrated_gemini_tokens(text) {
            Ok(count) => count,
            Err(e) => {
                error!("ML-calibrated tokenizer failed for text: {}", e);
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

        let absolute_error = (our_count as i32 - google_count as i32).unsigned_abs() as usize;
        total_absolute_error += absolute_error;

        // Вычисляем точность
        let accuracy = if google_count > 0 {
            let diff = absolute_error as f64;
            let accuracy = (1.0 - (diff / google_count as f64)) * 100.0;
            accuracy.max(0.0)
        } else if our_count == 0 {
            100.0
        } else {
            0.0
        };

        // Считаем тест точным если расхождение менее 10%
        if accuracy >= 90.0 {
            accurate_tests += 1;
        }

        // Считаем очень точным если расхождение менее 5%
        if accuracy >= 95.0 {
            very_accurate_tests += 1;
        }

        println!("  ML-Calibrated: {our_count} | Google: {google_count} | Error: {absolute_error} | Accuracy: {accuracy:.1}%");

        if accuracy >= 95.0 {
            println!("  🎯 Excellent accuracy!");
        } else if accuracy >= 90.0 {
            println!("  ✅ Good accuracy");
        } else if accuracy >= 80.0 {
            println!("  ⚠️  Acceptable accuracy");
        } else {
            println!("  ❌ Poor accuracy - needs improvement");
        }

        // Добавляем задержку между запросами к API
        sleep(Duration::from_millis(500)).await;
    }

    // Итоговая статистика
    println!("\n=== ML-Calibrated Tokenizer Summary ===");
    println!("Total tests: {total_tests}");
    println!("Accurate tests (>90%): {accurate_tests}");
    println!("Very accurate tests (>95%): {very_accurate_tests}");
    println!(
        "Overall accuracy: {:.1}%",
        (accurate_tests as f64 / total_tests as f64) * 100.0
    );
    println!(
        "Excellent accuracy: {:.1}%",
        (very_accurate_tests as f64 / total_tests as f64) * 100.0
    );

    let overall_ratio = if total_google_tokens > 0 {
        total_our_tokens as f64 / total_google_tokens as f64
    } else {
        1.0
    };

    let mean_absolute_error = total_absolute_error as f64 / total_tests as f64;

    println!("Total ML-calibrated tokens: {total_our_tokens}");
    println!("Total Google tokens: {total_google_tokens}");
    println!("Overall ratio (ML/google): {overall_ratio:.3}");
    println!("Mean Absolute Error: {mean_absolute_error:.2} tokens");

    if overall_ratio > 1.1 {
        println!("⚠️  Still slightly overestimating tokens");
    } else if overall_ratio < 0.9 {
        println!("⚠️  Still slightly underestimating tokens");
    } else {
        println!("🎯 Token ratio is excellently calibrated!");
    }

    // Проверяем качество ML-калибровки
    let overall_accuracy = (accurate_tests as f64 / total_tests as f64) * 100.0;
    let excellent_accuracy = (very_accurate_tests as f64 / total_tests as f64) * 100.0;

    println!("\nML-Calibration effectiveness:");
    if excellent_accuracy >= 70.0 {
        println!("🚀 Outstanding ML-calibration! Excellent accuracy: {excellent_accuracy:.1}%");
    } else if overall_accuracy >= 85.0 {
        println!("🎉 Excellent ML-calibration! Overall accuracy: {overall_accuracy:.1}%");
    } else if overall_accuracy >= 75.0 {
        println!("✅ Good ML-calibration! Overall accuracy: {overall_accuracy:.1}%");
    } else {
        println!("⚠️  ML-calibration needs more training data. Accuracy: {overall_accuracy:.1}%");
    }

    // Рекомендации по улучшению
    if mean_absolute_error > 2.0 {
        println!("\n📊 Recommendations:");
        println!("- Mean error is {mean_absolute_error:.2} tokens - consider more training data");
        println!("- Focus on cases with >2 token errors for model improvement");
    } else if mean_absolute_error > 1.0 {
        println!("\n📊 Good performance with room for improvement:");
        println!("- Mean error is {mean_absolute_error:.2} tokens - fine-tune model parameters");
    } else {
        println!("\n🎯 Excellent performance! Mean error: {mean_absolute_error:.2} tokens");
    }

    // Тест проходит если точность > 70% (реалистичный порог для ML-модели)
    assert!(
        overall_accuracy >= 70.0,
        "ML-calibrated tokenizer accuracy {overall_accuracy:.1}% is below 70% threshold"
    );

    // Дополнительная проверка на соотношение токенов
    assert!(
        (0.85..=1.15).contains(&overall_ratio),
        "Token ratio {overall_ratio:.3} is outside acceptable range [0.85, 1.15]"
    );
}

/// Получает количество токенов от Google API
async fn get_google_token_count(
    client: &Client,
    api_key: &str,
    text: &str,
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
    debug!(
        "Request body: {}",
        serde_json::to_string_pretty(&request_body)?
    );

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

/// Тест производительности ML-калиброванного токенизатора
#[tokio::test]
async fn test_ml_calibrated_performance() {
    tokenizer::gemini_ml_calibrated::GeminiMLCalibratedTokenizer::initialize()
        .await
        .unwrap();

    let test_text = "This is a comprehensive performance test for the ML-calibrated Gemini tokenizer implementation with Unicode symbols: 世界 🌍 and mathematical notation: ∑∫∂∇∞π and code: function test() { return 42; }";
    let iterations = 1000;

    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let _ = tokenizer::gemini_ml_calibrated::count_ml_calibrated_gemini_tokens(test_text)
            .unwrap();
    }
    let duration = start.elapsed();

    println!("\n=== ML-Calibrated Performance ===");
    println!("{iterations} ML-calibrated tokenizations took: {duration:?}");
    println!("Average: {:?} per tokenization", duration / iterations);

    // ML-калиброванный токенизатор не должен быть слишком медленным
    let avg_ms = duration.as_millis() as f64 / iterations as f64;
    println!("Average per operation: {avg_ms:.3}ms");

    assert!(
        avg_ms < 1.0,
        "ML-calibrated tokenizer should be < 1ms per operation, got {avg_ms:.3}ms"
    );

    let count =
        tokenizer::gemini_ml_calibrated::count_ml_calibrated_gemini_tokens(test_text).unwrap();
    println!("Token count for test text: {count}");
    println!("Text: \"{test_text}\"");
}
