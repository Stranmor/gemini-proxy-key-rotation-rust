//! Error handling utilities and middleware

use super::{AppError, ErrorResponse, Result};
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use tracing::{error, info_span, Instrument};
use uuid::Uuid;

/// Global error handler for unhandled errors
pub async fn global_error_handler(err: Box<dyn std::error::Error + Send + Sync>) -> Response {
    let request_id = Uuid::new_v4().to_string();
    
    error!(
        error = %err,
        request_id = %request_id,
        "Unhandled error occurred"
    );

    let error_response = ErrorResponse {
        error_type: "https://gemini-proxy.dev/errors/internal".to_string(),
        title: "Internal Server Error".to_string(),
        status: 500,
        detail: "An unexpected error occurred".to_string(),
        instance: format!("/errors/{}", request_id),
        request_id: Some(request_id),
        extensions: serde_json::Map::new(),
    };

    (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response)).into_response()
}

/// Middleware to catch panics and convert them to proper error responses
pub async fn panic_handler(req: Request, next: Next) -> Response {
    let request_id = Uuid::new_v4().to_string();
    let span = info_span!("request", request_id = %request_id);

    async move {
        // Set panic hook to capture panic info
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            error!(
                request_id = %request_id,
                panic_info = %panic_info,
                "Panic occurred during request processing"
            );
        }));

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tokio::runtime::Handle::current().block_on(next.run(req))
        }));

        // Restore original panic hook
        std::panic::set_hook(default_hook);

        match result {
            Ok(response) => response,
            Err(_) => {
                let error_response = ErrorResponse {
                    error_type: "https://gemini-proxy.dev/errors/panic".to_string(),
                    title: "Internal Server Error".to_string(),
                    status: 500,
                    detail: "A critical error occurred while processing the request".to_string(),
                    instance: format!("/errors/{}", request_id),
                    request_id: Some(request_id),
                    extensions: serde_json::Map::new(),
                };

                (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response)).into_response()
            }
        }
    }
    .instrument(span)
    .await
}

/// Helper function to create a standardized error response
pub fn create_error_response(
    error_type: &str,
    title: &str,
    status: StatusCode,
    detail: &str,
    request_id: Option<String>,
) -> ErrorResponse {
    let request_id = request_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    
    ErrorResponse {
        error_type: error_type.to_string(),
        title: title.to_string(),
        status: status.as_u16(),
        detail: detail.to_string(),
        instance: format!("/errors/{}", request_id),
        request_id: Some(request_id),
        extensions: serde_json::Map::new(),
    }
}

/// Trait for adding error context to results
pub trait ErrorContext<T> {
    fn with_error_context(self, context: &str) -> Result<T>;
    fn with_field_context(self, field: &str, context: &str) -> Result<T>;
}

impl<T, E> ErrorContext<T> for std::result::Result<T, E>
where
    E: Into<AppError>,
{
    fn with_error_context(self, context: &str) -> Result<T> {
        self.map_err(|e| {
            let app_error = e.into();
            match app_error {
                AppError::Internal { message } => AppError::Internal {
                    message: format!("{}: {}", context, message),
                },
                other => other,
            }
        })
    }

    fn with_field_context(self, field: &str, context: &str) -> Result<T> {
        self.map_err(|e| {
            let app_error = e.into();
            match app_error {
                AppError::Validation { field: _, message } => AppError::Validation {
                    field: field.to_string(),
                    message: format!("{}: {}", context, message),
                },
                other => other,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use serde_json::Value;

    #[tokio::test]
    async fn test_create_error_response() {
        let response = create_error_response(
            "test_error",
            "Test Error",
            StatusCode::BAD_REQUEST,
            "This is a test error",
            None,
        );

        assert_eq!(response.error_type, "test_error");
        assert_eq!(response.title, "Test Error");
        assert_eq!(response.status, 400);
        assert_eq!(response.detail, "This is a test error");
        assert!(response.request_id.is_some());
    }

    #[tokio::test]
    async fn test_error_context() {
        let result: std::result::Result<(), std::io::Error> = 
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "file not found"));
        
        let app_result = result.with_error_context("Failed to read config");
        assert!(app_result.is_err());
        
        let error = app_result.unwrap_err();
        assert!(error.to_string().contains("Failed to read config"));
    }
}