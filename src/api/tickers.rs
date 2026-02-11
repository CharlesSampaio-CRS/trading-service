use actix_web::{web, HttpResponse, Responder};
use serde::Deserialize;
use crate::{
    database::MongoDB,
    services::ticker_service,
};

#[derive(Debug, Deserialize)]
pub struct TickerQuery {
    pub user_id: String,
    pub symbols: String, // Comma-separated: BTC/USDT,ETH/USDT
}

// GET /api/v1/tickers?user_id=xxx&symbols=BTC/USDT,ETH/USDT
pub async fn get_tickers(
    query: web::Query<TickerQuery>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    log::info!("📈 GET /tickers - symbols: {}", query.symbols);

    let symbols: Vec<String> = query.symbols
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    match ticker_service::get_tickers(&db, &query.user_id, symbols).await {
        Ok(response) => {
            log::info!("✅ Fetched {} tickers", response.count);
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            log::error!("❌ Error fetching tickers: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}
