use actix_web::{delete, get, post, put, web, HttpResponse, Responder};
use mongodb::bson::doc;
use crate::database::MongoDB;
use crate::models::{
    UserStrategies, StrategyItem, CreateStrategyRequest, UpdateStrategyRequest,
    StrategyResponse, StrategyListItem, StrategyStatus,
};
use crate::middleware::auth::Claims;
use crate::services::strategy_service;

/// Nome da collection no MongoDB
const COLLECTION: &str = "user_strategy";

// ═══════════════════════════════════════════════════════════════════
// HELPERS
// ═══════════════════════════════════════════════════════════════════

async fn get_or_create_user_doc(db: &MongoDB, user_id: &str) -> Result<UserStrategies, String> {
    let collection = db.collection::<UserStrategies>(COLLECTION);
    match collection.find_one(doc! { "user_id": user_id }).await {
        Ok(Some(d)) => Ok(d),
        Ok(None) => {
            let now = chrono::Utc::now().timestamp();
            let new_doc = UserStrategies {
                id: None,
                user_id: user_id.to_string(),
                strategies: vec![],
                created_at: now,
                updated_at: now,
            };
            collection.insert_one(&new_doc).await
                .map_err(|e| format!("Failed to create user strategies doc: {}", e))?;
            collection.find_one(doc! { "user_id": user_id }).await
                .map_err(|e| format!("Failed to fetch: {}", e))?
                .ok_or_else(|| "Failed to fetch created doc".to_string())
        }
        Err(e) => Err(format!("Database error: {}", e)),
    }
}

fn find_strategy_in_doc<'a>(doc: &'a UserStrategies, strategy_id: &str) -> Option<&'a StrategyItem> {
    doc.strategies.iter().find(|s| s.strategy_id == strategy_id)
}

// ═══════════════════════════════════════════════════════════════════
// GET /api/v1/strategies
// ═══════════════════════════════════════════════════════════════════

#[get("")]
pub async fn get_strategies(user: web::ReqData<Claims>, db: web::Data<MongoDB>) -> impl Responder {
    let user_id = &user.sub;
    match get_or_create_user_doc(&db, user_id).await {
        Ok(user_doc) => {
            let mut strategies: Vec<StrategyListItem> = user_doc.strategies
                .into_iter().map(StrategyListItem::from).collect();
            strategies.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "strategies": strategies,
                "total": strategies.len()
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": format!("Failed to fetch strategies: {}", e)
        })),
    }
}

// ═══════════════════════════════════════════════════════════════════
// GET /api/v1/strategies/{id}
// ═══════════════════════════════════════════════════════════════════

#[get("/{id}")]
pub async fn get_strategy(user: web::ReqData<Claims>, path: web::Path<String>, db: web::Data<MongoDB>) -> impl Responder {
    let user_id = &user.sub;
    let strategy_id = path.into_inner();
    match get_or_create_user_doc(&db, user_id).await {
        Ok(user_doc) => {
            match user_doc.strategies.into_iter().find(|s| s.strategy_id == strategy_id) {
                Some(strategy) => HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "strategy": StrategyResponse::from(strategy)
                })),
                None => HttpResponse::NotFound().json(serde_json::json!({
                    "success": false,
                    "error": "Strategy not found"
                })),
            }
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": format!("Failed to fetch strategy: {}", e)
        })),
    }
}

// ═══════════════════════════════════════════════════════════════════
// GET /api/v1/strategies/{id}/stats
// ═══════════════════════════════════════════════════════════════════

#[get("/{id}/stats")]
pub async fn get_strategy_stats(user: web::ReqData<Claims>, path: web::Path<String>, db: web::Data<MongoDB>) -> impl Responder {
    let user_id = &user.sub;
    let strategy_id = path.into_inner();
    match get_or_create_user_doc(&db, user_id).await {
        Ok(user_doc) => {
            match user_doc.strategies.into_iter().find(|s| s.strategy_id == strategy_id) {
                Some(strategy) => {
                    let stats = strategy.compute_stats();
                    HttpResponse::Ok().json(serde_json::json!({
                        "success": true,
                        "stats": stats,
                        "strategy_info": {
                            "id": strategy.strategy_id,
                            "name": strategy.name,
                            "symbol": strategy.symbol,
                            "exchange_name": strategy.exchange_name,
                            "strategy_type": strategy.strategy_type,
                            "status": strategy.status,
                            "is_active": strategy.is_active,
                            "created_at": strategy.created_at
                        }
                    }))
                }
                None => HttpResponse::NotFound().json(serde_json::json!({
                    "success": false,
                    "error": "Strategy not found"
                })),
            }
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": format!("Failed to fetch strategy stats: {}", e)
        })),
    }
}

