use std::fmt;

#[derive(Debug)]
pub enum AppError {
    DatabaseError(String),
    CCXTError(String),
    NotFound(String),
    InvalidRequest(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::DatabaseError(msg) => write!(f, "Database error: {}", msg),
            AppError::CCXTError(msg) => write!(f, "CCXT error: {}", msg),
            AppError::NotFound(msg) => write!(f, "Not found: {}", msg),
            AppError::InvalidRequest(msg) => write!(f, "Invalid request: {}", msg),
        }
    }
}

impl std::error::Error for AppError {}
