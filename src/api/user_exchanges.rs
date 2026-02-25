use actix_web::{web, HttpResponse, Responder};
use crate::{
    database::MongoDB,
    services::user_exchanges_service,
    middleware::auth::Claims,
};

/// POST /api/v1/user/exchanges - Adiciona nova exchange
/// 
/// Recebe credenciais em texto plano, criptografa e salva no MongoDB
pub async fn add_exchange(
    user: web::ReqData<Claims>,
    db: web::Data<MongoDB>,
    request: web::Json<user_exchanges_service::AddExchangeRequest>,
) -> impl Responder {
    let user_id = &user.sub;
    
    log::info!("📝 POST /user/exchanges - Adding {} for user {}", request.exchange_type, user_id);
    
    match user_exchanges_service::add_user_exchange(&db, user_id, request.into_inner()).await {
        Ok(response) => {
            if response.success {
                log::info!("✅ Exchange added: {}", response.exchange_id);
                HttpResponse::Ok().json(response)
            } else {
                log::warn!("⚠️ Failed to add exchange: {:?}", response.error);
                HttpResponse::BadRequest().json(response)
            }
        }
        Err(e) => {
            log::error!("❌ Error adding exchange: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

/// GET /api/v1/user/exchanges - Lista exchanges do usuário (sem credenciais)
pub async fn list_exchanges(
    user: web::ReqData<Claims>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;
    
    log::info!("📋 GET /user/exchanges - Listing for user {}", user_id);
    
    match user_exchanges_service::list_user_exchanges(&db, user_id).await {
        Ok(response) => {
            log::info!("✅ Listed {} exchanges", response.count);
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            log::error!("❌ Error listing exchanges: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

/// PATCH /api/v1/user/exchanges/{exchange_id} - Atualiza exchange
pub async fn update_exchange(
    user: web::ReqData<Claims>,
    db: web::Data<MongoDB>,
    exchange_id: web::Path<String>,
    request: web::Json<user_exchanges_service::UpdateExchangeRequest>,
) -> impl Responder {
    let user_id = &user.sub;
    
    log::info!("🔧 PATCH /user/exchanges/{} - Updating for user {}", exchange_id, user_id);
    
    match user_exchanges_service::update_user_exchange(&db, user_id, &exchange_id, request.into_inner()).await {
        Ok(response) => {
            if response.success {
                log::info!("✅ Exchange updated");
                HttpResponse::Ok().json(response)
            } else {
                log::warn!("⚠️ Failed to update: {:?}", response.error);
                HttpResponse::BadRequest().json(response)
            }
        }
        Err(e) => {
            log::error!("❌ Error updating exchange: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

/// DELETE /api/v1/user/exchanges/{exchange_id} - Remove exchange
pub async fn delete_exchange(
    user: web::ReqData<Claims>,
    db: web::Data<MongoDB>,
    exchange_id: web::Path<String>,
) -> impl Responder {
    let user_id = &user.sub;
    
    log::info!("🗑️  DELETE /user/exchanges/{} - Removing for user {}", exchange_id, user_id);
    
    match user_exchanges_service::delete_user_exchange(&db, user_id, &exchange_id).await {
        Ok(response) => {
            if response.success {
                log::info!("✅ Exchange deleted");
                HttpResponse::Ok().json(response)
            } else {
                log::warn!("⚠️ Failed to delete: {:?}", response.error);
                HttpResponse::BadRequest().json(response)
            }
        }
        Err(e) => {
            log::error!("❌ Error deleting exchange: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}
