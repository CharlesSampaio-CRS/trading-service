use serde::{Deserialize, Serialize};
use mongodb::bson::oid::ObjectId;
use chrono::{DateTime, Utc};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Exchange {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub user_id: String,
    pub name: String,
    pub api_key: String,
    pub secret: String,
    pub passphrase: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectExchangeRequest {
    pub user_id: String,
    pub exchange_id: String,
    pub api_key: String,
    pub api_secret: String,
    pub passphrase: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExchangeResponse {
    pub id: String,
    pub name: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

impl From<Exchange> for ExchangeResponse {
    fn from(exchange: Exchange) -> Self {
        Self {
            id: exchange.id.map(|id| id.to_string()).unwrap_or_default(),
            name: exchange.name,
            is_active: exchange.is_active,
            created_at: exchange.created_at,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DisconnectExchangeRequest {
    pub user_id: String,
    pub exchange_id: String,
}
