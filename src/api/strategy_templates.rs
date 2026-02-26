use actix_web::{delete, get, post, put, web, HttpResponse, Responder};
use mongodb::bson::{doc, oid::ObjectId};
use crate::database::MongoDB;
use crate::models::{
    StrategyTemplate, CreateTemplateRequest, UpdateTemplateRequest,
    StrategyTemplateResponse, RiskLevel, TemplateConfig,
};
use crate::middleware::auth::Claims;

/// Retorna os 3 templates padrão do sistema (hardcoded)
fn default_templates() -> Vec<StrategyTemplate> {
    let now = chrono::Utc::now().timestamp();

    vec![
        StrategyTemplate {
            id: None,
            user_id: String::new(),
            name: "Simples".into(),
            icon: "📊".into(),
            strategy_type: "Grid Trading".into(),
            risk: RiskLevel { label: "Médio".into(), color: "#f59e0b".into() },
            summary: "Cria ordens de compra e venda em intervalos fixos de preço. Ideal para mercados laterais.".into(),
            configs: vec![
                TemplateConfig { label: "Tipo".into(), value: "Grid Trading".into(), detail: None },
                TemplateConfig { label: "Take Profit".into(), value: "5%".into(), detail: Some("1 nível — fecha 100% da posição".into()) },
                TemplateConfig { label: "Stop Loss".into(), value: "2%".into(), detail: Some("Fecha posição se cair 2%".into()) },
                TemplateConfig { label: "Grid Levels".into(), value: "5".into(), detail: Some("5 ordens espaçadas".into()) },
                TemplateConfig { label: "Espaçamento".into(), value: "0.5%".into(), detail: Some("Entre cada nível do grid".into()) },
                TemplateConfig { label: "Investimento mín.".into(), value: "50 USDT".into(), detail: None },
                TemplateConfig { label: "Modo".into(), value: "Spot".into(), detail: None },
            ],
            how_it_works: vec![
                "1. Divide o range de preço em 5 níveis (grid)".into(),
                "2. Coloca ordens de compra abaixo do preço atual".into(),
                "3. Coloca ordens de venda acima do preço atual".into(),
                "4. Lucra com as oscilações entre os níveis".into(),
                "5. Stop Loss em 2% protege contra queda forte".into(),
                "6. Take Profit em 5% encerra quando atingir o alvo".into(),
            ],
            is_default: true,
            created_at: now,
            updated_at: now,
        },
        StrategyTemplate {
            id: None,
            user_id: String::new(),
            name: "Conservadora".into(),
            icon: "🛡️".into(),
            strategy_type: "DCA (Dollar Cost Averaging)".into(),
            risk: RiskLevel { label: "Baixo".into(), color: "#10b981".into() },
            summary: "Compra em parcelas para diluir o preço médio. Proteção máxima com 2 TPs + trailing stop.".into(),
            configs: vec![
                TemplateConfig { label: "Tipo".into(), value: "Dollar Cost Averaging".into(), detail: None },
                TemplateConfig { label: "Take Profit 1".into(), value: "3%".into(), detail: Some("Vende 50% da posição".into()) },
                TemplateConfig { label: "Take Profit 2".into(), value: "6%".into(), detail: Some("Vende os 50% restantes".into()) },
                TemplateConfig { label: "Stop Loss".into(), value: "3%".into(), detail: Some("Proteção contra queda".into()) },
                TemplateConfig { label: "Trailing Stop".into(), value: "1.5%".into(), detail: Some("Protege lucros em alta".into()) },
                TemplateConfig { label: "Intervalo DCA".into(), value: "60 min".into(), detail: Some("Compra a cada 60 min".into()) },
                TemplateConfig { label: "Máx. DCA Orders".into(), value: "3".into(), detail: Some("Até 3 compras parciais".into()) },
                TemplateConfig { label: "Investimento mín.".into(), value: "100 USDT".into(), detail: None },
                TemplateConfig { label: "Modo".into(), value: "Spot".into(), detail: None },
            ],
            how_it_works: vec![
                "1. Primeira compra no preço atual".into(),
                "2. Se cair, compra mais a cada 60 min (até 3x)".into(),
                "3. Preço médio melhora a cada DCA".into(),
                "4. TP1 em +3%: vende metade, garante lucro".into(),
                "5. TP2 em +6%: vende o resto, lucro máximo".into(),
                "6. Trailing stop 1.5% acompanha o preço em alta".into(),
                "7. Stop loss 3% limita perda se não recuperar".into(),
            ],
            is_default: true,
            created_at: now,
            updated_at: now,
        },
        StrategyTemplate {
            id: None,
            user_id: String::new(),
            name: "Agressiva".into(),
            icon: "🚀".into(),
            strategy_type: "Trailing Stop + DCA".into(),
            risk: RiskLevel { label: "Alto".into(), color: "#ef4444".into() },
            summary: "Busca lucro máximo com 3 TPs progressivos, trailing stop agressivo e DCA ativo.".into(),
            configs: vec![
                TemplateConfig { label: "Tipo".into(), value: "Trailing Stop + DCA".into(), detail: None },
                TemplateConfig { label: "Take Profit 1".into(), value: "5%".into(), detail: Some("Vende 30% da posição".into()) },
                TemplateConfig { label: "Take Profit 2".into(), value: "10%".into(), detail: Some("Vende 30% da posição".into()) },
                TemplateConfig { label: "Take Profit 3".into(), value: "20%".into(), detail: Some("Vende 40% restantes".into()) },
                TemplateConfig { label: "Stop Loss".into(), value: "5%".into(), detail: Some("Margem maior para volatilidade".into()) },
                TemplateConfig { label: "Trailing Stop".into(), value: "2%".into(), detail: Some("Segue o preço em alta".into()) },
                TemplateConfig { label: "DCA Ativo".into(), value: "Sim".into(), detail: Some("Compra nas quedas".into()) },
                TemplateConfig { label: "Intervalo DCA".into(), value: "30 min".into(), detail: Some("Agressivo, a cada 30 min".into()) },
                TemplateConfig { label: "Máx. DCA Orders".into(), value: "5".into(), detail: Some("Até 5 compras parciais".into()) },
                TemplateConfig { label: "Investimento mín.".into(), value: "200 USDT".into(), detail: None },
                TemplateConfig { label: "Modo".into(), value: "Spot".into(), detail: None },
            ],
            how_it_works: vec![
                "1. Compra inicial no preço atual".into(),
                "2. DCA agressivo: compra a cada 30 min se cair (até 5x)".into(),
                "3. TP1 em +5%: realiza 30%, garante parcial".into(),
                "4. TP2 em +10%: realiza mais 30%".into(),
                "5. TP3 em +20%: fecha 40% restantes — lucro máximo".into(),
                "6. Trailing stop 2% sobe junto com o preço".into(),
                "7. Stop loss 5% — margem ampla para swing".into(),
                "⚠️ Recomendado para traders experientes".into(),
            ],
            is_default: true,
            created_at: now,
            updated_at: now,
        },
    ]
}

