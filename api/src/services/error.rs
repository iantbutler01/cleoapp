//! Error handling utilities for route handlers

use axum::http::StatusCode;

/// Extension trait for logging errors and converting to StatusCode
pub trait LogErr<T> {
    /// Log error with context and return INTERNAL_SERVER_ERROR
    fn log_500(self, context: &str) -> Result<T, StatusCode>;

    /// Log error with context and return a custom StatusCode
    fn log_status(self, context: &str, status: StatusCode) -> Result<T, StatusCode>;
}

impl<T, E: std::fmt::Display> LogErr<T> for Result<T, E> {
    fn log_500(self, context: &str) -> Result<T, StatusCode> {
        self.map_err(|e| {
            eprintln!("{}: {}", context, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
    }

    fn log_status(self, context: &str, status: StatusCode) -> Result<T, StatusCode> {
        self.map_err(|e| {
            eprintln!("{}: {}", context, e);
            status
        })
    }
}
