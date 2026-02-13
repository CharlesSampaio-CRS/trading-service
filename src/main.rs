mod api;
mod ccxt;
mod database;
mod middleware;
mod models;
mod services;
mod utils;

use actix_cors::Cors;
use actix_web::{middleware::Logger, web, App, HttpServer};
use dotenv::dotenv;
use std::env;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Load environment variables
    dotenv().ok();
    
    // Initialize logger
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));
    
    // Get configuration from environment
    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = env::var("PORT").unwrap_or_else(|_| "3002".to_string());
    let database_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");
    
    log::info!("🚀 Starting Trading Service...");
    log::info!("📊 Database: {}", database_url);
    
    // 🚀 FASE 3: Pré-aquecer Python GIL e CCXT
    log::info!("🐍 Pre-warming Python GIL and CCXT...");
    let warmup_result = tokio::task::spawn_blocking(|| {
        use pyo3::prelude::*;
        Python::with_gil(|py| {
            // Import CCXT para pré-carregar módulo
            match py.import("ccxt") {
                Ok(_) => {
                    log::info!("   ✅ CCXT module loaded");
                }
                Err(e) => {
                    log::warn!("   ⚠️  CCXT pre-load warning: {}", e);
                }
            }
        })
    }).await;
    
    match warmup_result {
        Ok(_) => log::info!("✅ Python GIL pre-warmed successfully"),
        Err(e) => log::warn!("⚠️  Python warmup warning: {}", e),
    }
    
    // Initialize MongoDB connection
    let db = database::MongoDB::new(&database_url)
        .await
        .expect("Failed to connect to MongoDB");
    
    let db_data = web::Data::new(db.clone());
    
    log::info!("✅ MongoDB connected successfully");
    log::info!("🌐 Server starting on {}:{}", host, port);
    log::info!("📚 Swagger UI available at: http://{}:{}/swagger-ui/", host, port);
    log::info!("📄 OpenAPI spec at: http://{}:{}/api-docs/openapi.json", host, port);
    
    // Start HTTP server
    HttpServer::new(move || {
        let cors = Cors::default()
            .allowed_origin("http://localhost:3000") // Frontend Web (Expo)
            .allowed_origin("http://localhost:8081")
            .allowed_origin("http://localhost:19006")
            .allowed_origin("http://127.0.0.1:3000")
            .allowed_origin("http://127.0.0.1:8081")
            .allowed_origin("http://127.0.0.1:19006")
            .allowed_methods(vec!["GET", "POST", "PUT", "DELETE", "OPTIONS"])
            .allowed_headers(vec![
                actix_web::http::header::AUTHORIZATION,
                actix_web::http::header::CONTENT_TYPE,
                actix_web::http::header::ACCEPT,
                actix_web::http::header::CACHE_CONTROL,
                actix_web::http::header::PRAGMA,
            ])
            .expose_headers(vec![
                actix_web::http::header::CONTENT_TYPE,
            ])
            .supports_credentials()
            .max_age(3600);
        
        // Generate OpenAPI specification
        let openapi = api::swagger::ApiDoc::openapi();
        
        App::new()
            .app_data(db_data.clone())
            .wrap(cors)
            .wrap(middleware::SecurityHeaders)
            .wrap(Logger::default())
            .wrap(Logger::new("%a %{User-Agent}i"))
            // Swagger UI with authentication
            .service(
                SwaggerUi::new("/swagger-ui/{_:.*}")
                    .url("/api-docs/openapi.json", openapi.clone())
            )
            // Health check
            .route("/health", web::get().to(api::health::health_check))
            // Metrics
            .route("/metrics", web::get().to(api::metrics::get_metrics))
            // Auth endpoints
            .service(
                web::scope("/api/v1/auth")
                    .route("/login", web::post().to(api::auth::login))
                    .route("/register", web::post().to(api::auth::register))                    
                    .route("/refresh", web::post().to(api::auth::refresh_token))
                    .route("/google", web::get().to(api::auth::google_auth))
                    .route("/callback", web::get().to(api::auth::google_callback))
                    .route("/verify", web::get().to(api::auth::verify_token))
                    .route("/me", web::get().to(api::auth::get_me))
                    .route("/delete-account", web::delete().to(api::auth::delete_account))
            )
            
            // ==================== CATALOG DATA (MongoDB) ====================
            
            // Exchanges: User credentials (READ ONLY)
            .service(
                web::scope("/api/v1/exchanges")
                    .route("/available", web::get().to(api::exchanges::get_available_exchanges))
                    .route("/{exchange_id}/token/{symbol}", web::get().to(api::exchanges::get_token_details))
            )
            
            // Tokens: Global catalog (READ ONLY)
            .service(
                web::scope("/api/v1/tokens")
                    .route("", web::get().to(api::tokens::get_tokens))
                    .route("/available", web::get().to(api::tokens::get_available_tokens))
                    .route("/by-ccxt", web::get().to(api::tokens::get_available_tokens_by_ccxt))  // Get tokens by CCXT ID
                    .route("/search", web::get().to(api::tokens::search_tokens))
                    .route("/search", web::post().to(api::tokens::post_token_search))  // Local-first: receives credentials
                    .route("/details", web::post().to(api::tokens::get_token_details_with_creds))  // Zero Database: receives credentials
                    .route("/details/multi", web::post().to(api::tokens::get_token_details_multi))  // Multi-exchange comparison
                    .route("/{symbol}", web::get().to(api::tokens::get_token))  // DEVE FICAR POR ÚLTIMO (catch-all)
            )
            
            // ==================== CCXT REAL-TIME DATA ====================
            
            // Balances: Real-time from exchanges via CCXT
            .service(
                web::scope("/api/v1/balances")
                    // Endpoint unificado: GET (MongoDB) ou POST (credenciais do frontend)
                    .route("", web::get().to(api::balances::get_balances))
                    .route("", web::post().to(api::balances::post_balances))
                    .route("/summary", web::get().to(api::balances::get_balance_summary))
                    .route("/exchange/{id}", web::get().to(api::balances::get_exchange_balance))
                    .route("/market-movers", web::get().to(api::balances::get_market_movers))
            )
            
            // Orders: Create/Cancel on exchanges via CCXT (Zero Database Architecture)
            .service(
                web::scope("/api/v1/orders")
                    .route("/fetch", web::post().to(api::orders::fetch_orders_from_credentials)) // ✅ Buscar orders com creds do frontend
                    .route("/create-with-creds", web::post().to(api::orders::create_order_with_creds)) // ✅ Criar com creds do frontend
                    .route("/cancel-with-creds", web::post().to(api::orders::cancel_order_with_creds)) // ✅ Cancelar com creds do frontend
            )
            
            // Tickers: Real-time prices via CCXT
            .service(
                web::scope("/api/v1/tickers")
                    .route("", web::get().to(api::tickers::get_tickers))
            )
            
            // ==================== EXTERNAL APIs ====================
            
            // CoinGecko: Token info and prices
            .service(
                web::scope("/api/v1/external/token")
                    .route("/info", web::get().to(api::external::get_token_info))
                    .route("/search", web::get().to(api::external::search_token))
                    .route("/prices", web::get().to(api::external::get_batch_prices))
            )
            
            // Exchange Rates: Currency conversion
            .service(
                web::scope("/api/v1/external")
                    .route("/exchange-rate", web::get().to(api::external::get_exchange_rate))
                    .route("/convert", web::get().to(api::external::convert_currency))
                    .route("/rates", web::get().to(api::external::get_all_rates))
            )
    })
    .bind(format!("{}:{}", host, port))?
    .run()
    .await
}

