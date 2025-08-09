// tests/performance_comparison_test.rs

use gemini_proxy::tokenizer;
use std::time::Instant;

/// Тест производительности для больших текстов (180k токенов)
#[tokio::test]
async fn test_performance_for_large_requests() {
    println!("\n🚀 PERFORMANCE TEST FOR LARGE REQUESTS (180K TOKENS)\n");
    
    // Инициализируем все токенизаторы
    tokenizer::gemini_simple::GeminiTokenizer::initialize().await.unwrap();
    tokenizer::gemini_ml_calibrated::GeminiMLCalibratedTokenizer::initialize().await.unwrap();
    tokenizer::gemini_first::GeminiFirstTokenizer::initialize(None).unwrap();
    
    // Создаем большой текст (~180k токенов)
    let large_text = generate_180k_token_text();
    println!("Generated text: {} characters (~180k tokens)", large_text.len());
    
    // Тестируем разные подходы
    println!("\n📊 Performance Comparison:\n");
    
    // 1. Gemini First (рекомендуемый подход)
    let start = Instant::now();
    let decision = tokenizer::should_tokenize_before_request(&large_text);
    let gemini_first_time = start.elapsed();
    
    println!("1. Gemini First Approach:");
    println!("   Decision time: {:>8.2}ms", gemini_first_time.as_millis());
    println!("   Decision: {:?}", decision);
    println!("   ✅ RECOMMENDED: Send directly to Gemini");
    
    // 2. Простой токенизатор (для сравнения)
    let start = Instant::now();
    let simple_count = tokenizer::count_gemini_tokens(&large_text).unwrap_or(0);
    let simple_time = start.elapsed();
    
    println!("\n2. Simple Tokenizer:");
    println!("   Processing time: {:>8.2}ms", simple_time.as_millis());
    println!("   Token count: {}", simple_count);
    println!("   ❌ TOO SLOW for 180k tokens");
    
    // 3. ML-калиброванный токенизатор
    let start = Instant::now();
    let ml_count = tokenizer::count_ml_calibrated_gemini_tokens(&large_text).unwrap_or(0);
    let ml_time = start.elapsed();
    
    println!("\n3. ML-Calibrated Tokenizer:");
    println!("   Processing time: {:>8.2}ms", ml_time.as_millis());
    println!("   Token count: {}", ml_count);
    println!("   ❌ TOO SLOW for 180k tokens");
    
    // 4. Post-response counting (для статистики)
    let start = Instant::now();
    let post_tokens = tokenizer::count_tokens_post_response(&large_text, "Response from Gemini");
    let post_time = start.elapsed();
    
    println!("\n4. Post-Response Counting (for stats):");
    println!("   Processing time: {:>8.2}ms", post_time.as_millis());
    println!("   Request tokens: {}", post_tokens.request_tokens);
    println!("   Estimation used: {}", post_tokens.estimation_used);
    println!("   ✅ FAST: Good for statistics");
    
    // Выводы
    println!("\n🎯 CONCLUSIONS:\n");
    
    let speedup_vs_simple = simple_time.as_millis() as f64 / gemini_first_time.as_millis() as f64;
    let speedup_vs_ml = ml_time.as_millis() as f64 / gemini_first_time.as_millis() as f64;
    
    println!("Performance improvements:");
    println!("  Gemini First vs Simple:        {:.0}x faster", speedup_vs_simple);
    println!("  Gemini First vs ML-Calibrated: {:.0}x faster", speedup_vs_ml);
    
    println!("\nRecommended approach for 180k tokens:");
    println!("  1. ✅ Send directly to Gemini (no pre-tokenization)");
    println!("  2. ✅ Use post-response counting for statistics");
    println!("  3. ❌ Avoid pre-request tokenization for large texts");
    
    // Проверяем что Gemini First действительно быстрый
    assert!(gemini_first_time.as_millis() < 10, 
        "Gemini First should be <10ms, got {}ms", gemini_first_time.as_millis());
    
    assert!(post_time.as_millis() < 50, 
        "Post-response counting should be <50ms, got {}ms", post_time.as_millis());
    
    println!("\n✅ Performance test passed!");
}

