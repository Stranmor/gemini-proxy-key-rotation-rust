//! Error type definitions and conversions

use super::AppError;

// Implement From traits for common error types
impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        Self::Io {
            operation: "io_operation".to_string(),
            message: err.to_string(),
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serialization {
            message: err.to_string(),
        }
    }
}

impl From<serde_yaml::Error> for AppError {
    fn from(err: serde_yaml::Error) -> Self {
        Self::Serialization {
            message: err.to_string(),
        }
    }
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        let status_code = err.status().map(|s| s.as_u16());
        Self::HttpClient {
            message: err.to_string(),
            status_code,
        }
    }
}

impl From<redis::RedisError> for AppError {
    fn from(err: redis::RedisError) -> Self {
        if err.is_connection_dropped() {
            Self::RedisConnection {
                message: err.to_string(),
            }
        } else {
            Self::RedisOperation {
                operation: "redis_operation".to_string(),
                message: err.to_string(),
            }
        }
    }
}

impl From<deadpool_redis::PoolError> for AppError {
    fn from(err: deadpool_redis::PoolError) -> Self {
        Self::RedisConnection {
            message: err.to_string(),
        }
    }
}

impl From<axum::http::header::InvalidHeaderValue> for AppError {
    fn from(err: axum::http::header::InvalidHeaderValue) -> Self {
        Self::InvalidRequest {
            message: format!("Invalid header value: {err}"),
        }
    }
}

impl From<hyper::Error> for AppError {
    fn from(err: hyper::Error) -> Self {
        Self::HttpClient {
            message: err.to_string(),
            status_code: None,
        }
    }
}

impl From<url::ParseError> for AppError {
    fn from(err: url::ParseError) -> Self {
        Self::InvalidRequest {
            message: format!("Invalid URL: {err}"),
        }
    }
}

impl From<config::ConfigError> for AppError {
    fn from(err: config::ConfigError) -> Self {
        match err {
            config::ConfigError::NotFound(_) => Self::ConfigNotFound {
                path: "config file".to_string(),
            },
            config::ConfigError::Type { .. } => {
                Self::config_validation(err.to_string(), None::<String>)
            }
            _ => Self::ConfigParse {
                message: err.to_string(),
                line: None,
            },
        }
    }
}

impl From<deadpool::managed::CreatePoolError<deadpool_redis::ConfigError>> for AppError {
    fn from(err: deadpool::managed::CreatePoolError<deadpool_redis::ConfigError>) -> Self {
        Self::RedisConnection {
            message: format!("Failed to create Redis pool: {err}"),
        }
    }
}

impl From<axum::Error> for AppError {
    fn from(err: axum::Error) -> Self {
        Self::Internal {
            message: err.to_string(),
        }
    }
}

impl From<validator::ValidationErrors> for AppError {
    fn from(err: validator::ValidationErrors) -> Self {
        let field = err
            .field_errors()
            .keys()
            .next()
            .unwrap_or(&"unknown")
            .to_string();
        let message = err.to_string();
        Self::Validation { field, message }
    }
}
