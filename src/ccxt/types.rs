use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct CCXTError {
    pub message: String,
    pub code: Option<String>,
}

impl std::fmt::Display for CCXTError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CCXT Error: {}", self.message)
    }
}

impl std::error::Error for CCXTError {}
