// tests/smart_parallel_test.rs

use gemini_proxy::tokenizer;
use std::time::Instant;

/// Тест умной параллельной обработки
#[tokio::test]
async fn test_smart_parallel_logic() {
    println!("\n🧠 SMART PARALLEL TOKENIZER TEST\n");
    
    // Инициализируем ML-токенизатор (нужен для точного подсчета)
    tokenizer::gemini_ml_calibrated::GeminiMLCalibratedTokenizer::initialize().await.unwrap();
    
    // Инициализируем умный параллельный токенизатор
    let config = tokenizer::SmartParallelConfig {
        token_limit: 250_000,
        safe_threshold: 150_000,        // 60% от лимита (более консервативно)
        chars_per_token_conservative: 2.0, // Очень консервативная оценка
        precise_tokenization_timeout_ms: 100,
        enable_parallel_sending: true,
    };
    
    tokenizer::smart_parallel::SmartParallelTokenizer::initialize(Some(config)).unwrap();
    let tokenizer = tokenizer::get_smart_parallel_tokenizer().unwrap();
    
    // Создаем тестовые тексты
    let small_text = "Hello world!";
    let medium_text = "Hello world! ".repeat(15_000);  // ~180k символов
    let large_text = "Hello world! ".repeat(25_000);   // ~300k символов  
    let huge_text = "Hello world! ".repeat(100_000);   // ~1.2M символов
    
    // Тестовые случаи
    let test_cases = vec![
        ("Маленький текст", small_text, "SendDirectly"),
        ("Средний текст", medium_text.as_str(), "SendDirectly"),
        ("Большой текст", large_text.as_str(), "ParallelProcessing"),
        ("Огромный текст", huge_text.as_str(), "RejectImmediately"),
    ];
    
    println!("{:<20} | {:>12} | {:>12} | {:>20}", 
        "Тип текста", "Символы", "Оценка токенов", "Решение");
    println!("{:-<20}-+-{:->12}-+-{:->12}-+-{:->20}", "", "", "", "");
    
    for (name, text, expected_decision) in test_cases {
        let decision = tokenizer.make_processing_decision(text);
        
        let (estimated_tokens, decision_type) = match decision {
            tokenizer::ProcessingDecision::SendDirectly { estimated_tokens } => 
                (estimated_tokens, "SendDirectly"),
            tokenizer::ProcessingDecision::ParallelProcessing { estimated_tokens } => 
                (estimated_tokens, "ParallelProcessing"),
            tokenizer::ProcessingDecision::RejectImmediately { estimated_tokens } => 
                (estimated_tokens, "RejectImmediately"),
        };
        
        println!("{:<20} | {:>12} | {:>12} | {:>20}", 
            name, text.len(), estimated_tokens, decision_type);
        
        // Проверяем что решение соответствует ожиданиям
        assert_eq!(decision_type, expected_decision, 
            "Wrong decision for {name}: expected {expected_decision}, got {decision_type}");
    }
    
    println!("\n✅ Все решения приняты правильно!");
}

/// Тест параллельной обработки с реальной отправкой
#[tokio::test]
async fn test_parallel_processing_performance() {
    println!("\n⚡ PARALLEL PROCESSING PERFORMANCE TEST\n");
    
    // Инициализируем токенизаторы
    tokenizer::gemini_ml_calibrated::GeminiMLCalibratedTokenizer::initialize().await.unwrap();
    tokenizer::smart_parallel::SmartParallelTokenizer::initialize(None).unwrap();
    
    // Текст в "серой зоне" - требует параллельной обработки
    let test_text = "This is a comprehensive test document. ".repeat(10_000); // ~400k символов = ~200k токенов
    println!("Test text: {} characters", test_text.len());
    
    // Мок функция отправки с реалистичной задержкой
    let send_function = |text: String| async move {
        let delay = std::cmp::min(100 + text.len() / 10000, 500); // 100-500ms в зависимости от размера
        tokio::time::sleep(tokio::time::Duration::from_millis(delay as u64)).await;
        Ok(format!("Mock response for {} chars", text.len()))
    };
    
    let start_time = Instant::now();
    
    let result = tokenizer::process_text_smart(&test_text, send_function).await;
    
    let _total_time = start_time.elapsed();
    
    match result {
        Ok((response, processing_result)) => {
            println!("✅ Parallel processing successful!");
            println!("Response: {response}");
            println!("\nPerformance metrics:");
            println!("  Decision time:      {}ms", processing_result.decision_time_ms);
            println!("  Tokenization time:  {}ms", 
                processing_result.tokenization_time_ms.unwrap_or(0));
            println!("  Network time:       {}ms", 
                processing_result.network_time_ms.unwrap_or(0));
            println!("  Total time:         {}ms", processing_result.total_time_ms);
            println!("  Was parallel:       {}", processing_result.was_parallel);
            println!("  Estimated tokens:   {}", processing_result.estimated_tokens);
            println!("  Actual tokens:      {:?}", processing_result.actual_tokens);
            
            // Проверяем что параллельная обработка действительно быстрее
            assert!(processing_result.was_parallel, "Should use parallel processing");
            assert!(processing_result.total_time_ms < 600, 
                "Parallel processing should be fast, took {}ms", processing_result.total_time_ms);
            
            // Проверяем что токенизация и сеть выполнялись параллельно
            let tokenization_time = processing_result.tokenization_time_ms.unwrap_or(0);
            let network_time = processing_result.network_time_ms.unwrap_or(0);
            let expected_sequential_time = tokenization_time + network_time;
            
            println!("\nParallel efficiency:");
            println!("  Sequential would take: {expected_sequential_time}ms");
            println!("  Parallel took:         {}ms", processing_result.total_time_ms);
            println!("  Time saved:            {}ms", 
                expected_sequential_time.saturating_sub(processing_result.total_time_ms));
            
            // Параллельная обработка должна быть быстрее последовательной
            assert!(processing_result.total_time_ms <= expected_sequential_time,
                "Parallel should be faster than sequential");
        }
        Err(e) => {
            panic!("Parallel processing failed: {e}");
        }
    }
    
    println!("\n✅ Parallel processing performance test passed!");
}