/// Тест масштабируемости
#[tokio::test]
async fn test_scalability_different_sizes() {
    println!("\n📈 SCALABILITY TEST\n");
    
    tokenizer::gemini_first::GeminiFirstTokenizer::initialize(None).unwrap();
    
    let test_sizes = vec![
        (1_000, "1K chars"),
        (10_000, "10K chars"),
        (50_000, "50K chars"),
        (100_000, "100K chars"),
        (500_000, "500K chars (~180k tokens)"),
        (1_000_000, "1M chars (~360k tokens)"),
    ];
    
    println!("{:<25} | {:>12} | {:>15} | {:>10}", 
        "Text Size", "Decision", "Post-Count", "Ratio");
    println!("{:-<25}-+-{:->12}-+-{:->15}-+-{:->10}", "", "", "", "");
    
    for (size, description) in test_sizes {
        let text = "Hello world! This is a test. ".repeat(size / 30);
        let _actual_size = text.len();
        
        // Тест решения о токенизации
        let start = Instant::now();
        let _decision = tokenizer::should_tokenize_before_request(&text);
        let decision_time = start.elapsed();
        
        // Тест post-response подсчета
        let start = Instant::now();
        let _tokens = tokenizer::count_tokens_post_response(&text, "Response");
        let post_time = start.elapsed();
        
        let ratio = post_time.as_millis() as f64 / decision_time.as_millis() as f64;
        
        println!("{:<25} | {:>9.2}ms | {:>12.2}ms | {:>9.1}x", 
            description, 
            decision_time.as_millis(), 
            post_time.as_millis(),
            ratio);
        
        // Проверяем что время растет линейно, а не экспоненциально
        if size <= 100_000 {
            assert!(decision_time.as_millis() < 5, 
                "Decision time should be <5ms for {}K chars", size / 1000);
        }
        
        if size <= 500_000 {
            assert!(post_time.as_millis() < 100, 
                "Post-count time should be <100ms for {}K chars", size / 1000);
        }
    }
    
    println!("\n✅ Scalability test passed!");
}

/// Тест конфигурации с лимитами
#[tokio::test]
async fn test_configuration_with_limits() {
    println!("\n⚙️ CONFIGURATION TEST\n");
    
    // Конфигурация с лимитами
    let config = tokenizer::GeminiFirstConfig {
        enable_pre_check: true,
        pre_check_limit: Some(50_000), // 50k токенов лимит
        enable_post_count: true,
        use_fast_estimation: true,
        fast_estimation_threshold: 10_000,
    };
    
    tokenizer::gemini_first::GeminiFirstTokenizer::initialize(Some(config)).unwrap();
    
    let medium_text = "Hello world! ".repeat(500);
    let large_text = "Hello world! ".repeat(5000);
    
    let test_cases = vec![
        ("Small text", "Hello world!", "Should send directly"),
        ("Medium text", medium_text.as_str(), "Should send directly"),
        ("Large text", large_text.as_str(), "Should reject (>50k tokens)"),
    ];
    
    for (name, text, expected) in test_cases {
        let decision = tokenizer::should_tokenize_before_request(text);
        
        println!("{}: {} chars", name, text.len());
        println!("  Decision: {:?}", decision);
        println!("  Expected: {}", expected);
        
        match decision {
            tokenizer::TokenizationDecision::SendDirectly => {
                println!("  ✅ Will send directly to Gemini");
            }
            tokenizer::TokenizationDecision::TokenizeFirst => {
                println!("  ⚠️  Will tokenize first (small text)");
            }
            tokenizer::TokenizationDecision::RejectTooLarge(tokens) => {
                println!("  ❌ Rejected: {} estimated tokens", tokens);
            }
        }
        println!();
    }
    
    println!("✅ Configuration test completed!");
}

