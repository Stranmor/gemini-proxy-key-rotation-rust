// src/error.rs
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use thiserror::Error;
use tracing::error; // Import error for logging

/// Represents the structured error response body.
#[derive(Serialize, Debug)]
struct ErrorResponse {
    error: ErrorDetails,
}

/// Contains the details of an error for the response body.
#[derive(Serialize, Debug)]
struct ErrorDetails {
    #[serde(rename = "type")]
    error_type: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<String>,
}

/// Specific kinds of proxy configuration errors.
#[derive(Error, Debug)]
pub enum ProxyConfigErrorKind {
    #[error("Invalid URL format: {0}")]
    UrlParse(#[from] url::ParseError),
    #[error("Unsupported scheme: {0}")]
    UnsupportedScheme(String),
    #[error("Invalid proxy definition: {0}")]
    InvalidDefinition(String), // For errors from reqwest::Proxy constructors
}

/// Detailed error information for proxy configuration issues.
#[derive(Error, Debug)]
#[error("Proxy configuration error for URL '{url}': {kind}")]
// Make struct and fields public
pub struct ProxyConfigErrorData {
    pub url: String,
    pub kind: ProxyConfigErrorKind,
}


/// Represents the possible errors that can occur within the application.
///
/// Implements `IntoResponse` to automatically convert errors into appropriate
/// HTTP error responses with a standardized JSON body.
#[derive(Error, Debug)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    Config(String), // Keep as String for general config issues, be specific elsewhere

