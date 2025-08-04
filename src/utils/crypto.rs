// src/utils/crypto.rs

use secrecy::{ExposeSecret, Secret};

/// A wrapper around Secret<String> with additional utility methods
pub struct SecureString(Secret<String>);

impl SecureString {
    pub fn new(value: String) -> Self {
        Self(Secret::new(value))
    }
    
    pub fn expose_secret(&self) -> &str {
        self.0.expose_secret()
    }
    
    pub fn preview(&self) -> String {
        let value = self.0.expose_secret();
        if value.len() > 8 {
            format!("{}...{}", &value[..4], &value[value.len() - 4..])
        } else {
            value.to_string()
        }
    }
}

impl From<String> for SecureString {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for SecureString {
    fn from(value: &str) -> Self {
        Self::new(value.to_string())
    }
}