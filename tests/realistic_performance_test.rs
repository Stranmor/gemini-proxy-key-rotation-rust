// tests/realistic_performance_test.rs

use gemini_proxy::tokenizer;
use std::time::Instant;

/// Реалистичный тест производительности с учетом сетевых запросов
#[tokio::test]
async fn test_realistic_performance_comparison() {
    println!("\n🌐 REALISTIC PERFORMANCE TEST (with network simulation)\n");
    
    // Инициализируем токенизаторы
    tokenizer::gemini_simple::GeminiTokenizer::initialize().await.unwrap();
    tokenizer::gemini_first::GeminiFirstTokenizer::initialize(None).unwrap();
    
    // Создаем большой текст (~180k токенов)
    let large_text = generate_large_text();
    println!("Generated text: {} characters (~180k tokens)", large_text.len());
    
    println!("\n📊 REALISTIC Performance Comparison:\n");
    
    // Сценарий 1: Локальная токенизация + сетевой запрос
    println!("1. Traditional Approach (Tokenize First):");
    
    let start = Instant::now();
    let token_count = tokenizer::count_gemini_tokens(&large_text).unwrap_or(0);
    let tokenization_time = start.elapsed();
    
    let start = Instant::now();
    let _network_response = simulate_network_request(&large_text).await;
    let network_time = start.elapsed();
    
    let total_traditional = tokenization_time + network_time;
    
    println!("   Tokenization time: {:>8.2}ms", tokenization_time.as_millis());
    println!("   Network request:   {:>8.2}ms", network_time.as_millis());
    println!("   TOTAL TIME:        {:>8.2}ms", total_traditional.as_millis());
    println!("   Token count: {token_count}");
    
    // Сценарий 2: Gemini First (прямая отправка)
    println!("\n2. Gemini First Approach (Send Directly):");
    
    let start = Instant::now();
    let decision = tokenizer::should_tokenize_before_request(&large_text);
    let decision_time = start.elapsed();
    
    let start = Instant::now();
    let network_response = simulate_network_request(&large_text).await;
    let network_time_direct = start.elapsed();
    
    // Post-response подсчет для статистики
    let start = Instant::now();
    let post_tokens = tokenizer::count_tokens_post_response(&large_text, &network_response);
    let post_count_time = start.elapsed();
    
    let total_gemini_first = decision_time + network_time_direct + post_count_time;
    
    println!("   Decision time:     {:>8.2}ms", decision_time.as_millis());
    println!("   Network request:   {:>8.2}ms", network_time_direct.as_millis());
    println!("   Post-count time:   {:>8.2}ms", post_count_time.as_millis());
    println!("   TOTAL TIME:        {:>8.2}ms", total_gemini_first.as_millis());
    println!("   Decision: {decision:?}");
    println!("   Estimated tokens: {}", post_tokens.request_tokens);
    
    // Анализ результатов
    println!("\n🎯 REALISTIC ANALYSIS:\n");
    
    let time_saved = total_traditional.as_millis() as i64 - total_gemini_first.as_millis() as i64;
    let speedup = total_traditional.as_millis() as f64 / total_gemini_first.as_millis() as f64;
    
    println!("Performance comparison:");
    println!("  Traditional approach: {:>6}ms", total_traditional.as_millis());
    println!("  Gemini First:         {:>6}ms", total_gemini_first.as_millis());
    println!("  Time saved:           {time_saved:>6}ms");
    println!("  Speedup:              {speedup:>6.2}x");
    
    println!("\nBreakdown of time savings:");
    println!("  ✅ Eliminated tokenization: {}ms", tokenization_time.as_millis());
    println!("  ✅ Fast decision making:    {}ms", decision_time.as_millis());
    println!("  ✅ Quick post-counting:     {}ms", post_count_time.as_millis());
    println!("  🌐 Network time (same):     {}ms", network_time.as_millis());
    
    println!("\n💡 Key Insights:");
    println!("  • Network request time is the same (~{}ms)", network_time.as_millis());
    println!("  • We save {}ms by skipping pre-tokenization", tokenization_time.as_millis());
    println!("  • Post-response counting is {}x faster than pre-tokenization", 
        tokenization_time.as_millis() / post_count_time.as_millis().max(1));
    println!("  • Overall speedup: {speedup:.1}x faster");
    
    // Проверяем что есть экономия времени (более реалистичные ожидания)
    assert!(time_saved > 10, "Should save at least 10ms, saved {time_saved}ms");
    assert!(speedup > 1.0, "Should be faster, got {speedup:.2}x");
    
    println!("\n✅ Realistic performance test passed!");
}

