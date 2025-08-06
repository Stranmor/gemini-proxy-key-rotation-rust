use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use gemini_proxy::{config::AppConfig, create_router, AppState};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use std::{sync::Arc, time::Duration};
use tokio::runtime::Runtime;
use tower::ServiceExt;

/// Benchmark configuration
struct BenchConfig {
    name: &'static str,
    key_count: usize,
    concurrent_requests: usize,
}

const BENCH_CONFIGS: &[BenchConfig] = &[
    BenchConfig {
        name: "single_key",
        key_count: 1,
        concurrent_requests: 1,
    },
    BenchConfig {
        name: "multiple_keys",
        key_count: 5,
        concurrent_requests: 1,
    },
    BenchConfig {
        name: "concurrent_single_key",
        key_count: 1,
        concurrent_requests: 10,
    },
    BenchConfig {
        name: "concurrent_multiple_keys",
        key_count: 5,
        concurrent_requests: 10,
    },
];

/// Create a test configuration with specified number of keys
fn create_test_config(key_count: usize) -> AppConfig {
    let mut config = AppConfig::default();
    config.server.port = 8080;
    
    // Create API keys
    let api_keys: Vec<String> = (1..=key_count)
        .map(|i| format!("test-key-{}", i))
        .collect();
    
    config.groups = vec![gemini_proxy::config::KeyGroup {
        name: "benchmark".to_string(),
        api_keys,
        target_url: "https://generativelanguage.googleapis.com/v1beta/openai/".to_string(),
        proxy_url: None,
    }];
    
    config
}

/// Create a test request
fn create_test_request() -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri("/health")
        .body(Body::empty())
        .unwrap()
}

/// Benchmark key rotation performance
fn bench_key_rotation(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let mut group = c.benchmark_group("key_rotation");
    group.measurement_time(Duration::from_secs(10));
    
    for config in BENCH_CONFIGS {
        group.bench_with_input(
            BenchmarkId::new("rotation_speed", config.name),
            config,
            |b, bench_config| {
                let app_config = create_test_config(bench_config.key_count);
                
                b.to_async(&rt).iter(|| async {
                    let (state, _rx) = AppState::new(&app_config, &std::path::PathBuf::from("test"))
                        .await
                        .unwrap();
                    
                    // Simulate key rotation
                    for _ in 0..10 {
                        let _key = state.key_manager.get_next_key().await;
                        black_box(_key);
                    }
                });
            },
        );
    }
    
    group.finish();
}

/// Benchmark request handling performance
fn bench_request_handling(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let mut group = c.benchmark_group("request_handling");
    group.measurement_time(Duration::from_secs(10));
    
    for config in BENCH_CONFIGS {
        group.bench_with_input(
            BenchmarkId::new("health_check", config.name),
            config,
            |b, bench_config| {
                let app_config = create_test_config(bench_config.key_count);
                
                b.to_async(&rt).iter(|| async {
                    let (state, _rx) = AppState::new(&app_config, &std::path::PathBuf::from("test"))
                        .await
                        .unwrap();
                    
                    let app = create_router(Arc::new(state));
                    let request = create_test_request();
                    
                    let response = app.oneshot(request).await.unwrap();
                    black_box(response.status());
                });
            },
        );
    }
    
    group.finish();
}

/// Benchmark concurrent request handling
fn bench_concurrent_requests(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let mut group = c.benchmark_group("concurrent_requests");
    group.measurement_time(Duration::from_secs(15));
    
    for config in BENCH_CONFIGS {
        if config.concurrent_requests == 1 {
            continue; // Skip single request benchmarks
        }
        
        group.bench_with_input(
            BenchmarkId::new("concurrent_health", config.name),
            config,
            |b, bench_config| {
                let app_config = create_test_config(bench_config.key_count);
                
                b.to_async(&rt).iter(|| async {
                    let (state, _rx) = AppState::new(&app_config, &std::path::PathBuf::from("test"))
                        .await
                        .unwrap();
                    
                    let app = Arc::new(create_router(Arc::new(state)));
                    
                    // Create concurrent requests
                    let mut handles = Vec::new();
                    
                    for _ in 0..bench_config.concurrent_requests {
                        let app_clone = app.clone();
                        let handle = tokio::spawn(async move {
                            let request = create_test_request();
                            let response = app_clone.clone().oneshot(request).await.unwrap();
                            black_box(response.status());
                        });
                        handles.push(handle);
                    }
                    
                    // Wait for all requests to complete
                    for handle in handles {
                        handle.await.unwrap();
                    }
                });
            },
        );
    }
    
    group.finish();
}

/// Benchmark memory allocation patterns
fn bench_memory_usage(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let mut group = c.benchmark_group("memory_usage");
    group.measurement_time(Duration::from_secs(10));
    
    group.bench_function("app_state_creation", |b| {
        let app_config = create_test_config(5);
        
        b.to_async(&rt).iter(|| async {
            let (state, _rx) = AppState::new(&app_config, &std::path::PathBuf::from("test"))
                .await
                .unwrap();
            black_box(state);
        });
    });
    
    group.bench_function("router_creation", |b| {
        let app_config = create_test_config(5);
        
        b.to_async(&rt).iter(|| async {
            let (state, _rx) = AppState::new(&app_config, &std::path::PathBuf::from("test"))
                .await
                .unwrap();
            
            let router = create_router(Arc::new(state));
            black_box(router);
        });
    });
    
    group.finish();
}

/// Benchmark configuration loading
fn bench_config_loading(c: &mut Criterion) {
    let mut group = c.benchmark_group("config_loading");
    
    group.bench_function("config_creation", |b| {
        b.iter(|| {
            let config = create_test_config(black_box(5));
            black_box(config);
        });
    });
    
    group.bench_function("config_validation", |b| {
        b.iter(|| {
            let config = create_test_config(5);
            let _validated = gemini_proxy::config::validate_config(&config);
            black_box(_validated);
        });
    });
    
    group.finish();
}

criterion_group!(
    benches,
    bench_key_rotation,
    bench_request_handling,
    bench_concurrent_requests,
    bench_memory_usage,
    bench_config_loading
);

criterion_main!(benches);