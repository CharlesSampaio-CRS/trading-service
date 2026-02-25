use actix_web::{web, HttpResponse};
use crate::{database::MongoDB, services::token_service};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TokenSearchQuery {
    pub q: String,
}

// POST body for token search with exchange credentials (single exchange)
#[derive(Debug, Deserialize)]
pub struct TokenSearchWithCredsRequest {
    pub query: String,
    pub exchange: token_service::ExchangeCredentials,
}

// POST body for multi-exchange token details comparison
#[derive(Debug, Deserialize)]
pub struct TokenDetailsMultiRequest {
    pub symbol: String,
    pub exchanges: Vec<token_service::ExchangeCredentials>,
}

#[derive(Deserialize)]
pub struct AvailableTokensQuery {
    pub exchange_id: String,
    pub quote: Option<String>,
}

#[derive(Deserialize)]
pub struct AvailableTokensByCcxtIdQuery {
    pub ccxt_id: String,
    pub quote: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/tokens",
    tag = "Tokens",
    responses(
        (status = 200, description = "List of all tokens"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn get_tokens(
    db: web::Data<MongoDB>,
) -> HttpResponse {
    log::info!("🪙 GET /tokens - Listing all tokens");
    
    match token_service::get_all_tokens(&db).await {
        Ok(response) => {
            log::info!("✅ Tokens retrieved: {}", response.count);
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            log::error!("❌ Failed to get tokens: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

pub async fn get_token(
    db: web::Data<MongoDB>,
    path: web::Path<String>,
) -> HttpResponse {
    let symbol = path.into_inner();
    log::info!("🪙 GET /tokens/{} - Getting token details", symbol);
    
    match token_service::get_token_by_symbol(&db, &symbol).await {
        Ok(response) => {
            if response.token.is_some() {
                log::info!("✅ Token {} found", symbol);
                HttpResponse::Ok().json(response)
            } else {
                log::warn!("⚠️ Token {} not found", symbol);
                HttpResponse::NotFound().json(serde_json::json!({
                    "success": false,
                    "error": "Token not found"
                }))
            }
        }
        Err(e) => {
            log::error!("❌ Failed to get token {}: {}", symbol, e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/tokens/search",
    tag = "Tokens",
    params(
        ("q" = String, Query, description = "Search query")
    ),
    responses(
        (status = 200, description = "Search results"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn search_tokens(
    db: web::Data<MongoDB>,
    query: web::Query<TokenSearchQuery>,
) -> HttpResponse {
    log::info!("🔍 GET /tokens/search?q={}", query.q);
    
    match token_service::search_tokens(&db, &query.q).await {
        Ok(response) => {
            log::info!("✅ Found {} tokens matching '{}'", response.count, query.q);
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            log::error!("❌ Failed to search tokens: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

pub async fn get_available_tokens(
    db: web::Data<MongoDB>,
    query: web::Query<AvailableTokensQuery>,
) -> HttpResponse {
    log::info!("🪙 GET /tokens/available - exchange_id: {}, quote: {:?}", 
        query.exchange_id, query.quote);
    
    // Validate exchange_id
    if query.exchange_id.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "error": "exchange_id is required as query parameter"
        }));
    }
    
    // Validate quote filter if provided
    if let Some(ref quote) = query.quote {
        let valid_quotes = vec!["USDT", "USD", "USDC", "BUSD", "BRL"];
        let quote_upper = quote.to_uppercase();
        if !valid_quotes.contains(&quote_upper.as_str()) {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "error": format!("Invalid quote filter. Must be one of: {}", valid_quotes.join(", "))
            }));
        }
    }
    
    match token_service::get_available_tokens(&db, &query.exchange_id, query.quote.as_deref()).await {
        Ok(response) => {
            log::info!("✅ Returned {} cached tokens", response.total_tokens);
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            log::error!("❌ Failed to get available tokens: {}", e);
            
            // Check if it's a "not found" error
            if e.contains("not available in cache") {
                return HttpResponse::ServiceUnavailable().json(serde_json::json!({
                    "success": false,
                    "error": "Token list not available in cache",
                    "message": "Please run the token update job first or wait for the nightly update",
                    "hint": "Run: python scripts/update_exchange_tokens.py"
                }));
            }
            
            if e.contains("not found") {
                return HttpResponse::NotFound().json(serde_json::json!({
                    "success": false,
                    "error": e
                }));
            }
            
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

// GET /tokens/by-ccxt - Get available tokens by CCXT ID (binance, bybit, etc)
pub async fn get_available_tokens_by_ccxt(
    db: web::Data<MongoDB>,
    query: web::Query<AvailableTokensByCcxtIdQuery>,
) -> HttpResponse {
    log::info!("🪙 GET /tokens/by-ccxt - ccxt_id: {}, quote: {:?}", 
        query.ccxt_id, query.quote);
    
    // Validate ccxt_id
    if query.ccxt_id.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "error": "ccxt_id is required as query parameter"
        }));
    }
    
    match token_service::get_available_tokens_by_ccxt(&db, &query.ccxt_id, query.quote.as_deref()).await {
        Ok(response) => {
            log::info!("✅ Returned {} cached tokens for {}", response.total_tokens, query.ccxt_id);
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            log::error!("❌ Failed to get available tokens by ccxt_id: {}", e);
            
            if e.contains("not available in cache") {
                return HttpResponse::ServiceUnavailable().json(serde_json::json!({
                    "success": false,
                    "error": "Token list not available in cache",
                    "message": "Token cache not found for this exchange",
                    "ccxt_id": query.ccxt_id,
                    "hint": "The exchange tokens may not have been cached yet"
                }));
            }
            
            if e.contains("not found") {
                return HttpResponse::NotFound().json(serde_json::json!({
                    "success": false,
                    "error": e
                }));
            }
            
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

// ============================================================================
// TOKEN DETAILS WITH CREDENTIALS - ZERO DATABASE PATTERN
// ============================================================================
// POST /tokens/details - Busca detalhes do token usando credenciais do frontend
pub async fn get_token_details_with_creds(
    body: web::Json<token_service::GetTokenDetailsRequest>,
) -> HttpResponse {
    log::info!("🪙 POST /tokens/details - symbol: {}, exchange: {}", 
        body.symbol, body.exchange.name);
    
    match token_service::get_token_details_with_creds(&body).await {
        Ok(response) => {
            log::info!("✅ Token details retrieved for {}", body.symbol);
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            log::error!("❌ Failed to get token details: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

// ============================================================================
// TOKEN SEARCH WITH CREDENTIALS - LOCAL-FIRST PATTERN
// ============================================================================
// POST /tokens/search - Search tokens using exchange credentials from frontend
pub async fn post_token_search(
    body: web::Json<TokenSearchWithCredsRequest>,
) -> HttpResponse {
    let query = body.query.trim();
    if query.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "error": "Query cannot be empty"
        }));
    }

    log::info!("🔍 POST /tokens/search - query: {}, exchange: {} ({})",
        query, body.exchange.name, body.exchange.ccxt_id);

    match token_service::search_tokens_with_creds(query, &body.exchange).await {
        Ok(response) => {
            log::info!("✅ Found {} tokens for '{}' via {}", 
                response.count, query, body.exchange.ccxt_id);
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            log::error!("❌ Token search failed: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

// ============================================================================
// MULTI-EXCHANGE TOKEN DETAILS - PRICE COMPARISON & ARBITRAGE
// ============================================================================
// POST /tokens/details/multi - Get token details from multiple exchanges simultaneously
pub async fn get_token_details_multi(
    body: web::Json<TokenDetailsMultiRequest>,
) -> HttpResponse {
    if body.exchanges.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "error": "At least one exchange is required"
        }));
    }

    log::info!("🔍 POST /tokens/details/multi - symbol: {}, exchanges: {}",
        body.symbol,
        body.exchanges.iter().map(|e| e.name.as_str()).collect::<Vec<_>>().join(", "));

    match token_service::get_token_details_multi(&body.symbol, &body.exchanges).await {
        Ok(response) => {
            log::info!("✅ Retrieved {} from {} exchanges", body.symbol, response.exchanges.len());
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            log::error!("❌ Multi-exchange token details failed: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}
