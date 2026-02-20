use actix_web::{web, HttpResponse, Responder};
use crate::{
    database::MongoDB,
    middleware::auth::Claims,
    jobs::snapshot_scheduler,
    utils::crypto,
};
use serde::Serialize;
use mongodb::bson::doc;
use futures::stream::StreamExt;
use std::env;

#[derive(Debug, Serialize)]
pub struct SnapshotResponse {
    user_id: String,
    date: String,
    total_usd: f64,
    total_brl: f64,
    timestamp: i64,
    exchanges: Vec<ExchangeSnapshotDetail>,
}

#[derive(Debug, Serialize)]
pub struct ExchangeSnapshotDetail {
    exchange_id: String,
    exchange_name: String,
    balance_usd: f64,
    is_active: bool,
    tokens_count: i32,
}

/// POST /api/v1/snapshots/save - Salva snapshot manual do usuário autenticado
pub async fn save_snapshot(
    user: web::ReqData<Claims>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;
    
    log::info!("💾 POST /snapshots/save - Manual snapshot for user {}", user_id);
    
    match snapshot_scheduler::save_snapshot_now(&db, user_id).await {
        Ok(_) => {
            log::info!("✅ Snapshot saved successfully");
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": "Snapshot saved successfully"
            }))
        }
        Err(e) => {
            log::error!("❌ Error saving snapshot: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

/// GET /api/v1/snapshots - Busca snapshots do usuário (valores descriptografados)
pub async fn get_snapshots(
    user: web::ReqData<Claims>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;
    
    log::info!("📊 GET /snapshots - Fetching snapshots for user {}", user_id);
    
    // Obter chave de criptografia
    let encryption_key = match env::var("ENCRYPTION_KEY") {
        Ok(key) => key,
        Err(_) => {
            log::error!("❌ ENCRYPTION_KEY not found in environment");
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": "Server configuration error"
            }));
        }
    };
    
    let snapshots_collection = db.collection::<mongodb::bson::Document>("balance_snapshots");
    
    let filter = doc! {
        "user_id": user_id,
    };
    
    // Busca documento do usuário
    let user_doc = match snapshots_collection.find_one(filter).await {
        Ok(Some(doc)) => doc,
        Ok(None) => {
            log::info!("ℹ️  No snapshots found for user {}", user_id);
            return HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "snapshots": [],
                "count": 0
            }));
        }
        Err(e) => {
            log::error!("❌ Database error: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": "Failed to fetch snapshots"
            }));
        }
    };
    
    let mut snapshots = Vec::new();
    
    // Extrai array de snapshots do documento
    if let Ok(snapshots_array) = user_doc.get_array("snapshots") {
        for snapshot_value in snapshots_array {
            if let Some(snapshot_doc) = snapshot_value.as_document() {
                // Descriptografar total_usd e total_brl
                let encrypted_total_usd = snapshot_doc.get_str("total_usd").unwrap_or("");
                let encrypted_total_brl = snapshot_doc.get_str("total_brl").unwrap_or("");
                
                let total_usd = crypto::decrypt_fernet_via_python(encrypted_total_usd, &encryption_key)
                    .ok()
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
                
                let total_brl = crypto::decrypt_fernet_via_python(encrypted_total_brl, &encryption_key)
                    .ok()
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
                
                // Descriptografar balances das exchanges
                let mut exchanges_details = Vec::new();
                if let Ok(exchanges) = snapshot_doc.get_array("exchanges") {
                    for exchange_doc in exchanges {
                        if let Some(exchange) = exchange_doc.as_document() {
                            let encrypted_balance = exchange.get_str("balance_usd").unwrap_or("");
                            let balance_usd = crypto::decrypt_fernet_via_python(encrypted_balance, &encryption_key)
                                .ok()
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(0.0);
                            
                            exchanges_details.push(ExchangeSnapshotDetail {
                                exchange_id: exchange.get_str("exchange_id").unwrap_or("").to_string(),
                                exchange_name: exchange.get_str("exchange_name").unwrap_or("").to_string(),
                                balance_usd,
                                is_active: exchange.get_bool("is_active").unwrap_or(false),
                                tokens_count: exchange.get_i32("tokens_count").unwrap_or(0),
                            });
                        }
                    }
                }
                
                snapshots.push(SnapshotResponse {
                    user_id: user_id.to_string(),
                    date: snapshot_doc.get_str("date").unwrap_or("").to_string(),
                    total_usd,
                    total_brl,
                    timestamp: snapshot_doc.get_i64("timestamp").unwrap_or(0),
                    exchanges: exchanges_details,
                });
            }
        }
    }
    
    // Ordena por timestamp decrescente (mais recente primeiro)
    snapshots.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    
    log::info!("✅ Found {} snapshots for user {}", snapshots.len(), user_id);
    
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "snapshots": snapshots,
        "count": snapshots.len()
    }))
}
