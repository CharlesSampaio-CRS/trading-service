use crate::{
    database::MongoDB,
};
use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDateTime};
use serde::{Deserialize, Serialize};
use bcrypt::{hash, verify, DEFAULT_COST};
use jsonwebtoken::{encode, decode, Header, Validation, EncodingKey, DecodingKey, Algorithm};
use chrono::{Utc, Duration};
use uuid::Uuid;
use std::collections::HashSet;

// JWT Claims
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,           // user_id
    pub email: String,
    pub name: Option<String>,
    pub roles: Vec<String>,
    pub is_active: bool,
    pub iat: usize,            // issued at
    pub exp: usize,            // expiration
    pub jti: String,           // JWT ID
    pub aud: String,           // audience
    pub iss: String,           // issuer
}

// User model
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct User {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub _id: Option<ObjectId>,
    pub user_id: String,  // PRIMARY IDENTIFIER - matches MongoDB structure
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,  // Optional: None for OAuth users
    pub name: Option<String>,
    pub picture: Option<String>,
    pub google_id: Option<String>,
    pub apple_id: Option<String>,
    pub provider: Option<String>,  // "google", "apple", "local", etc.
    #[serde(default = "default_roles")]
    pub roles: Vec<String>,
    #[serde(default = "default_is_active")]
    pub is_active: bool,
    pub created_at: Option<BsonDateTime>,
    pub updated_at: Option<BsonDateTime>,
    pub last_login: Option<BsonDateTime>,
}

// Default functions for serde
fn default_roles() -> Vec<String> {
    vec!["user".to_string()]
}

fn default_is_active() -> bool {
    true
}

// Request/Response structures
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RegisterRequest {
    pub email: Option<String>,
    pub password: Option<String>,
    pub name: Option<String>,
    pub google_id: Option<String>,
    pub apple_id: Option<String>,
    pub picture: Option<String>,
    pub provider: Option<String>,  // "local", "google", or "apple"
}

#[derive(Debug, Deserialize)]
pub struct RefreshTokenRequest {
    pub refresh_token: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AuthResponse {
    pub success: bool,
    pub token: String,
    pub refresh_token: Option<String>,
    pub user: UserInfo,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UserInfo {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub picture: Option<String>,
    pub roles: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct GoogleAuthUrlResponse {
    pub success: bool,
    pub auth_url: String,
    pub state: String,
}

// Type aliases for API documentation
pub type LoginResponse = AuthResponse;
pub type RegisterResponse = AuthResponse;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct VerifyTokenResponse {
    pub valid: bool,
    pub user: Option<UserInfo>,
}

fn get_jwt_secret() -> String {
    std::env::var("JWT_SECRET").unwrap_or_else(|_| "default-secret-change-me".to_string())
}

fn get_jwt_issuer() -> String {
    std::env::var("JWT_ISSUER").unwrap_or_else(|_| "trading-service".to_string())
}

fn get_jwt_audience() -> String {
    std::env::var("JWT_AUDIENCE").unwrap_or_else(|_| "trading-api".to_string())
}

// Generate JWT token
pub fn generate_jwt(user: &User) -> Result<String, String> {
    let iat = Utc::now().timestamp() as usize;
    let exp = (Utc::now() + Duration::hours(24)).timestamp() as usize;
    let jti = Uuid::new_v4().to_string();
    
    let claims = Claims {
        sub: user.user_id.clone(),  // Use user_id instead of _id
        email: user.email.clone(),
        name: user.name.clone(),
        roles: user.roles.clone(),
        is_active: user.is_active,
        iat,
        exp,
        jti,
        aud: get_jwt_audience(),
        iss: get_jwt_issuer(),
    };
    
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(get_jwt_secret().as_ref())
    ).map_err(|e| format!("Failed to generate token: {}", e))
}

// Generate refresh token (longer expiry)
pub fn generate_refresh_token(user_id: &str) -> Result<String, String> {
    let iat = Utc::now().timestamp() as usize;
    let exp = (Utc::now() + Duration::days(30)).timestamp() as usize;
    let jti = Uuid::new_v4().to_string();
    
    let claims = Claims {
        sub: user_id.to_string(),
        email: String::new(),
        name: None,
        roles: vec![],
        is_active: true,
        iat,
        exp,
        jti,
        aud: get_jwt_audience(),
        iss: get_jwt_issuer(),
    };
    
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(get_jwt_secret().as_ref())
    ).map_err(|e| format!("Failed to generate refresh token: {}", e))
}

// Verify JWT token
pub fn verify_token(token: &str) -> Result<Claims, String> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_audience(&[get_jwt_audience()]);
    
