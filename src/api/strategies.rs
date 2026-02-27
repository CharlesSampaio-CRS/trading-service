use actix_web::{delete, get, post, put, web, HttpResponse, Responder};
use mongodb::bson::doc;
use crate::database::MongoDB;
use crate::models::{
    UserStrategies, StrategyItem, CreateStrategyRequest, UpdateStrategyRequest,
    StrategyResponse, StrategyListItem, StrategyStatus, GradualLot,
};
use crate::middleware::auth::Claims;
use crate::services::strategy_service;

const COLLECTION: &str = "user_strategy";

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
                .map_err(|e| format!("Failed to create doc: {}", e))?;
            collection.find_one(doc! { "user_id": user_id }).await
                .map_err(|e| format!("Failed to fetch: {}", e))?
                .ok_or_else(|| "Failed to fetch created doc".to_string())
        }
        Err(e) => Err(format!("Database error: {}", e)),
    }
}

#[get("")]
pub async fn get_strategies(user: web::ReqData<Claims>, db: web::Data<MongoDB>) -> impl Responder {
    match get_or_create_user_doc(&db, &user.sub).await {
        Ok(user_doc) => {
            let mut list: Vec<StrategyListItem> = user_doc.strategies.into_iter().map(StrategyListItem::from).collect();
            list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            let total = list.len();
            HttpResponse::Ok().json(serde_json::json!({ "success": true, "strategies": list, "total": total }))
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "error": e })),
    }
}

#[get("/{id}")]
pub async fn get_strategy(user: web::ReqData<Claims>, path: web::Path<String>, db: web::Data<MongoDB>) -> impl Responder {
    let sid = path.into_inner();
    match get_or_create_user_doc(&db, &user.sub).await {
        Ok(ud) => match ud.strategies.into_iter().find(|s| s.strategy_id == sid) {
            Some(s) => HttpResponse::Ok().json(serde_json::json!({ "success": true, "strategy": StrategyResponse::from(s) })),
            None => HttpResponse::NotFound().json(serde_json::json!({ "success": false, "error": "Strategy not found" })),
        },
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "error": e })),
    }
}

#[get("/{id}/stats")]
pub async fn get_strategy_stats(user: web::ReqData<Claims>, path: web::Path<String>, db: web::Data<MongoDB>) -> impl Responder {
    let sid = path.into_inner();
    match get_or_create_user_doc(&db, &user.sub).await {
        Ok(ud) => match ud.strategies.into_iter().find(|s| s.strategy_id == sid) {
            Some(s) => HttpResponse::Ok().json(serde_json::json!({ "success": true, "stats": s.compute_stats() })),
            None => HttpResponse::NotFound().json(serde_json::json!({ "success": false, "error": "Strategy not found" })),
        },
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "error": e })),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct PaginationQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[get("/{id}/executions")]
pub async fn get_strategy_executions(user: web::ReqData<Claims>, path: web::Path<String>, query: web::Query<PaginationQuery>, db: web::Data<MongoDB>) -> impl Responder {
    let sid = path.into_inner();
    match get_or_create_user_doc(&db, &user.sub).await {
        Ok(ud) => match ud.strategies.into_iter().find(|s| s.strategy_id == sid) {
            Some(s) => {
                let limit = query.limit.unwrap_or(50).min(200) as usize;
                let offset = query.offset.unwrap_or(0) as usize;
                let total = s.executions.len();
                let execs: Vec<_> = s.executions.iter().rev().skip(offset).take(limit).cloned().collect();
                HttpResponse::Ok().json(serde_json::json!({ "success": true, "executions": execs, "total": total }))
            }
            None => HttpResponse::NotFound().json(serde_json::json!({ "success": false, "error": "Strategy not found" })),
        },
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "error": e })),
    }
}

#[get("/{id}/signals")]
pub async fn get_strategy_signals(user: web::ReqData<Claims>, path: web::Path<String>, query: web::Query<PaginationQuery>, db: web::Data<MongoDB>) -> impl Responder {
    let sid = path.into_inner();
    match get_or_create_user_doc(&db, &user.sub).await {
        Ok(ud) => match ud.strategies.into_iter().find(|s| s.strategy_id == sid) {
            Some(s) => {
                let limit = query.limit.unwrap_or(50).min(200) as usize;
                let offset = query.offset.unwrap_or(0) as usize;
                let total = s.signals.len();
                let sigs: Vec<_> = s.signals.iter().rev().skip(offset).take(limit).cloned().collect();
                HttpResponse::Ok().json(serde_json::json!({ "success": true, "signals": sigs, "total": total }))
            }
            None => HttpResponse::NotFound().json(serde_json::json!({ "success": false, "error": "Strategy not found" })),
        },
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "error": e })),
    }
}

