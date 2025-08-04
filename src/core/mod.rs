// src/core/mod.rs

pub mod key_rotation;
pub mod health_check;

pub use key_rotation::{KeyRotationStrategy, RoundRobinStrategy, KeySelector};
pub use health_check::HealthChecker;