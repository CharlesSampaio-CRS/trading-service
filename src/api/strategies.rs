use actix_web::{delete, get, post, put, web, HttpResponse, Responder};
use mongodb::bson::{doc, oid::ObjectId};
use crate::database::MongoDB;
use crate::models::{Strategy, CreateStrategyRequest, UpdateStrategyRequest, StrategyResponse, StrategyListItem, StrategyStatus};
use crate::middleware::auth::Claims;

/// GET /api/v1/strategies - Lista todas as estratégias do usuário (compacta)
#[get("")]
pub async fn get_strategies(user: web::ReqData<Claims>, db: web::Data<MongoDB>) -> impl Responder {
    let user_id = &user.sub;

    let collection = db.collection::<Strategy>("strategies");

    match collection
        .find(doc! { "user_id": &user_id })
        .await
    {
        Ok(mut cursor) => {
            let mut strategies = Vec::new();

            use futures::stream::StreamExt;
            while let Some(result) = cursor.next().await {
                match result {
                    Ok(strategy) => strategies.push(StrategyListItem::from(strategy)),
                    Err(e) => {
                        eprintln!("❌ Erro ao processar estratégia: {}", e);
                    }
                }
            }

            // Ordena por data de atualização (mais recentes primeiro)
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

/// GET /api/v1/strategies/{id} - Busca estratégia completa com executions e signals
#[get("/{id}")]
pub async fn get_strategy(user: web::ReqData<Claims>, path: web::Path<String>, db: web::Data<MongoDB>) -> impl Responder {
    let user_id = &user.sub;

    let strategy_id = path.into_inner();
    let object_id = match ObjectId::parse_str(&strategy_id) {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "error": "Invalid strategy ID"
            }))
        }
    };

    let collection = db.collection::<Strategy>("strategies");

    match collection
        .find_one(doc! { "_id": object_id, "user_id": &user_id })
        .await
    {
        Ok(Some(strategy)) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "strategy": StrategyResponse::from(strategy)
        })),
        Ok(None) => HttpResponse::NotFound().json(serde_json::json!({
            "success": false,
            "error": "Strategy not found"
        })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": format!("Failed to fetch strategy: {}", e)
        })),
    }
}

/// GET /api/v1/strategies/{id}/stats - Estatísticas da estratégia
#[get("/{id}/stats")]
pub async fn get_strategy_stats(user: web::ReqData<Claims>, path: web::Path<String>, db: web::Data<MongoDB>) -> impl Responder {
    let user_id = &user.sub;

    let strategy_id = path.into_inner();
    let object_id = match ObjectId::parse_str(&strategy_id) {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "error": "Invalid strategy ID"
            }))
        }
    };

    let collection = db.collection::<Strategy>("strategies");

    match collection
        .find_one(doc! { "_id": object_id, "user_id": &user_id })
        .await
    {
        Ok(Some(strategy)) => {
            let stats = strategy.compute_stats();
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "stats": stats,
                "strategy_info": {
                    "id": strategy.id.map(|id| id.to_hex()).unwrap_or_default(),
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
        Ok(None) => HttpResponse::NotFound().json(serde_json::json!({
            "success": false,
            "error": "Strategy not found"
        })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": format!("Failed to fetch strategy stats: {}", e)
        })),
    }
}

/// GET /api/v1/strategies/{id}/executions - Lista execuções paginadas
#[get("/{id}/executions")]
pub async fn get_strategy_executions(
    user: web::ReqData<Claims>,
    path: web::Path<String>,
    query: web::Query<PaginationQuery>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;

    let strategy_id = path.into_inner();
    let object_id = match ObjectId::parse_str(&strategy_id) {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "error": "Invalid strategy ID"
            }))
        }
    };

    let collection = db.collection::<Strategy>("strategies");

    match collection
        .find_one(doc! { "_id": object_id, "user_id": &user_id })
        .await
    {
        Ok(Some(strategy)) => {
            let limit = query.limit.unwrap_or(50).min(200) as usize;
            let offset = query.offset.unwrap_or(0) as usize;

            let total = strategy.executions.len();
            let executions: Vec<_> = strategy.executions.iter()
                .rev() // Mais recentes primeiro
                .skip(offset)
                .take(limit)
                .cloned()
                .collect();

            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "executions": executions,
                "total": total,
                "limit": limit,
                "offset": offset
            }))
        }
        Ok(None) => HttpResponse::NotFound().json(serde_json::json!({
            "success": false,
            "error": "Strategy not found"
        })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": format!("Failed to fetch executions: {}", e)
        })),
    }
}

/// GET /api/v1/strategies/{id}/signals - Lista sinais paginados
#[get("/{id}/signals")]
pub async fn get_strategy_signals(
    user: web::ReqData<Claims>,
    path: web::Path<String>,
    query: web::Query<PaginationQuery>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;

    let strategy_id = path.into_inner();
    let object_id = match ObjectId::parse_str(&strategy_id) {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "error": "Invalid strategy ID"
            }))
        }
    };

    let collection = db.collection::<Strategy>("strategies");

    match collection
        .find_one(doc! { "_id": object_id, "user_id": &user_id })
        .await
    {
        Ok(Some(strategy)) => {
            let limit = query.limit.unwrap_or(50).min(200) as usize;
            let offset = query.offset.unwrap_or(0) as usize;

            let total = strategy.signals.len();
            let signals: Vec<_> = strategy.signals.iter()
                .rev() // Mais recentes primeiro
                .skip(offset)
                .take(limit)
                .cloned()
                .collect();

            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "signals": signals,
                "total": total,
                "limit": limit,
                "offset": offset
            }))
        }
        Ok(None) => HttpResponse::NotFound().json(serde_json::json!({
            "success": false,
            "error": "Strategy not found"
        })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": format!("Failed to fetch signals: {}", e)
        })),
    }
}

