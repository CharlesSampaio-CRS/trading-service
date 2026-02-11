use crate::{
    database::MongoDB,
    models::{TokensExchangeCache, TokenInfo, DecryptedExchange},
    ccxt::CCXTClient,
    utils::thread_pool::spawn_ccxt_blocking,
};
use mongodb::bson::{doc, oid::ObjectId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Token {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub _id: Option<ObjectId>,
    pub symbol: String,
    pub name: String,
    pub logo: Option<String>,
    pub decimals: Option<i32>,
    pub coingecko_id: Option<String>,
    pub is_active: bool,
}

#[derive(Debug, Serialize)]
pub struct TokensResponse {
    pub success: bool,
    pub tokens: Vec<Token>,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub success: bool,
    pub token: Option<Token>,
}

pub async fn get_all_tokens(
    db: &MongoDB,
) -> Result<TokensResponse, String> {
    let collection = db.collection::<Token>("tokens");
    
    let filter = doc! { "is_active": true };
    let options = mongodb::options::FindOptions::builder()
        .sort(doc! { "symbol": 1 })
        .build();
    
    let mut cursor = collection
        .find(filter)
        .with_options(options)
        .await
        .map_err(|e| format!("Database error: {}", e))?;
    
    let mut tokens = Vec::new();
    use futures::stream::StreamExt;
    
    while let Some(result) = cursor.next().await {
        match result {
            Ok(token) => tokens.push(token),
            Err(e) => log::error!("Error reading token: {}", e),
        }
    }
    
    let count = tokens.len();
    
    Ok(TokensResponse {
        success: true,
        tokens,
        count,
    })
}

pub async fn get_token_by_symbol(
    db: &MongoDB,
    symbol: &str,
) -> Result<TokenResponse, String> {
    let collection = db.collection::<Token>("tokens");
    
    let filter = doc! {
        "symbol": symbol.to_uppercase(),
        "is_active": true,
    };
    
    let token = collection
        .find_one(filter)
        .await
        .map_err(|e| format!("Database error: {}", e))?;
    
    Ok(TokenResponse {
        success: true,
        token,
    })
}

// Search tokens
pub async fn search_tokens(
    db: &MongoDB,
    query: &str,
) -> Result<TokensResponse, String> {
    let collection = db.collection::<Token>("tokens");
    
    // Case-insensitive regex search on symbol or name
    let filter = doc! {
        "$or": [
            { "symbol": { "$regex": query, "$options": "i" } },
            { "name": { "$regex": query, "$options": "i" } }
        ],
        "is_active": true,
    };
    
    let options = mongodb::options::FindOptions::builder()
        .sort(doc! { "symbol": 1 })
        .limit(50)
        .build();
    
    let mut cursor = collection
        .find(filter)
        .with_options(options)
        .await
        .map_err(|e| format!("Database error: {}", e))?;
    
    let mut tokens = Vec::new();
    use futures::stream::StreamExt;
    
    while let Some(result) = cursor.next().await {
        match result {
            Ok(token) => tokens.push(token),
            Err(e) => log::error!("Error reading token: {}", e),
        }
    }
    
    let count = tokens.len();
    
    Ok(TokensResponse {
        success: true,
        tokens,
        count,
    })
}


#[derive(Debug, Serialize)]
pub struct ExchangeInfoToken {
    pub id: String,
    pub name: String,
    pub ccxt_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AvailableTokensResponse {
    pub success: bool,
    pub exchange: ExchangeInfoToken,
    pub quote_filter: String,
    pub total_tokens: usize,
    pub tokens: Vec<TokenInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_age_hours: Option<f64>,
    pub from_cache: bool,
}

pub async fn get_available_tokens(
    db: &MongoDB,
    exchange_id: &str,
    quote_filter: Option<&str>,
) -> Result<AvailableTokensResponse, String> {
    // Get cached tokens from MongoDB
    let tokens_exchanges_collection = db.collection::<TokensExchangeCache>("tokens_exchanges");
    
    let cached_data = tokens_exchanges_collection
        .find_one(doc! { "exchange_id": exchange_id })
        .await
        .map_err(|e| format!("Database error: {}", e))?;
    
    let cached_data = match cached_data {
        Some(data) => data,
        None => return Err("Token list not available in cache".to_string()),
    };
    
    // Check if update was successful
    if cached_data.update_status != "success" {
        return Err(format!(
            "Last update failed: {}",
            cached_data.error.unwrap_or_else(|| "Unknown error".to_string())
        ));
    }
    
    // Get exchange info
    let exchanges_collection = db.collection::<crate::models::ExchangeCatalog>("exchanges");
    let exchange_oid = ObjectId::parse_str(exchange_id)
        .map_err(|e| format!("Invalid exchange_id: {}", e))?;
    
    let exchange_info = exchanges_collection
        .find_one(doc! { "_id": exchange_oid })
        .await
        .map_err(|e| format!("Database error: {}", e))?;
    
    let exchange_info = match exchange_info {
        Some(info) => info,
        None => return Err("Exchange not found".to_string()),
    };
    
    // Filter tokens by quote if specified
    let tokens_list: Vec<TokenInfo> = if let Some(quote) = quote_filter {
        let quote_upper = quote.to_uppercase();
        cached_data
            .tokens_by_quote
            .get(&quote_upper)
            .cloned()
            .unwrap_or_else(Vec::new)
    } else {
        // Return all quotes
        cached_data
            .tokens_by_quote
            .values()
            .flat_map(|tokens| tokens.clone())
            .collect()
    };
    
    // Calculate cache age
    let (updated_at_str, cache_age_hours) = if let Some(updated_at) = cached_data.updated_at {
        let now = chrono::Utc::now();
        // Convert BsonDateTime to timestamp and then to chrono DateTime
        let timestamp_millis = updated_at.timestamp_millis();
        let updated_chrono = chrono::DateTime::from_timestamp_millis(timestamp_millis)
            .unwrap_or_else(|| now);
        let duration = now.signed_duration_since(updated_chrono);
        let hours = duration.num_seconds() as f64 / 3600.0;
        (Some(updated_chrono.to_rfc3339()), Some((hours * 10.0).round() / 10.0))
    } else {
        (None, None)
    };
    
    Ok(AvailableTokensResponse {
        success: true,
        exchange: ExchangeInfoToken {
            id: exchange_id.to_string(),
            name: exchange_info.nome.unwrap_or_else(|| "Unknown".to_string()),
            ccxt_id: exchange_info.ccxt_id,
            icon: exchange_info.icon,
        },
        quote_filter: quote_filter
            .map(|q| q.to_uppercase())
            .unwrap_or_else(|| "all".to_string()),
        total_tokens: tokens_list.len(),
        tokens: tokens_list,
        updated_at: updated_at_str,
        cache_age_hours,
        from_cache: true,
    })
}

// ============================================================================
// AVAILABLE TOKENS BY CCXT ID - MONGODB CACHE
// ============================================================================

// Get available tokens by CCXT ID (binance, bybit, mexc, etc) from MongoDB cache
pub async fn get_available_tokens_by_ccxt(
    db: &MongoDB,
    ccxt_id: &str,
    quote_filter: Option<&str>,
) -> Result<AvailableTokensResponse, String> {
    // Get cached tokens from MongoDB using ccxt_id
    let tokens_exchanges_collection = db.collection::<TokensExchangeCache>("tokens_exchanges");
    
    let cached_data = tokens_exchanges_collection
        .find_one(doc! { "exchange_ccxt_id": ccxt_id })
        .await
        .map_err(|e| format!("Database error: {}", e))?;
    
    let cached_data = match cached_data {
        Some(data) => data,
        None => return Err(format!("Token list not available in cache for exchange: {}", ccxt_id)),
    };
    
    // Check if update was successful
    if cached_data.update_status != "success" {
        return Err(format!(
            "Last update failed: {}",
            cached_data.error.unwrap_or_else(|| "Unknown error".to_string())
        ));
    }
    
    // Get exchange info from catalog
    let exchanges_collection = db.collection::<crate::models::ExchangeCatalog>("exchanges");
    
    let exchange_info = exchanges_collection
        .find_one(doc! { "ccxt_id": ccxt_id })
        .await
        .map_err(|e| format!("Database error: {}", e))?;
    
    let exchange_info = match exchange_info {
        Some(info) => info,
        None => return Err(format!("Exchange not found in catalog: {}", ccxt_id)),
    };
    
    // Filter tokens by quote if specified
    let tokens_list: Vec<TokenInfo> = if let Some(quote) = quote_filter {
        let quote_upper = quote.to_uppercase();
        cached_data
            .tokens_by_quote
            .get(&quote_upper)
            .cloned()
            .unwrap_or_else(Vec::new)
    } else {
        // Return all quotes
        cached_data
            .tokens_by_quote
            .values()
            .flat_map(|tokens| tokens.clone())
            .collect()
    };
    
    // Calculate cache age
    let (updated_at_str, cache_age_hours) = if let Some(updated_at) = cached_data.updated_at {
        let now = chrono::Utc::now();
        let timestamp_millis = updated_at.timestamp_millis();
        let updated_chrono = chrono::DateTime::from_timestamp_millis(timestamp_millis)
            .unwrap_or_else(|| now);
        let duration = now.signed_duration_since(updated_chrono);
        let hours = duration.num_seconds() as f64 / 3600.0;
        (Some(updated_chrono.to_rfc3339()), Some((hours * 10.0).round() / 10.0))
    } else {
        (None, None)
    };
    
    Ok(AvailableTokensResponse {
        success: true,
        exchange: ExchangeInfoToken {
            id: cached_data.exchange_id.clone(),
            name: exchange_info.nome.unwrap_or_else(|| "Unknown".to_string()),
            ccxt_id: exchange_info.ccxt_id,
            icon: exchange_info.icon,
        },
        quote_filter: quote_filter
            .map(|q| q.to_uppercase())
            .unwrap_or_else(|| "all".to_string()),
        total_tokens: tokens_list.len(),
        tokens: tokens_list,
        updated_at: updated_at_str,
        cache_age_hours,
        from_cache: true,
    })
}

// Token details function will be added at the end of file

// ============================================================================
// TOKEN DETAILS WITH CREDENTIALS - ZERO DATABASE PATTERN
// ============================================================================

#[derive(Debug, Deserialize, Serialize)]
pub struct GetTokenDetailsRequest {
    pub exchange: DecryptedExchange,
    pub symbol: String,
}

#[derive(Debug, Serialize)]
pub struct TokenDetailsResponse {
    pub success: bool,
    pub symbol: String,
    pub pair: String,
    pub quote: String,
    pub exchange: ExchangeInfoDetails,
    pub price: PriceInfo,
    pub change: ChangeInfo,
    pub volume: VolumeInfo,
    pub market_info: MarketInfo,
    pub timestamp: i64,
    pub datetime: String,
}

#[derive(Debug, Serialize)]
pub struct ExchangeInfoDetails {
    pub id: String,
    pub name: String,
    pub ccxt_id: String,
}

#[derive(Debug, Serialize)]
pub struct PriceInfo {
    pub current: String,
    pub bid: String,
    pub ask: String,
    pub high_24h: String,
    pub low_24h: String,
}

#[derive(Debug, Serialize)]
pub struct ChangeInfo {
    #[serde(rename = "1h")]
    pub one_hour: ChangeDetail,
    #[serde(rename = "4h")]
    pub four_hours: ChangeDetail,
    #[serde(rename = "24h")]
    pub twenty_four_hours: ChangeDetail,
}

#[derive(Debug, Serialize)]
pub struct ChangeDetail {
    pub price_change: String,
    pub price_change_percent: String,
}

#[derive(Debug, Serialize)]
pub struct VolumeInfo {
    pub base_24h: String,
    pub quote_24h: String,
}

#[derive(Debug, Serialize)]
pub struct MarketInfo {
    pub active: bool,
    pub limits: Limits,
    pub precision: Precision,
}

#[derive(Debug, Serialize)]
pub struct Limits {
    pub amount: LimitRange,
    pub cost: LimitRange,
    pub price: LimitRange,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leverage: Option<LimitRange>,
}

#[derive(Debug, Serialize)]
pub struct LimitRange {
    pub min: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct Precision {
    pub amount: i32,
    pub price: i32,
}

pub async fn get_token_details_with_creds(
    request: &GetTokenDetailsRequest,
) -> Result<TokenDetailsResponse, String> {
    let exchange_clone = request.exchange.clone();
    let symbol_clone = request.symbol.clone();
    
    let ticker_task = spawn_ccxt_blocking(move || {
        let client = CCXTClient::new(
            &exchange_clone.ccxt_id,
            &exchange_clone.api_key,
            &exchange_clone.api_secret,
            exchange_clone.passphrase.as_deref(),
        )?;
        
        client.fetch_ticker_sync(&symbol_clone)
    });
    
    let ticker_json = ticker_task.await
        .map_err(|e| format!("Task join error: {}", e))?
        .map_err(|e| format!("Failed to fetch ticker: {}", e))?;
    
    // Parse symbol
    let parts: Vec<&str> = request.symbol.split('/').collect();
    let (base, quote) = if parts.len() == 2 {
        (parts[0].to_string(), parts[1].to_string())
    } else {
        (request.symbol.clone(), "USDT".to_string())
    };
    
    let current_price = ticker_json.get("last").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let change_24h_percent = ticker_json.get("percentage").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let change_24h_value = (current_price * change_24h_percent) / 100.0;
    
    // Estimativas (CCXT não fornece 1h/4h)
    let change_1h_percent = change_24h_percent * 0.1;
    let change_4h_percent = change_24h_percent * 0.4;
    
    Ok(TokenDetailsResponse {
        success: true,
        symbol: base.clone(),
        pair: request.symbol.clone(),
        quote: quote.clone(),
        exchange: ExchangeInfoDetails {
            id: request.exchange.exchange_id.clone(),
            name: request.exchange.name.clone(),
            ccxt_id: request.exchange.ccxt_id.clone(),
        },
        price: PriceInfo {
            current: current_price.to_string(),
            bid: ticker_json.get("bid").and_then(|v| v.as_f64()).map(|v| v.to_string())
                .unwrap_or_else(|| current_price.to_string()),
            ask: ticker_json.get("ask").and_then(|v| v.as_f64()).map(|v| v.to_string())
                .unwrap_or_else(|| current_price.to_string()),
            high_24h: ticker_json.get("high").and_then(|v| v.as_f64()).map(|v| v.to_string())
                .unwrap_or_else(|| "0".to_string()),
            low_24h: ticker_json.get("low").and_then(|v| v.as_f64()).map(|v| v.to_string())
                .unwrap_or_else(|| "0".to_string()),
        },
        change: ChangeInfo {
            one_hour: ChangeDetail {
                price_change: (current_price * change_1h_percent / 100.0).to_string(),
                price_change_percent: change_1h_percent.to_string(),
            },
            four_hours: ChangeDetail {
                price_change: (current_price * change_4h_percent / 100.0).to_string(),
                price_change_percent: change_4h_percent.to_string(),
            },
            twenty_four_hours: ChangeDetail {
                price_change: change_24h_value.to_string(),
                price_change_percent: change_24h_percent.to_string(),
            },
        },
        volume: VolumeInfo {
            base_24h: ticker_json.get("baseVolume").and_then(|v| v.as_f64()).map(|v| v.to_string())
                .unwrap_or_else(|| "0".to_string()),
            quote_24h: ticker_json.get("quoteVolume").and_then(|v| v.as_f64()).map(|v| v.to_string())
                .unwrap_or_else(|| "0".to_string()),
        },
        market_info: MarketInfo {
            active: true,
            limits: Limits {
                amount: LimitRange { min: None, max: None },
                cost: LimitRange { min: None, max: None },
                price: LimitRange { min: None, max: None },
                leverage: None,
            },
            precision: Precision {
                amount: 8,
                price: 8,
            },
        },
        timestamp: ticker_json.get("timestamp").and_then(|v| v.as_i64())
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis()),
        datetime: ticker_json.get("datetime").and_then(|v| v.as_str()).map(|s| s.to_string())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
    })
}
