//! Error context and correlation utilities

use std::collections::HashMap;
use uuid::Uuid;

/// Request context for error correlation and debugging
#[derive(Debug, Clone)]
pub struct ErrorContext {
    pub request_id: String,
    pub user_id: Option<String>,
    pub operation: String,
    pub metadata: HashMap<String, String>,
}

impl ErrorContext {
    pub fn new(operation: impl Into<String>) -> Self {
        Self {
            request_id: Uuid::new_v4().to_string(),
            user_id: None,
            operation: operation.into(),
            metadata: HashMap::new(),
        }
    }

    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = request_id.into();
        self
    }

    pub fn with_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    pub fn add_metadata(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.metadata.insert(key.into(), value.into());
    }
}

impl Default for ErrorContext {
    fn default() -> Self {
        Self::new("unknown")
    }
}

/// Thread-local storage for error context
thread_local! {
    static ERROR_CONTEXT: std::cell::RefCell<Option<ErrorContext>> = std::cell::RefCell::new(None);
}

/// Set the current error context for the thread
pub fn set_error_context(context: ErrorContext) {
    ERROR_CONTEXT.with(|c| {
        *c.borrow_mut() = Some(context);
    });
}

/// Get the current error context for the thread
pub fn get_error_context() -> Option<ErrorContext> {
    ERROR_CONTEXT.with(|c| c.borrow().clone())
}

/// Clear the current error context
pub fn clear_error_context() {
    ERROR_CONTEXT.with(|c| {
        *c.borrow_mut() = None;
    });
}

/// Macro to execute code with error context
#[macro_export]
macro_rules! with_error_context {
    ($context:expr, $code:block) => {{
        $crate::error::context::set_error_context($context);
        let result = $code;
        $crate::error::context::clear_error_context();
        result
    }};
}

/// Macro to add metadata to current error context
#[macro_export]
macro_rules! add_error_metadata {
    ($key:expr, $value:expr) => {
        if let Some(mut context) = $crate::error::context::get_error_context() {
            context.add_metadata($key, $value);
            $crate::error::context::set_error_context(context);
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_context_creation() {
        let context = ErrorContext::new("test_operation")
            .with_user_id("user123")
            .with_metadata("key1", "value1");

        assert_eq!(context.operation, "test_operation");
        assert_eq!(context.user_id, Some("user123".to_string()));
        assert_eq!(context.metadata.get("key1"), Some(&"value1".to_string()));
    }

    #[test]
    fn test_thread_local_context() {
        let context = ErrorContext::new("test");
        set_error_context(context.clone());

        let retrieved = get_error_context();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().operation, "test");

        clear_error_context();
        assert!(get_error_context().is_none());
    }

    #[test]
    fn test_with_error_context_macro() {
        let context = ErrorContext::new("macro_test");
        
        let result = with_error_context!(context, {
            let ctx = get_error_context();
            assert!(ctx.is_some());
            assert_eq!(ctx.unwrap().operation, "macro_test");
            42
        });

        assert_eq!(result, 42);
        assert!(get_error_context().is_none());
    }
}