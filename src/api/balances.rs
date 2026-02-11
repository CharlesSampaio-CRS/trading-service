use actix_web::{web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use crate::{database::MongoDB, services::balance_service};

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

