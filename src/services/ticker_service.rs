use crate::{
    ccxt::CCXTClient,
    database::MongoDB,
    models::{DecryptedExchange, UserExchanges, ExchangeCatalog},
    utils::crypto::decrypt_fernet_via_python,
};
use mongodb::bson::{doc, oid::ObjectId};
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Serialize, Deserialize)]
pub struct Ticker {
    pub symbol: String,
    pub exchange: String,
    pub last: f64,
    pub bid: Option<f64>,
    pub ask: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub volume: Option<f64>,
    pub timestamp: i64,
}

#[derive(Debug, Serialize)]
pub struct TickersResponse {
    pub success: bool,
    pub tickers: Vec<Ticker>,
    pub count: usize,
}

// GET /tickers?symbols=BTC/USDT,ETH/USDT&user_id=xxx
pub async fn get_tickers(
    db: &MongoDB,
    user_id: &str,
    symbols: Vec<String>,
) -> Result<TickersResponse, String> {
    let exchanges = get_user_exchanges(db, user_id).await?;
    
    if exchanges.is_empty() {
        return Ok(TickersResponse {
            success: true,
            tickers: vec![],
            count: 0,
        });
    }
    
    log::info!("Fetching {} tickers from {} exchanges", symbols.len(), exchanges.len());
    
    let mut all_tickers = Vec::new();
    
    for exchange in exchanges {
        for symbol in &symbols {
            match fetch_ticker(exchange.clone(), symbol, user_id).await {
                Ok(ticker) => {
                    all_tickers.push(ticker);
                }
                Err(e) => {
                    log::warn!("Error fetching {} from {}: {}", symbol, exchange.name, e);
                }
            }
        }
    }
    
    let count = all_tickers.len();
    
    Ok(TickersResponse {
        success: true,
        tickers: all_tickers,
        count,
    })
}

async fn get_user_exchanges(
    db: &MongoDB,
    user_id: &str,
) -> Result<Vec<DecryptedExchange>, String> {
    let user_exchanges_collection = db.collection::<UserExchanges>("user_exchanges");
    
    let filter = doc! { "user_id": user_id };
    
    let user_exchanges = user_exchanges_collection
        .find_one(filter)
        .await
        .map_err(|e| format!("Database error: {}", e))?;
    
    let user_exchanges = match user_exchanges {
        Some(ue) => ue,
        None => return Ok(vec![]),
    };
    
    let active_exchanges: Vec<_> = user_exchanges
        .exchanges
        .into_iter()
        .filter(|e| e.is_active)
        .collect();
    
    if active_exchanges.is_empty() {
        return Ok(vec![]);
    }
    
    let exchanges_collection = db.collection::<ExchangeCatalog>("exchanges");
    let encryption_key = env::var("ENCRYPTION_KEY")
        .map_err(|_| "ENCRYPTION_KEY not found".to_string())?;
    
    let mut decrypted_exchanges = Vec::new();
    
    for user_exchange in active_exchanges {
        let exchange_id = ObjectId::parse_str(&user_exchange.exchange_id)
            .map_err(|e| format!("Invalid exchange_id: {}", e))?;
        
        let filter = doc! { "_id": exchange_id };
        let catalog = exchanges_collection
            .find_one(filter)
            .await
            .map_err(|e| format!("Database error: {}", e))?;
        
        if let Some(catalog) = catalog {
            let api_key = decrypt_fernet_via_python(&user_exchange.api_key_encrypted, &encryption_key)
                .unwrap_or_else(|_| user_exchange.api_key_encrypted.clone());
            
            let api_secret = decrypt_fernet_via_python(&user_exchange.api_secret_encrypted, &encryption_key)
                .unwrap_or_else(|_| user_exchange.api_secret_encrypted.clone());
            
            let passphrase = user_exchange.passphrase_encrypted.as_ref()
                .and_then(|p| decrypt_fernet_via_python(p, &encryption_key).ok());
            
            decrypted_exchanges.push(DecryptedExchange {
                exchange_id: user_exchange.exchange_id,
                ccxt_id: catalog.ccxt_id,
                name: catalog.nome.unwrap_or_else(|| "Unknown".to_string()),
                api_key,
                api_secret,
                passphrase,
                is_active: user_exchange.is_active,
            });
        }
    }
    
    Ok(decrypted_exchanges)
}

async fn fetch_ticker(
    exchange: DecryptedExchange,
    symbol: &str,
    _user_id: &str,
) -> Result<Ticker, String> {
    let exchange_name_clone = exchange.name.clone();
    let symbol_clone = symbol.to_string();
    
    tokio::task::spawn_blocking(move || {
        let client = CCXTClient::new(
            &exchange.ccxt_id,
            &exchange.api_key,
            &exchange.api_secret,
            exchange.passphrase.as_deref(),
        )?;
        
        let ticker_json = client.fetch_ticker_sync(&symbol_clone)?;
        
        Ok(Ticker {
            symbol: symbol_clone.clone(),
            exchange: exchange_name_clone,
            last: ticker_json.get("last").and_then(|v| v.as_f64()).unwrap_or(0.0),
            bid: ticker_json.get("bid").and_then(|v| v.as_f64()),
            ask: ticker_json.get("ask").and_then(|v| v.as_f64()),
            high: ticker_json.get("high").and_then(|v| v.as_f64()),
            low: ticker_json.get("low").and_then(|v| v.as_f64()),
            volume: ticker_json.get("volume").and_then(|v| v.as_f64()),
            timestamp: ticker_json.get("timestamp")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| chrono::Utc::now().timestamp_millis()),
        })
    }).await.map_err(|e| format!("Task error: {}", e))?
}