/// GET /api/v1/strategy-templates - Lista todos os templates (padrão + do usuário)
#[get("")]
pub async fn get_templates(user: web::ReqData<Claims>, db: web::Data<MongoDB>) -> impl Responder {
    let user_id = &user.sub;

    let collection = db.collection::<StrategyTemplate>("strategy_templates");

    // Busca templates do usuário no MongoDB
    let mut user_templates: Vec<StrategyTemplateResponse> = Vec::new();

    match collection.find(doc! { "user_id": &user_id }).await {
        Ok(mut cursor) => {
            use futures::stream::StreamExt;
            while let Some(result) = cursor.next().await {
                match result {
                    Ok(tpl) => user_templates.push(StrategyTemplateResponse::from(tpl)),
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

    // Templates padrão (sempre presentes, com ids fictícios)
    let defaults: Vec<StrategyTemplateResponse> = default_templates()
        .into_iter()
        .enumerate()
        .map(|(i, mut t)| {
            // IDs fixos para os defaults: "default_0", "default_1", "default_2"
            let mut resp = StrategyTemplateResponse::from(t);
            resp.id = format!("default_{}", i);
            resp
        })
        .collect();

    // Ordena templates do usuário por data de criação (mais recentes primeiro)
    user_templates.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    // Concatena: padrão primeiro, depois os do usuário
    let mut all = defaults;
    all.extend(user_templates);

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "templates": all,
        "total": all.len()
    }))
}

/// GET /api/v1/strategy-templates/{id} - Busca template específico
#[get("/{id}")]
pub async fn get_template(user: web::ReqData<Claims>, path: web::Path<String>, db: web::Data<MongoDB>) -> impl Responder {
    let template_id = path.into_inner();

    // Se começa com "default_" é um template padrão
    if template_id.starts_with("default_") {
        if let Ok(idx) = template_id.replace("default_", "").parse::<usize>() {
            let defaults = default_templates();
            if let Some(t) = defaults.into_iter().nth(idx) {
                let mut resp = StrategyTemplateResponse::from(t);
                resp.id = template_id;
                return HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "template": resp
                }));
            }
        }
        return HttpResponse::NotFound().json(serde_json::json!({
            "success": false,
            "error": "Default template not found"
        }));
    }

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

    match collection.find_one(doc! { "_id": object_id, "user_id": &user_id }).await {
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

    // Não permite editar templates padrão
    if template_id.starts_with("default_") {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "success": false,
            "error": "Cannot edit default templates"
        }));
    }

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

    // Verifica se existe e pertence ao usuário
    match collection.find_one(doc! { "_id": object_id, "user_id": &user_id }).await {
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
        doc! { "_id": object_id, "user_id": &user_id },
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

    // Não permite deletar templates padrão
    if template_id.starts_with("default_") {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "success": false,
            "error": "Cannot delete default templates"
        }));
    }

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

    match collection.delete_one(doc! { "_id": object_id, "user_id": &user_id, "is_default": false }).await {
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