    let mut issuers = HashSet::new();
    issuers.insert(get_jwt_issuer());
    validation.iss = Some(issuers);
    
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(get_jwt_secret().as_ref()),
        &validation
    )
    .map(|data| data.claims)
    .map_err(|e| format!("Invalid token: {}", e))
}

// User login
pub async fn login(
    db: &MongoDB,
    request: &LoginRequest,
) -> Result<AuthResponse, String> {
    let collection = db.collection::<User>("users");
    
    let filter = doc! {
        "email": &request.email,
    };
    
    let user = collection
        .find_one(filter)
        .await
        .map_err(|e| format!("Database error: {}", e))?
        .ok_or_else(|| "Invalid credentials".to_string())?;
    
    // Check if user has a password (not OAuth-only account)
    let stored_password = user.password
        .as_ref()
        .ok_or_else(|| "This account uses Google login. Please sign in with Google.".to_string())?;
    
    // Verify password
    let valid = verify(&request.password, stored_password)
        .map_err(|e| format!("Password verification error: {}", e))?;
    
    if !valid {
        return Err("Invalid credentials".to_string());
    }
    
    if !user.is_active {
        return Err("Account is inactive".to_string());
    }
    
    let token = generate_jwt(&user)?;
    let refresh_token = generate_refresh_token(&user.user_id)?;
    
    Ok(AuthResponse {
        success: true,
        token,
        refresh_token: Some(refresh_token),
        user: UserInfo {
            id: user.user_id,
            email: user.email,
            name: user.name,
            picture: user.picture,
            roles: user.roles,
        },
    })
}

// User registration
pub async fn register(
    db: &MongoDB,
    request: &RegisterRequest,
) -> Result<AuthResponse, String> {
    let collection = db.collection::<User>("users");
    
    // Validação: email sempre é obrigatório
    let email = request.email.as_ref()
        .ok_or_else(|| "Email is required".to_string())?;
    
    // Determina o provider (default: local)
    let provider = request.provider.as_deref().unwrap_or("local");
    
    // Validação baseada no provider
    match provider {
        "local" => {
            // Registro local: password obrigatório
            if request.password.is_none() {
                return Err("Password is required for local registration".to_string());
            }
        }
        "google" => {
            // Registro Google: google_id obrigatório
            if request.google_id.is_none() {
                return Err("Google ID is required for Google registration".to_string());
            }
        }
        "apple" => {
            // Registro Apple: apple_id obrigatório
            if request.apple_id.is_none() {
                return Err("Apple ID is required for Apple registration".to_string());
            }
        }
        _ => return Err(format!("Invalid provider: {}. Supported: local, google, apple", provider)),
    }
    
    // Check if user already exists (por email ou OAuth ID)
    let mut filter = doc! { "email": email };
    
    // Também verificar por OAuth ID se fornecido
    if let Some(google_id) = &request.google_id {
        filter = doc! {
            "$or": [
                { "email": email },
                { "google_id": google_id }
            ]
        };
    } else if let Some(apple_id) = &request.apple_id {
        filter = doc! {
            "$or": [
                { "email": email },
                { "apple_id": apple_id }
            ]
        };
    }
    
    if let Some(_) = collection.find_one(filter).await.map_err(|e| format!("Database error: {}", e))? {
        return Err("User already exists".to_string());
    }
    
    // Hash password (apenas se fornecido para registro local)
    let hashed_password = if let Some(pwd) = &request.password {
        Some(hash(pwd, DEFAULT_COST)
            .map_err(|e| format!("Failed to hash password: {}", e))?)
    } else {
        None
    };
    
    // Generate user_id
    let new_user_id = ObjectId::new().to_hex();
    
    let new_user = User {
        _id: None,
        user_id: new_user_id.clone(),
        email: email.clone(),
        password: hashed_password,
        name: request.name.clone(),
        picture: request.picture.clone(),
        google_id: request.google_id.clone(),
        apple_id: request.apple_id.clone(),
        provider: Some(provider.to_string()),
        roles: vec!["user".to_string()],
        is_active: true,
        created_at: Some(BsonDateTime::now()),
        updated_at: Some(BsonDateTime::now()),
        last_login: Some(BsonDateTime::now()),
    };
    
    collection
        .insert_one(&new_user)
        .await
        .map_err(|e| format!("Failed to create user: {}", e))?;
    
    let token = generate_jwt(&new_user)?;
    let refresh_token = generate_refresh_token(&new_user_id)?;
    
    log::info!("✅ User registered successfully: {} (provider: {})", email, provider);
    
    Ok(AuthResponse {
        success: true,
        token,
        refresh_token: Some(refresh_token),
        user: UserInfo {
            id: new_user_id,
            email: new_user.email,
            name: new_user.name,
            picture: new_user.picture,
            roles: new_user.roles,
        },
    })
}

