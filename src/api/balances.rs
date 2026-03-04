use actix_web::{web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use crate::{database::MongoDB, services::balance_service, middleware::auth::Claims};
use mongodb::bson::doc;

#[derive(Debug, Deserialize)]
pub struct BalanceQuery {
    pub user_id: String,
    #[serde(default)]
    pub use_summary: bool,
}

// Request body para POST /balances (envia credenciais do frontend)
#[derive(Debug, Deserialize, Serialize)]
pub struct FetchBalancesRequest {
    pub exchanges: Vec<ExchangeCredentials>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ExchangeCredentials {
    pub exchange_id: String,
    pub ccxt_id: String,
    pub name: String,
    pub api_key: String,
    pub api_secret: String,
    pub passphrase: Option<String>,
}

// /api/v1/balances (GET) - Fetch balances from MongoDB + CCXT
pub async fn get_balances(
    query: web::Query<BalanceQuery>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    log::info!("📊 GET /balances - user_id: {}", query.user_id);
    
    match balance_service::get_user_balances(&db, &query.user_id).await {
        Ok(response) => {
            log::info!("✅ Balances fetched from MongoDB: {} exchanges", response.exchanges.len());
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            log::error!("❌ Error fetching balances from MongoDB: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

// /api/v1/balances (POST) - Fetch balances from provided credentials (local-first)
pub async fn post_balances(
    body: web::Json<FetchBalancesRequest>,
) -> impl Responder {
    log::info!("� POST /balances - {} exchanges provided from frontend", body.exchanges.len());
    
    // Log das exchanges (sem mostrar credenciais completas)
    for ex in &body.exchanges {
        log::info!("  📍 Exchange: {} ({}) - id: {}", ex.name, ex.ccxt_id, ex.exchange_id);
    }
    
    if body.exchanges.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "error": "No exchanges provided"
        }));
    }
    
    // Converte ExchangeCredentials para DecryptedExchange
    let exchanges: Vec<crate::models::DecryptedExchange> = body.exchanges.iter().map(|e| {
        crate::models::DecryptedExchange {
            exchange_id: e.exchange_id.clone(),
            ccxt_id: e.ccxt_id.clone(),
            name: e.name.clone(),
            api_key: e.api_key.clone(),
            api_secret: e.api_secret.clone(),
            passphrase: e.passphrase.clone(),
            is_active: true,
        }
    }).collect();
    
    match balance_service::fetch_balances_from_exchanges(exchanges).await {
        Ok(response) => {
            log::info!("✅ Balances fetched from frontend credentials: {} exchanges", response.exchanges.len());
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            log::error!("❌ Error fetching balances from frontend credentials: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

// /api/v1/balances/secure (POST) - ✅ SECURE VERSION - Fetch balances from MongoDB using JWT
/// New secure endpoint that uses JWT to identify user and fetches credentials from MongoDB
/// Body is EMPTY - user identification comes from JWT token
/// 🚀 OTIMIZAÇÃO: Salva resultado em balance_cache para servir em /balances/cached
pub async fn post_balances_secure(
    user: web::ReqData<Claims>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;
    
    log::info!("🔐 POST /balances/secure - user {} (from JWT)", user_id);
    
    // Buscar exchanges do MongoDB (já descriptografadas)
    match crate::services::user_exchanges_service::get_user_exchanges_decrypted(&db, user_id).await {
        Ok(exchanges) => {
            if exchanges.is_empty() {
                log::warn!("⚠️ No exchanges found for user {}", user_id);
                return HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "total_usd": 0.0,
                    "exchanges": [],
                    "timestamp": chrono::Utc::now().timestamp()
                }));
            }
            
            log::info!("📊 Fetching balances from {} exchanges", exchanges.len());
            
            // Chamar serviço de balance
            match balance_service::fetch_balances_from_exchanges(exchanges).await {
                Ok(response) => {
                    log::info!("✅ Balances fetched: {} exchanges", response.exchanges.len());
                    
                    // 🚀 SAVE TO CACHE: Salva resposta completa em balance_cache (fire-and-forget)
                    let db_clone = db.clone();
                    let user_id_clone = user_id.to_string();
                    let response_json = serde_json::to_value(&response).unwrap_or_default();
                    tokio::spawn(async move {
                        if let Err(e) = save_balance_cache(&db_clone, &user_id_clone, &response_json).await {
                            log::warn!("⚠️ Failed to save balance cache: {}", e);
                        }
                    });
                    
                    HttpResponse::Ok().json(response)
                }
                Err(e) => {
                    log::error!("❌ Error fetching balances: {}", e);
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "success": false,
                        "error": e
                    }))
                }
            }
        }
        Err(e) => {
            log::error!("❌ Error fetching user exchanges: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

// ==================== 🚀 BALANCE CACHE (Stale-While-Revalidate) ====================

/// POST /api/v1/balances/cached - Retorna o último balance cacheado do MongoDB
/// Resposta instantânea (<100ms) - Sem chamar CCXT
/// O frontend usa isso para exibir dados imediatamente e depois chama /secure em background
pub async fn get_balances_cached(
    user: web::ReqData<Claims>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;
    
    log::info!("⚡ POST /balances/cached - user {} (instant cache)", user_id);
    
    let collection = db.collection::<mongodb::bson::Document>("balance_cache");
    
    let filter = doc! { "user_id": user_id.as_str() };
    
    match collection.find_one(filter).await {
        Ok(Some(cached_doc)) => {
            // Extrai o timestamp do cache para o frontend saber quão "stale" é
            let cached_at = cached_doc.get_i64("cached_at").unwrap_or(0);
            let age_seconds = chrono::Utc::now().timestamp() - cached_at;
            
            log::info!("✅ Cache hit for user {} (age: {}s)", user_id, age_seconds);
            
            // Retorna os dados cacheados com metadata
            let cached_data = cached_doc.get_document("data").cloned().unwrap_or_default();
            
            HttpResponse::Ok().json(serde_json::json!({
                "from_cache": true,
                "cached_at": cached_at,
                "age_seconds": age_seconds,
                "data": mongodb::bson::from_document::<serde_json::Value>(cached_data).unwrap_or_default()
            }))
        }
        Ok(None) => {
            log::info!("📭 No cache found for user {}", user_id);
            HttpResponse::Ok().json(serde_json::json!({
                "from_cache": true,
                "cached_at": null,
                "age_seconds": null,
                "data": null
            }))
        }
        Err(e) => {
            log::error!("❌ Error reading balance cache: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": format!("Cache read error: {}", e)
            }))
        }
    }
}

/// Salva a resposta de balance no MongoDB como cache
/// Collection: balance_cache (1 documento por user_id, upsert)
async fn save_balance_cache(
    db: &MongoDB,
    user_id: &str,
    response_data: &serde_json::Value,
) -> Result<(), String> {
    let collection = db.collection::<mongodb::bson::Document>("balance_cache");
    
    let now = chrono::Utc::now().timestamp();
    
    // Converte serde_json::Value para BSON Document
    let bson_data = mongodb::bson::to_document(response_data)
        .map_err(|e| format!("BSON conversion error: {}", e))?;
    
    let filter = doc! { "user_id": user_id };
    let update = doc! {
        "$set": {
            "user_id": user_id,
            "data": bson_data,
            "cached_at": now,
            "updated_at": mongodb::bson::DateTime::now(),
        }
    };
    
    let options = mongodb::options::UpdateOptions::builder()
        .upsert(true)
        .build();
    
    collection
        .update_one(filter, update)
        .with_options(options)
        .await
        .map_err(|e| format!("Failed to save balance cache: {}", e))?;
    
    log::info!("💾 Balance cache saved for user {} at {}", user_id, now);
    Ok(())
}

// GET /api/v1/balances/summary - Fast summary from CCXT
pub async fn get_balance_summary(
    query: web::Query<BalanceQuery>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    log::info!("📊 GET /balances/summary - user_id: {}", query.user_id);
    
    match balance_service::get_balance_summary(&db, &query.user_id).await {
        Ok(summary) => {
            log::info!("✅ Summary fetched");
            HttpResponse::Ok().json(summary)
        }
        Err(e) => {
            log::error!("❌ Error fetching summary: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

// GET /api/v1/balances/exchange/{id} - Balance from specific exchange via CCXT
pub async fn get_exchange_balance(
    db: web::Data<MongoDB>,
    path: web::Path<String>,
    query: web::Query<BalanceQuery>,
) -> HttpResponse {
    let exchange_id = path.into_inner();
    log::info!("📊 GET /balances/exchange/{} - user_id: {}", exchange_id, query.user_id);
    
    match balance_service::get_exchange_balance(&db, &query.user_id, &exchange_id).await {
        Ok(balance) => {
            log::info!("✅ Exchange balance retrieved");
            HttpResponse::Ok().json(balance)
        }
        Err(e) => {
            log::error!("❌ Failed to get exchange balance: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

// GET /api/v1/balances/market-movers - Top gainers/losers via CCXT
pub async fn get_market_movers(
    db: web::Data<MongoDB>,
    query: web::Query<BalanceQuery>,
) -> HttpResponse {
    log::info!("📈 GET /balances/market-movers - user_id: {}", query.user_id);
    
    match balance_service::get_market_movers(&db, &query.user_id).await {
        Ok(response) => {
            log::info!("✅ Market movers retrieved");
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            log::error!("❌ Failed to get market movers: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

