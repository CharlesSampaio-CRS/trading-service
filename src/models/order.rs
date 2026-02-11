use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use mongodb::bson::oid::ObjectId;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Order {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub _id: Option<ObjectId>,
    #[serde(rename = "exchange_order_id", alias = "id")]
    pub id: String,
    pub user_id: String,
    pub exchange: String,
    pub exchange_id: String,
    pub symbol: String,
    #[serde(rename = "type")]
    pub order_type: String, // market, limit
    pub side: String, // buy, sell
    pub price: Option<f64>,
    pub amount: f64,
    pub filled: f64,
    pub remaining: f64,
    pub cost: f64,
    pub status: String, // open, closed, canceled, expired
    pub fee: Option<OrderFee>,
    pub timestamp: i64,
    pub datetime: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OrderFee {
    pub currency: String,
    pub cost: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OrdersResponse {
    pub success: bool,
    pub orders: Vec<Order>,
    pub count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateOrderRequest {
    pub user_id: String,
    #[serde(alias = "exchange_id")] // Aceita exchange_id como alias
    pub exchange: String,
    pub symbol: String,
    #[serde(rename = "type")]
    pub order_type: String, // market, limit
    pub side: String, // buy, sell
    pub amount: f64,
    pub price: Option<f64>, // Required for limit orders
}

/// 🆕 Create order com credenciais do frontend
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateOrderWithCredsRequest {
    pub ccxt_id: String,
    pub exchange_name: String,
    pub api_key: String,
    pub api_secret: String,
    pub passphrase: Option<String>,
    pub symbol: String,
    #[serde(rename = "type")]
    pub order_type: String, // market, limit
    pub side: String, // buy, sell
    pub amount: f64,
    pub price: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateOrderResponse {
    pub success: bool,
    pub order: Option<Order>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CancelOrderRequest {
    pub user_id: String,
    #[serde(alias = "exchange_id")] // Aceita exchange_id como alias
    pub exchange: String,
    pub order_id: String,
    pub symbol: Option<String>,
}

/// 🆕 Cancel order com credenciais do frontend
#[derive(Debug, Serialize, Deserialize)]
pub struct CancelOrderWithCredsRequest {
    pub ccxt_id: String,
    pub exchange_name: String,
    pub api_key: String,
    pub api_secret: String,
    pub passphrase: Option<String>,
    pub order_id: String,
    pub symbol: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CancelOrderResponse {
    pub success: bool,
    pub message: String,
    pub order_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CancelAllOrdersRequest {
    pub user_id: String,
    pub exchange: Option<String>,
    pub symbol: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CancelAllOrdersResponse {
    pub success: bool,
    pub canceled_count: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OrderStatusResponse {
    pub success: bool,
    pub order: Option<Order>,
    pub error: Option<String>,
}
