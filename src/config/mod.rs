// src/config/mod.rs

pub mod app;
pub mod validation;
pub mod loader;

pub use app::{AppConfig, ServerConfig, KeyGroup};
pub use loader::{load_config, save_config, validate_config};
pub use validation::ConfigValidator;