// Refresh token
pub async fn refresh_token(
    db: &MongoDB,
    request: &RefreshTokenRequest,
) -> Result<AuthResponse, String> {
    let claims = verify_token(&request.refresh_token)?;
    
    let collection = db.collection::<User>("users");
    
    // Claims.sub now contains user_id (not _id)
    let filter = doc! {
        "user_id": &claims.sub,
    };
    
    let user = collection
        .find_one(filter)
        .await
        .map_err(|e| format!("Database error: {}", e))?
        .ok_or_else(|| "User not found".to_string())?;
    
    if !user.is_active {
        return Err("Account is inactive".to_string());
    }
    
    let token = generate_jwt(&user)?;
    let new_refresh_token = generate_refresh_token(&user.user_id)?;
    
    Ok(AuthResponse {
        success: true,
        token,
        refresh_token: Some(new_refresh_token),
        user: UserInfo {
            id: user.user_id,
            email: user.email,
            name: user.name,
            picture: user.picture,
            roles: user.roles,
        },
    })
}

// Get current user
pub async fn get_current_user(
    db: &MongoDB,
    user_id: &str,
) -> Result<UserInfo, String> {
    let collection = db.collection::<User>("users");
    
    // user_id is now a string, not ObjectId
    let filter = doc! {
        "user_id": user_id,
    };
    
    let user = collection
        .find_one(filter)
        .await
        .map_err(|e| format!("Database error: {}", e))?
        .ok_or_else(|| "User not found".to_string())?;
    
    Ok(UserInfo {
        id: user.user_id,
        email: user.email,
        name: user.name,
        picture: user.picture,
        roles: user.roles,
    })
}