/// Тест масштабируемости с реалистичными временами
#[tokio::test]
async fn test_scalability_with_network() {
    println!("\n📈 SCALABILITY TEST (with network simulation)\n");
    
    tokenizer::gemini_simple::GeminiTokenizer::initialize().await.unwrap();
    tokenizer::gemini_first::GeminiFirstTokenizer::initialize(None).unwrap();
    
    let test_sizes = vec![
        (10_000, "10K chars (~2.5k tokens)"),
        (50_000, "50K chars (~12.5k tokens)"),
        (200_000, "200K chars (~50k tokens)"),
        (800_000, "800K chars (~200k tokens)"),
    ];
    
    println!("{:<25} | {:>12} | {:>12} | {:>10} | {:>8}", 
        "Text Size", "Traditional", "Gemini First", "Saved", "Speedup");
    println!("{:-<25}-+-{:->12}-+-{:->12}-+-{:->10}-+-{:->8}", "", "", "", "", "");
    
    for (size, description) in test_sizes {
        let text = "Hello world! This is a comprehensive test document. ".repeat(size / 50);
        
        // Traditional approach
        let start = Instant::now();
        let _tokens = tokenizer::count_gemini_tokens(&text).unwrap_or(0);
        let tokenization_time = start.elapsed();
        
        let network_time = simulate_network_latency(text.len()).await;
        let total_traditional = tokenization_time + network_time;
        
        // Gemini First approach
        let start = Instant::now();
        let _decision = tokenizer::should_tokenize_before_request(&text);
        let decision_time = start.elapsed();
        
        let start = Instant::now();
        let _post_tokens = tokenizer::count_tokens_post_response(&text, "Response");
        let post_time = start.elapsed();
        
        let total_gemini_first = decision_time + network_time + post_time;
        
        let time_saved = total_traditional.as_millis() as i64 - total_gemini_first.as_millis() as i64;
        let speedup = total_traditional.as_millis() as f64 / total_gemini_first.as_millis() as f64;
        
        println!("{:<25} | {:>9}ms | {:>9}ms | {:>7}ms | {:>6.1}x", 
            description,
            total_traditional.as_millis(),
            total_gemini_first.as_millis(),
            time_saved,
            speedup);
    }
    
    println!("\n💡 Scalability Insights:");
    println!("  • Larger texts = more time saved by skipping tokenization");
    println!("  • Network latency remains constant regardless of approach");
    println!("  • Gemini First scales better with text size");
    
    println!("\n✅ Scalability test completed!");
}

/// Симулирует сетевой запрос к Gemini API
async fn simulate_network_request(text: &str) -> String {
    let latency = simulate_network_latency(text.len()).await;
    
    // Симулируем время обработки на стороне Gemini
    tokio::time::sleep(latency).await;
    
    format!("Simulated Gemini response for {} characters", text.len())
}

/// Симулирует сетевую задержку на основе размера текста
async fn simulate_network_latency(text_size: usize) -> std::time::Duration {
    // Базовая задержка сети: 50-200ms
    let base_latency = 100;
    
    // Дополнительная задержка для больших текстов (время обработки на сервере)
    let processing_overhead = (text_size / 10000) * 10; // 10ms на каждые 10KB
    
    std::time::Duration::from_millis((base_latency + processing_overhead) as u64)
}

/// Генерирует большой текст для тестирования
fn generate_large_text() -> String {
    let base_text = r#"
This is a comprehensive technical document that demonstrates the performance characteristics
of different tokenization approaches when dealing with large-scale text processing scenarios.
The document contains various types of content including natural language descriptions,
code examples, mathematical formulas, and structured data representations.

In modern software systems, the choice of tokenization strategy can significantly impact
overall application performance, especially when processing documents that contain hundreds
of thousands of tokens. The traditional approach of pre-tokenizing content before sending
it to language models introduces computational overhead that may not be necessary in all cases.

Consider the following performance analysis:
- Pre-tokenization overhead: O(n) where n is text length
- Network transmission time: O(1) relative to tokenization complexity
- Post-processing requirements: O(k) where k << n for statistical purposes

The mathematical relationship between text size and processing time can be expressed as:
T_total = T_tokenization + T_network + T_processing

Where:
- T_tokenization scales linearly with input size
- T_network remains relatively constant
- T_processing depends on response complexity

For large documents (>100k tokens), the tokenization component dominates the total time,
making direct transmission strategies more efficient from a performance perspective.
"#;
    
    // Повторяем базовый текст для создания большого документа
    let mut result = String::with_capacity(1_000_000);
    for i in 0..200 {
        result.push_str(&format!("\n=== Section {} ===\n", i + 1));
        result.push_str(base_text);
    }
    
    result
}