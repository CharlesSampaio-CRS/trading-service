// ==================== DAILY SNAPSHOT SCHEDULER ====================
// Job automático que salva snapshots de balance diariamente às 00:00 UTC
// para todos os usuários com exchanges ativas

use crate::{
    database::MongoDB,
    services::balance_service,
    services::exchange_rate_service,
    utils::crypto,
};
use mongodb::bson::doc;
use tokio::time::{interval, Duration};
use chrono::{Utc, Timelike};
use std::env;

/// Inicia o scheduler de snapshots diários
/// Roda a cada hora e garante que existe snapshot do dia para todos os usuários.
/// Se o servidor reiniciar ou perder algum dia, o snapshot é criado no próximo tick.
/// Cada `save_user_snapshot` já verifica se o snapshot de hoje existe antes de salvar.
pub async fn start_daily_snapshot_scheduler(db: MongoDB) {
    log::info!("📅 Starting daily snapshot scheduler (runs every hour, saves once per day)");
    
    // Spawn task em background
    tokio::spawn(async move {
        // 🔥 EXECUTA IMEDIATAMENTE na inicialização para garantir snapshot de hoje
        log::info!("🚀 Running initial snapshot check on startup...");
        match save_all_user_snapshots(&db).await {
            Ok(count) => {
                log::info!("✅ Startup snapshot check completed: {} users processed", count);
            }
            Err(e) => {
                log::error!("❌ Startup snapshot check failed: {}", e);
            }
        }
        
        // Depois roda a cada hora
        let mut interval = interval(Duration::from_secs(3600)); // 1 hora
        
        loop {
            interval.tick().await;
            
            let now = Utc::now();
            let hour = now.hour();
            
            // Executa a cada hora — save_user_snapshot já faz skip se já existe snapshot de hoje
            // Preferimos rodar nas primeiras horas do dia UTC mas não falhamos se perder
            log::debug!("⏰ Hourly snapshot check ({}:00 UTC)...", hour);
            
            match save_all_user_snapshots(&db).await {
                Ok(count) => {
                    log::debug!("✅ Hourly snapshot check: {} users processed", count);
                }
                Err(e) => {
                    log::error!("❌ Hourly snapshot check failed: {}", e);
                }
            }
        }
    });
    
    log::info!("✅ Daily snapshot scheduler started successfully");
}