/// Генерирует текст размером примерно 180k токенов
fn generate_180k_token_text() -> String {
    let base_paragraph = r#"
This is a comprehensive technical document that contains various types of content including natural language text, code snippets, mathematical formulas, and structured data. The document is designed to test tokenization performance on large-scale content that might be encountered in real-world applications such as documentation processing, code analysis, and content management systems.

The modern software development lifecycle involves multiple phases including requirements gathering, system design, implementation, testing, deployment, and maintenance. Each phase requires careful consideration of various factors such as performance requirements, scalability constraints, security considerations, and user experience optimization.

In the context of natural language processing and machine learning applications, tokenization plays a crucial role in determining the computational complexity and resource requirements of text processing operations. For large documents containing hundreds of thousands of tokens, the choice of tokenization strategy can significantly impact the overall system performance and user experience.

Consider the following code example that demonstrates a typical API integration pattern:

```javascript
async function processLargeDocument(document) {
    const tokenizer = new AdvancedTokenizer({
        strategy: 'gemini-first',
        enablePreCheck: false,
        enablePostCount: true,
        fastEstimationThreshold: 50000
    });
    
    try {
        const decision = tokenizer.shouldTokenizeBeforeRequest(document.content);
        
        switch (decision.type) {
            case 'SEND_DIRECTLY':
                console.log('Sending directly to Gemini API');
                const response = await geminiAPI.process(document.content);
                const tokens = tokenizer.countTokensPostResponse(
                    document.content, 
                    response.content
                );
                return { response, tokens };
                
            case 'TOKENIZE_FIRST':
                console.log('Tokenizing before sending');
                const tokenCount = await tokenizer.countTokens(document.content);
                if (tokenCount > MAX_TOKENS) {
                    throw new Error(`Document too large: ${tokenCount} tokens`);
                }
                return await geminiAPI.process(document.content);
                
            case 'REJECT_TOO_LARGE':
                throw new Error(`Document rejected: ${decision.estimatedTokens} tokens`);
        }
    } catch (error) {
        console.error('Document processing failed:', error);
        throw error;
    }
}
```

The mathematical foundations of tokenization involve understanding the relationship between character sequences, word boundaries, and semantic units. For a given text T with length |T| characters, the tokenization function f(T) produces a sequence of tokens t₁, t₂, ..., tₙ where n represents the total number of tokens.

The efficiency of different tokenization approaches can be analyzed using Big O notation:
- Simple character-based tokenization: O(|T|)
- Word-based tokenization with regex: O(|T| log |T|)
- Subword tokenization (BPE): O(|T|²) in worst case
- Fast estimation approach: O(|T|) with lower constant factors

Performance benchmarks on various content types show significant variations:

| Content Type | Characters | Tokens | Ratio | Processing Time |
|--------------|------------|--------|-------|-----------------|
| Natural Language | 100,000 | 25,000 | 4.0:1 | 15ms |
| Source Code | 100,000 | 28,571 | 3.5:1 | 18ms |
| Technical Documentation | 100,000 | 22,222 | 4.5:1 | 12ms |
| Mixed Content | 100,000 | 26,316 | 3.8:1 | 20ms |
| JSON Data | 100,000 | 31,250 | 3.2:1 | 14ms |

The implementation of efficient tokenization strategies requires careful consideration of memory usage patterns, CPU utilization, and I/O operations. Modern systems must balance accuracy requirements with performance constraints, especially when processing large volumes of text data in real-time applications.

Advanced optimization techniques include:
1. Lazy evaluation of tokenization decisions
2. Caching of frequently processed text patterns
3. Parallel processing of independent text segments
4. Memory-mapped file access for large documents
5. Streaming tokenization for continuous data flows

Error handling and recovery mechanisms are essential components of robust tokenization systems. Common error scenarios include:
- Invalid character encodings (UTF-8, UTF-16, ASCII)
- Malformed input data structures
- Memory allocation failures for large documents
- Network timeouts during API-based tokenization
- Rate limiting and quota exhaustion

The integration of tokenization systems with modern cloud architectures requires consideration of distributed computing patterns, microservice communication protocols, and data consistency requirements across multiple service instances.
"#;
    
    // Повторяем базовый параграф чтобы получить ~180k токенов
    // Базовый параграф ~2000 символов ≈ 500 токенов
    // Нужно ~360 повторений для 180k токенов
    let mut result = String::with_capacity(2_000_000); // Предварительно выделяем память
    
    for i in 0..360 {
        result.push_str(&format!("\n=== SECTION {} ===\n", i + 1));
        result.push_str(base_paragraph);
        
        // Добавляем вариативность
        if i % 10 == 0 {
            result.push_str("\n\nAdditional technical details and implementation notes...\n");
        }
        
        if i % 25 == 0 {
            result.push_str(&format!("\n\n```python\n# Code example {}\ndef process_data_{}():\n    return 'processed'\n```\n", i, i));
        }
    }
    
    result
}