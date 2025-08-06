//! Error handling module following industry standards
//! 
//! This module provides comprehensive error handling with:
//! - RFC 7807 Problem Details compliance
//! - Structured logging with correlation IDs
//! - Proper error categorization and HTTP status mapping
//! - Context preservation for debugging

pub mod types;
pub mod handlers;
pub mod context;

// pub use types::*;
// pub use handlers::*;
pub use context::{ErrorContext, set_error_context};
pub use crate::with_error_context;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{error, warn};
use uuid::Uuid;

/// Standard error response format following RFC 7807 Problem Details
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// A URI reference that identifies the problem type
    #[serde(rename = "type")]
    pub error_type: String,
    
    /// A short, human-readable summary of the problem type
    pub title: String,
    
    /// The HTTP status code
    pub status: u16,
    
    /// A human-readable explanation specific to this occurrence
    pub detail: String,
    
    /// A URI reference that identifies the specific occurrence
    pub instance: String,
    
    /// Request ID for tracing
    pub request_id: Option<String>,
    
    /// Additional error-specific properties
    #[serde(flatten)]
    pub extensions: serde_json::Map<String, serde_json::Value>,
}

/// Main application error type with comprehensive categorization
#[derive(Error, Debug)]
pub enum AppError {
    // Configuration errors
    #[error("Configuration validation failed: {message}")]
    ConfigValidation { message: String, field: Option<String> },
    
    #[error("Configuration file not found: {path}")]
    ConfigNotFound { path: String },
    
    #[error("Configuration parse error: {message}")]
    ConfigParse { message: String, line: Option<usize> },

    // Storage errors
    #[error("Redis connection failed: {message}")]
    RedisConnection { message: String },
    
    #[error("Redis operation failed: {operation} - {message}")]
    RedisOperation { operation: String, message: String },
    
    #[error("Storage persistence failed: {message}")]
    StoragePersistence { message: String },

    // HTTP and network errors
    #[error("HTTP client error: {message}")]
    HttpClient { message: String, status_code: Option<u16> },
    
    #[error("Upstream service unavailable: {service}")]
    UpstreamUnavailable { service: String },
    
    #[error("Request timeout after {timeout_secs}s")]
    RequestTimeout { timeout_secs: u64 },
    
    #[error("Invalid request: {message}")]
    InvalidRequest { message: String },

    // Authentication and authorization
    #[error("Authentication failed: {message}")]
    Authentication { message: String },
    
    #[error("Authorization failed: insufficient permissions")]
    Authorization,
    
    #[error("Invalid API key: {key_id}")]
    InvalidApiKey { key_id: String },
    
    #[error("API key quota exceeded: {key_id}")]
    ApiKeyQuotaExceeded { key_id: String },

    // Rate limiting and circuit breaking
    #[error("Rate limit exceeded: {limit} requests per {window}")]
    RateLimit { limit: u32, window: String },
    
    #[error("Circuit breaker open for service: {service}")]
    CircuitBreakerOpen { service: String },

    // Key management
    #[error("No healthy API keys available")]
    NoHealthyKeys,
    
    #[error("Key rotation failed: {message}")]
    KeyRotation { message: String },
    
    #[error("Key health check failed: {key_id} - {message}")]
    KeyHealthCheck { key_id: String, message: String },

    // Validation errors
    #[error("Validation failed: {field} - {message}")]
    Validation { field: String, message: String },
    
    #[error("Request body too large: {size} bytes (max: {max_size})")]
    RequestTooLarge { size: usize, max_size: usize },

    // System errors
    #[error("Internal server error: {message}")]
    Internal { message: String },
    
    #[error("Service unavailable: {message}")]
    ServiceUnavailable { message: String },
    
    #[error("Tokenizer initialization failed: {message}")]
    TokenizerInit { message: String },

    // External service errors
    #[error("Serialization error: {message}")]
    Serialization { message: String },
    
    #[error("IO operation failed: {operation} - {message}")]
    Io { operation: String, message: String },
}

impl AppError {
    /// Create a new configuration validation error
    pub fn config_validation(message: impl Into<String>, field: Option<impl Into<String>>) -> Self {
        Self::ConfigValidation {
            message: message.into(),
            field: field.map(Into::into),
        }
    }

    /// Create a new internal error with context
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    /// Create a new validation error
    pub fn validation(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Validation {
            field: field.into(),
            message: message.into(),
        }
    }