#[post("")]
pub async fn create_strategy(user: web::ReqData<Claims>, body: web::Json<CreateStrategyRequest>, db: web::Data<MongoDB>) -> impl Responder {
    let user_id = &user.sub;
    let collection = db.collection::<UserStrategies>(COLLECTION);
    let now = chrono::Utc::now().timestamp();
    let strategy_id = uuid::Uuid::new_v4().to_string();
    let mut config = body.config.clone();
    if config.gradual_sell && config.gradual_lots.is_empty() {
        config.gradual_lots = vec![
            GradualLot { lot_number: 1, sell_percent: 25.0, executed: false, executed_at: None, executed_price: None, realized_pnl: None },
            GradualLot { lot_number: 2, sell_percent: 25.0, executed: false, executed_at: None, executed_price: None, realized_pnl: None },
            GradualLot { lot_number: 3, sell_percent: 25.0, executed: false, executed_at: None, executed_price: None, realized_pnl: None },
            GradualLot { lot_number: 4, sell_percent: 25.0, executed: false, executed_at: None, executed_price: None, realized_pnl: None },
        ];
    }
    let new_strategy = StrategyItem {
        strategy_id: strategy_id.clone(), name: body.name.clone(), symbol: body.symbol.clone(),
        exchange_id: body.exchange_id.clone(), exchange_name: body.exchange_name.clone(),
        is_active: true, status: StrategyStatus::Monitoring, config,
        position: None, executions: vec![], signals: vec![],
        last_checked_at: None, last_price: None, last_gradual_sell_at: None,
        error_message: None, total_pnl_usd: 0.0, total_executions: 0,
        started_at: now, created_at: now, updated_at: now,
    };
    let bson = match mongodb::bson::to_bson(&new_strategy) {
        Ok(b) => b,
        Err(e) => return HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "error": format!("Serialize: {}", e) })),
    };
    let _ = get_or_create_user_doc(&db, user_id).await;
    match collection.update_one(doc! { "user_id": user_id }, doc! { "$push": { "strategies": bson }, "$set": { "updated_at": now } }).await {
        Ok(r) if r.modified_count > 0 => HttpResponse::Created().json(serde_json::json!({ "success": true, "strategy": StrategyResponse::from(new_strategy) })),
        Ok(_) => HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "error": "Failed to add strategy" })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "error": format!("Create failed: {}", e) })),
    }
}

#[put("/{id}")]
pub async fn update_strategy(user: web::ReqData<Claims>, path: web::Path<String>, body: web::Json<UpdateStrategyRequest>, db: web::Data<MongoDB>) -> impl Responder {
    let user_id = &user.sub;
    let sid = path.into_inner();
    let collection = db.collection::<UserStrategies>(COLLECTION);
    match get_or_create_user_doc(&db, user_id).await {
        Ok(ud) => {
            if !ud.strategies.iter().any(|s| s.strategy_id == sid) {
                return HttpResponse::NotFound().json(serde_json::json!({ "success": false, "error": "Strategy not found" }));
            }
        }
        Err(e) => return HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "error": e })),
    }
    let now = chrono::Utc::now().timestamp();
    let p = "strategies.$[elem]";
    let mut udoc = doc! { format!("{}.updated_at", p): now, "updated_at": now };
    if let Some(v) = &body.name { udoc.insert(format!("{}.name", p), v); }
    if let Some(v) = &body.symbol { udoc.insert(format!("{}.symbol", p), v); }
    if let Some(v) = &body.exchange_id { udoc.insert(format!("{}.exchange_id", p), v); }
    if let Some(v) = &body.exchange_name { udoc.insert(format!("{}.exchange_name", p), v); }
    if let Some(active) = body.is_active {
        udoc.insert(format!("{}.is_active", p), active);
        udoc.insert(format!("{}.status", p), if active { "monitoring" } else { "paused" });
    }
    if let Some(cfg) = &body.config { udoc.insert(format!("{}.config", p), mongodb::bson::to_bson(cfg).unwrap()); }
    let af = doc! { "elem.strategy_id": &sid };
    match collection.update_one(doc! { "user_id": user_id }, doc! { "$set": udoc }).array_filters(vec![af]).await {
        Ok(_) => match get_or_create_user_doc(&db, user_id).await {
            Ok(ud) => match ud.strategies.into_iter().find(|s| s.strategy_id == sid) {
                Some(s) => HttpResponse::Ok().json(serde_json::json!({ "success": true, "strategy": StrategyResponse::from(s) })),
                _ => HttpResponse::Ok().json(serde_json::json!({ "success": true, "message": "Updated" })),
            },
            _ => HttpResponse::Ok().json(serde_json::json!({ "success": true, "message": "Updated" })),
        },
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "error": format!("Update failed: {}", e) })),
    }
}