// Generate Google OAuth URL
pub fn generate_google_oauth_url() -> Result<GoogleAuthUrlResponse, String> {
    let client_id = std::env::var("GOOGLE_CLIENT_ID")
        .map_err(|_| "GOOGLE_CLIENT_ID not configured".to_string())?;

    // Para mobile Expo Go/iPhone, use o IP local e porta do Metro Bundler
    let redirect_uri = std::env::var("GOOGLE_REDIRECT_URI")
        .unwrap_or_else(|_| "exp://192.168.18.15:8081/--/auth/callback".to_string());
    
    // Generate state for CSRF protection
    let state = Uuid::new_v4().to_string();
    
    let params = vec![
        ("client_id", client_id.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("response_type", "code"),
        ("scope", "openid email profile"),
        ("state", state.as_str()),
        ("access_type", "offline"),
        ("prompt", "select_account"),
    ];
    
    let query_string = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    
    let auth_url = format!("https://accounts.google.com/o/oauth2/v2/auth?{}", query_string);
    
    Ok(GoogleAuthUrlResponse {
        success: true,
        auth_url,
        state,
    })
}

// Handle Google OAuth callback
pub async fn handle_google_callback(
    db: &MongoDB,
    code: &str,
) -> Result<AuthResponse, String> {
    let client_id = std::env::var("GOOGLE_CLIENT_ID")
        .map_err(|_| "GOOGLE_CLIENT_ID not configured".to_string())?;
    let client_secret = std::env::var("GOOGLE_CLIENT_SECRET")
        .map_err(|_| "GOOGLE_CLIENT_SECRET not configured".to_string())?;
    let redirect_uri = std::env::var("GOOGLE_REDIRECT_URI")
        .unwrap_or_else(|_| "http://localhost:3000/auth/callback".to_string());
    
    // Exchange code for tokens
    let client = reqwest::Client::new();
    let token_response = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("code", code),
            ("client_id", &client_id),
            ("client_secret", &client_secret),
            ("redirect_uri", &redirect_uri),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .map_err(|e| format!("Failed to exchange code: {}", e))?;
    
    if !token_response.status().is_success() {
        return Err("Failed to exchange authorization code".to_string());
    }
    
    let tokens: serde_json::Value = token_response
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {}", e))?;
    
    let access_token = tokens["access_token"]
        .as_str()
        .ok_or_else(|| "No access token in response".to_string())?;
    
    // Get user info
    let user_info_response = client
        .get("https://www.googleapis.com/oauth2/v2/userinfo")
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| format!("Failed to get user info: {}", e))?;
    
    let user_info: serde_json::Value = user_info_response
        .json()
        .await
        .map_err(|e| format!("Failed to parse user info: {}", e))?;
    
    let email = user_info["email"]
        .as_str()
        .ok_or_else(|| "No email in user info".to_string())?;
    let name = user_info["name"].as_str().map(String::from);
    let picture = user_info["picture"].as_str().map(String::from);
    let google_id = user_info["id"]
        .as_str()
        .ok_or_else(|| "No google_id in user info".to_string())?;
    
    // Find or create user using google_id or email
    let collection = db.collection::<User>("users");
    
    // First try to find by google_id
    let filter = doc! {
        "google_id": google_id,
    };
    
    let user = if let Some(mut existing_user) = collection.find_one(filter.clone()).await
        .map_err(|e| format!("Database error: {}", e))? {
        
        log::info!("✅ Found existing user by google_id: {}", existing_user.user_id);
        
        // Update user info and last_login
        existing_user.name = name.clone();
        existing_user.picture = picture.clone();
        existing_user.last_login = Some(BsonDateTime::now());
        existing_user.updated_at = Some(BsonDateTime::now());
        
        // Ensure roles exists (for old users without this field)
        if existing_user.roles.is_empty() {
            existing_user.roles = vec!["user".to_string()];
        }
        
        let update = doc! {
            "$set": {
                "name": name.clone(),
                "picture": picture.clone(),
                "last_login": BsonDateTime::now(),
                "roles": existing_user.roles.clone(),
                "updated_at": BsonDateTime::now(),
            }
        };
        
        collection
            .update_one(doc! { "user_id": &existing_user.user_id }, update)
            .await
            .map_err(|e| format!("Failed to update user: {}", e))?;
        
        existing_user
    } else {
        // Check if user exists by email (for migration from old auth)
        let email_filter = doc! { "email": email };
        
        if let Some(mut existing_user) = collection.find_one(email_filter.clone()).await
            .map_err(|e| format!("Database error: {}", e))? {
            
            log::info!("✅ Found existing user by email, adding google_id: {}", existing_user.user_id);
            
            // Update existing user with google_id
            existing_user.google_id = Some(google_id.to_string());
            existing_user.provider = Some("google".to_string());
            existing_user.name = name.clone();
            existing_user.picture = picture.clone();
            existing_user.last_login = Some(BsonDateTime::now());
            existing_user.updated_at = Some(BsonDateTime::now());
            
            let update = doc! {
                "$set": {
                    "google_id": google_id,
                    "provider": "google",
                    "name": name.clone(),
                    "picture": picture.clone(),
                    "last_login": BsonDateTime::now(),
                    "updated_at": BsonDateTime::now(),
                }
            };
            
            collection
                .update_one(doc! { "user_id": &existing_user.user_id }, update)
                .await
                .map_err(|e| format!("Failed to update user with google_id: {}", e))?;
            
            existing_user
        } else {
            // Create new user with generated user_id
            let new_user_id = ObjectId::new().to_hex();
            
            log::info!("✅ Creating new user with user_id: {}", new_user_id);
            
            let new_user = User {
                _id: None,
                user_id: new_user_id.clone(),
                email: email.to_string(),
                password: None,  // OAuth users don't have passwords
                name: name.clone(),
                picture: picture.clone(),
                google_id: Some(google_id.to_string()),
                apple_id: None,
                provider: Some("google".to_string()),
                roles: vec!["user".to_string()],
                is_active: true,
                created_at: Some(BsonDateTime::now()),
                updated_at: Some(BsonDateTime::now()),
                last_login: Some(BsonDateTime::now()),
            };
            
            collection
                .insert_one(&new_user)
                .await
                .map_err(|e| format!("Failed to create user: {}", e))?;
            
            new_user
        }
    };
    
    let token = generate_jwt(&user)?;
    let refresh_token = generate_refresh_token(&user.user_id)?;
    
    Ok(AuthResponse {
        success: true,
        token,
        refresh_token: Some(refresh_token),
        user: UserInfo {
            id: user.user_id.clone(),
            email: user.email,
            name: user.name,
            picture: user.picture,
            roles: user.roles,
        },
    })
}

