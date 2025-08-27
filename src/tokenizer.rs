// src/tokenizer.rs

pub mod gemini_ml_calibrated;
pub mod smart_parallel;

// --- Public API ---
// Main entry point for text processing
pub use smart_parallel::process_text_smart;
// Processing result returned to the user
pub use smart_parallel::ProcessingResult;
// Configuration that the user may want to customize
pub use smart_parallel::SmartParallelConfig;

// --- Crate-internal API ---
// Hide implementation details from the outside world
pub(crate) use gemini_ml_calibrated::GeminiMLCalibratedTokenizer;
pub(crate) use smart_parallel::SmartParallelTokenizer;
