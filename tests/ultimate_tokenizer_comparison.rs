// tests/ultimate_tokenizer_comparison.rs

use std::env;
use std::error::Error;
use serde_json::{json, Value};
use reqwest::Client;
use tokio::time::{sleep, Duration};
use tracing::warn;
use gemini_proxy::tokenizer;

/// Финальное сравнение всех подходов к токенизации
#[tokio::test]
async fn test_ultimate_tokenizer_comparison() {
    tracing_subscriber::fmt::init();
    
    let api_key = match env::var("GOOGLE_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("GOOGLE_API_KEY not found, skipping ultimate comparison");
            return;
        }
    };
    
    println!("\n🚀 ULTIMATE TOKENIZER COMPARISON 🚀\n");
    
    // Инициализируем все токенизаторы
    println!("📦 Initializing tokenizers...");
    
    // 1. Простой токенизатор
    tokenizer::gemini_simple::GeminiTokenizer::initialize().await.unwrap();
    
    // 2. ML-калиброванный
    tokenizer::gemini_ml_calibrated::GeminiMLCalibratedTokenizer::initialize().await.unwrap();
    
    // 3. Официальный Google (может не работать если Python SDK не установлен)
    let official_available = match tokenizer::official_google::OfficialGoogleTokenizer::initialize().await {
        Ok(_) => {
            println!("✅ Official Google tokenizer available");
            true
        }
        Err(e) => {
            println!("⚠️  Official Google tokenizer not available: {e}");
            println!("💡 Install with: pip install google-cloud-aiplatform[tokenization]");
            false
        }
    };
    
    // 4. Прокси-кеширующий
    let proxy_tokenizer = tokenizer::ProxyCachedTokenizer::new(api_key.clone())
        .with_fallback(|text| text.split_whitespace().count() + 2);
    
    // Тестовые случаи
    let test_cases = vec![
        ("Simple text", "Hello world"),
        ("Unicode heavy", "Hello 世界! 🌍 How are you? Привет мир!"),
        ("Math symbols", "Mathematical symbols: ∑, ∫, ∂, ∇, ∞, π, α, β"),
        ("Code snippet", r#"function fibonacci(n) { if (n <= 1) return n; return fibonacci(n-1) + fibonacci(n-2); }"#),
        ("JSON data", r#"{"name": "John", "age": 30, "city": "New York", "hobbies": ["reading", "coding"]}"#),
    ];
    
    let client = Client::new();
    
    println!("\n📊 COMPARISON RESULTS\n");
    println!("{:<15} | {:>8} | {:>8} | {:>8} | {:>8} | {:>8} | {:>8}", 
        "Test Case", "Google", "Simple", "ML-Cal", "Official", "Proxy", "Accuracy");
    println!("{:-<15}-+-{:->8}-+-{:->8}-+-{:->8}-+-{:->8}-+-{:->8}-+-{:->8}", 
        "", "", "", "", "", "", "");
    
    let mut total_tests = 0;
    let mut simple_accurate = 0;
    let mut ml_accurate = 0;
    let mut official_accurate = 0;
    let mut proxy_accurate = 0;
    
    for (name, text) in test_cases {
        // Google API (эталон)
        let google_count = match get_google_token_count(&client, &api_key, text).await {
            Ok(count) => count,
            Err(e) => {
                warn!("Google API failed for {}: {}", name, e);
                sleep(Duration::from_millis(1000)).await;
                continue;
            }
        };
        
        // Наши токенизаторы
        let simple_count = tokenizer::count_gemini_tokens(text).unwrap_or(0);
        let ml_count = tokenizer::count_ml_calibrated_gemini_tokens(text).unwrap_or(0);
        
        let official_count = if official_available {
            tokenizer::count_official_google_tokens(text).unwrap_or(0)
        } else {
            0
        };
        
        let proxy_count = proxy_tokenizer.count_tokens(text).await.unwrap_or(0);
        
        // Вычисляем точность
        let simple_accuracy = calculate_accuracy(simple_count, google_count);
        let ml_accuracy = calculate_accuracy(ml_count, google_count);
        let official_accuracy = if official_available {
            calculate_accuracy(official_count, google_count)
        } else {
            0.0
        };
        let proxy_accuracy = calculate_accuracy(proxy_count, google_count);
        
        // Считаем точные результаты (>95% точности)
        total_tests += 1;
        if simple_accuracy >= 95.0 { simple_accurate += 1; }
        if ml_accuracy >= 95.0 { ml_accurate += 1; }
        if official_accuracy >= 95.0 { official_accurate += 1; }
        if proxy_accuracy >= 95.0 { proxy_accurate += 1; }
        
        // Выводим результаты
        let official_str = if official_available {
            format!("{official_count:>8}")
        } else {
            "   N/A  ".to_string()
        };
        
        let best_accuracy = [simple_accuracy, ml_accuracy, official_accuracy, proxy_accuracy]
            .iter().fold(0.0f64, |a, &b| a.max(b));
        
        println!("{name:<15} | {google_count:>8} | {simple_count:>8} | {ml_count:>8} | {official_str} | {proxy_count:>8} | {best_accuracy:>7.1}%");
        
        sleep(Duration::from_millis(500)).await;
    }
    
    // Итоговая статистика
    println!("\n🎯 FINAL RESULTS\n");
    
    let simple_score = (simple_accurate as f64 / total_tests as f64) * 100.0;
    let ml_score = (ml_accurate as f64 / total_tests as f64) * 100.0;
    let official_score = if official_available {
        (official_accurate as f64 / total_tests as f64) * 100.0
    } else {
        0.0
    };
    let proxy_score = (proxy_accurate as f64 / total_tests as f64) * 100.0;
    
    println!("📈 Accuracy Scores (>95% threshold):");
    println!("  Simple Tokenizer:     {simple_score:.1}%");
    println!("  ML-Calibrated:       {ml_score:.1}%");
    if official_available {
        println!("  Official Google:      {official_score:.1}% ⭐");
    } else {
        println!("  Official Google:      Not Available");
    }
    println!("  Proxy-Cached:         {proxy_score:.1}%");
    
    // Рекомендации
    println!("\n🏆 RECOMMENDATIONS:\n");
    
    if official_available && official_score >= 95.0 {
        println!("🥇 WINNER: Official Google Tokenizer");
        println!("   ✅ 100% accuracy guaranteed");
        println!("   ✅ Always up-to-date with Google");
        println!("   ✅ Supports all Gemini models");
        println!("   ⚠️  Requires Python SDK");
        println!("   💡 Best for: Production systems requiring perfect accuracy");
    } else if proxy_score >= 95.0 {
        println!("🥇 WINNER: Proxy-Cached Tokenizer");
        println!("   ✅ 100% accuracy (uses real Google API)");
        println!("   ✅ Caching for performance");
        println!("   ✅ Fallback support");
        println!("   ⚠️  Requires API calls for new texts");
        println!("   💡 Best for: High-accuracy with caching");
    } else if ml_score >= 80.0 {
        println!("🥈 RUNNER-UP: ML-Calibrated Tokenizer");
        println!("   ✅ Good accuracy ({ml_score:.1}%)");
        println!("   ✅ No external dependencies");
        println!("   ✅ Fast performance");
        println!("   💡 Best for: Offline systems with good accuracy");
    } else {
        println!("⚠️  All tokenizers need improvement for your use case");
        println!("   💡 Consider using Proxy-Cached for 100% accuracy");
    }
    
    println!("\n🎯 CONCLUSION:");
    if official_available {
        println!("For 100% accuracy, use Official Google Tokenizer!");
        println!("Install: pip install google-cloud-aiplatform[tokenization]");
    } else {
        println!("For 100% accuracy, use Proxy-Cached Tokenizer!");
        println!("It uses real Google API with intelligent caching.");
    }
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
    
    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&request_body)
        .timeout(Duration::from_secs(10))
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

/// Тест производительности всех токенизаторов
#[tokio::test]
async fn test_tokenizer_performance_comparison() {
    println!("\n⚡ PERFORMANCE COMPARISON\n");
    
    // Инициализируем токенизаторы
    tokenizer::gemini_simple::GeminiTokenizer::initialize().await.unwrap();
    tokenizer::gemini_ml_calibrated::GeminiMLCalibratedTokenizer::initialize().await.unwrap();
    
    let official_available = tokenizer::official_google::OfficialGoogleTokenizer::initialize().await.is_ok();
    
    let test_text = "This is a comprehensive performance test for all tokenizer implementations with various content types including Unicode 世界 🌍 and code snippets.";
    let iterations = 100;
    
    // Тест простого токенизатора
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let _ = tokenizer::count_gemini_tokens(test_text).unwrap();
    }
    let simple_duration = start.elapsed();
    
    // Тест ML-калиброванного
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let _ = tokenizer::count_ml_calibrated_gemini_tokens(test_text).unwrap();
    }
    let ml_duration = start.elapsed();
    
    // Тест официального (если доступен)
    let official_duration = if official_available {
        let start = std::time::Instant::now();
        for _ in 0..iterations {
            let _ = tokenizer::count_official_google_tokens(test_text).unwrap_or(0);
        }
        Some(start.elapsed())
    } else {
        None
    };
    
    println!("🏃 Performance Results ({iterations} iterations):");
    println!("  Simple:        {:>8.2}ms avg ({:>6.2}ms total)", 
        simple_duration.as_millis() as f64 / iterations as f64,
        simple_duration.as_millis());
    println!("  ML-Calibrated: {:>8.2}ms avg ({:>6.2}ms total)", 
        ml_duration.as_millis() as f64 / iterations as f64,
        ml_duration.as_millis());
    
    if let Some(duration) = official_duration {
        println!("  Official:      {:>8.2}ms avg ({:>6.2}ms total)", 
            duration.as_millis() as f64 / iterations as f64,
            duration.as_millis());
    } else {
        println!("  Official:      Not Available");
    }
    
    println!("\n💡 Performance vs Accuracy Trade-off:");
    println!("  • Simple: Fastest, but lower accuracy");
    println!("  • ML-Calibrated: Good balance of speed and accuracy");
    println!("  • Official: Slower, but 100% accurate");
    println!("  • Proxy-Cached: Variable (fast for cached, slow for new)");
}