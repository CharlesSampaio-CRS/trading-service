use actix_web::{web, HttpResponse, HttpRequest};
use crate::{database::MongoDB, services::auth_service};
use crate::services::auth_service::{LoginRequest, RegisterRequest, AuthResponse, UserInfo};
use base64::Engine;
use utoipa::OpenApi;

#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    tag = "Auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful", body = AuthResponse),
        (status = 401, description = "Invalid credentials")
    )
)]
pub async fn login(
    db: web::Data<MongoDB>,
    request: web::Json<auth_service::LoginRequest>,
) -> HttpResponse {
    log::info!("🔐 POST /auth/login - email: {}", request.email);
    
    match auth_service::login(&db, &request).await {
        Ok(response) => {
            log::info!("✅ Login successful: {}", request.email);
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            log::warn!("❌ Login failed: {} - {}", request.email, e);
            HttpResponse::Unauthorized().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/register",
    tag = "Auth",
    request_body = RegisterRequest,
    responses(
        (status = 201, description = "Registration successful", body = AuthResponse),
        (status = 400, description = "Invalid request or user already exists")
    )
)]
pub async fn register(
    db: web::Data<MongoDB>,
    request: web::Json<auth_service::RegisterRequest>,
) -> HttpResponse {
    let email_str = request.email.as_deref().unwrap_or("N/A");
    let provider = request.provider.as_deref().unwrap_or("local");
    log::info!("📝 POST /auth/register - email: {}, provider: {}", email_str, provider);
    
    match auth_service::register(&db, &request).await {
        Ok(response) => {
            log::info!("✅ Registration successful: {}", email_str);
            HttpResponse::Created().json(response)
        }
        Err(e) => {
            log::warn!("❌ Registration failed: {} - {}", email_str, e);
            HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

pub async fn refresh_token(
    db: web::Data<MongoDB>,
    request: web::Json<auth_service::RefreshTokenRequest>,
) -> HttpResponse {
    log::info!("🔄 POST /auth/refresh");
    
    match auth_service::refresh_token(&db, &request).await {
        Ok(response) => {
            log::info!("✅ Token refreshed");
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            log::warn!("❌ Token refresh failed: {}", e);
            HttpResponse::Unauthorized().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/verify",
    tag = "Auth",
    responses(
        (status = 200, description = "Token is valid"),
        (status = 401, description = "Invalid or expired token")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn verify_token(
    req: HttpRequest,
) -> HttpResponse {
    log::info!("✓ GET /auth/verify");
    
    // Extract token from Authorization header
    let auth_header = req.headers().get("Authorization");
    
    if let Some(auth_value) = auth_header {
        if let Ok(auth_str) = auth_value.to_str() {
            if auth_str.starts_with("Bearer ") {
                let token = &auth_str[7..];
                
                match auth_service::verify_token(token) {
                    Ok(claims) => {
                        log::info!("✅ Token valid for user: {}", claims.sub);
                        return HttpResponse::Ok().json(serde_json::json!({
                            "success": true,
                            "valid": true,
                            "user_id": claims.sub,
                            "email": claims.email,
                            "exp": claims.exp
                        }));
                    }
                    Err(e) => {
                        log::warn!("❌ Invalid token: {}", e);
                        return HttpResponse::Unauthorized().json(serde_json::json!({
                            "success": false,
                            "valid": false,
                            "error": e
                        }));
                    }
                }
            }
        }
    }
    
    HttpResponse::BadRequest().json(serde_json::json!({
        "success": false,
        "error": "No valid Authorization header"
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    tag = "Auth",
    responses(
        (status = 200, description = "User information retrieved", body = UserInfo),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn get_me(
    db: web::Data<MongoDB>,
    req: HttpRequest,
) -> HttpResponse {
    log::info!("👤 GET /auth/me");
    
    // Extract token from Authorization header
    let auth_header = req.headers().get("Authorization");
    
    if let Some(auth_value) = auth_header {
        if let Ok(auth_str) = auth_value.to_str() {
            if auth_str.starts_with("Bearer ") {
                let token = &auth_str[7..];
                
                match auth_service::verify_token(token) {
                    Ok(claims) => {
                        match auth_service::get_current_user(&db, &claims.sub).await {
                            Ok(user) => {
                                log::info!("✅ User info retrieved: {}", claims.sub);
                                return HttpResponse::Ok().json(serde_json::json!({
                                    "success": true,
                                    "user": user
                                }));
                            }
                            Err(e) => {
                                log::error!("❌ Failed to get user: {}", e);
                                return HttpResponse::InternalServerError().json(serde_json::json!({
                                    "success": false,
                                    "error": e
                                }));
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("❌ Invalid token: {}", e);
                        return HttpResponse::Unauthorized().json(serde_json::json!({
                            "success": false,
                            "error": e
                        }));
                    }
                }
            }
        }
    }
    
    HttpResponse::BadRequest().json(serde_json::json!({
        "success": false,
        "error": "No valid Authorization header"
    }))
}

pub async fn google_auth() -> HttpResponse {
    log::info!("🔐 GET /auth/google - Generating OAuth URL");
    
    match auth_service::generate_google_oauth_url() {
        Ok(response) => {
            log::info!("✅ Google OAuth URL generated");
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            log::error!("❌ Failed to generate Google OAuth URL: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

use serde::Deserialize;

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

pub async fn google_callback(
    db: web::Data<MongoDB>,
    query: web::Query<CallbackQuery>,
) -> HttpResponse {
    log::info!("🔐 GET /auth/callback - Processing Google OAuth");
    
    // Get frontend URL from environment or use default
    let frontend_url = std::env::var("FRONTEND_URL")
        .unwrap_or_else(|_| "http://localhost:3000".to_string());
    
    if let Some(error) = &query.error {
        log::error!("❌ OAuth error: {}", error);
        return HttpResponse::Found()
            .append_header(("Location", format!("{}/auth-callback.html?error={}", frontend_url, error)))
            .finish();
    }
    
    let code = match &query.code {
        Some(c) => c,
        None => {
            log::error!("❌ No authorization code provided");
            return HttpResponse::Found()
                .append_header(("Location", format!("{}/auth-callback.html?error=no_code", frontend_url)))
                .finish();
        }
    };
    
    match auth_service::handle_google_callback(&db, code).await {
        Ok(response) => {
            log::info!("✅ Google OAuth successful");
            
            // Decodifica o token para extrair user_id e email
            let token_parts: Vec<&str> = response.token.split('.').collect();
            if token_parts.len() < 2 {
                log::error!("❌ Invalid JWT token format");
                return HttpResponse::Found()
                    .append_header(("Location", format!("{}/auth/callback?error=invalid_token", frontend_url)))
                    .finish();
            }
            
            // Decodifica o payload do JWT (base64)
            let payload_base64 = token_parts[1];
            let payload_bytes = match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload_base64) {
                Ok(bytes) => bytes,
                Err(e) => {
                    log::error!("❌ Failed to decode JWT payload: {}", e);
                    return HttpResponse::Found()
                        .append_header(("Location", format!("{}/auth/callback?error=invalid_token", frontend_url)))
                        .finish();
                }
            };
            
            let payload_str = match String::from_utf8(payload_bytes) {
                Ok(s) => s,
                Err(e) => {
                    log::error!("❌ Failed to parse JWT payload as UTF-8: {}", e);
                    return HttpResponse::Found()
                        .append_header(("Location", format!("{}/auth/callback?error=invalid_token", frontend_url)))
                        .finish();
                }
            };
            
            let payload: serde_json::Value = match serde_json::from_str(&payload_str) {
                Ok(p) => p,
                Err(e) => {
                    log::error!("❌ Failed to parse JWT payload JSON: {}", e);
                    return HttpResponse::Found()
                        .append_header(("Location", format!("{}/auth/callback?error=invalid_token", frontend_url)))
                        .finish();
                }
            };
            
            let user_id = payload["sub"].as_str().unwrap_or("unknown");
            let email = payload["email"].as_str().unwrap_or("unknown@email.com");
            let name = payload["name"].as_str().unwrap_or("");
            
            // Redireciona para a página de callback HTML estática
            let redirect_url = format!(
                "{}/auth-callback.html?access_token={}&user_id={}&email={}&name={}",
                frontend_url,
                response.token,
                urlencoding::encode(user_id),
                urlencoding::encode(email),
                urlencoding::encode(name)
            );
            
            HttpResponse::Found()
                .append_header(("Location", redirect_url))
                .finish()
        }
        Err(e) => {
            log::error!("❌ Google OAuth failed: {}", e);
            HttpResponse::Found()
                .append_header(("Location", format!("{}/auth/callback?error={}", frontend_url, urlencoding::encode(&e))))
                .finish()
        }
    }
}

// Dev login (for development only)
pub async fn dev_login(
    db: web::Data<MongoDB>,
    request: web::Json<auth_service::LoginRequest>,
) -> HttpResponse {
    log::info!("🔧 POST /auth/dev-login - email: {}", request.email);
    
    // Same as regular login but with relaxed requirements for dev
    match auth_service::login(&db, &request).await {
        Ok(response) => {
            log::info!("✅ Dev login successful: {}", request.email);
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            log::warn!("❌ Dev login failed: {} - {}", request.email, e);
            HttpResponse::Unauthorized().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

/// 🗑️ Delete account endpoint
/// Deletes the user account and all associated data
pub async fn delete_account(
    db: web::Data<MongoDB>,
    req: HttpRequest,
) -> HttpResponse {
    log::info!("🗑️ DELETE /auth/delete-account");
    
    // Extract token from Authorization header
    let auth_header = req.headers().get("Authorization");
    
    if let Some(auth_value) = auth_header {
        if let Ok(auth_str) = auth_value.to_str() {
            if auth_str.starts_with("Bearer ") {
                let token = &auth_str[7..];
                
                match auth_service::verify_token(token) {
                    Ok(claims) => {
                        let user_id = &claims.sub;
                        log::info!("🗑️ Deleting account for user: {}", user_id);
                        
                        match auth_service::delete_user_account(&db, user_id).await {
                            Ok(_) => {
                                log::info!("✅ Account deleted successfully: {}", user_id);
                                return HttpResponse::Ok().json(serde_json::json!({
                                    "success": true,
                                    "message": "Account deleted successfully"
                                }));
                            }
                            Err(e) => {
                                log::error!("❌ Failed to delete account {}: {}", user_id, e);
                                return HttpResponse::InternalServerError().json(serde_json::json!({
                                    "success": false,
                                    "error": format!("Failed to delete account: {}", e)
                                }));
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("❌ Invalid token: {}", e);
                        return HttpResponse::Unauthorized().json(serde_json::json!({
                            "success": false,
                            "error": "Invalid or expired token"
                        }));
                    }
                }
            }
        }
    }
    
    log::warn!("❌ No valid Authorization header");
    HttpResponse::Unauthorized().json(serde_json::json!({
        "success": false,
        "error": "No valid Authorization header"
    }))
}
