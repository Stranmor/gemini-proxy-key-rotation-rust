// src/tokenizer.rs

pub mod gemini_ml_calibrated;
pub mod smart_parallel;

// --- Public API ---
// Основная точка входа для обработки текста
pub use smart_parallel::process_text_smart;
// Результат обработки, который возвращается пользователю
pub use smart_parallel::ProcessingResult;
// Конфигурация, которую пользователь может захотеть настроить
pub use smart_parallel::SmartParallelConfig;

// --- Crate-internal API ---
// Скрываем детали реализации от внешнего мира
pub(crate) use gemini_ml_calibrated::GeminiMLCalibratedTokenizer;
pub(crate) use smart_parallel::SmartParallelTokenizer;
