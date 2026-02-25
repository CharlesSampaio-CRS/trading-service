use serde::{Deserialize, Serialize};
use mongodb::bson::oid::ObjectId;
use mongodb::bson::Bson;

/// Estrutura real do MongoDB - documento na collection "user_exchanges"
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserExchanges {
    #[serde(rename = "_id")]
    pub id: ObjectId,
    pub user_id: String,
    pub exchanges: Vec<UserExchangeItem>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub created_at: Option<Bson>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub updated_at: Option<Bson>,
}

/// Item dentro do array exchanges
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserExchangeItem {
    #[serde(deserialize_with = "deserialize_exchange_id")]
    pub exchange_id: String,
    pub api_key_encrypted: String,
    pub api_secret_encrypted: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub passphrase_encrypted: Option<String>,
    #[serde(default = "default_true")]
    pub is_active: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub created_at: Option<Bson>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub updated_at: Option<Bson>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reconnected_at: Option<Bson>,
}

fn default_true() -> bool {
    true
}

fn deserialize_exchange_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let bson_value = Bson::deserialize(deserializer)?;
    match bson_value {
        Bson::ObjectId(oid) => Ok(oid.to_hex()),
        Bson::String(s) => Ok(s),
        _ => Err(serde::de::Error::custom("Expected ObjectId or String")),
    }
}

/// Documento da collection "exchanges" (catálogo)
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExchangeCatalog {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub _id: Option<ObjectId>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub nome: Option<String>,
    pub ccxt_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pais_de_origem: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub logo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub supports_spot: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub supports_futures: Option<bool>,
    #[serde(default)]
    pub requires_passphrase: bool,
    #[serde(default = "default_true")]
    pub is_active: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub created_at: Option<Bson>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub updated_at: Option<Bson>,
}

/// Exchange com dados descriptografados (para uso interno)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DecryptedExchange {
    pub exchange_id: String,
    pub ccxt_id: String,
    pub name: String,
    pub api_key: String,
    pub api_secret: String,
    pub passphrase: Option<String>,
    pub is_active: bool,
}
