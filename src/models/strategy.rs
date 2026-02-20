use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

/// Estratégia de trading (armazenada no MongoDB)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Strategy {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    
    /// ID do usuário dono da estratégia
    pub user_id: String,
    
    /// Nome da estratégia
    pub name: String,
    
    /// Descrição opcional
    pub description: Option<String>,
    
    /// Tipo de estratégia (ex: "dca", "grid", "scalping")
    pub strategy_type: String,
    
    /// Símbolo (ex: "BTC/USDT")
    pub symbol: String,
    
    /// ID da exchange (ObjectId como string)
    pub exchange_id: String,
    
    /// Nome da exchange (para facilitar queries)
    pub exchange_name: String,
    
    /// Status ativo/inativo
    pub is_active: bool,
    
    /// Configuração da estratégia (JSON)
    pub config: serde_json::Value,
    
    /// Timestamp de criação (Unix timestamp)
    pub created_at: i64,
    
    /// Timestamp de última atualização
    pub updated_at: i64,
}

/// Request para criar estratégia
#[derive(Debug, Deserialize)]
pub struct CreateStrategyRequest {
    pub name: String,
    pub description: Option<String>,
    pub strategy_type: String,
    pub symbol: String,
    pub exchange_id: String,
    pub exchange_name: String,
    pub config: serde_json::Value,
}

/// Request para atualizar estratégia
#[derive(Debug, Deserialize)]
pub struct UpdateStrategyRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub strategy_type: Option<String>,
    pub symbol: Option<String>,
    pub exchange_id: Option<String>,
    pub exchange_name: Option<String>,
    pub is_active: Option<bool>,
    pub config: Option<serde_json::Value>,
}

/// Response de estratégia
#[derive(Debug, Serialize)]
pub struct StrategyResponse {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub description: Option<String>,
    pub strategy_type: String,
    pub symbol: String,
    pub exchange_id: String,
    pub exchange_name: String,
    pub is_active: bool,
    pub config: serde_json::Value,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<Strategy> for StrategyResponse {
    fn from(strategy: Strategy) -> Self {
        StrategyResponse {
            id: strategy.id.map(|id| id.to_hex()).unwrap_or_default(),
            user_id: strategy.user_id,
            name: strategy.name,
            description: strategy.description,
            strategy_type: strategy.strategy_type,
            symbol: strategy.symbol,
            exchange_id: strategy.exchange_id,
            exchange_name: strategy.exchange_name,
            is_active: strategy.is_active,
            config: strategy.config,
            created_at: strategy.created_at,
            updated_at: strategy.updated_at,
        }
    }
}