/// Query de paginação
#[derive(Debug, serde::Deserialize)]
pub struct PaginationQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// POST /api/v1/strategies - Cria nova estratégia
#[post("")]
pub async fn create_strategy(
    user: web::ReqData<Claims>,
    body: web::Json<CreateStrategyRequest>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;

    let collection = db.collection::<Strategy>("strategies");

    let now = chrono::Utc::now().timestamp();
    let strategy = Strategy {
        id: None,
        user_id: user_id.to_string(),
        name: body.name.clone(),
        description: body.description.clone(),
        strategy_type: body.strategy_type.clone(),
        symbol: body.symbol.clone(),
        exchange_id: body.exchange_id.clone(),
        exchange_name: body.exchange_name.clone(),
        is_active: true,
        // Fase 2: novos campos
        status: StrategyStatus::Idle,
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

    match collection.insert_one(&strategy).await {
        Ok(result) => {
            let mut created_strategy = strategy;
            created_strategy.id = Some(result.inserted_id.as_object_id().unwrap());

            HttpResponse::Created().json(serde_json::json!({
                "success": true,
                "strategy": StrategyResponse::from(created_strategy)
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": format!("Failed to create strategy: {}", e)
        })),
    }
}

/// PUT /api/v1/strategies/{id} - Atualiza estratégia
#[put("/{id}")]
pub async fn update_strategy(
    user: web::ReqData<Claims>,
    path: web::Path<String>,
    body: web::Json<UpdateStrategyRequest>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;

    let strategy_id = path.into_inner();
    let object_id = match ObjectId::parse_str(&strategy_id) {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "error": "Invalid strategy ID"
            }))
        }
    };

    let collection = db.collection::<Strategy>("strategies");

    // Verifica se a estratégia existe e pertence ao usuário
    match collection
        .find_one(doc! { "_id": object_id, "user_id": &user_id })
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => {
            return HttpResponse::NotFound().json(serde_json::json!({
                "success": false,
                "error": "Strategy not found"
            }))
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": format!("Failed to verify strategy: {}", e)
            }))
        }
    }

    // Constrói o documento de atualização
    let mut update_doc = doc! {
        "updated_at": chrono::Utc::now().timestamp()
    };

    if let Some(name) = &body.name {
        update_doc.insert("name", name);
    }
    if let Some(description) = &body.description {
        update_doc.insert("description", description);
    }
    if let Some(strategy_type) = &body.strategy_type {
        update_doc.insert("strategy_type", strategy_type);
    }
    if let Some(symbol) = &body.symbol {
        update_doc.insert("symbol", symbol);
    }
    if let Some(exchange_id) = &body.exchange_id {
        update_doc.insert("exchange_id", exchange_id);
    }
    if let Some(exchange_name) = &body.exchange_name {
        update_doc.insert("exchange_name", exchange_name);
    }
    if let Some(is_active) = body.is_active {
        update_doc.insert("is_active", is_active);
        // Sync status com is_active
        if is_active {
            update_doc.insert("status", "monitoring");
        } else {
            update_doc.insert("status", "paused");
        }
    }
    if let Some(status) = &body.status {
        update_doc.insert("status", mongodb::bson::to_bson(status).unwrap());
        // Sync is_active com status
        match status {
            StrategyStatus::Paused | StrategyStatus::Completed | StrategyStatus::Error => {
                update_doc.insert("is_active", false);
            }
            StrategyStatus::Monitoring | StrategyStatus::InPosition | StrategyStatus::BuyPending | StrategyStatus::SellPending => {
                update_doc.insert("is_active", true);
            }
            _ => {}
        }
    }
    if let Some(config) = &body.config {
        update_doc.insert("config", mongodb::bson::to_bson(config).unwrap());
    }
    if let Some(config_legacy) = &body.config_legacy {
        update_doc.insert("config_legacy", mongodb::bson::to_bson(config_legacy).unwrap());
    }
    if let Some(check_interval) = body.check_interval_secs {
        update_doc.insert("check_interval_secs", check_interval);
    }

    match collection
        .update_one(
            doc! { "_id": object_id, "user_id": &user_id },
            doc! { "$set": update_doc },
        )
        .await
    {
        Ok(_) => {
            // Busca a estratégia atualizada
            match collection
                .find_one(doc! { "_id": object_id })
                .await
            {
                Ok(Some(strategy)) => HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "strategy": StrategyResponse::from(strategy)
                })),
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

/// DELETE /api/v1/strategies/{id} - Deleta estratégia
#[delete("/{id}")]
pub async fn delete_strategy(user: web::ReqData<Claims>, path: web::Path<String>, db: web::Data<MongoDB>) -> impl Responder {
    let user_id = &user.sub;

    let strategy_id = path.into_inner();
    let object_id = match ObjectId::parse_str(&strategy_id) {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "error": "Invalid strategy ID"
            }))
        }
    };

    let collection = db.collection::<Strategy>("strategies");

    match collection
        .delete_one(doc! { "_id": object_id, "user_id": &user_id })
        .await
    {
        Ok(result) => {
            if result.deleted_count > 0 {
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
