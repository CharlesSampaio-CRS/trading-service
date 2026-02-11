use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Balance {
    pub symbol: String,
    pub free: f64,
    pub used: f64,
    pub total: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usd_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_24h: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExchangeBalance {
    pub exchange: String,
    pub exchange_id: String,  // MongoDB ObjectId as string
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub balances: HashMap<String, Balance>,
    pub total_usd: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BalanceResponse {
    pub success: bool,
    pub exchanges: Vec<ExchangeBalance>,
    pub total_usd: f64,
    pub timestamp: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BalanceSummary {
    pub total_usd: f64,
    pub exchanges_count: usize,
    pub tokens_count: usize,
    pub timestamp: i64,
}
