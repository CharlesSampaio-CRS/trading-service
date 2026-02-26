use actix_web::{delete, get, post, put, web, HttpResponse, Responder};
use mongodb::bson::{doc, oid::ObjectId};
use crate::database::MongoDB;
use crate::models::{
    StrategyTemplate, CreateTemplateRequest, UpdateTemplateRequest,
    StrategyTemplateResponse,
};
use crate::middleware::auth::Claims;

/// GET /api/v1/strategy-templates - Lista todos os templates (defaults do banco + do usuário)
#[get("")]
pub async fn get_templates(user: web::ReqData<Claims>, db: web::Data<MongoDB>) -> impl Responder {
    let user_id = &user.sub;
    let collection = db.collection::<StrategyTemplate>("strategy_templates");

    // Busca todos: defaults (is_default=true) + templates do usuário
    let filter = doc! {
        "$or": [
            { "is_default": true },
            { "user_id": user_id }
        ]
    };

    let mut all_templates: Vec<StrategyTemplateResponse> = Vec::new();

    match collection.find(filter).await {
        Ok(mut cursor) => {
            use futures::stream::StreamExt;
            while let Some(result) = cursor.next().await {
                match result {
                    Ok(tpl) => all_templates.push(StrategyTemplateResponse::from(tpl)),
                    Err(e) => eprintln!("❌ Erro ao processar template: {}", e),
                }
            }
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": format!("Failed to fetch templates: {}", e)
            }));
        }
    }

    // Ordena: defaults primeiro (por nome), depois os do usuário (mais recentes primeiro)
    all_templates.sort_by(|a, b| {
        match (a.is_default, b.is_default) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            (true, true) => a.name.cmp(&b.name),
            (false, false) => b.created_at.cmp(&a.created_at),
        }
    });

    let total = all_templates.len();
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "templates": all_templates,
        "total": total
    }))
}

/// GET /api/v1/strategy-templates/{id} - Busca template específico
#[get("/{id}")]
pub async fn get_template(user: web::ReqData<Claims>, path: web::Path<String>, db: web::Data<MongoDB>) -> impl Responder {
    let template_id = path.into_inner();

    let object_id = match ObjectId::parse_str(&template_id) {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "error": "Invalid template ID"
            }));
        }
    };

    let user_id = &user.sub;
    let collection = db.collection::<StrategyTemplate>("strategy_templates");

    // Busca: ou é default (visível pra todos) ou pertence ao usuário
    let filter = doc! {
        "_id": object_id,
        "$or": [
            { "is_default": true },
            { "user_id": user_id }
        ]
    };

    match collection.find_one(filter).await {
        Ok(Some(tpl)) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "template": StrategyTemplateResponse::from(tpl)
        })),
        Ok(None) => HttpResponse::NotFound().json(serde_json::json!({
            "success": false,
            "error": "Template not found"
        })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": format!("Failed to fetch template: {}", e)
        })),
    }
}

/// POST /api/v1/strategy-templates - Cria novo template customizado
#[post("")]
pub async fn create_template(
    user: web::ReqData<Claims>,
    body: web::Json<CreateTemplateRequest>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;
    let collection = db.collection::<StrategyTemplate>("strategy_templates");

    let now = chrono::Utc::now().timestamp();
    let template = StrategyTemplate {
        id: None,
        user_id: user_id.to_string(),
        name: body.name.clone(),
        icon: body.icon.clone(),
        strategy_type: body.strategy_type.clone(),
        risk: body.risk.clone(),
        summary: body.summary.clone(),
        configs: body.configs.clone(),
        how_it_works: body.how_it_works.clone(),
        is_default: false,
        created_at: now,
        updated_at: now,
    };

    match collection.insert_one(&template).await {
        Ok(result) => {
            let mut created = template;
            created.id = Some(result.inserted_id.as_object_id().unwrap());

            HttpResponse::Created().json(serde_json::json!({
                "success": true,
                "template": StrategyTemplateResponse::from(created)
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": format!("Failed to create template: {}", e)
        })),
    }
}

/// PUT /api/v1/strategy-templates/{id} - Atualiza template customizado
#[put("/{id}")]
pub async fn update_template(
    user: web::ReqData<Claims>,
    path: web::Path<String>,
    body: web::Json<UpdateTemplateRequest>,
    db: web::Data<MongoDB>,
) -> impl Responder {
    let user_id = &user.sub;
    let template_id = path.into_inner();

    let object_id = match ObjectId::parse_str(&template_id) {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "error": "Invalid template ID"
            }));
        }
    };

    let collection = db.collection::<StrategyTemplate>("strategy_templates");

    // Verifica se existe e pertence ao usuário (não permite editar defaults)
    match collection.find_one(doc! { "_id": object_id, "user_id": user_id }).await {
        Ok(Some(existing)) => {
            if existing.is_default {
                return HttpResponse::Forbidden().json(serde_json::json!({
                    "success": false,
                    "error": "Cannot edit default templates"
                }));
            }
        }
        Ok(None) => {
            return HttpResponse::NotFound().json(serde_json::json!({
                "success": false,
                "error": "Template not found"
            }));
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": format!("Failed to verify template: {}", e)
            }));
        }
    }

    let mut update_doc = doc! { "updated_at": chrono::Utc::now().timestamp() };

    if let Some(name) = &body.name { update_doc.insert("name", name); }
    if let Some(icon) = &body.icon { update_doc.insert("icon", icon); }
    if let Some(strategy_type) = &body.strategy_type { update_doc.insert("strategy_type", strategy_type); }
    if let Some(summary) = &body.summary { update_doc.insert("summary", summary); }
    if let Some(risk) = &body.risk { update_doc.insert("risk", mongodb::bson::to_bson(risk).unwrap()); }
    if let Some(configs) = &body.configs { update_doc.insert("configs", mongodb::bson::to_bson(configs).unwrap()); }
    if let Some(how_it_works) = &body.how_it_works { update_doc.insert("how_it_works", mongodb::bson::to_bson(how_it_works).unwrap()); }

    match collection.update_one(
        doc! { "_id": object_id, "user_id": user_id },
        doc! { "$set": update_doc },
    ).await {
        Ok(_) => {
            match collection.find_one(doc! { "_id": object_id }).await {
                Ok(Some(tpl)) => HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "template": StrategyTemplateResponse::from(tpl)
                })),
                _ => HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": "Template updated successfully"
                })),
            }
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": format!("Failed to update template: {}", e)
        })),
    }
}

/// DELETE /api/v1/strategy-templates/{id} - Deleta template customizado
#[delete("/{id}")]
pub async fn delete_template(user: web::ReqData<Claims>, path: web::Path<String>, db: web::Data<MongoDB>) -> impl Responder {
    let user_id = &user.sub;
    let template_id = path.into_inner();

    let object_id = match ObjectId::parse_str(&template_id) {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "error": "Invalid template ID"
            }));
        }
    };

    let collection = db.collection::<StrategyTemplate>("strategy_templates");

    match collection.delete_one(doc! { "_id": object_id, "user_id": user_id, "is_default": false }).await {
        Ok(result) => {
            if result.deleted_count > 0 {
                HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": "Template deleted successfully"
                }))
            } else {
                HttpResponse::NotFound().json(serde_json::json!({
                    "success": false,
                    "error": "Template not found or is a default template"
                }))
            }
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": format!("Failed to delete template: {}", e)
        })),
    }
}