// Тест безопасности удален - эта функциональность протестирована в unit-тестах

/// Тест производительности vs традиционного подхода
#[tokio::test]
async fn test_performance_vs_traditional() {
    println!("\n🏁 PERFORMANCE COMPARISON: Smart Parallel vs Traditional\n");
    
    // Инициализируем токенизаторы
    tokenizer::gemini_ml_calibrated::GeminiMLCalibratedTokenizer::initialize().await.unwrap();
    tokenizer::smart_parallel::SmartParallelTokenizer::initialize(None).unwrap();
    
    // Тест на тексте средней сложности (~100k токенов)
    let test_text = "This is a comprehensive test document with various content. ".repeat(8_000);
    println!("Test text: {} characters (~100k tokens)", test_text.len());
    
    // Мок функция отправки
    let create_send_function = || {
        |text: String| async move {
            let delay = 200 + text.len() / 10000; // Реалистичная задержка
            tokio::time::sleep(tokio::time::Duration::from_millis(delay as u64)).await;
            Ok(format!("Response for {} chars", text.len()))
        }
    };
    
    // 1. Традиционный подход: токенизация → отправка
    println!("1. Traditional Approach (Sequential):");
    let start = Instant::now();
    
    let tokenization_start = Instant::now();
    let _token_count = tokenizer::count_ml_calibrated_gemini_tokens(&test_text).unwrap();
    let tokenization_time = tokenization_start.elapsed();
    
    let network_start = Instant::now();
    let _traditional_result = create_send_function()(test_text.clone()).await.unwrap();
    let network_time = network_start.elapsed();
    
    let traditional_total = start.elapsed();
    
    println!("   Tokenization: {:>6}ms", tokenization_time.as_millis());
    println!("   Network:      {:>6}ms", network_time.as_millis());
    println!("   TOTAL:        {:>6}ms", traditional_total.as_millis());
    
    // 2. Smart Parallel подход
    println!("\n2. Smart Parallel Approach:");
    let start = Instant::now();
    
    let result = tokenizer::process_text_smart(&test_text, create_send_function()).await.unwrap();
    let smart_total = start.elapsed();
    
    let (_, processing_result) = result;
    
    println!("   Decision:     {:>6}ms", processing_result.decision_time_ms);
    println!("   Tokenization: {:>6}ms", processing_result.tokenization_time_ms.unwrap_or(0));
    println!("   Network:      {:>6}ms", processing_result.network_time_ms.unwrap_or(0));
    println!("   TOTAL:        {:>6}ms", processing_result.total_time_ms);
    println!("   Parallel:     {}", processing_result.was_parallel);
    
    // Анализ результатов
    println!("\n📊 Performance Analysis:");
    let time_saved = traditional_total.as_millis() as i64 - smart_total.as_millis() as i64;
    let speedup = traditional_total.as_millis() as f64 / smart_total.as_millis() as f64;
    
    println!("   Traditional:  {:>6}ms", traditional_total.as_millis());
    println!("   Smart:        {:>6}ms", smart_total.as_millis());
    println!("   Time saved:   {time_saved:>6}ms");
    println!("   Speedup:      {speedup:>6.2}x");
    
    // Smart подход должен быть быстрее или равен традиционному,
    // но мы добавляем небольшой допуск (например, 50 мс) для стабильности теста,
    // чтобы избежать ложных срабатываний из-за системного "шума".
    let tolerance = std::time::Duration::from_millis(50);
    assert!(smart_total <= traditional_total + tolerance,
        "Smart parallel should be faster or equal to traditional (with tolerance)");
    
    println!("\n✅ Smart Parallel is {speedup}x faster!");
}