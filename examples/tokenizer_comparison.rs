// examples/tokenizer_comparison.rs

use std::env;
use gemini_proxy::tokenizer;

#[tokio::main]
async fn main() {
    // Инициализируем все токенизаторы
    println!("🚀 Инициализация токенизаторов...");
    
    if let Err(e) = tokenizer::gemini_simple::GeminiTokenizer::initialize().await {
        eprintln!("Ошибка инициализации простого токенизатора: {e}");
        return;
    }
    
    if let Err(e) = tokenizer::gemini_calibrated::GeminiCalibratedTokenizer::initialize().await {
        eprintln!("Ошибка инициализации калиброванного токенизатора: {e}");
        return;
    }
    
    if let Err(e) = tokenizer::gemini_ml_calibrated::GeminiMLCalibratedTokenizer::initialize().await {
        eprintln!("Ошибка инициализации ML-калиброванного токенизатора: {e}");
        return;
    }
    
    println!("✅ Все токенизаторы инициализированы\n");
    
    // Тестовые тексты
    let test_cases = vec![
        "Hello world",
        "Hello, world!",
        "The quick brown fox jumps over the lazy dog.",
        "Hello 世界! 🌍 How are you?",
        "Mathematical symbols: ∑, ∫, ∂, ∇, ∞, π",
        r#"function test() { return 42; }"#,
        r#"{"name": "John", "age": 30}"#,
    ];
    
    println!("📊 Сравнение токенизаторов:\n");
    println!("{:<50} | {:>8} | {:>12} | {:>15}", "Текст", "Простой", "Калибр.", "ML-Калибр.");
    println!("{:-<50}-+-{:->8}-+-{:->12}-+-{:->15}", "", "", "", "");
    
    for text in test_cases {
        let simple = tokenizer::count_gemini_tokens(text).unwrap_or(0);
        let calibrated = tokenizer::count_calibrated_gemini_tokens(text).unwrap_or(0);
        let ml_calibrated = tokenizer::count_ml_calibrated_gemini_tokens(text).unwrap_or(0);
        
        let display_text = if text.chars().count() > 47 {
            format!("{}...", text.chars().take(44).collect::<String>())
        } else {
            text.to_string()
        };
        
        println!("{display_text:<50} | {simple:>8} | {calibrated:>12} | {ml_calibrated:>15}");
    }
    
    println!("\n📈 Информация о токенизаторах:");
    
    if let Some(info) = tokenizer::get_gemini_tokenizer_info() {
        println!("• Простой: {info}");
    }
    
    if let Some(info) = tokenizer::get_calibrated_gemini_tokenizer_info() {
        println!("• Калиброванный: {info}");
    }
    
    if let Some(info) = tokenizer::get_ml_calibrated_gemini_tokenizer_info() {
        println!("• ML-Калиброванный: {info}");
    }
    
    // Если есть Google API ключ, сравним с реальным API
    if let Ok(_api_key) = env::var("GOOGLE_API_KEY") {
        println!("\n🔍 Сравнение с Google API:");
        
        let test_text = "Hello 世界! How are you today?";
        let our_ml = tokenizer::count_ml_calibrated_gemini_tokens(test_text).unwrap_or(0);
        
        println!("Тест: \"{test_text}\"");
        println!("Наш ML-токенизатор: {our_ml} токенов");
        
        // Здесь можно добавить запрос к Google API для сравнения
        println!("💡 Для полного сравнения запустите: cargo test test_ml_calibrated_tokenizer_accuracy --features=full");
    } else {
        println!("\n💡 Установите GOOGLE_API_KEY для сравнения с реальным API Google");
    }
    
    println!("\n🎯 Рекомендация: Используйте ML-калиброванный токенизатор для максимальной точности!");
}