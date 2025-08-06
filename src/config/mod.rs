// src/config/mod.rs

pub mod app;
pub mod loader;
pub mod validation;

pub use app::{AppConfig, KeyGroup, ServerConfig};
pub use loader::{load_config, save_config, validate_config};
pub use validation::ConfigValidator;