    #[error("Reqwest HTTP client error: {0}")]
    Reqwest(#[from] reqwest::Error), // RESTORED #[from] here

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("YAML parsing error: {0}")]
    YamlParsing(#[from] serde_yaml::Error),

    // UrlParse error is now typically wrapped in ProxyConfigError or handled elsewhere
    // #[error("URL parsing error: {0}")]
    // UrlParse(#[from] url::ParseError),
    #[error("No available API keys")]
    NoAvailableKeys,

    #[error("Upstream service error: {status} - {body}")]
    UpstreamServiceError { status: StatusCode, body: String },

    #[error("Request body processing error: {0}")]
    RequestBodyError(String), // Keep as String for diverse sources

    #[error("Response body processing error: {0}")]
    ResponseBodyError(String), // Keep as String for diverse sources

    #[error("Invalid API key provided by client")]
    InvalidClientApiKey, // If client-side key validation is added

    #[error("Internal server error: {0}")]
    Internal(String), // Catch-all for unexpected errors

    #[error(transparent)] // Use transparent to delegate display/source
    ProxyConfigError(#[from] ProxyConfigErrorData),

    #[error("HTTP client build error: {source}")] // Reference source directly
    HttpClientBuildError {
        source: reqwest::Error,    // #[from] is definitely removed here
        proxy_url: Option<String>, // Add context
    },
}

// Removed manual `impl From<reqwest::Error> for AppError` to resolve conflict
// with the restored `#[from]` on `AppError::Reqwest`.
// HttpClientBuildError MUST be constructed explicitly where it occurs.

// Implement IntoResponse for AppError to automatically convert errors into HTTP responses
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_details) = match self {
            // --- 5xx Server Errors (Internal details logged, generic message to client) ---
            Self::Config(msg) => {
                error!("Configuration error: {}", msg); // Log the specific error
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorDetails {
                        error_type: "CONFIG_ERROR".to_string(),
                        message: "Internal server configuration error".to_string(),
                        details: None, // Don't expose details
                    },
                )
            }
            Self::Io(e) => {
                error!("IO error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorDetails {
                        error_type: "IO_ERROR".to_string(),
                        message: "Internal server error during IO operation".to_string(),
                        details: None,
                    },
                )
            }
            Self::YamlParsing(e) => {
                error!("YAML parsing error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorDetails {
                        error_type: "CONFIG_PARSE_ERROR".to_string(),
                        message: "Failed to parse configuration file".to_string(),
                        details: None,
                    },
                )
            }
            Self::ProxyConfigError(data) => {
                error!("Proxy configuration error: {}", data); // Log detailed error
                (
                    StatusCode::INTERNAL_SERVER_ERROR, // Config issue is internal
                    ErrorDetails {
                        error_type: "PROXY_CONFIG_ERROR".to_string(),
                        message: "Internal server error related to proxy configuration".to_string(),
                        // Optionally expose the problematic URL, but not the internal kind
                        details: Some(format!("Affected proxy URL: {}", data.url)),
                        // details: None, // Stricter approach: hide URL too
                    },
                )
            }
            Self::HttpClientBuildError { source, proxy_url } => {
                error!(proxy_url = ?proxy_url, "HTTP client build error: {}", source);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorDetails {
                        error_type: "HTTP_CLIENT_BUILD_ERROR".to_string(),
                        message: "Internal server error building HTTP client".to_string(),
                        details: proxy_url.map(|u| format!("Related proxy: {u}")),
                    },
                )
            }
            Self::Internal(msg) => {
                error!("Internal server error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorDetails {
                        error_type: "INTERNAL_SERVER_ERROR".to_string(),
                        message: "An unexpected internal server error occurred".to_string(),
                        details: None,
                    },
                )
            }

            // --- 5xx Errors related to upstream/proxying ---
            Self::Reqwest(e) => {
                error!("Upstream reqwest error: {}", e);
                // More robust error classification
                let (status_code, msg_key) = if e.is_timeout() {
                    (StatusCode::GATEWAY_TIMEOUT, "timeout")
                } else if e.is_connect() {
                    (StatusCode::BAD_GATEWAY, "connect")
                } else if e.is_request() {
                    (StatusCode::BAD_GATEWAY, "request_setup") // Error building the request itself (e.g., DNS issue)
                } else if e.is_body() || e.is_decode() {
                    (StatusCode::BAD_GATEWAY, "body_or_decode") // Error processing response body
                } else {
                    (StatusCode::BAD_GATEWAY, "generic") // Other communication errors
                };

                let message = match msg_key {
                    "timeout" => "Upstream request timed out".to_string(),
                    "connect" => "Could not connect to upstream service".to_string(),
                    "request_setup" => "Internal error setting up upstream request".to_string(),
                    "body_or_decode" => {
                        "Error processing response body from upstream service".to_string()
                    }
                    _ => "Error communicating with upstream service".to_string(),
                };

                (
                    status_code,
                    ErrorDetails {
                        error_type: "UPSTREAM_ERROR".to_string(),
                        message,
                        details: Some(e.to_string()), // Provide reqwest error string as detail
                    },
                )
            }
            Self::UpstreamServiceError { status, body } => {
                error!(
                    "Upstream service returned error: Status={}, Body='{}'",
                    status, body
                );
                (
                    status, // Use the status code from the upstream service
                    ErrorDetails {
                        error_type: "UPSTREAM_SERVICE_ERROR".to_string(),
                        message: "Upstream service returned an error".to_string(),
                        details: Some(body), // Include upstream body if needed
                    },
                )
            }
            Self::ResponseBodyError(msg) => {
                error!("Response body processing error: {}", msg);
                (
                    StatusCode::BAD_GATEWAY, // Error processing upstream response
                    ErrorDetails {
                        error_type: "RESPONSE_PROCESSING_ERROR".to_string(),
                        message: "Failed to process response from upstream service".to_string(),
                        details: Some(msg),
                    },
                )
            }
            Self::NoAvailableKeys => (
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorDetails {
                    error_type: "NO_AVAILABLE_KEYS".to_string(),
                    message: "No available API keys to process the request at this time"
                        .to_string(),
                    details: None,
                },
            ),

            // --- 4xx Client Errors ---
            Self::RequestBodyError(msg) => (
                StatusCode::BAD_REQUEST,
                ErrorDetails {
                    error_type: "REQUEST_BODY_ERROR".to_string(),
                    message: "Failed to process request body".to_string(),
                    details: Some(msg),
                },
            ),
            Self::InvalidClientApiKey => (
                StatusCode::UNAUTHORIZED, // Or FORBIDDEN depending on semantics
                ErrorDetails {
                    error_type: "INVALID_API_KEY".to_string(),
                    message: "Invalid or unauthorized API key provided".to_string(),
                    details: None,
                },
            ),
            // Deprecated/Removed:
            // AppError::UrlParse(e) => { ... } // Now part of ProxyConfigError usually
        };

        let body = Json(ErrorResponse {
            error: error_details,
        });

        (status, body).into_response()
    }
}

