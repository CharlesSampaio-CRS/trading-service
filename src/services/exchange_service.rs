// ==================== READ ONLY CATALOG ====================
// Exchange catalog from MongoDB - Only available exchanges list
// User exchange management happens in frontend (WatermelonDB)

use crate::{
    database::MongoDB,
    models::ExchangeCatalog,
};
use mongodb::bson::{doc, oid::ObjectId};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct AvailableExchangesResponse {
    pub success: bool,
    pub exchanges: Vec<ExchangeCatalogInfo>,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct ExchangeCatalogInfo {
    #[serde(rename = "_id")]
    pub id: String,
    pub ccxt_id: String,
    pub nome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pais_de_origem: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    pub supports_spot: bool,
    pub supports_futures: bool,
    pub requires_passphrase: bool,
}

/// GET /exchanges/available - Lista todas exchanges disponíveis do catálogo
/// 
/// Read-only endpoint - apenas retorna o catálogo de exchanges disponíveis.
/// Gerenciamento de exchanges do usuário (link/unlink/toggle) acontece no frontend.
pub async fn get_available_exchanges(
    db: &MongoDB,
) -> Result<AvailableExchangesResponse, String> {
    let collection = db.collection::<ExchangeCatalog>("exchanges");
    
    let mut cursor = collection
        .find(doc! {})
        .await
        .map_err(|e| format!("Database error: {}", e))?;
    
    let mut exchanges = Vec::new();
    
    use futures::stream::StreamExt;
    
    while let Some(result) = cursor.next().await {
        match result {
            Ok(catalog) => {
                exchanges.push(ExchangeCatalogInfo {
                    id: catalog._id.map(|id| id.to_hex()).unwrap_or_default(),
                    ccxt_id: catalog.ccxt_id,
                    nome: catalog.nome.unwrap_or_else(|| "Unknown".to_string()),
                    url: catalog.url,
                    pais_de_origem: catalog.pais_de_origem,
                    icon: catalog.icon,
                    logo: catalog.logo,
                    supports_spot: catalog.supports_spot.unwrap_or(true),
                    supports_futures: catalog.supports_futures.unwrap_or(false),
                    requires_passphrase: catalog.requires_passphrase,
                });
            }
            Err(e) => {
                log::error!("Error reading exchange catalog: {}", e);
            }
        }
    }
    
    let count = exchanges.len();
    
    Ok(AvailableExchangesResponse {
        success: true,
        exchanges,
        count,
    })
}

/// GET /exchanges/{exchange_id}/token/{symbol} - Busca detalhes completos do token via CCXT
/// Retorna dados de mercado (ticker, orderbook, volume, etc) diretamente da exchange
pub async fn get_token_details(
    db: &MongoDB,
    user_id: &str,
    exchange_id: &str,
    symbol: &str,
) -> Result<serde_json::Value, String> {
    use crate::models::user_exchange::{UserExchanges, UserExchangeItem};
    use crate::models::ExchangeCatalog;
    use crate::utils::crypto::decrypt_fernet_via_python;
    use crate::utils::thread_pool::spawn_ccxt_blocking;
    use std::env;
    
    log::info!("🔍 Fetching token details for {} on exchange {}", symbol, exchange_id);
    
    // 1. Busca o documento user_exchanges
    let user_exchanges_collection = db.collection::<UserExchanges>("user_exchanges");
    let user_exchanges = user_exchanges_collection
        .find_one(doc! { "user_id": user_id })
        .await
        .map_err(|e| format!("Database error: {}", e))?
        .ok_or_else(|| "User exchanges not found".to_string())?;
    
    // 2. Encontra a exchange específica no array
    let user_exchange = user_exchanges.exchanges
        .iter()
        .find(|ex| ex.exchange_id == exchange_id)
        .ok_or_else(|| "Exchange not found in user's exchanges".to_string())?
        .clone(); // Clone para mover para thread
    
    if !user_exchange.is_active {
        return Err("Exchange is not active".to_string());
    }
    
    // 3. Busca info da exchange no catálogo
    let exchange_oid = ObjectId::parse_str(exchange_id)
        .map_err(|e| format!("Invalid exchange_id: {}", e))?;
    
    let exchanges_collection = db.collection::<ExchangeCatalog>("exchanges");
    let catalog = exchanges_collection
        .find_one(doc! { "_id": exchange_oid })
        .await
        .map_err(|e| format!("Database error: {}", e))?
        .ok_or_else(|| "Exchange catalog not found".to_string())?;
    
    let exchange_name = catalog.nome.clone();
    let ccxt_id = catalog.ccxt_id.clone();
    
    // 4. Descriptografa as credenciais
    let encryption_key = env::var("ENCRYPTION_KEY")
        .map_err(|_| "ENCRYPTION_KEY not found".to_string())?;
    
    let api_key = decrypt_fernet_via_python(&user_exchange.api_key_encrypted, &encryption_key)
        .map_err(|e| format!("Failed to decrypt API key: {}", e))?;
    
    let secret_key = decrypt_fernet_via_python(&user_exchange.api_secret_encrypted, &encryption_key)
        .map_err(|e| format!("Failed to decrypt secret key: {}", e))?;
    
    let passphrase = if let Some(ref enc_pass) = user_exchange.passphrase_encrypted {
        Some(decrypt_fernet_via_python(enc_pass, &encryption_key)
            .map_err(|e| format!("Failed to decrypt passphrase: {}", e))?)
    } else {
        None
    };
    
    // 5. Formata o símbolo para o padrão CCXT (ex: BTC → BTC/USDT)
    let market_symbol = if symbol.contains('/') {
        symbol.to_string()
    } else {
        format!("{}/USDT", symbol.to_uppercase())
    };
    
    let market_symbol_clone = market_symbol.clone();
    let symbol_upper = symbol.to_uppercase();
    
    // Clone para usar na closure e depois no JSON final
    let ccxt_id_clone = ccxt_id.clone();
    let api_key_clone = api_key.clone();
    let secret_key_clone = secret_key.clone();
    let passphrase_clone = passphrase.clone();
    
    // 6. Executa fetch em thread bloqueante (CCXT usa Python/GIL)
    log::info!("📊 Fetching market data for {}", market_symbol);
    
    let ticker_task = spawn_ccxt_blocking(move || {
        let client = crate::ccxt::client::CCXTClient::new(
            &ccxt_id_clone,
            &api_key_clone,
            &secret_key_clone,
            passphrase_clone.as_deref(),
        )?;
        
        // Busca ticker
        let ticker_data = client.fetch_ticker_sync(&market_symbol_clone)?;
        
        Ok::<serde_json::Value, String>(ticker_data)
    });
    
    let ticker_data = ticker_task.await
        .map_err(|e| format!("Task join error: {}", e))?
        .map_err(|e| format!("Failed to fetch ticker: {}", e))?;
    
    // 7. Monta resposta com todos os dados disponíveis
    Ok(serde_json::json!({
        "symbol": symbol_upper,
        "pair": market_symbol,
        "quote": "USDT",
        "exchange": {
            "id": exchange_id,
            "name": exchange_name,
            "ccxt_id": ccxt_id,
        },
        "ticker": ticker_data,
        "timestamp": chrono::Utc::now().timestamp_millis(),
        "datetime": chrono::Utc::now().to_rfc3339(),
    }))
}
