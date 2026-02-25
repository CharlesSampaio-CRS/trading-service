use utoipa::OpenApi;
use utoipa::openapi::security::{SecurityScheme, HttpAuthScheme, HttpBuilder};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Trading Service API - Core System",
        version = "1.0.0",
        description = "Complete API documentation for Trading Service. \n\n**Authentication:** Most endpoints require JWT Bearer token authentication.\n\n**Features:**\n- Multi-provider authentication (Local, Google, Apple)\n- Exchange catalog management\n- Token catalog and search\n- CoinGecko integration\n- Real-time exchange rates\n- Health monitoring and metrics",
        contact(
            name = "Trading Service Team",
            email = "support@trading-service.com"
        )
    ),
    paths(
        // Auth endpoints
        crate::api::auth::login,
        crate::api::auth::register,
        crate::api::auth::verify_token,
        crate::api::auth::get_me,
        
        // Health & Metrics
        crate::api::health::health_check,
        crate::api::metrics::get_metrics,
        
        // Exchanges
        crate::api::exchanges::get_available_exchanges,
        
        // Tokens
        crate::api::tokens::get_tokens,
        crate::api::tokens::search_tokens,
        
        // External APIs
        crate::api::external::get_token_info,
        crate::api::external::search_token,
        crate::api::external::get_exchange_rate,
    ),
    components(
        schemas(
            // Auth
            crate::services::auth_service::LoginRequest,
            crate::services::auth_service::RegisterRequest,
            crate::services::auth_service::AuthResponse,
            crate::services::auth_service::UserInfo,
            crate::services::auth_service::VerifyTokenResponse,
            
            // Health & Metrics
            crate::api::health::HealthResponse,
            crate::api::metrics::MetricsResponse,
            
            // Exchanges
            crate::services::exchange_service::AvailableExchangesResponse,
            crate::services::exchange_service::ExchangeCatalogInfo,
        )
    ),
    tags(
        (name = "Auth", description = "Authentication and user management endpoints. Supports local (email/password), Google, and Apple authentication."),
        (name = "Health", description = "Health check and system metrics endpoints for monitoring service status."),
        (name = "Exchanges", description = "Exchange catalog endpoints. List available cryptocurrency exchanges and their capabilities."),
        (name = "Tokens", description = "Token catalog and search endpoints. Browse and search cryptocurrency tokens."),
        (name = "External", description = "External API integrations. CoinGecko for token data and exchange rates for currency conversion."),
    ),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .description(Some("Enter your JWT token"))
                        .build()
                ),
            );
        }
    }
}
