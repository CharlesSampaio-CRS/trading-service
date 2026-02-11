use actix_web::{web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use crate::{
    services::order_service,
    models::{
        DecryptedExchange,
        CreateOrderWithCredsRequest, CancelOrderWithCredsRequest,
    },
};

// ==================== ZERO DATABASE ARCHITECTURE ====================
// Orders operations via CCXT - NO MongoDB persistence needed
// Credentials come from frontend (decrypted from IndexedDB/WatermelonDB)

/// 🆕 Request body para POST /orders/fetch (com credenciais do frontend)
#[derive(Debug, Deserialize, Serialize)]
pub struct FetchOrdersRequest {
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

/// 🆕 POST /api/v1/orders/fetch - Fetch orders from exchanges with credentials from frontend
pub async fn fetch_orders_from_credentials(
    body: web::Json<FetchOrdersRequest>,
) -> impl Responder {
    log::info!("📊 POST /orders/fetch - {} exchanges", body.exchanges.len());
    
    // Converte para DecryptedExchange
    let exchanges: Vec<DecryptedExchange> = body.exchanges.iter().map(|e| {
        DecryptedExchange {
            exchange_id: e.exchange_id.clone(),
            ccxt_id: e.ccxt_id.clone(),
            name: e.name.clone(),
            api_key: e.api_key.clone(),
            api_secret: e.api_secret.clone(),
            passphrase: e.passphrase.clone(),
            is_active: true,
        }
    }).collect();
    
    match order_service::fetch_orders_from_exchanges(exchanges).await {
        Ok(response) => HttpResponse::Ok().json(response),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": e
        }))
    }
}

/// 🆕 POST /api/v1/orders/create-with-creds - Create order com credenciais do frontend
pub async fn create_order_with_creds(
    request: web::Json<CreateOrderWithCredsRequest>,
) -> impl Responder {
    log::info!("🛒 Creating order with frontend credentials");
    
    match order_service::create_order_with_creds(&request).await {
        Ok(response) => {
            if response.success {
                log::info!("✅ Order created successfully");
                HttpResponse::Ok().json(response)
            } else {
                log::warn!("⚠️ Order creation failed: {:?}", response.error);
                HttpResponse::BadRequest().json(response)
            }
        }
        Err(e) => {
            log::error!("❌ Error creating order: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

/// 🆕 POST /api/v1/orders/cancel-with-creds - Cancel order com credenciais do frontend
pub async fn cancel_order_with_creds(
    request: web::Json<CancelOrderWithCredsRequest>,
) -> impl Responder {
    log::info!("🚫 Canceling order with frontend credentials");
    
    match order_service::cancel_order_with_creds(&request).await {
        Ok(response) => {
            if response.success {
                log::info!("✅ Order canceled successfully");
                HttpResponse::Ok().json(response)
            } else {
                log::warn!("⚠️ Order cancellation failed: {:?}", response.error);
                HttpResponse::BadRequest().json(response)
            }
        }
        Err(e) => {
            log::error!("❌ Error canceling order: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}
