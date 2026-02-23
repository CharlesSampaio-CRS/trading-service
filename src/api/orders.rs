use actix_web::{web, HttpResponse, Responder};
use serde::Deserialize;
use crate::{
    services::order_service,
    models::{
        DecryptedExchange,
        CreateOrderWithCredsRequest, 
        CancelOrderWithCredsRequest,
    },
    middleware::auth::Claims,
    database::MongoDB,
};

// ==================== ORDERS API - ZERO DATABASE ARCHITECTURE ====================
// Orders são buscadas diretamente das exchanges via CCXT
// Nenhuma persistência em MongoDB - apenas cache temporário se necessário
// Credenciais vêm do MongoDB (descriptografadas) usando JWT

// ============================================================================
// 📊 FETCH ORDERS - Buscar ordens abertas
// ============================================================================

/// 🔒 POST /api/v1/orders/fetch/secure
/// Busca orders usando JWT - credenciais vêm do MongoDB
/// Body: vazio (user_id vem do JWT)
pub async fn fetch_orders_secure(
    user: web::ReqData<Claims>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;
    
    log::info!("🔐 Fetching orders for user {}", user_id);
    
    // 1. Buscar exchanges do MongoDB (descriptografadas)
    let exchanges = match crate::services::user_exchanges_service::get_user_exchanges_decrypted(&db, user_id).await {
        Ok(exs) => exs,
        Err(e) => {
            log::error!("❌ Error fetching exchanges: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": format!("Error fetching exchanges: {}", e)
            }));
        }
    };
    
    if exchanges.is_empty() {
        log::info!("⚠️ No exchanges found for user {}", user_id);
        return HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "orders": [],
            "count": 0
        }));
    }
    
    log::info!("📊 Fetching orders from {} exchanges", exchanges.len());
    
    // 2. Buscar orders via CCXT
    match order_service::fetch_orders_from_exchanges(exchanges).await {
        Ok(response) => {
            log::info!("✅ Fetched {} orders", response.count);
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            log::error!("❌ Error fetching orders: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

// ============================================================================
// ➕ CREATE ORDER - Criar nova ordem
// ============================================================================

/// 🔒 POST /api/v1/orders/create
/// Cria ordem usando JWT - credenciais vêm do MongoDB
#[derive(Debug, Deserialize)]
pub struct CreateOrderRequest {
    pub exchange_id: String,     // MongoDB ID da exchange
    pub symbol: String,           // Ex: "BTC/USDT"
    pub order_type: String,       // "market" ou "limit"
    pub side: String,             // "buy" ou "sell"
    pub amount: f64,              // Quantidade
    pub price: Option<f64>,       // Preço (obrigatório para limit orders)
}

pub async fn create_order_secure(
    user: web::ReqData<Claims>,
    db: web::Data<MongoDB>,
    request: web::Json<CreateOrderRequest>,
) -> impl Responder {
    let user_id = &user.sub;
    
    log::info!("🔒 Creating {} {} order for {} on exchange {}", 
        request.side, request.order_type, request.symbol, request.exchange_id);
    
    // 1. Buscar exchanges do MongoDB
    let exchanges = match crate::services::user_exchanges_service::get_user_exchanges_decrypted(&db, user_id).await {
        Ok(exs) => exs,
        Err(e) => {
            log::error!("❌ Error fetching exchanges: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": format!("Error fetching exchanges: {}", e)
            }));
        }
    };
    
    // 2. Encontrar a exchange específica
    let exchange = match exchanges.iter().find(|ex| ex.exchange_id == request.exchange_id) {
        Some(ex) => ex,
        None => {
            log::error!("❌ Exchange not found: {}", request.exchange_id);
            return HttpResponse::NotFound().json(serde_json::json!({
                "success": false,
                "error": format!("Exchange not found: {}", request.exchange_id)
            }));
        }
    };
    
    log::info!("✅ Found exchange {} ({})", exchange.name, exchange.ccxt_id);
    
    // 3. Criar ordem via CCXT
    let create_request = CreateOrderWithCredsRequest {
        ccxt_id: exchange.ccxt_id.clone(),
        exchange_name: exchange.name.clone(),
        api_key: exchange.api_key.clone(),
        api_secret: exchange.api_secret.clone(),
        passphrase: exchange.passphrase.clone(),
        symbol: request.symbol.clone(),
        order_type: request.order_type.clone(),
        side: request.side.clone(),
        amount: request.amount,
        price: request.price,
    };
    
    match order_service::create_order_with_creds(&create_request).await {
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

// ============================================================================
// ❌ CANCEL ORDER - Cancelar ordem
// ============================================================================

/// 🔒 POST /api/v1/orders/cancel
/// Cancela ordem usando JWT - credenciais vêm do MongoDB
#[derive(Debug, Deserialize)]
pub struct CancelOrderRequest {
    pub exchange_id: String,  // MongoDB ID da exchange
    pub symbol: String,       // Par de negociação
    pub order_id: String,     // ID da ordem
}

pub async fn cancel_order_secure(
    user: web::ReqData<Claims>,
    db: web::Data<MongoDB>,
    request: web::Json<CancelOrderRequest>,
) -> impl Responder {
    let user_id = &user.sub;
    
    log::info!("🔒 Canceling order {} for {} on exchange {}", 
        request.order_id, request.symbol, request.exchange_id);
    
    // 1. Buscar exchanges do MongoDB
    let exchanges = match crate::services::user_exchanges_service::get_user_exchanges_decrypted(&db, user_id).await {
        Ok(exs) => exs,
        Err(e) => {
            log::error!("❌ Error fetching exchanges: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": format!("Error fetching exchanges: {}", e)
            }));
        }
    };
    
    // 2. Encontrar a exchange específica
    let exchange = match exchanges.iter().find(|ex| ex.exchange_id == request.exchange_id) {
        Some(ex) => ex,
        None => {
            log::error!("❌ Exchange not found: {}", request.exchange_id);
            return HttpResponse::NotFound().json(serde_json::json!({
                "success": false,
                "error": format!("Exchange not found: {}", request.exchange_id)
            }));
        }
    };
    
    log::info!("✅ Found exchange {} ({})", exchange.name, exchange.ccxt_id);
    
    // 3. Cancelar ordem via CCXT
    let cancel_request = CancelOrderWithCredsRequest {
        ccxt_id: exchange.ccxt_id.clone(),
        exchange_name: exchange.name.clone(),
        api_key: exchange.api_key.clone(),
        api_secret: exchange.api_secret.clone(),
        passphrase: exchange.passphrase.clone(),
        symbol: Some(request.symbol.clone()),
        order_id: request.order_id.clone(),
    };
    
    match order_service::cancel_order_with_creds(&cancel_request).await {
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