// ═══════════════════════════════════════════════════════════════════
// GET /api/v1/strategies/{id}/executions
// ═══════════════════════════════════════════════════════════════════

#[get("/{id}/executions")]
pub async fn get_strategy_executions(
    user: web::ReqData<Claims>,
    path: web::Path<String>,
    query: web::Query<PaginationQuery>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;
    let strategy_id = path.into_inner();
    match get_or_create_user_doc(&db, user_id).await {
        Ok(user_doc) => {
            match user_doc.strategies.into_iter().find(|s| s.strategy_id == strategy_id) {
                Some(strategy) => {
                    let limit = query.limit.unwrap_or(50).min(200) as usize;
                    let offset = query.offset.unwrap_or(0) as usize;
                    let total = strategy.executions.len();
                    let executions: Vec<_> = strategy.executions.iter()
                        .rev().skip(offset).take(limit).cloned().collect();
                    HttpResponse::Ok().json(serde_json::json!({
                        "success": true,
                        "executions": executions,
                        "total": total,
                        "limit": limit,
                        "offset": offset
                    }))
                }
                None => HttpResponse::NotFound().json(serde_json::json!({
                    "success": false,
                    "error": "Strategy not found"
                })),
            }
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": format!("Failed to fetch executions: {}", e)
        })),
    }
}

// ═══════════════════════════════════════════════════════════════════
// GET /api/v1/strategies/{id}/signals
// ═══════════════════════════════════════════════════════════════════

#[get("/{id}/signals")]
pub async fn get_strategy_signals(
    user: web::ReqData<Claims>,
    path: web::Path<String>,
    query: web::Query<PaginationQuery>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;
    let strategy_id = path.into_inner();
    match get_or_create_user_doc(&db, user_id).await {
        Ok(user_doc) => {
            match user_doc.strategies.into_iter().find(|s| s.strategy_id == strategy_id) {
                Some(strategy) => {
                    let limit = query.limit.unwrap_or(50).min(200) as usize;
                    let offset = query.offset.unwrap_or(0) as usize;
                    let total = strategy.signals.len();
                    let signals: Vec<_> = strategy.signals.iter()
                        .rev().skip(offset).take(limit).cloned().collect();
                    HttpResponse::Ok().json(serde_json::json!({
                        "success": true,
                        "signals": signals,
                        "total": total,
                        "limit": limit,
                        "offset": offset
                    }))
                }
                None => HttpResponse::NotFound().json(serde_json::json!({
                    "success": false,
                    "error": "Strategy not found"
                })),
            }
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": format!("Failed to fetch signals: {}", e)
        })),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct PaginationQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ═══════════════════════════════════════════════════════════════════
// POST /api/v1/strategies — $push no array
// ═══════════════════════════════════════════════════════════════════

#[post("")]
pub async fn create_strategy(
    user: web::ReqData<Claims>,
    body: web::Json<CreateStrategyRequest>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;
    let collection = db.collection::<UserStrategies>(COLLECTION);
    let now = chrono::Utc::now().timestamp();
    let strategy_id = uuid::Uuid::new_v4().to_string();

    let new_strategy = StrategyItem {
        strategy_id: strategy_id.clone(),
        name: body.name.clone(),
        description: body.description.clone(),
        strategy_type: body.strategy_type.clone(),
        symbol: body.symbol.clone(),
        exchange_id: body.exchange_id.clone(),
        exchange_name: body.exchange_name.clone(),
        is_active: true,
        status: StrategyStatus::Monitoring,
        config: body.config.clone().unwrap_or_default(),
        config_legacy: body.config_legacy.clone(),
        position: None,
        executions: vec![],
        signals: vec![],
        last_checked_at: None,
        last_price: None,
        check_interval_secs: body.check_interval_secs.unwrap_or(60),
        error_message: None,
        total_pnl_usd: 0.0,
        total_executions: 0,
        created_at: now,
        updated_at: now,
    };

    let strategy_bson = match mongodb::bson::to_bson(&new_strategy) {
        Ok(b) => b,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": format!("Failed to serialize strategy: {}", e)
            }));
        }
    };

    // Garantir doc do usuario existe
    let _ = get_or_create_user_doc(&db, user_id).await;

    match collection.update_one(
        doc! { "user_id": user_id },
        doc! {
            "$push": { "strategies": strategy_bson },
            "$set": { "updated_at": now }
        },
    ).await {
        Ok(result) => {
            if result.modified_count > 0 {
                HttpResponse::Created().json(serde_json::json!({
                    "success": true,
                    "strategy": StrategyResponse::from(new_strategy)
                }))
            } else {
                HttpResponse::InternalServerError().json(serde_json::json!({
                    "success": false,
                    "error": "Failed to add strategy to user document"
                }))
            }
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": format!("Failed to create strategy: {}", e)
        })),
    }
}

