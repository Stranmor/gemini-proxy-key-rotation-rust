// src/lib.rs

// Declare modules that constitute the library's public API or internal structure
pub mod config;
pub mod error;
pub mod handler;
pub mod key_manager;
pub mod proxy;
pub mod state;

// Re-export key types for easier use by the binary or tests
pub use config::AppConfig;
pub use error::{AppError, Result};
pub use state::AppState;
// Add other re-exports if needed