/// 🗑️ Delete user account and all associated data
pub async fn delete_user_account(
    db: &MongoDB,
    user_id: &str,
) -> Result<(), String> {
    log::info!("🗑️ Deleting account for user_id: {}", user_id);
    
    // 1. Delete user from users collection
    let users_collection = db.database().collection::<User>("users");
    let delete_user_result = users_collection
        .delete_one(doc! { "user_id": user_id })
        .await
        .map_err(|e| format!("Failed to delete user: {}", e))?;
    
    if delete_user_result.deleted_count == 0 {
        log::warn!("⚠️ User {} not found in database", user_id);
        return Err(format!("User {} not found", user_id));
    }
    
    log::info!("✅ User {} deleted from users collection", user_id);
    
    // 2. Delete all exchanges linked to this user
    let exchanges_collection = db.database().collection::<mongodb::bson::Document>("exchanges");
    let delete_exchanges_result = exchanges_collection
        .delete_many(doc! { "user_id": user_id })
        .await
        .map_err(|e| format!("Failed to delete exchanges: {}", e))?;
    
    log::info!("✅ Deleted {} exchanges for user {}", delete_exchanges_result.deleted_count, user_id);
    
    // 3. Delete all balance history for this user
    let balance_history_collection = db.database().collection::<mongodb::bson::Document>("balance_history");
    let delete_history_result = balance_history_collection
        .delete_many(doc! { "user_id": user_id })
        .await
        .map_err(|e| format!("Failed to delete balance history: {}", e))?;
    
    log::info!("✅ Deleted {} balance history records for user {}", delete_history_result.deleted_count, user_id);
    
    // 4. Delete all orders for this user
    let orders_collection = db.database().collection::<mongodb::bson::Document>("orders");
    let delete_orders_result = orders_collection
        .delete_many(doc! { "user_id": user_id })
        .await
        .map_err(|e| format!("Failed to delete orders: {}", e))?;
    
    log::info!("✅ Deleted {} orders for user {}", delete_orders_result.deleted_count, user_id);
    
    // 5. Delete all strategies for this user
    let strategies_collection = db.database().collection::<mongodb::bson::Document>("strategies");
    let delete_strategies_result = strategies_collection
        .delete_many(doc! { "user_id": user_id })
        .await
        .map_err(|e| format!("Failed to delete strategies: {}", e))?;
    
    log::info!("✅ Deleted {} strategies for user {}", delete_strategies_result.deleted_count, user_id);
    
    // NOTE: Notifications are stored locally in WatermelonDB (Zero Database architecture)
    // No backend cleanup needed - they're automatically removed when app is uninstalled
    
    log::info!("🎉 Account and all data successfully deleted for user {}", user_id);
    Ok(())
}
