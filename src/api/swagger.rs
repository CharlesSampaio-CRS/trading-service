use utoipa::OpenApi;
use utoipa::openapi::security::{SecurityScheme, HttpAuthScheme, HttpBuilder};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Trading Service API",
        version = "1.0.0",
        description = "API for cryptocurrency trading management with multiple exchanges integration via CCXT",
        contact(
            name = "Trading Service Team",
            email = "support@trading-service.com"
        )
    ),
    paths(
        // Auth endpoints
        crate::api::auth::login,
        crate::api::auth::register,
        
        // Health & Metrics
        crate::api::health::health_check,
        crate::api::metrics::get_metrics,
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
        )
    ),
    tags(
        (name = "Auth", description = "Authentication and user management endpoints"),
        (name = "Health", description = "Health check and system metrics"),
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
