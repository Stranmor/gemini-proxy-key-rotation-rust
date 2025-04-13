// src/error.rs
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

/// Represents the possible errors that can occur within the application.
///
/// Implements `IntoResponse` to automatically convert errors into appropriate
/// HTTP error responses with JSON bodies.
#[derive(Error, Debug)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    #[allow(dead_code)]
    Config(String),

    #[error("Reqwest HTTP client error: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("YAML parsing error: {0}")]
    YamlParsing(#[from] serde_yaml::Error),

    #[error("URL parsing error: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("No available API keys")]
    NoAvailableKeys,

    #[allow(dead_code)] // Temporarily allow unused variant
    #[error("Upstream service error: {status} - {body}")]
    UpstreamServiceError { status: StatusCode, body: String },

    #[allow(dead_code)] // Temporarily allow unused variant
    #[error("Request body processing error: {0}")]
    RequestBodyError(String), // More specific than generic Reqwest error

    #[error("Response body processing error: {0}")]
    ResponseBodyError(String), // More specific than generic Reqwest error

    #[allow(dead_code)] // Temporarily allow unused variant
    #[error("Invalid API key provided by client")]
    InvalidClientApiKey, // If you add client-side key validation

    #[error("Internal server error: {0}")]
    Internal(String), // Catch-all for unexpected errors

    // Add more specific error types as needed
}

// Implement IntoResponse for AppError to automatically convert errors into HTTP responses
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            AppError::Config(msg) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Configuration error: {}", msg)),
            AppError::Reqwest(e) => (StatusCode::BAD_GATEWAY, format!("Upstream request failed: {}", e)),
            AppError::Io(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("IO error: {}", e)),
            AppError::YamlParsing(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Configuration parsing error: {}", e)),
            AppError::UrlParse(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("URL configuration error: {}", e)),
            AppError::NoAvailableKeys => (StatusCode::SERVICE_UNAVAILABLE, "No available API keys to process the request".to_string()),
            AppError::UpstreamServiceError { status, body } => (status, format!("Upstream error: {}", body)),
            AppError::RequestBodyError(msg) => (StatusCode::BAD_REQUEST, format!("Request body error: {}", msg)),
            AppError::ResponseBodyError(msg) => (StatusCode::BAD_GATEWAY, format!("Response body error: {}", msg)),
            AppError::InvalidClientApiKey => (StatusCode::UNAUTHORIZED, "Invalid API key provided".to_string()),
            AppError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Internal server error: {}", msg)),
            // Ensure all variants are handled
        };

        let body = Json(json!({
            "error": error_message,
        }));

        (status, body).into_response()
    }
}

// Optional: Define a type alias for Result using the AppError
pub type Result<T> = std::result::Result<T, AppError>;