// ═══════════════════════════════════════════════════════════════════
// PUT /api/v1/strategies/{id} — array filter update
// ═══════════════════════════════════════════════════════════════════

#[put("/{id}")]
pub async fn update_strategy(
    user: web::ReqData<Claims>,
    path: web::Path<String>,
    body: web::Json<UpdateStrategyRequest>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;
    let strategy_id = path.into_inner();
    let collection = db.collection::<UserStrategies>(COLLECTION);

    // Verificar existencia
    match get_or_create_user_doc(&db, user_id).await {
        Ok(user_doc) => {
            if find_strategy_in_doc(&user_doc, &strategy_id).is_none() {
                return HttpResponse::NotFound().json(serde_json::json!({
                    "success": false,
                    "error": "Strategy not found"
                }));
            }
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }));
        }
    }

    let now = chrono::Utc::now().timestamp();
    let p = "strategies.$[elem]";
    let mut update_doc = doc! {
        format!("{}.updated_at", p): now,
        "updated_at": now,
    };

    if let Some(name) = &body.name {
        update_doc.insert(format!("{}.name", p), name);
    }
    if let Some(description) = &body.description {
        update_doc.insert(format!("{}.description", p), description);
    }
    if let Some(strategy_type) = &body.strategy_type {
        update_doc.insert(format!("{}.strategy_type", p), strategy_type);
    }
    if let Some(symbol) = &body.symbol {
        update_doc.insert(format!("{}.symbol", p), symbol);
    }
    if let Some(exchange_id) = &body.exchange_id {
        update_doc.insert(format!("{}.exchange_id", p), exchange_id);
    }
    if let Some(exchange_name) = &body.exchange_name {
        update_doc.insert(format!("{}.exchange_name", p), exchange_name);
    }
    if let Some(is_active) = body.is_active {
        update_doc.insert(format!("{}.is_active", p), is_active);
        if is_active {
            update_doc.insert(format!("{}.status", p), "monitoring");
        } else {
            update_doc.insert(format!("{}.status", p), "paused");
        }
    }
    if let Some(status) = &body.status {
        update_doc.insert(format!("{}.status", p), mongodb::bson::to_bson(status).unwrap());
        match status {
            StrategyStatus::Paused | StrategyStatus::Completed | StrategyStatus::Error => {
                update_doc.insert(format!("{}.is_active", p), false);
            }
            StrategyStatus::Monitoring | StrategyStatus::InPosition
            | StrategyStatus::BuyPending | StrategyStatus::SellPending => {
                update_doc.insert(format!("{}.is_active", p), true);
            }
            _ => {}
        }
    }
    if let Some(config) = &body.config {
        update_doc.insert(format!("{}.config", p), mongodb::bson::to_bson(config).unwrap());
    }
    if let Some(config_legacy) = &body.config_legacy {
        update_doc.insert(format!("{}.config_legacy", p), mongodb::bson::to_bson(config_legacy).unwrap());
    }
    if let Some(check_interval) = body.check_interval_secs {
        update_doc.insert(format!("{}.check_interval_secs", p), check_interval);
    }

    let array_filter = doc! { "elem.strategy_id": &strategy_id };

    match collection.update_one(
        doc! { "user_id": user_id },
        doc! { "$set": update_doc },
    ).array_filters(vec![array_filter]).await {
        Ok(_) => {
            match get_or_create_user_doc(&db, user_id).await {
                Ok(user_doc) => {
                    match user_doc.strategies.into_iter().find(|s| s.strategy_id == strategy_id) {
                        Some(strategy) => HttpResponse::Ok().json(serde_json::json!({
                            "success": true,
                            "strategy": StrategyResponse::from(strategy)
                        })),
                        _ => HttpResponse::Ok().json(serde_json::json!({
                            "success": true,
                            "message": "Strategy updated successfully"
                        })),
                    }
                }
                _ => HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": "Strategy updated successfully"
                })),
            }
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": format!("Failed to update strategy: {}", e)
        })),
    }
}

