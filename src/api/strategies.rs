use actix_web::{delete, get, post, put, web, HttpResponse, Responder};
use mongodb::bson::{doc, oid::ObjectId};
use crate::database::MongoDB;
use crate::models::{Strategy, CreateStrategyRequest, UpdateStrategyRequest, StrategyResponse};
use crate::middleware::auth::Claims;

/// GET /api/v1/strategies - Lista todas as estratégias do usuário
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
                    Ok(strategy) => strategies.push(StrategyResponse::from(strategy)),
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

/// GET /api/v1/strategies/{id} - Busca estratégia específica
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
        is_active: true, // Nova estratégia começa ativa
        config: body.config.clone(),
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
    }
    if let Some(config) = &body.config {
        update_doc.insert("config", mongodb::bson::to_bson(config).unwrap());
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
