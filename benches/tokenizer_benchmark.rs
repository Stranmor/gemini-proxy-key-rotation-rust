use criterion::{black_box, criterion_group, criterion_main, Criterion};
use gemini_proxy::tokenizer::{count_tokens, initialize_tokenizer};
use std::time::Duration;
use tokio::runtime::Runtime;

// Тестовые тексты разной длины
const SHORT_TEXT: &str = "Hello, how are you today?";
const MEDIUM_TEXT: &str = r#"
The quick brown fox jumps over the lazy dog. This is a sample text that contains
multiple sentences and should give us a good idea of tokenization performance for
medium-length content. We want to test how fast our tokenizer can process this
kind of typical user input that might be sent to an AI model.
"#;
const LONG_TEXT: &str = r#"
In the realm of artificial intelligence and natural language processing, tokenization
represents a fundamental preprocessing step that transforms raw text into a format
suitable for machine learning models. The process involves breaking down text into
smaller units called tokens, which can be words, subwords, or even individual characters,
depending on the tokenization strategy employed. Modern tokenizers like GPT-4's cl100k_base
or Claude's tokenizer use sophisticated algorithms such as Byte Pair Encoding (BPE) to
create a vocabulary that balances between having enough tokens to represent the language
efficiently while keeping the vocabulary size manageable. This balance is crucial because
it directly impacts both the model's performance and computational requirements. When
implementing a proxy service that needs to validate token counts before forwarding
requests to upstream AI services, the choice between local tokenization and API-based
counting becomes critical for both performance and accuracy. Local tokenization offers
speed and reliability but may sacrifice some accuracy, while API-based counting provides
perfect accuracy at the cost of latency and potential service dependencies.
"#;

fn bench_local_tokenization(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    // Инициализируем токенизатор один раз
    rt.block_on(async {
        // Используем современный токенизатор
        if (initialize_tokenizer("gpt2").await).is_err() {
            // Fallback для тестов без HF_TOKEN
            println!("Warning: Using minimal tokenizer for benchmark");
        }
    });

    let mut group = c.benchmark_group("local_tokenization");

    group.bench_function("short_text", |b| {
        b.iter(|| {
            let result = count_tokens(black_box(SHORT_TEXT));
            black_box(result)
        })
    });

    group.bench_function("medium_text", |b| {
        b.iter(|| {
            let result = count_tokens(black_box(MEDIUM_TEXT));
            black_box(result)
        })
    });

    group.bench_function("long_text", |b| {
        b.iter(|| {
            let result = count_tokens(black_box(LONG_TEXT));
            black_box(result)
        })
    });

    group.finish();
}

fn bench_api_simulation(c: &mut Criterion) {
    let _rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("api_simulation");
    // Симулируем типичную латентность API
    group.measurement_time(Duration::from_secs(10));

    group.bench_function("api_call_simulation", |b| {
        b.iter_batched(
            || (),
            |_| async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                black_box(42)
            },
            criterion::BatchSize::SmallInput,
        )
    });

    group.finish();
}

criterion_group!(benches, bench_local_tokenization, bench_api_simulation);
criterion_main!(benches);