/// Salva snapshot para todos os usuários ativos
async fn save_all_user_snapshots(db: &MongoDB) -> Result<usize, String> {
    log::info!("💾 Saving snapshots for all active users...");
    
    // 1. Buscar todos os usuários que têm exchanges ativas
    let user_exchanges_collection = db.collection::<mongodb::bson::Document>("user_exchanges");
    
    let filter = doc! {
        "exchanges": {
            "$elemMatch": {
                "is_active": true
            }
        }
    };
    
    let mut cursor = user_exchanges_collection
        .find(filter)
        .await
        .map_err(|e| format!("Database error: {}", e))?;
    
    use futures::stream::StreamExt;
    let mut user_count = 0;
    let mut success_count = 0;
    let mut error_count = 0;
    
    while let Some(result) = cursor.next().await {
        match result {
            Ok(doc) => {
                if let Ok(user_id) = doc.get_str("user_id") {
                    user_count += 1;
                    log::info!("  📊 Processing user {}/{}: {}", user_count, "?", user_id);
                    
                    match save_user_snapshot(db, user_id).await {
                        Ok(_) => {
                            success_count += 1;
                            log::info!("    ✅ Snapshot saved for user: {}", user_id);
                        }
                        Err(e) => {
                            error_count += 1;
                            log::error!("    ❌ Failed to save snapshot for {}: {}", user_id, e);
                        }
                    }
                    
                    // Pequeno delay entre usuários para não sobrecarregar
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
            Err(e) => {
                log::error!("  ❌ Error reading document: {}", e);
                error_count += 1;
            }
        }
    }
    
    log::info!("📊 Snapshot job summary: {} users, {} success, {} errors", 
        user_count, success_count, error_count);
    
    Ok(user_count)
}

/// Salva snapshot diário para um usuário específico
async fn save_user_snapshot(db: &MongoDB, user_id: &str) -> Result<(), String> {
    let today = Utc::now().format("%Y-%m-%d").to_string();
    
    // 0. Obter chave de criptografia
    let encryption_key = env::var("ENCRYPTION_KEY")
        .map_err(|_| "ENCRYPTION_KEY not found in environment".to_string())?;
    
    let snapshots_collection = db.collection::<mongodb::bson::Document>("balance_snapshots");
    
    // 1. Buscar documento do usuário
    let filter = doc! { "user_id": user_id };
    let user_doc = snapshots_collection.find_one(filter.clone()).await
        .map_err(|e| format!("Failed to query snapshots: {}", e))?;
    
    // 2. Verificar se já existe snapshot para hoje
    if let Some(doc) = &user_doc {
        if let Ok(snapshots) = doc.get_array("snapshots") {
            for snapshot in snapshots {
                if let Some(snap_doc) = snapshot.as_document() {
                    if let Ok(date) = snap_doc.get_str("date") {
                        if date == today {
                            log::debug!("    ℹ️  Snapshot already exists for user {} today ({}), skipping", user_id, today);
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
    
    // 3. Buscar balance atual do usuário
    let balance_response = balance_service::get_user_balances(db, user_id).await
        .map_err(|e| format!("Failed to get balance: {}", e))?;
    
    let total_usd = balance_response.total_usd;
    
    // 4. Converter USD para BRL
    let total_brl = match exchange_rate_service::get_exchange_rate("USD", "BRL").await {
        Ok(rate) => {
            let brl = total_usd * rate;
            log::debug!("    💱 Converted: ${:.2} USD × {:.4} = R$ {:.2} BRL", total_usd, rate, brl);
            brl
        }
        Err(e) => {
            log::warn!("    ⚠️  Failed to get USD/BRL rate: {}, using fallback 5.0", e);
            total_usd * 5.0 // Fallback rate
        }
    };
    
    // 5. Criptografar valores sensíveis (USD e BRL)
    let encrypted_total_usd = crypto::encrypt_fernet_via_python(&total_usd.to_string(), &encryption_key)
        .map_err(|e| format!("Failed to encrypt total_usd: {}", e))?;
    
    let encrypted_total_brl = crypto::encrypt_fernet_via_python(&total_brl.to_string(), &encryption_key)
        .map_err(|e| format!("Failed to encrypt total_brl: {}", e))?;
    
    log::debug!("    🔒 Values encrypted successfully");
    
    // 6. Preparar detalhes de cada exchange
    let mut exchanges_details = Vec::new();
    
    for exchange_balance in balance_response.exchanges {
        // Criptografar balance_usd de cada exchange
        let encrypted_exchange_balance_usd = crypto::encrypt_fernet_via_python(&exchange_balance.total_usd.to_string(), &encryption_key)
            .map_err(|e| format!("Failed to encrypt exchange balance: {}", e))?;
        
        exchanges_details.push(doc! {
            "exchange_id": exchange_balance.exchange_id,
            "exchange_name": exchange_balance.exchange,
            "balance_usd": encrypted_exchange_balance_usd,
            "is_active": true,
            "tokens_count": exchange_balance.balances.len() as i32,
        });
    }
    
    // 7. Criar novo snapshot
    let new_snapshot = doc! {
        "date": &today,
        "total_usd": encrypted_total_usd,
        "total_brl": encrypted_total_brl,
        "timestamp": Utc::now().timestamp_millis(),
        "exchanges": exchanges_details,
    };
    
    // 8. Atualizar ou criar documento do usuário
    if user_doc.is_some() {
        // Usuário já tem documento: adiciona snapshot ao array
        let update = doc! {
            "$push": {
                "snapshots": new_snapshot
            },
            "$set": {
                "updated_at": mongodb::bson::DateTime::now()
            }
        };
        
        snapshots_collection
            .update_one(filter, update)
            .await
            .map_err(|e| format!("Failed to update snapshots: {}", e))?;
        
        log::debug!("    💾 Snapshot appended: ${:.2} USD (R$ {:.2} BRL)", total_usd, total_brl);
    } else {
        // Primeiro snapshot do usuário: cria novo documento
        let new_doc = doc! {
            "user_id": user_id,
            "snapshots": vec![new_snapshot],
            "created_at": mongodb::bson::DateTime::now(),
            "updated_at": mongodb::bson::DateTime::now(),
        };
        
        snapshots_collection
            .insert_one(new_doc)
            .await
            .map_err(|e| format!("Failed to insert snapshot document: {}", e))?;
        
        log::debug!("    💾 First snapshot created: ${:.2} USD (R$ {:.2} BRL)", total_usd, total_brl);
    }
    
    Ok(())
}

/// Salva snapshot manualmente para um usuário (chamado via API)
pub async fn save_snapshot_now(db: &MongoDB, user_id: &str) -> Result<(), String> {
    log::info!("💾 Manual snapshot request for user: {}", user_id);
    save_user_snapshot(db, user_id).await
}
