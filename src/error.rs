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
     #[allow(dead_code)] // Keep for now, as it might be used indirectly or planned
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

     #[allow(dead_code)] // Keep for now, as it might be used indirectly or planned
     #[error("Upstream service error: {status} - {body}")]
     UpstreamServiceError { status: StatusCode, body: String },

     // Removed #[allow(dead_code)] as this variant is used in handler.rs
     #[error("Request body processing error: {0}")]
     RequestBodyError(String), // More specific than generic Reqwest error

     #[error("Response body processing error: {0}")]
     ResponseBodyError(String), // More specific than generic Reqwest error

     #[allow(dead_code)] // Keep for now, as it might be used indirectly or planned
     #[error("Invalid API key provided by client")]
     InvalidClientApiKey, // If you add client-side key validation

     #[error("Internal server error: {0}")]
     Internal(String), // Catch-all for unexpected errors
 
     #[error("Proxy configuration error: {0}")]
     ProxyConfigError(String),
 
     #[error("HTTP client build error: {0}")]
     HttpClientBuildError(reqwest::Error),
 }

 // Implement IntoResponse for AppError to automatically convert errors into HTTP responses
 impl IntoResponse for AppError {
     fn into_response(self) -> Response {
         let (status, error_message) = match self {
             AppError::Config(msg) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Configuration error: {}", msg)),
             // Simulate Reqwest error for testing response generation
             AppError::Reqwest(e) if e.is_timeout() => (StatusCode::BAD_GATEWAY, format!("Upstream request failed: operation timed out")), // Example specific handling
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
             AppError::ProxyConfigError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Proxy configuration error: {}", msg)),
             AppError::HttpClientBuildError(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("HTTP client build error: {}", e)),
         };

         let body = Json(json!({
             "error": error_message,
         }));

         (status, body).into_response()
     }
 }

 // Optional: Define a type alias for Result using the AppError
 pub type Result<T> = std::result::Result<T, AppError>;


 #[cfg(test)]
 mod tests {
     use super::*;
     use axum::body::to_bytes; // Use axum::body::to_bytes
    // Removed unused import: http_body_util::BodyExt;
     use serde_json::Value;

     async fn check_response(error: AppError, expected_status: StatusCode, expected_substring: &str) {
         let response = error.into_response();
         assert_eq!(response.status(), expected_status, "Status code mismatch");

         let body = response.into_body();
         let bytes = to_bytes(body, usize::MAX).await.expect("Failed to read response body"); // Use axum::body::to_bytes
         let body_json: Value = serde_json::from_slice(&bytes).expect("Response body is not valid JSON");

         let error_msg = body_json["error"].as_str().expect("JSON 'error' field is not a string");
         assert!(
             error_msg.contains(expected_substring),
             "Expected error message '{}' to contain '{}'", error_msg, expected_substring
         );
     }

     #[tokio::test]
     async fn test_into_response_config() {
         check_response(AppError::Config("Test config issue".to_string()), StatusCode::INTERNAL_SERVER_ERROR, "Configuration error: Test config issue").await;
     }

     #[tokio::test]
     async fn test_into_response_io() {
         let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "File not found");
         check_response(AppError::Io(io_error), StatusCode::INTERNAL_SERVER_ERROR, "IO error: File not found").await;
     }

      #[tokio::test]
     async fn test_into_response_yaml() {
         // Simulate a serde_yaml error (actual error creation is complex, use string representation)
         // Specify the target type as () since we only care about the error
         let yaml_error: serde_yaml::Error = serde_yaml::from_str::<()>("invalid: yaml:").unwrap_err();
         check_response(AppError::YamlParsing(yaml_error), StatusCode::INTERNAL_SERVER_ERROR, "Configuration parsing error:").await;
     }

     #[tokio::test]
     async fn test_into_response_url_parse() {
         let url_error = url::Url::parse("::invalid url").unwrap_err();
         check_response(AppError::UrlParse(url_error), StatusCode::INTERNAL_SERVER_ERROR, "URL configuration error:").await;
     }

     #[tokio::test]
     async fn test_into_response_no_keys() {
         check_response(AppError::NoAvailableKeys, StatusCode::SERVICE_UNAVAILABLE, "No available API keys").await;
     }

     #[tokio::test]
     async fn test_into_response_upstream_service_error() {
         check_response(
             AppError::UpstreamServiceError { status: StatusCode::BAD_GATEWAY, body: "Upstream failed".to_string() },
             StatusCode::BAD_GATEWAY,
             "Upstream error: Upstream failed"
         ).await;
     }

     #[tokio::test]
     async fn test_into_response_request_body_error() {
         check_response(AppError::RequestBodyError("Bad body".to_string()), StatusCode::BAD_REQUEST, "Request body error: Bad body").await;
     }

     #[tokio::test]
     async fn test_into_response_response_body_error() {
         check_response(AppError::ResponseBodyError("Bad response body".to_string()), StatusCode::BAD_GATEWAY, "Response body error: Bad response body").await;
     }

      #[tokio::test]
     async fn test_into_response_invalid_client_key() {
         check_response(AppError::InvalidClientApiKey, StatusCode::UNAUTHORIZED, "Invalid API key provided").await;
     }

     #[tokio::test]
     async fn test_into_response_internal() {
         check_response(AppError::Internal("Something went wrong".to_string()), StatusCode::INTERNAL_SERVER_ERROR, "Internal server error: Something went wrong").await;
     }

     // Note: Testing AppError::Reqwest directly is tricky as reqwest::Error creation isn't trivial.
     // The current implementation covers the generic conversion logic. More specific tests
     // might require constructing reqwest::Error instances carefully, which is complex.
 }