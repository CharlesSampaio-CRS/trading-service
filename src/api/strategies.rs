use actix_web::{delete, get, post, put, web, HttpResponse, Responder};
use mongodb::bson::doc;
use crate::database::MongoDB;
use crate::models::{
    UserStrategies, StrategyItem, CreateStrategyRequest, UpdateStrategyRequest,
    StrategyResponse, StrategyListItem, StrategyStatus, GradualLot, StrategySignal,
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
    log::info!("📝 POST /strategies - user: {}, name: '{}', symbol: '{}'", user_id, body.name, body.symbol);

    // ── Input Validation ────────────────────────────────────────────
    if body.name.trim().is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false, "error": "Strategy name is required",
            "field": "name"
        }));
    }
    if body.name.len() > 100 {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false, "error": "Strategy name must be at most 100 characters",
            "field": "name"
        }));
    }
    if body.symbol.trim().is_empty() || !body.symbol.contains('/') {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false, "error": "Symbol must be a valid trading pair (e.g. BTC/USDT)",
            "field": "symbol"
        }));
    }
    if body.exchange_id.trim().is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false, "error": "Exchange ID is required",
            "field": "exchange_id"
        }));
    }
    if body.config.base_price <= 0.0 {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false, "error": "Base price must be greater than 0",
            "field": "config.base_price"
        }));
    }
    if body.config.take_profit_percent <= 0.0 || body.config.take_profit_percent > 1000.0 {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false, "error": "Take profit must be between 0.01% and 1000%",
            "field": "config.take_profit_percent"
        }));
    }
    if body.config.stop_loss_percent <= 0.0 || body.config.stop_loss_percent > 100.0 {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false, "error": "Stop loss must be between 0.01% and 100%",
            "field": "config.stop_loss_percent"
        }));
    }
    if body.config.fee_percent < 0.0 || body.config.fee_percent > 50.0 {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false, "error": "Fee must be between 0% and 50%",
            "field": "config.fee_percent"
        }));
    }
    if body.config.gradual_sell && (body.config.gradual_take_percent <= 0.0 || body.config.gradual_take_percent > 100.0) {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false, "error": "Gradual take percent must be between 0.01% and 100% when gradual sell is enabled",
            "field": "config.gradual_take_percent"
        }));
    }
    if body.config.time_execution_min < 1 || body.config.time_execution_min > 43200 {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false, "error": "Execution time must be between 1 minute and 30 days (43200 min)",
            "field": "config.time_execution_min"
        }));
    }
    if body.config.timer_gradual_min < 1 || body.config.timer_gradual_min > 1440 {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false, "error": "Gradual timer must be between 1 minute and 24 hours (1440 min)",
            "field": "config.timer_gradual_min"
        }));
    }

    // ── Limit check: max 20 strategies per user ─────────────────────
    let collection = db.collection::<UserStrategies>(COLLECTION);
    match get_or_create_user_doc(&db, user_id).await {
        Ok(ud) => {
            let active_count = ud.strategies.iter().filter(|s| s.is_active).count();
            if active_count >= 20 {
                return HttpResponse::BadRequest().json(serde_json::json!({
                    "success": false, "error": "Maximum of 20 active strategies reached. Pause or delete existing strategies first.",
                    "limit": 20, "current": active_count
                }));
            }
        }
        Err(e) => {
            log::error!("❌ Failed to check strategy limit for user {}: {}", user_id, e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false, "error": "Failed to verify strategy limit. Please try again."
            }));
        }
    }

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
    log::info!("⚡ POST /strategies/{}/tick - user: {}", sid, uid);

    let ud = match get_or_create_user_doc(&db, uid).await {
        Ok(d) => d,
        Err(e) => {
            log::error!("❌ Tick failed (DB): user={}, strategy={}, error={}", uid, sid, e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": "Failed to load strategies. Please try again later."
            }));
        }
    };
    let strategy = match ud.strategies.into_iter().find(|s| s.strategy_id == sid) {
        Some(s) => s,
        None => {
            log::warn!("⚠️ Tick: strategy {} not found for user {}", sid, uid);
            return HttpResponse::NotFound().json(serde_json::json!({
                "success": false,
                "error": "Strategy not found. It may have been deleted."
            }));
        }
    };

    if !strategy.is_active {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "error": format!("Strategy '{}' is not active (status: {}). Activate it first.", strategy.name, strategy.status)
        }));
    }

    let tr = strategy_service::tick(&db, uid, &strategy).await;

    if let Err(e) = strategy_service::persist_tick_result(&db, uid, &strategy, &tr).await {
        log::error!("❌ Tick persist failed: strategy={}, error={}", sid, e);
        return HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": "Tick executed but failed to save results. Data may be inconsistent.",
            "tick": {
                "strategy_id": tr.strategy_id, "symbol": tr.symbol, "price": tr.price,
                "signals_count": tr.signals.len(), "error": tr.error,
            }
        }));
    }

    let has_error = tr.error.is_some();
    let status_changed = tr.new_status.is_some();

    if has_error {
        log::warn!("⚠️ Tick warning: strategy={}, symbol={}, error={}", sid, tr.symbol, tr.error.as_deref().unwrap_or("unknown"));
    } else if status_changed {
        log::info!("📊 Tick status change: strategy={}, symbol={}, price={:.4}, new_status={:?}, signals={}, execs={}",
            sid, tr.symbol, tr.price, tr.new_status, tr.signals.len(), tr.executions.len());
    }

    // ── Build rich tick summary for frontend ──────────────────────
    let last_signal_message = tr.signals.last().map(|s| s.message.clone());
    let last_signal_type = tr.signals.last().map(|s| &s.signal_type);
    let summary = if let Some(ref err) = tr.error {
        err.clone()
    } else if let Some(msg) = &last_signal_message {
        msg.clone()
    } else {
        format!("Tick processado — sem sinais para {}", tr.symbol)
    };

    let acted_signals: Vec<&StrategySignal> = tr.signals.iter().filter(|s| s.acted).collect();
    let info_signals: Vec<&StrategySignal> = tr.signals.iter().filter(|s| !s.acted).collect();

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "tick": {
            "strategy_id": tr.strategy_id,
            "symbol": tr.symbol,
            "price": tr.price,
            "signals_count": tr.signals.len(),
            "executions_count": tr.executions.len(),
            "new_status": tr.new_status,
            "error": tr.error,
            "summary": summary,
            "signals": tr.signals,
            "executions": tr.executions,
            "acted_count": acted_signals.len(),
            "info_count": info_signals.len(),
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
