use serde::{Deserialize, Serialize};
use mongodb::bson::oid::ObjectId;
use mongodb::bson::DateTime as BsonDateTime;
use std::collections::HashMap;

/// Token info stored in cache
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TokenInfo {
    pub symbol: String,
    pub pair: String,
    pub quote: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_amount: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_amount: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_cost: Option<f64>,
}

/// Cached tokens for an exchange (collection: tokens_exchanges)
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TokensExchangeCache {
    #[serde(rename = "_id")]
    pub id: ObjectId,
    pub exchange_id: String,
    pub tokens_by_quote: HashMap<String, Vec<TokenInfo>>,
    pub update_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<BsonDateTime>,
}
