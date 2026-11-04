use serde::Serialize;

/// Error returned to the frontend: a stable machine code plus an actionable message.
#[derive(Debug, thiserror::Error, Serialize)]
#[error("{code}: {message}")]
pub struct AppError {
    pub code: &'static str,
    pub message: String,
}

impl AppError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::new("io", e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::new("json", e.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
