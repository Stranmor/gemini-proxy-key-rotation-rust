// examples/circuit_breaker_demo.rs
// Демонстрация работы circuit breaker

use gemini_proxy_key_rotation_rust::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitBreakerError};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() {
    // Настройка логирования
    tracing_subscriber::fmt::init();

    // Создание circuit breaker с агрессивными настройками для демо
    let config = CircuitBreakerConfig {
        failure_threshold: 3,
        recovery_timeout: Duration::from_secs(2),
        success_threshold: 2,
    };
    
    let cb = Arc::new(CircuitBreaker::new("demo-service".to_string(), config));
    
    println!("🔧 Circuit Breaker Demo");
    println!("📊 Config: 3 failures → open, 2s recovery, 2 successes → close\n");

    // Симуляция нормальной работы
    println!("✅ Phase 1: Normal operation");
    for i in 1..=3 {
        let result = cb.call(|| async {
            println!("  📤 Request {i} - Success");
            Ok::<(), &str>(())
        }).await;
        
        match result {
            Ok(_) => println!("  ✅ Request {i} completed successfully"),
            Err(e) => println!("  ❌ Request {i} failed: {:?}", e),
        }
    }
    
    println!("\n🔥 Phase 2: Simulating failures");
    // Симуляция ошибок для размыкания circuit breaker
    for i in 1..=4 {
        let result = cb.call(|| async {
            println!("  📤 Request {i} - Simulating failure");
            Err::<(), &str>("Service unavailable")
        }).await;
        
        match result {
            Ok(_) => println!("  ✅ Request {i} completed successfully"),
            Err(CircuitBreakerError::CircuitOpen) => {
                println!("  🚫 Request {i} - Circuit breaker is OPEN (failing fast)");
            }
            Err(CircuitBreakerError::OperationFailed(_)) => {
                println!("  ❌ Request {i} - Operation failed");
            }
        }
        
        // Показать текущее состояние
        let state = cb.get_state().await;
        println!("  📊 Circuit state: {:?}", state);
        
        sleep(Duration::from_millis(100)).await;
    }
    
    println!("\n⏳ Phase 3: Waiting for recovery timeout...");
    sleep(Duration::from_secs(3)).await;
    
    println!("🔄 Phase 4: Testing recovery (half-open state)");
    // Первый запрос после таймаута должен перевести в half-open
    let result = cb.call(|| async {
        println!("  📤 Recovery test request - Success");
        Ok::<(), &str>(())
    }).await;
    
    match result {
        Ok(_) => {
            println!("  ✅ Recovery test successful");
            println!("  📊 Circuit state: {:?}", cb.get_state().await);
        }
        Err(e) => println!("  ❌ Recovery test failed: {:?}", e),
    }
    
    // Еще один успешный запрос должен закрыть circuit
    let result = cb.call(|| async {
        println!("  📤 Second recovery request - Success");
        Ok::<(), &str>(())
    }).await;
    
    match result {
        Ok(_) => {
            println!("  ✅ Second recovery test successful");
            println!("  📊 Circuit state: {:?}", cb.get_state().await);
        }
        Err(e) => println!("  ❌ Second recovery test failed: {:?}", e),
    }
    
    println!("\n📈 Final statistics:");
    let stats = cb.get_stats();
    println!("  Total requests: {}", stats.total_requests);
    println!("  Total failures: {}", stats.total_failures);
    println!("  Success rate: {:.1}%", 
        (stats.total_requests - stats.total_failures) as f64 / stats.total_requests as f64 * 100.0);
    
    println!("\n🎉 Demo completed!");
}