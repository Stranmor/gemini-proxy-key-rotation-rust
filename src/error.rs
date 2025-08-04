// src/error.rs
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use thiserror::Error;
use tracing::error;

/// Представляет структурированное тело ответа об ошибке.
#[derive(Serialize, Debug)]
struct ErrorResponse {
    error: ErrorDetails,
}

/// Содержит детали ошибки для тела ответа.
#[derive(Serialize, Debug)]
struct ErrorDetails {
    #[serde(rename = "type")]
    error_type: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<String>,
}

/// Конкретные виды ошибок конфигурации прокси.
#[derive(Error, Debug)]
pub enum ProxyConfigErrorKind {
    #[error("Invalid URL format: {0}")]
    UrlParse(#[from] url::ParseError),
    #[error("Unsupported scheme: {0}")]
    UnsupportedScheme(String),
    #[error("Invalid proxy definition: {0}")]
    InvalidDefinition(String),
}

/// Детальная информация об ошибках конфигурации прокси.
#[derive(Error, Debug)]
#[error("Proxy configuration error for URL '{url}': {kind}")]
pub struct ProxyConfigErrorData {
    pub url: String,
    pub kind: ProxyConfigErrorKind,
}

/// Представляет возможные ошибки, которые могут возникнуть в приложении.
///
/// Реализует `IntoResponse` для автоматического преобразования ошибок в
/// соответствующие HTTP-ответы с стандартизированным телом JSON.
#[derive(Error, Debug)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Reqwest HTTP client error: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("YAML parsing error: {0}")]
    YamlParsing(#[from] serde_yaml::Error),

    #[error("No available API keys")]
    NoAvailableKeys,

    #[error("Upstream service error: {status} - {body}")]
    UpstreamServiceError { status: StatusCode, body: String },

    // Источники ошибок обработки тела запроса могут быть разнообразными,
    // поэтому использование String обеспечивает гибкость.
    #[error("Request body processing error: {0}")]
    RequestBodyError(String),

    #[error("JSON processing error: {0}")]
    JsonProcessing(String, #[source] serde_json::Error),

    #[error("Response body processing error: {0}")]
    ResponseBodyError(String),

    #[error("Invalid API key provided by client")]
    InvalidClientApiKey,

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Not Found: {0}")]
    NotFound(String),

    #[error("Internal server error: {0}")]
    Internal(String),

    #[error(transparent)]
    ProxyConfigError(#[from] ProxyConfigErrorData),

    #[error("HTTP client build error: {source}")]
    HttpClientBuildError {
        source: reqwest::Error,
        proxy_url: Option<String>,
    },

    #[error("Failed to join URL components: {0}")]
    UrlJoinError(String),

    #[error("CSRF token invalid")]
    Csrf,

    #[error("Failed to read response body from upstream: {0}")]
    BodyReadError(String),

    #[error("Axum error: {0}")]
    Axum(#[from] axum::Error),

    #[error("URL parsing error: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("HTTP response builder error: {0}")]
    HttpResponseBuilder(#[from] http::Error),

    #[error("Invalid HTTP header")]
    InvalidHttpHeader,

    #[error("Internal retry mechanism exhausted without a final response")]
    InternalRetryExhausted,

    #[error("Redis pool error: {0}")]
    RedisError(#[from] deadpool_redis::PoolError),

    #[error("Redis command error: {0}")]
    RedisErrorGeneric(#[from] redis::RedisError),

    #[error("Tokenizer initialization failed: {0}")]
    TokenizerInitializationError(String),
}

impl From<deadpool_redis::CreatePoolError> for AppError {
    fn from(e: deadpool_redis::CreatePoolError) -> Self {
        AppError::Internal(format!("Failed to create Redis pool: {e}"))
    }
}

// РЕФАКТОРИНГ: Новый блок impl для разделения логики.
impl AppError {
    /// Преобразует AppError в кортеж из StatusCode и ErrorDetails.
    /// Этот метод инкапсулирует логику сопоставления ошибок с их представлением для клиента,
    /// не смешивая её с созданием самого HTTP-ответа.
    fn to_status_and_details(&self) -> (StatusCode, ErrorDetails) {
        match self {
            // --- 5xx Серверные ошибки (внутренние детали логируются, клиенту отправляется общее сообщение) ---
            Self::Config(msg) => {
                error!("Configuration error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorDetails {
                        error_type: "CONFIG_ERROR".to_string(),
                        message: "Internal server configuration error".to_string(),
                        details: None,
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
                error!("Proxy configuration error: {}", data);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorDetails {
                        error_type: "PROXY_CONFIG_ERROR".to_string(),
                        message: "Internal server error related to proxy configuration".to_string(),
                        details: Some(format!("Affected proxy URL: {}", data.url)),
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
                        details: proxy_url.as_ref().map(|u| format!("Related proxy: {u}")),
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
            Self::RedisError(e) => {
                error!("Redis pool error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorDetails {
                        error_type: "REDIS_POOL_ERROR".to_string(),
                        message: "Internal error with data storage connection pool".to_string(),
                        details: Some(e.to_string()),
                    },
                )
            }
            Self::RedisErrorGeneric(e) => {
                error!("Generic Redis error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorDetails {
                        error_type: "REDIS_ERROR".to_string(),
                        message: "Internal error with data storage".to_string(),
                        details: Some(e.to_string()),
                    },
                )
            }
            Self::UrlJoinError(msg) => {
                error!("URL join error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorDetails {
                        error_type: "URL_CONSTRUCTION_ERROR".to_string(),
                        message: "Internal error during URL construction".to_string(),
                        details: None,
                    },
                )
            }
            Self::Axum(e) => {
                error!("Internal Axum error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorDetails {
                        error_type: "AXUM_INTERNAL_ERROR".to_string(),
                        message: "An internal server error occurred".to_string(),
                        details: Some(e.to_string()),
                    },
                )
            }
            Self::UrlParse(e) => {
                error!("Internal URL parsing error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorDetails {
                        error_type: "URL_PARSE_ERROR".to_string(),
                        message: "An internal error occurred while parsing a URL".to_string(),
                        details: Some(e.to_string()),
                    },
                )
            }
            Self::HttpResponseBuilder(e) => {
                error!("HTTP response builder error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorDetails {
                        error_type: "HTTP_RESPONSE_BUILD_ERROR".to_string(),
                        message: "An internal error occurred while building an HTTP response"
                            .to_string(),
                        details: Some(e.to_string()),
                    },
                )
            }
            Self::InternalRetryExhausted => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorDetails {
                    error_type: "INTERNAL_RETRY_EXHAUSTED".to_string(),
                    message: "Internal retry mechanism failed to produce a final response"
                        .to_string(),
                    details: None,
                },
            ),
            Self::TokenizerInitializationError(msg) => {
                error!("CRITICAL: Tokenizer initialization failed: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorDetails {
                        error_type: "TOKENIZER_INIT_FAILURE".to_string(),
                        message: "Internal server error: critical component failed to initialize."
                            .to_string(),
                        details: None, // Do not expose internal details to the client
                    },
                )
            }

            // --- 5xx Ошибки, связанные с вышестоящими сервисами/проксированием ---
            Self::Reqwest(e) => {
                error!("Upstream reqwest error: {}", e);
                let (status_code, msg_key) = if e.is_timeout() {
                    (StatusCode::GATEWAY_TIMEOUT, "timeout")
                } else if e.is_connect() {
                    (StatusCode::BAD_GATEWAY, "connect")
                } else if e.is_request() {
                    (StatusCode::BAD_GATEWAY, "request_setup")
                } else if e.is_body() || e.is_decode() {
                    (StatusCode::BAD_GATEWAY, "body_or_decode")
                } else {
                    (StatusCode::BAD_GATEWAY, "generic")
                };

                let message = match msg_key {
                    "timeout" => "Upstream request timed out".to_string(),
                    "connect" => "Internal error setting up upstream request".to_string(),
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
                        details: Some(e.to_string()),
                    },
                )
            }
            Self::UpstreamServiceError { status, body } => {
                error!(
                    "Upstream service returned error: Status={}, Body='{}'",
                    status, body
                );
                (
                    *status,
                    ErrorDetails {
                        error_type: "UPSTREAM_SERVICE_ERROR".to_string(),
                        message: "Upstream service returned an error".to_string(),
                        details: Some(body.clone()),
                    },
                )
            }
            Self::ResponseBodyError(msg) => {
                error!("Response body processing error: {}", msg);
                (
                    StatusCode::BAD_GATEWAY,
                    ErrorDetails {
                        error_type: "RESPONSE_PROCESSING_ERROR".to_string(),
                        message: "Failed to process response from upstream service".to_string(),
                        details: Some(msg.clone()),
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
            Self::BodyReadError(msg) => {
                error!("Failed to read upstream response body: {}", msg);
                (
                    StatusCode::BAD_GATEWAY,
                    ErrorDetails {
                        error_type: "UPSTREAM_RESPONSE_READ_ERROR".to_string(),
                        message: "Failed to read response from upstream service".to_string(),
                        details: Some(msg.clone()),
                    },
                )
            }

            // --- 4xx Клиентские ошибки ---
            Self::RequestBodyError(msg) => (
                StatusCode::BAD_REQUEST,
                ErrorDetails {
                    error_type: "REQUEST_BODY_ERROR".to_string(),
                    message: "Failed to process request body".to_string(),
                    details: Some(msg.clone()),
                },
            ),
            Self::JsonProcessing(msg, source) => {
                error!("JSON processing error: {} - Source: {}", msg, source);
                (
                    StatusCode::BAD_REQUEST,
                    ErrorDetails {
                        error_type: "JSON_PROCESSING_ERROR".to_string(),
                        message: msg.clone(),
                        details: Some(source.to_string()),
                    },
                )
            }
            Self::InvalidClientApiKey => (
                StatusCode::UNAUTHORIZED,
                ErrorDetails {
                    error_type: "INVALID_API_KEY".to_string(),
                    message: "Invalid or unauthorized API key provided".to_string(),
                    details: None,
                },
            ),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                ErrorDetails {
                    error_type: "UNAUTHORIZED".to_string(),
                    message: "Authentication token is missing or invalid".to_string(),
                    details: None,
                },
            ),
            Self::NotFound(resource) => (
                StatusCode::NOT_FOUND,
                ErrorDetails {
                    error_type: "NOT_FOUND".to_string(),
                    message: format!("Resource not found: {resource}"),
                    details: None,
                },
            ),
            Self::Csrf => (
                StatusCode::FORBIDDEN,
                ErrorDetails {
                    error_type: "CSRF_TOKEN_INVALID".to_string(),
                    message: "CSRF token is missing or invalid.".to_string(),
                    details: None,
                },
            ),
            Self::InvalidHttpHeader => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorDetails {
                    error_type: "INVALID_HTTP_HEADER".to_string(),
                    message: "Failed to construct a valid HTTP header".to_string(),
                    details: None,
                },
            ),
        }
    }
}

// РЕФАКТОРИНГ: `IntoResponse` теперь более простой и сфокусированный.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        // Для всех остальных ошибок используем вспомогательный метод для получения деталей.
        let (status, error_details) = self.to_status_and_details();

        // Создаем стандартный JSON-ответ.
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
        expect_details: bool,
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
            "Assertion failed: Expected message '{error_msg}' to contain '{expected_message_substring}'"
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
        .await;
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
        .await;
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
            true,
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
    async fn test_into_response_http_client_build_error() {
        let build_error = reqwest::Client::builder()
            .min_tls_version(reqwest::tls::Version::TLS_1_3)
            .max_tls_version(reqwest::tls::Version::TLS_1_2)
            .build()
            .expect_err("Client build should fail with an impossible TLS version range");

        check_response(
            AppError::HttpClientBuildError {
                source: build_error,
                proxy_url: None,
            },
            StatusCode::INTERNAL_SERVER_ERROR,
            "HTTP_CLIENT_BUILD_ERROR",
            "Internal server error building HTTP client",
            false,
        )
        .await;
    }

    #[tokio::test]
    async fn test_into_response_reqwest_generic() {
        // Примечание: этот тест выполняет реальный сетевой запрос и может быть нестабильным
        // или медленным в зависимости от сетевых условий.
        let e = reqwest::get("http://invalid-url-that-does-not-exist-and-causes-error")
            .await
            .unwrap_err();
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
