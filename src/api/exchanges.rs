use actix_web::{web, HttpResponse};
use crate::database::MongoDB;
use crate::services::exchange_service::{self, AvailableExchangesResponse};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TokenDetailsQuery {
    user_id: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/exchanges/available",
    tag = "Exchanges",
    responses(
        (status = 200, description = "List of available exchanges", body = AvailableExchangesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn get_available_exchanges(db: web::Data<MongoDB>) -> HttpResponse {
    log::info!("📋 GET /exchanges/available - fetching catalog");
    
    match exchange_service::get_available_exchanges(&db).await {
        Ok(response) => {
            log::info!("✅ Retrieved {} available exchanges", response.exchanges.len());
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            log::error!("❌ Error fetching available exchanges: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

/// GET /api/v1/exchanges/{exchange_id}/token/{symbol}?user_id={user_id}
/// Busca detalhes completos do token via CCXT
pub async fn get_token_details(
    db: web::Data<MongoDB>,
    path: web::Path<(String, String)>,
    query: web::Query<TokenDetailsQuery>,
) -> HttpResponse {
    let (exchange_id, symbol) = path.into_inner();
    let user_id = &query.user_id;
    
    log::info!("🪙 GET /exchanges/{}/token/{} (user: {})", exchange_id, symbol, user_id);
    
    match exchange_service::get_token_details(&db, user_id, &exchange_id, &symbol).await {
        Ok(token_data) => {
            log::info!("✅ Token details retrieved for {}", symbol);
            HttpResponse::Ok().json(token_data)
        }
        Err(e) => {
            log::error!("❌ Error fetching token details: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}