#[delete("/{id}")]
pub async fn delete_strategy(user: web::ReqData<Claims>, path: web::Path<String>, db: web::Data<MongoDB>) -> impl Responder {
    let sid = path.into_inner();
    let collection = db.collection::<UserStrategies>(COLLECTION);
    let now = chrono::Utc::now().timestamp();
    match collection.update_one(doc! { "user_id": &user.sub }, doc! { "$pull": { "strategies": { "strategy_id": &sid } }, "$set": { "updated_at": now } }).await {
        Ok(r) if r.modified_count > 0 => HttpResponse::Ok().json(serde_json::json!({ "success": true, "message": "Deleted" })),
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({ "success": false, "error": "Not found" })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "error": format!("Delete failed: {}", e) })),
    }
}

#[post("/{id}/activate")]
pub async fn activate_strategy(user: web::ReqData<Claims>, path: web::Path<String>, db: web::Data<MongoDB>) -> impl Responder {
    match strategy_service::activate_strategy(&db, &path.into_inner(), &user.sub).await {
        Ok(s) => HttpResponse::Ok().json(serde_json::json!({ "success": true, "strategy": StrategyResponse::from(s) })),
        Err(e) => HttpResponse::BadRequest().json(serde_json::json!({ "success": false, "error": e })),
    }
}

#[post("/{id}/pause")]
pub async fn pause_strategy(user: web::ReqData<Claims>, path: web::Path<String>, db: web::Data<MongoDB>) -> impl Responder {
    match strategy_service::pause_strategy(&db, &path.into_inner(), &user.sub).await {
        Ok(s) => HttpResponse::Ok().json(serde_json::json!({ "success": true, "strategy": StrategyResponse::from(s) })),
        Err(e) => HttpResponse::BadRequest().json(serde_json::json!({ "success": false, "error": e })),
    }
}

#[post("/{id}/tick")]
pub async fn tick_strategy(user: web::ReqData<Claims>, path: web::Path<String>, db: web::Data<MongoDB>) -> impl Responder {
    let sid = path.into_inner();
    let uid = &user.sub;
    let ud = match get_or_create_user_doc(&db, uid).await {
        Ok(d) => d,
        Err(e) => return HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "error": e })),
    };
    let strategy = match ud.strategies.into_iter().find(|s| s.strategy_id == sid) {
        Some(s) => s,
        None => return HttpResponse::NotFound().json(serde_json::json!({ "success": false, "error": "Strategy not found" })),
    };
    let tr = strategy_service::tick(&db, uid, &strategy).await;
    let _ = strategy_service::persist_tick_result(&db, uid, &strategy, &tr).await;
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "tick": {
            "strategy_id": tr.strategy_id, "symbol": tr.symbol, "price": tr.price,
            "signals_count": tr.signals.len(), "executions_count": tr.executions.len(),
            "new_status": tr.new_status, "error": tr.error, "signals": tr.signals,
        }
    }))
}

#[post("/process-all")]
pub async fn process_all_strategies(_user: web::ReqData<Claims>, db: web::Data<MongoDB>) -> impl Responder {
    match strategy_service::process_active_strategies(&db).await {
        Ok(r) => HttpResponse::Ok().json(serde_json::json!({ "success": true, "result": r })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "error": e })),
    }
}
