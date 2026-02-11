use actix_web::{web, HttpResponse};
use serde::Deserialize;
use crate::services::{coingecko_service, exchange_rate_service};

#[derive(Deserialize)]
pub struct TokenInfoQuery {
    pub coingecko_id: String,
}

#[derive(Deserialize)]
pub struct TokenSearchQuery {
    pub symbol: String,
}

#[derive(Deserialize)]
pub struct BatchPricesQuery {
    pub ids: String, // Comma-separated coingecko IDs
}

#[derive(Deserialize)]
pub struct ExchangeRateQuery {
    pub from: String,
    pub to: String,
}

#[derive(Deserialize)]
pub struct ConvertQuery {
    pub from: String,
    pub to: String,
    pub amount: f64,
}

#[derive(Deserialize)]
pub struct AllRatesQuery {
    pub base: String,
}

/// GET /api/v1/external/token/info?coingecko_id=bitcoin
/// Retorna informações detalhadas de um token do CoinGecko
pub async fn get_token_info(
    query: web::Query<TokenInfoQuery>,
) -> HttpResponse {
    log::info!("🦎 GET /external/token/info?coingecko_id={}", query.coingecko_id);

    match coingecko_service::get_token_info_from_coingecko(&query.coingecko_id).await {
        Ok(info) => {
            log::info!("✅ Token info retrieved: {} ({})", info.name, info.symbol);
            HttpResponse::Ok().json(info)
        }
        Err(e) => {
            log::error!("❌ Failed to get token info: {}", e);
            
            if e.contains("404") || e.contains("not found") {
                return HttpResponse::NotFound().json(serde_json::json!({
                    "success": false,
                    "error": format!("Token '{}' not found on CoinGecko", query.coingecko_id)
                }));
            }
            
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

/// GET /api/v1/external/token/search?symbol=BTC
/// Busca tokens no CoinGecko por símbolo
pub async fn search_token(
    query: web::Query<TokenSearchQuery>,
) -> HttpResponse {
    log::info!("🔍 GET /external/token/search?symbol={}", query.symbol);

    match coingecko_service::search_token_by_symbol(&query.symbol).await {
        Ok(results) => {
            log::info!("✅ Found {} results for '{}'", results.len(), query.symbol);
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "results": results,
                "count": results.len()
            }))
        }
        Err(e) => {
            log::error!("❌ Failed to search token: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

/// GET /api/v1/external/token/prices?ids=bitcoin,ethereum,cardano
/// Retorna preços USD de múltiplos tokens (batch)
pub async fn get_batch_prices(
    query: web::Query<BatchPricesQuery>,
) -> HttpResponse {
    let ids: Vec<String> = query.ids
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    log::info!("💰 GET /external/token/prices - {} tokens", ids.len());

    if ids.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "error": "No token IDs provided"
        }));
    }

    if ids.len() > 100 {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "error": "Maximum 100 tokens per request"
        }));
    }

    match coingecko_service::get_prices_from_coingecko(ids).await {
        Ok(prices) => {
            log::info!("✅ Retrieved {} prices", prices.len());
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "prices": prices,
                "count": prices.len()
            }))
        }
        Err(e) => {
            log::error!("❌ Failed to get prices: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

/// GET /api/v1/external/exchange-rate?from=USD&to=BRL
/// Retorna taxa de câmbio entre duas moedas
pub async fn get_exchange_rate(
    query: web::Query<ExchangeRateQuery>,
) -> HttpResponse {
    log::info!("💱 GET /external/exchange-rate?from={}&to={}", query.from, query.to);

    match exchange_rate_service::get_exchange_rate(&query.from, &query.to).await {
        Ok(rate) => {
            log::info!("✅ Exchange rate {}/{}: {:.4}", query.from, query.to, rate);
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "from": query.from.to_uppercase(),
                "to": query.to.to_uppercase(),
                "rate": rate,
                "last_updated": chrono::Utc::now().to_rfc3339()
            }))
        }
        Err(e) => {
            log::error!("❌ Failed to get exchange rate: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

/// GET /api/v1/external/convert?from=USD&to=BRL&amount=100
/// Converte valor de uma moeda para outra
pub async fn convert_currency(
    query: web::Query<ConvertQuery>,
) -> HttpResponse {
    log::info!("💱 GET /external/convert?from={}&to={}&amount={}", 
        query.from, query.to, query.amount);

    if query.amount <= 0.0 {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "error": "Amount must be greater than 0"
        }));
    }

    match exchange_rate_service::convert_currency(&query.from, &query.to, query.amount).await {
        Ok(conversion) => {
            log::info!("✅ Converted: {:.2} {} = {:.2} {}", 
                query.amount, query.from, conversion.converted.unwrap(), query.to);
            HttpResponse::Ok().json(conversion)
        }
        Err(e) => {
            log::error!("❌ Failed to convert currency: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

/// GET /api/v1/external/rates?base=USD
/// Retorna todas as taxas de câmbio baseadas em uma moeda
pub async fn get_all_rates(
    query: web::Query<AllRatesQuery>,
) -> HttpResponse {
    log::info!("💱 GET /external/rates?base={}", query.base);

    match exchange_rate_service::get_all_rates(&query.base).await {
        Ok(rates) => {
            log::info!("✅ Retrieved {} rates for {}", rates.rates.len(), query.base);
            HttpResponse::Ok().json(rates)
        }
        Err(e) => {
            log::error!("❌ Failed to get rates: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}
