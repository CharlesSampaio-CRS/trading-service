mod api;
mod ccxt;
mod database;
mod jobs;
mod middleware;
mod models;
mod seeds;
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
    
    // 🌱 Seed default strategy templates
    seeds::strategy_templates_seed::seed_default_templates(&db).await;
    
    // 📅 Start daily snapshot scheduler
    log::info!("📅 Starting background jobs...");
    jobs::snapshot_scheduler::start_daily_snapshot_scheduler(db.clone()).await;
    log::info!("✅ Background jobs started");
    
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
            
            // User Exchanges: Manage connected exchanges (CRUD) - Requires JWT
            .service(
                web::scope("/api/v1/user/exchanges")
                    .wrap(middleware::auth::AuthMiddleware)
                    .route("", web::post().to(api::user_exchanges::add_exchange))
                    .route("", web::get().to(api::user_exchanges::list_exchanges))
                    .route("/{exchange_id}", web::patch().to(api::user_exchanges::update_exchange))
                    .route("/{exchange_id}", web::delete().to(api::user_exchanges::delete_exchange))
            )
            
            // Snapshots: Daily balance snapshots for PNL calculation
            .service(
                web::scope("/api/v1/snapshots")
                    .wrap(middleware::auth::AuthMiddleware)
                    .route("/save", web::post().to(api::snapshots::save_snapshot))
                    .route("", web::get().to(api::snapshots::get_snapshots))
            )
            
            // Strategies: Trading strategies management
            .service(
                web::scope("/api/v1/strategies")
                    .wrap(middleware::auth::AuthMiddleware)
                    .service(api::strategies::get_strategies)
                    .service(api::strategies::get_strategy)
                    .service(api::strategies::get_strategy_stats)
                    .service(api::strategies::get_strategy_executions)
                    .service(api::strategies::get_strategy_signals)
                    .service(api::strategies::create_strategy)
                    .service(api::strategies::update_strategy)
                    .service(api::strategies::delete_strategy)
            )
            
            // Strategy Templates: Independent template management
            .service(
                web::scope("/api/v1/strategy-templates")
                    .wrap(middleware::auth::AuthMiddleware)
                    .service(api::strategy_templates::get_templates)
                    .service(api::strategy_templates::get_template)
                    .service(api::strategy_templates::create_template)
                    .service(api::strategy_templates::update_template)
                    .service(api::strategy_templates::delete_template)
            )
            
            // Balances: Real-time from exchanges via CCXT
            .service(
                web::scope("/api/v1/balances")
                    // Public endpoints (no JWT required)
                    .route("", web::get().to(api::balances::get_balances))
                    .route("", web::post().to(api::balances::post_balances))
                    .route("/summary", web::get().to(api::balances::get_balance_summary))
                    .route("/exchange/{id}", web::get().to(api::balances::get_exchange_balance))
                    .route("/market-movers", web::get().to(api::balances::get_market_movers))
                    // Protected endpoint requiring JWT authentication
                    .service(
                        web::resource("/secure")
                            .wrap(middleware::auth::AuthMiddleware)
                            .route(web::post().to(api::balances::post_balances_secure))
                    )
            )
            
            // ==================== ORDERS API ====================
            // Zero Database Architecture - Orders fetched directly from exchanges via CCXT
            // All endpoints require JWT authentication
            .service(
                web::scope("/api/v1/orders")
                    .wrap(middleware::auth::AuthMiddleware)
                    // 📊 Fetch orders from user's exchanges
                    .route("/fetch/secure", web::post().to(api::orders::fetch_orders_secure))
                    // ➕ Create new order
                    .route("/create", web::post().to(api::orders::create_order_secure))
                    // ❌ Cancel existing order
                    .route("/cancel", web::post().to(api::orders::cancel_order_secure))
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