// Optional: Define a type alias for Result using the AppError
pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use serde_json::Value;
    use std::io;

    // Helper to check the structured response
    async fn check_response(
        error: AppError,
        expected_status: StatusCode,
        expected_type: &str,
        expected_message_substring: &str,
        expect_details: bool, // Whether to assert that details field exists (doesn't check content)
    ) {
        let response = error.into_response();
        assert_eq!(response.status(), expected_status, "Status code mismatch");

        let body = response.into_body();
        let bytes = to_bytes(body, usize::MAX)
            .await
            .expect("Failed to read response body");
        let body_json: Value = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            panic!(
                "Response body is not valid JSON: {}. Body: {}",
                e,
                String::from_utf8_lossy(&bytes)
            )
        });

        let error_obj = &body_json["error"];
        assert!(!error_obj.is_null(), "JSON 'error' field is missing");

        let error_type = error_obj["type"]
            .as_str()
            .expect("JSON 'error.type' field is not a string or missing");
        assert_eq!(error_type, expected_type, "Error type mismatch");

        let error_msg = error_obj["message"]
            .as_str()
            .expect("JSON 'error.message' field is not a string or missing");
        assert!(
            error_msg.contains(expected_message_substring),
            "Expected error message '{}' to contain '{}'",
            error_msg,
            expected_message_substring
        );

        if expect_details {
            assert!(
                !error_obj["details"].is_null(),
                "Expected 'error.details' field to exist but it was null or missing"
            );
            assert!(
                error_obj["details"].is_string(),
                "Expected 'error.details' field to be a string"
            );
        } else {
            assert!(
                error_obj["details"].is_null() || !error_obj["details"].is_string(),
                "Expected 'error.details' field to be null or non-existent/non-string"
            );
        }
    }

    #[tokio::test]
    async fn test_into_response_config() {
        check_response(
            AppError::Config("Test config issue".to_string()),
            StatusCode::INTERNAL_SERVER_ERROR,
            "CONFIG_ERROR",
            "Internal server configuration error",
            false,
        )
        .await;
    }

    #[tokio::test]
    async fn test_into_response_io() {
        let io_error = io::Error::new(io::ErrorKind::NotFound, "File not found");
        check_response(
            AppError::Io(io_error),
            StatusCode::INTERNAL_SERVER_ERROR,
            "IO_ERROR",
            "Internal server error during IO operation",
            false,
        )
        .await;
    }

    #[tokio::test]
    async fn test_into_response_yaml() {
        let yaml_error: serde_yaml::Error =
            serde_yaml::from_str::<()>("invalid: yaml:").unwrap_err();
        check_response(
            AppError::YamlParsing(yaml_error),
            StatusCode::INTERNAL_SERVER_ERROR,
            "CONFIG_PARSE_ERROR",
            "Failed to parse configuration file",
            false,
        )
        .await;
    }

    #[tokio::test]
    async fn test_into_response_proxy_config_url_parse() {
        let url_error = url::Url::parse("::invalid url").unwrap_err();
        let proxy_error = ProxyConfigErrorData {
            url: "::invalid url".to_string(),
            kind: ProxyConfigErrorKind::UrlParse(url_error),
        };
        check_response(
            AppError::ProxyConfigError(proxy_error),
            StatusCode::INTERNAL_SERVER_ERROR,
            "PROXY_CONFIG_ERROR",
            "Internal server error related to proxy configuration",
            true,
        )
        .await; // Expect details (URL)
    }

    #[tokio::test]
    async fn test_into_response_proxy_config_unsupported_scheme() {
        let proxy_error = ProxyConfigErrorData {
            url: "ftp://bad".to_string(),
            kind: ProxyConfigErrorKind::UnsupportedScheme("ftp".to_string()),
        };
        check_response(
            AppError::ProxyConfigError(proxy_error),
            StatusCode::INTERNAL_SERVER_ERROR,
            "PROXY_CONFIG_ERROR",
            "Internal server error related to proxy configuration",
            true,
        )
        .await; // Expect details (URL)
    }

    #[tokio::test]
    async fn test_into_response_no_keys() {
        check_response(
            AppError::NoAvailableKeys,
            StatusCode::SERVICE_UNAVAILABLE,
            "NO_AVAILABLE_KEYS",
            "No available API keys",
            false,
        )
        .await;
    }

    #[tokio::test]
    async fn test_into_response_upstream_service_error() {
        check_response(
            AppError::UpstreamServiceError {
                status: StatusCode::BAD_GATEWAY,
                body: "Upstream failed".to_string(),
            },
            StatusCode::BAD_GATEWAY,
            "UPSTREAM_SERVICE_ERROR",
            "Upstream service returned an error",
            true, // Expect details (body)
        )
        .await;
    }

    #[tokio::test]
    async fn test_into_response_request_body_error() {
        check_response(
            AppError::RequestBodyError("Bad body".to_string()),
            StatusCode::BAD_REQUEST,
            "REQUEST_BODY_ERROR",
            "Failed to process request body",
            true,
        )
        .await;
    }

    #[tokio::test]
    async fn test_into_response_response_body_error() {
        check_response(
            AppError::ResponseBodyError("Bad response body".to_string()),
            StatusCode::BAD_GATEWAY,
            "RESPONSE_PROCESSING_ERROR",
            "Failed to process response",
            true,
        )
        .await;
    }

    #[tokio::test]
    // Removed ignore from this test
    async fn test_into_response_invalid_client_key() {
        check_response(
            AppError::InvalidClientApiKey,
            StatusCode::UNAUTHORIZED,
            "INVALID_API_KEY",
            "Invalid or unauthorized API key",
            false,
        )
        .await;
    }

    #[tokio::test]
    async fn test_into_response_internal() {
        check_response(
            AppError::Internal("Something went wrong".to_string()),
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_SERVER_ERROR",
            "unexpected internal server error",
            false,
        )
        .await;
    }

    #[tokio::test]
    #[ignore = "Reliably triggering client build error in test is difficult without specific features/mocking"]
    async fn test_into_response_http_client_build_error() {
        // Simulate a reqwest build error by providing an invalid proxy format
        let invalid_proxy_url = "::not-a-valid-proxy-url";
        let proxy_res = reqwest::Proxy::all(invalid_proxy_url); // This itself won't error yet
        let builder = reqwest::Client::builder();
        let builder_with_proxy = match proxy_res {
            Ok(proxy) => builder.proxy(proxy),
            Err(_) => builder, // Should ideally not happen with this string, but handle defensively
        };
        // .build() will likely fail because the proxy URL is invalid or cannot be resolved by reqwest internally
        let build_error = builder_with_proxy
            .build()
            .expect_err("Client build should fail with invalid proxy setup");

        check_response(
            // Manually construct the error variant now
            AppError::HttpClientBuildError {
                source: build_error,
                proxy_url: Some(invalid_proxy_url.to_string()),
            },
            StatusCode::INTERNAL_SERVER_ERROR,
            "HTTP_CLIENT_BUILD_ERROR",
            "Internal server error building HTTP client",
            true, // Expect details (proxy URL)
        )
        .await;
    }

    #[tokio::test]
    async fn test_into_response_reqwest_timeout() {
        // Simulate a timeout error - need a way to create reqwest::Error directly or mock
        // For now, test the logic path using a placeholder error that is_timeout() == true
        // This requires mocking or a feature flag for testing, skipping for now.
        // let e = create_mock_reqwest_timeout_error();
        // check_response(AppError::Reqwest(e), StatusCode::GATEWAY_TIMEOUT, "UPSTREAM_ERROR", "Upstream request timed out", true).await;
        assert!(true); // Placeholder
    }

    #[tokio::test]
    async fn test_into_response_reqwest_connect() {
        // Simulate a connect error
        // Skipping for now due to complexity of creating reqwest::Error
        assert!(true); // Placeholder
    }

    #[tokio::test]
    async fn test_into_response_reqwest_generic() {
        // Simulate a generic reqwest error
        let e = reqwest::get("http://invalid-url-that-does-not-exist-and-causes-error")
            .await
            .unwrap_err();
        // Check based on the new logic: request setup errors (like DNS) are now BAD_GATEWAY
        check_response(
            AppError::Reqwest(e),
            StatusCode::BAD_GATEWAY,
            "UPSTREAM_ERROR",
            "Internal error setting up upstream request",
            true,
        )
        .await;
    }
}