// ═══════════════════════════════════════════════════════════════════
// DELETE /api/v1/strategies/{id} — $pull do array
// ═══════════════════════════════════════════════════════════════════

#[delete("/{id}")]
pub async fn delete_strategy(user: web::ReqData<Claims>, path: web::Path<String>, db: web::Data<MongoDB>) -> impl Responder {
    let user_id = &user.sub;
    let strategy_id = path.into_inner();
    let collection = db.collection::<UserStrategies>(COLLECTION);
    let now = chrono::Utc::now().timestamp();

    match collection.update_one(
        doc! { "user_id": user_id },
        doc! {
            "$pull": { "strategies": { "strategy_id": &strategy_id } },
            "$set": { "updated_at": now }
        },
    ).await {
        Ok(result) => {
            if result.modified_count > 0 {
                HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": "Strategy deleted successfully"
                }))
            } else {
                HttpResponse::NotFound().json(serde_json::json!({
                    "success": false,
                    "error": "Strategy not found"
                }))
            }
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": format!("Failed to delete strategy: {}", e)
        })),
    }
}

// ═══════════════════════════════════════════════════════════════════
// STRATEGY ENGINE ENDPOINTS
// ═══════════════════════════════════════════════════════════════════

#[post("/{id}/activate")]
pub async fn activate_strategy(
    user: web::ReqData<Claims>,
    path: web::Path<String>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;
    let strategy_id = path.into_inner();
    match strategy_service::activate_strategy(&db, &strategy_id, user_id).await {
        Ok(strategy) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "strategy": StrategyResponse::from(strategy)
        })),
        Err(e) => HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "error": e
        })),
    }
}

#[post("/{id}/pause")]
pub async fn pause_strategy(
    user: web::ReqData<Claims>,
    path: web::Path<String>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;
    let strategy_id = path.into_inner();
    match strategy_service::pause_strategy(&db, &strategy_id, user_id).await {
        Ok(strategy) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "strategy": StrategyResponse::from(strategy)
        })),
        Err(e) => HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "error": e
        })),
    }
}

#[post("/{id}/tick")]
pub async fn tick_strategy(
    user: web::ReqData<Claims>,
    path: web::Path<String>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;
    let strategy_id = path.into_inner();

    let user_doc = match get_or_create_user_doc(&db, user_id).await {
        Ok(d) => d,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": format!("Database error: {}", e)
            }));
        }
    };

    let strategy = match user_doc.strategies.into_iter().find(|s| s.strategy_id == strategy_id) {
        Some(s) => s,
        None => {
            return HttpResponse::NotFound().json(serde_json::json!({
                "success": false,
                "error": "Strategy not found"
            }));
        }
    };

    let tick_result = strategy_service::tick(&db, user_id, &strategy).await;

    if let Err(e) = strategy_service::persist_tick_result(&db, user_id, &strategy, &tick_result).await {
        log::error!("Failed to persist tick: {}", e);
    }

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "tick": {
            "strategy_id": tick_result.strategy_id,
            "symbol": tick_result.symbol,
            "price": tick_result.price,
            "signals_count": tick_result.signals.len(),
            "executions_count": tick_result.executions.len(),
            "new_status": tick_result.new_status,
            "error": tick_result.error,
            "signals": tick_result.signals,
        }
    }))
}

#[post("/process-all")]
pub async fn process_all_strategies(
    user: web::ReqData<Claims>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;
    log::info!("Manual process-all triggered by user: {}", user_id);
    match strategy_service::process_active_strategies(&db).await {
        Ok(result) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "result": result
        })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": e
        })),
    }
}