    /// Get the HTTP status code for this error
    pub fn status_code(&self) -> StatusCode {
        match self {
            // 400 Bad Request
            Self::ConfigParse { .. } | 
            Self::InvalidRequest { .. } | 
            Self::Validation { .. } | 
            Self::RequestTooLarge { .. } | 
            Self::Serialization { .. } => StatusCode::BAD_REQUEST,

            // 401 Unauthorized
            Self::Authentication { .. } | 
            Self::InvalidApiKey { .. } => StatusCode::UNAUTHORIZED,

            // 403 Forbidden
            Self::Authorization => StatusCode::FORBIDDEN,

            // 404 Not Found
            Self::ConfigNotFound { .. } => StatusCode::NOT_FOUND,

            // 408 Request Timeout
            Self::RequestTimeout { .. } => StatusCode::REQUEST_TIMEOUT,

            // 429 Too Many Requests
            Self::RateLimit { .. } | 
            Self::ApiKeyQuotaExceeded { .. } => StatusCode::TOO_MANY_REQUESTS,

            // 500 Internal Server Error
            Self::ConfigValidation { .. } | 
            Self::Internal { .. } | 
            Self::TokenizerInit { .. } | 
            Self::Io { .. } => StatusCode::INTERNAL_SERVER_ERROR,

            // 502 Bad Gateway
            Self::HttpClient { .. } | 
            Self::UpstreamUnavailable { .. } => StatusCode::BAD_GATEWAY,

            // 503 Service Unavailable
            Self::ServiceUnavailable { .. } | 
            Self::CircuitBreakerOpen { .. } | 
            Self::NoHealthyKeys | 
            Self::RedisConnection { .. } => StatusCode::SERVICE_UNAVAILABLE,

            // 504 Gateway Timeout
            Self::RedisOperation { .. } |
            Self::StoragePersistence { .. } |
            Self::KeyRotation { .. } |
            Self::KeyHealthCheck { .. } => StatusCode::GATEWAY_TIMEOUT,
                }
    }

    /// Get the error type URI for RFC 7807 compliance
    pub fn error_type(&self) -> &'static str {
        match self {
            Self::ConfigValidation { .. } | Self::ConfigNotFound { .. } | Self::ConfigParse { .. } => 
                "https://gemini-proxy.dev/errors/configuration",
            Self::RedisConnection { .. } | Self::RedisOperation { .. } | Self::StoragePersistence { .. } => 
                "https://gemini-proxy.dev/errors/storage",
            Self::HttpClient { .. } | Self::UpstreamUnavailable { .. } | Self::RequestTimeout { .. } => 
                "https://gemini-proxy.dev/errors/network",
            Self::Authentication { .. } | Self::Authorization | Self::InvalidApiKey { .. } => 
                "https://gemini-proxy.dev/errors/authentication",
            Self::RateLimit { .. } | Self::ApiKeyQuotaExceeded { .. } => 
                "https://gemini-proxy.dev/errors/rate-limit",
            Self::CircuitBreakerOpen { .. } => 
                "https://gemini-proxy.dev/errors/circuit-breaker",
            Self::NoHealthyKeys | Self::KeyRotation { .. } | Self::KeyHealthCheck { .. } => 
                "https://gemini-proxy.dev/errors/key-management",
            Self::Validation { .. } | Self::InvalidRequest { .. } | Self::RequestTooLarge { .. } => 
                "https://gemini-proxy.dev/errors/validation",
            _ => "https://gemini-proxy.dev/errors/internal",
        }
    }

    /// Get a human-readable title for the error
    pub fn title(&self) -> &'static str {
        match self {
            Self::ConfigValidation { .. } | Self::ConfigNotFound { .. } | Self::ConfigParse { .. } => 
                "Configuration Error",
            Self::RedisConnection { .. } | Self::RedisOperation { .. } | Self::StoragePersistence { .. } => 
                "Storage Error",
            Self::HttpClient { .. } | Self::UpstreamUnavailable { .. } | Self::RequestTimeout { .. } => 
                "Network Error",
            Self::Authentication { .. } | Self::Authorization | Self::InvalidApiKey { .. } => 
                "Authentication Error",
            Self::RateLimit { .. } | Self::ApiKeyQuotaExceeded { .. } => 
                "Rate Limit Exceeded",
            Self::CircuitBreakerOpen { .. } => 
                "Circuit Breaker Open",
            Self::NoHealthyKeys | Self::KeyRotation { .. } | Self::KeyHealthCheck { .. } => 
                "Key Management Error",
            Self::Validation { .. } | Self::InvalidRequest { .. } | Self::RequestTooLarge { .. } => 
                "Validation Error",
            _ => "Internal Server Error",
        }
    }

    /// Log the error with appropriate level
    pub fn log(&self, request_id: Option<&str>) {
        let request_id = request_id.unwrap_or("unknown");
        
        match self.status_code() {
            StatusCode::INTERNAL_SERVER_ERROR | 
            StatusCode::BAD_GATEWAY | 
            StatusCode::SERVICE_UNAVAILABLE | 
            StatusCode::GATEWAY_TIMEOUT => {
                error!(
                    error = %self,
                    request_id = request_id,
                    error_type = self.error_type(),
                    "Application error occurred"
                );
            }
            _ => {
                warn!(
                    error = %self,
                    request_id = request_id,
                    error_type = self.error_type(),
                    "Client error occurred"
                );
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let request_id = Uuid::new_v4().to_string();
        
        // Log the error
        self.log(Some(&request_id));

        let status = self.status_code();
        let error_response = ErrorResponse {
            error_type: self.error_type().to_string(),
            title: self.title().to_string(),
            status: status.as_u16(),
            detail: self.to_string(),
            instance: format!("/errors/{}", request_id),
            request_id: Some(request_id),
            extensions: serde_json::Map::new(),
        };

        (status, Json(error_response)).into_response()
    }
}

/// Result type alias for the application
pub type Result<T, E = AppError> = std::result::Result<T, E>;