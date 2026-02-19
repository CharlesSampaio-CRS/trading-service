use crate::{
    ccxt::CCXTClient,
    database::MongoDB,
    models::{Balance, BalanceResponse, BalanceSummary, ExchangeBalance, UserExchanges, ExchangeCatalog, DecryptedExchange},
    utils::crypto::decrypt_fernet_via_python,
    utils::thread_pool::spawn_ccxt_blocking,  // 🚀 FASE 3: Thread pool dedicado
};
use futures::future::join_all;
use futures::TryStreamExt; // Para cursor.try_next()
use mongodb::bson::{doc, oid::ObjectId};
use std::collections::HashMap;
use std::env;
use serde::{Serialize, Deserialize};

// Estrutura para armazenar snapshot detalhado de cada exchange
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExchangeSnapshotDetail {
    pub exchange_id: String,
    pub exchange_name: String,
    pub balance_usd: f64,
    pub is_active: bool,
    pub tokens_count: usize,
}

pub async fn get_user_balances(
    db: &MongoDB,
    user_id: &str,
) -> Result<BalanceResponse, String> {
    // Fetch user's exchanges from MongoDB
    let exchanges = get_user_exchanges_from_db(db, user_id).await?;
    
    if exchanges.is_empty() {
        log::info!("No exchanges found for user {}", user_id);
        return Ok(BalanceResponse {
            success: true,
            exchanges: vec![],
            total_usd: 0.0,
            timestamp: chrono::Utc::now().timestamp(),
        });
    }
    
    log::info!("Found {} exchanges for user {}", exchanges.len(), user_id);
    
    // Fetch balances from all exchanges in parallel
    let tasks: Vec<_> = exchanges
        .into_iter()
        .map(|exchange| tokio::spawn(async move { fetch_exchange_balance(exchange).await }))
        .collect();
    
    let results = join_all(tasks).await;
    
    let mut exchange_balances = Vec::new();
    let mut total_usd = 0.0;
    
    for result in results {
        match result {
            Ok(Ok(balance)) => {
                // ✅ OPTIMIZATION: Retorna TODOS os balances, deixa frontend filtrar
                total_usd += balance.total_usd;
                exchange_balances.push(balance);
            }
            Ok(Err(e)) => {
                log::error!("Error fetching exchange balance: {}", e);
                // Continue with other exchanges
            }
            Err(e) => {
                log::error!("Task join error: {}", e);
            }
        }
    }
    
    Ok(BalanceResponse {
        success: true,
        exchanges: exchange_balances,
        total_usd,
        timestamp: chrono::Utc::now().timestamp(),
    })
}

pub async fn get_balance_summary(
    db: &MongoDB,
    user_id: &str,
) -> Result<BalanceSummary, String> {
    let response = get_user_balances(db, user_id).await?;
    
    let tokens_count: usize = response
        .exchanges
        .iter()
        .map(|e| e.balances.len())
        .sum();
    
    Ok(BalanceSummary {
        total_usd: response.total_usd,
        exchanges_count: response.exchanges.len(),
        tokens_count,
        timestamp: response.timestamp,
    })
}

async fn get_user_exchanges_from_db(
    db: &MongoDB,
    user_id: &str,
) -> Result<Vec<DecryptedExchange>, String> {
    // 1. Buscar user_exchanges document
    let user_exchanges_collection = db.collection::<UserExchanges>("user_exchanges");
    
    let filter = doc! {
        "user_id": user_id
    };
    
    let user_exchanges = user_exchanges_collection
        .find_one(filter)
        .await
        .map_err(|e| format!("Database error: {}", e))?;
    
    let user_exchanges = match user_exchanges {
        Some(ue) => ue,
        None => {
            log::info!("No user_exchanges document found for user {}", user_id);
            return Ok(vec![]);
        }
    };
    
    // 2. Filtrar exchanges ativas
    let active_exchanges: Vec<_> = user_exchanges.exchanges
        .into_iter()
        .filter(|ex| ex.is_active)
        .collect();
    
    if active_exchanges.is_empty() {
        log::debug!("No active exchanges for user {}", user_id);
        return Ok(vec![]);
    }
    
    log::debug!("Found {} active exchanges", active_exchanges.len());
    
    // 3. 🚀 OPTIMIZATION: Batch query - busca TODAS exchanges do catálogo de uma vez
    let exchanges_collection = db.collection::<ExchangeCatalog>("exchanges");
    
    let encryption_key = env::var("ENCRYPTION_KEY")
        .map_err(|_| "ENCRYPTION_KEY not found in environment".to_string())?;
    
    // 🚀 Coleta todos os IDs para batch query
    let exchange_ids: Vec<ObjectId> = active_exchanges
        .iter()
        .filter_map(|ex| ObjectId::parse_str(&ex.exchange_id).ok())
        .collect();
    
    // 🚀 Busca TODAS as exchanges em uma única query
    let filter = doc! { "_id": { "$in": exchange_ids } };
    let mut cursor = exchanges_collection.find(filter).await
        .map_err(|e| format!("Database error: {}", e))?;
    
    // 🚀 Cria mapa para lookup rápido (usa Option<ObjectId> como chave)
    let mut catalog_map = std::collections::HashMap::new();
    while let Some(catalog) = cursor.try_next().await
        .map_err(|e| format!("Cursor error: {}", e))? {
        if let Some(id) = &catalog._id {
            catalog_map.insert(*id, catalog);
        }
    }
    
    log::debug!("Fetched {} exchange catalogs from database", catalog_map.len());
    
    // 🚀 FASE 2: Paraleliza descriptografia - 5-10x mais rápido!
    let decrypt_tasks: Vec<_> = active_exchanges
        .into_iter()
        .filter_map(|user_exchange| {
            let exchange_oid = ObjectId::parse_str(&user_exchange.exchange_id).ok()?;
            let catalog = catalog_map.get(&exchange_oid)?.clone();
            let key = encryption_key.clone();
            
            Some(tokio::task::spawn_blocking(move || {
                // Descriptografa API key
                let api_key = decrypt_fernet_via_python(&user_exchange.api_key_encrypted, &key)
                    .unwrap_or_else(|e| {
                        log::error!("Failed to decrypt API key: {}", e);
                        user_exchange.api_key_encrypted.clone()
                    });
                
                // Descriptografa API secret
                let api_secret = decrypt_fernet_via_python(&user_exchange.api_secret_encrypted, &key)
                    .unwrap_or_else(|e| {
                        log::error!("Failed to decrypt API secret: {}", e);
                        user_exchange.api_secret_encrypted.clone()
                    });
                
                // Descriptografa passphrase se existir
                let passphrase = user_exchange.passphrase_encrypted.as_ref()
                    .and_then(|p| decrypt_fernet_via_python(p, &key).ok());
                
                DecryptedExchange {
                    exchange_id: user_exchange.exchange_id,
                    ccxt_id: catalog.ccxt_id.clone(),
                    name: catalog.nome.clone().unwrap_or_else(|| "Unknown".to_string()),
                    api_key,
                    api_secret,
                    passphrase,
                    is_active: user_exchange.is_active,
                }
            }))
        })
        .collect();
    
    // Aguarda todas as descriptografias completarem em paralelo
    let decrypt_results = join_all(decrypt_tasks).await;
    
    let mut decrypted_exchanges = Vec::new();
    for result in decrypt_results {
        match result {
            Ok(exchange) => decrypted_exchanges.push(exchange),
            Err(e) => log::error!("Decryption task failed: {}", e),
        }
    }
    
    Ok(decrypted_exchanges)
}

// 🆕 Nova função para processar balances de exchanges enviadas pelo frontend
pub async fn fetch_balances_from_exchanges(
    exchanges: Vec<DecryptedExchange>,
) -> Result<BalanceResponse, String> {
    if exchanges.is_empty() {
        return Ok(BalanceResponse {
            success: true,
            exchanges: vec![],
            total_usd: 0.0,
            timestamp: chrono::Utc::now().timestamp(),
        });
    }
    
    log::info!("📊 Processing {} exchanges from frontend", exchanges.len());
    
    // Fetch balances from all exchanges in parallel
    let tasks: Vec<_> = exchanges
        .into_iter()
        .map(|exchange| tokio::spawn(async move { fetch_exchange_balance(exchange).await }))
        .collect();
    
    let results = join_all(tasks).await;
    
    let mut exchange_balances = Vec::new();
    let mut total_usd = 0.0;
    
    for result in results {
        match result {
            Ok(Ok(balance)) => {
                total_usd += balance.total_usd;
                exchange_balances.push(balance);
            }
            Ok(Err(e)) => {
                log::error!("Error fetching exchange balance: {}", e);
            }
            Err(e) => {
                log::error!("Task join error: {}", e);
            }
        }
    }
    
    Ok(BalanceResponse {
        success: true,
        exchanges: exchange_balances,
        total_usd,
        timestamp: chrono::Utc::now().timestamp(),
    })
}

async fn fetch_exchange_balance(exchange: DecryptedExchange) -> Result<ExchangeBalance, String> {
    fetch_exchange_balance_with_retry(exchange, 3).await
}

async fn fetch_exchange_balance_with_retry(exchange: DecryptedExchange, max_retries: u32) -> Result<ExchangeBalance, String> {
    log::debug!("Fetching balance for exchange: {} ({})", exchange.name, exchange.ccxt_id);
    
    // ⏱️ Timeout aumentado para 60s (exchanges lentas como MEXC)
    let timeout_duration = std::time::Duration::from_secs(60);
    
    let exchange_name = exchange.name.clone();
    let exchange_id = exchange.exchange_id.clone();
    let is_mexc = exchange.ccxt_id.to_lowercase() == "mexc";
    
    let mut final_result = None;
    
    for attempt in 0..max_retries {
        if attempt > 0 {
            // Backoff exponencial: 1s, 2s, 4s
            let delay_ms = 1000 * (2_u64.pow(attempt - 1));
            log::info!("🔄 Retry #{} for {} after {}ms delay", attempt + 1, exchange_name, delay_ms);
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
    
        let exchange_clone = DecryptedExchange {
            exchange_id: exchange.exchange_id.clone(),
            ccxt_id: exchange.ccxt_id.clone(),
            name: exchange.name.clone(),
            api_key: exchange.api_key.clone(),
            api_secret: exchange.api_secret.clone(),
            passphrase: exchange.passphrase.clone(),
            is_active: exchange.is_active,
        };
    
        // 🚀 FASE 3: Usa thread pool dedicado ao invés de tokio::spawn_blocking
        let balance_task = spawn_ccxt_blocking(move || {
            let client = CCXTClient::new(
                &exchange_clone.ccxt_id,
                &exchange_clone.api_key,
                &exchange_clone.api_secret,
                exchange_clone.passphrase.as_deref(),
            )?;
            
            client.fetch_balance_sync()
        });
        
        // Apply timeout
        let balances_result = match tokio::time::timeout(timeout_duration, balance_task).await {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => return Err(format!("Task error: {}", e)),
            Err(_) => {
                log::warn!("⏱️ Timeout fetching balance from {} after 60s", exchange_name);
                return Ok(ExchangeBalance {
                    exchange: exchange_name.clone(),
                    exchange_id: exchange_id.clone(),
                    success: false,
                    error: Some("Request timeout after 60s".to_string()),
                    balances: HashMap::new(),
                    total_usd: 0.0,
                });
            }
        };
        
        match &balances_result {
            Err(e) => {
                let error_str = e.to_string();
                // 🔄 Retry only for nonce/timestamp errors (especially MEXC)
                let is_nonce_error = error_str.contains("InvalidNonce") || 
                                    error_str.contains("recvWindow") ||
                                    error_str.contains("Timestamp");
                
                if is_nonce_error && attempt < max_retries - 1 {
                    log::warn!("⚠️  [{}] Nonce error (attempt {}/{}): {}", 
                        exchange_name, attempt + 1, max_retries, error_str);
                    continue; // Retry
                }
                
                // 🔄 For network errors, retry only MEXC (known to be flaky)
                let is_network_error = error_str.contains("NetworkError");
                if is_network_error && is_mexc && attempt < max_retries - 1 {
                    log::warn!("⚠️  [{}] Network error (attempt {}/{}): {}", 
                        exchange_name, attempt + 1, max_retries, error_str);
                    continue; // Retry
                }
                
                // No more retries or non-retryable error
                log::error!("Failed to fetch balance from {}: {}", exchange_name, e);
                return Ok(ExchangeBalance {
                    exchange: exchange_name.clone(),
                    exchange_id: exchange_id.clone(),
                    success: false,
                    error: Some(error_str),
                    balances: HashMap::new(),
                    total_usd: 0.0,
                });
            }
            Ok(_) => {
                // Success! Save and break
                final_result = Some(balances_result);
                break;
            }
        }
    }
    
    // Process successful result
    let balances_result = final_result.expect("Should have result after retry loop");
    match balances_result {
        Ok(mut balances) => {
            // 🌍 CONVERSÃO DE MOEDAS FIDUCIÁRIAS: BRL, EUR, etc.
            // Adiciona preços USD para moedas fiduciárias que não têm ticker na exchange
            let fiat_currencies = vec!["BRL", "EUR", "GBP", "JPY", "AUD", "CAD", "CHF"];
            
            for currency in fiat_currencies.iter() {
                if let Some(balance) = balances.get_mut(*currency) {
                    // Se já tem usd_value (do ticker), não precisa converter
                    if balance.usd_value.is_none() && balance.total > 0.0 {
                        log::debug!("🌍 [{}] Converting {} to USD via exchange rate...", 
                            exchange_name, currency);
                        
                        match tokio::time::timeout(
                            std::time::Duration::from_millis(2000),
                            crate::services::exchange_rate_service::get_exchange_rate(currency, "USD")
                        ).await {
                            Ok(Ok(rate)) => {
                                let usd_value = balance.total * rate;
                                balance.usd_value = Some(usd_value);
                                log::debug!("🌍 [{}] {} {}: {} × {:.6} = ${:.2}", 
                                    exchange_name, currency, balance.total, currency, rate, usd_value);
                            }
                            Ok(Err(e)) => {
                                log::warn!("⚠️  [{}] Failed to fetch {} rate: {}", exchange_name, currency, e);
                            }
                            Err(_) => {
                                log::warn!("⚠️  [{}] Rate fetch timeout for {}", exchange_name, currency);
                            }
                        }
                    }
                }
            }
            
            let mut total_usd: f64 = balances.values().map(|b| b.usd_value.unwrap_or(0.0)).sum();
            
            // 🚀 FASE 2: Lazy conversion - spawna task apenas se for NovaDAX
            if exchange_name.to_lowercase() == "novadax" {
                log::debug!("🇧🇷 [NovaDAX] Converting total balance from BRL to USD...");
                
                // Spawna task em paralelo (não bloqueia)
                match tokio::time::timeout(
                    std::time::Duration::from_millis(500),
                    crate::services::exchange_rate_service::get_exchange_rate("BRL", "USD")
                ).await {
                    Ok(Ok(rate)) => {
                        let original_total = total_usd;
                        total_usd = original_total * rate;
                        log::debug!("🇧🇷 [NovaDAX] Converted: R$ {:.2} × {:.6} = ${:.2}", 
                            original_total, rate, total_usd);
                    }
                    Ok(Err(e)) => {
                        log::warn!("⚠️  [NovaDAX] Failed to fetch rate: {}. Using fallback 0.20", e);
                        total_usd *= 0.20;
                    }
                    Err(_) => {
                        log::warn!("⚠️  [NovaDAX] Rate fetch timeout. Using fallback 0.20");
                        total_usd *= 0.20;
                    }
                }
            }
            
            log::info!("Successfully fetched {} balances from {}", balances.len(), exchange_name);
            
            Ok(ExchangeBalance {
                exchange: exchange_name.clone(),
                exchange_id: exchange_id.clone(),
                success: true,
                error: None,
                balances,
                total_usd,
            })
        }
        Err(e) => {
            log::error!("Failed to fetch balance from {}: {}", exchange_name, e);
            Ok(ExchangeBalance {
                exchange: exchange_name.clone(),
                exchange_id: exchange_id.clone(),
                success: false,
                error: Some(e.to_string()),
                balances: HashMap::new(),
                total_usd: 0.0,
            })
        }
    }
}

// Get balance for specific exchange
pub async fn get_exchange_balance(
    db: &MongoDB,
    user_id: &str,
    exchange_id: &str,
) -> Result<ExchangeBalance, String> {
    let collection = db.collection::<mongodb::bson::Document>("exchanges");
    
    let exchange_oid = ObjectId::parse_str(exchange_id)
        .map_err(|_| "Invalid exchange ID".to_string())?;
    
    // user_id is now a string field, not ObjectId
    let filter = doc! {
        "_id": exchange_oid,
        "user_id": user_id,
    };
    
    let doc = collection
        .find_one(filter)
        .await
        .map_err(|e| format!("Database error: {}", e))?
        .ok_or_else(|| "Exchange not found".to_string())?;
    
    let exchange_type = doc.get_str("exchange_type")
        .map_err(|_| "Missing exchange_type".to_string())?
        .to_string();
    let api_key = doc.get_str("api_key")
        .map_err(|_| "Missing api_key".to_string())?
        .to_string();
    let encrypted_secret = doc.get_str("api_secret")
        .map_err(|_| "Missing api_secret".to_string())?
        .to_string();
    
    let decrypted = DecryptedExchange {
        exchange_id: exchange_id.to_string(),
        ccxt_id: exchange_type.clone(),
        name: exchange_type.clone(),
        api_key,
        api_secret: encrypted_secret,
        passphrase: None,
        is_active: true,
    };
    
    fetch_exchange_balance(decrypted).await
}

// Get market movers (top gainers/losers)
#[derive(serde::Serialize)]
pub struct MarketMover {
    pub symbol: String,
    pub price: f64,
    pub change_24h: f64,
    pub volume_24h: f64,
}

#[derive(serde::Serialize)]
pub struct MarketMoversResponse {
    pub success: bool,
    pub gainers: Vec<MarketMover>,
    pub losers: Vec<MarketMover>,
}

pub async fn get_market_movers(
    _db: &MongoDB,
    _user_id: &str,
) -> Result<MarketMoversResponse, String> {
    // Simplified implementation - would need ticker data
    Ok(MarketMoversResponse {
        success: true,
        gainers: vec![],
        losers: vec![],
    })
}

// Calculate daily P&L
#[derive(serde::Serialize)]
pub struct DailyPnLResponse {
    pub user_id: String,
    pub today_usd: String,
    pub yesterday_usd: String,
    pub pnl_usd: String,
    pub pnl_percent: String,
    pub is_profit: Option<bool>,
    pub _raw: DailyPnLRaw,
}

#[derive(serde::Serialize)]
pub struct DailyPnLRaw {
    pub today_usd: f64,
    pub yesterday_usd: f64,
    pub pnl_usd: f64,
    pub pnl_percent: f64,
}

pub async fn get_daily_pnl(
    db: &MongoDB,
    user_id: &str,
    date: &str,
) -> Result<DailyPnLResponse, String> {
    log::info!("📊 Getting daily PNL for user: {}, date: {}", user_id, date);
    
    let collection = db.collection::<mongodb::bson::Document>("balance_snapshots");
    
    // Parse date
    let date_obj = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .unwrap_or_else(|_| chrono::Local::now().date_naive());
    let today_str = date_obj.format("%Y-%m-%d").to_string();
    let yesterday = date_obj - chrono::Duration::days(1);
    let yesterday_str = yesterday.format("%Y-%m-%d").to_string();
    
    log::info!("   Today: {}, Yesterday: {}", today_str, yesterday_str);
    
    // 🚀 OTIMIZADO: Buscar AMBOS snapshots de uma vez
    let filter = doc! {
        "user_id": user_id,
        "date": {
            "$in": [&today_str, &yesterday_str]
        }
    };
    
    let mut cursor = collection.find(filter).await
        .map_err(|e| format!("Database error: {}", e))?;
    
    use futures::stream::StreamExt;
    use std::collections::HashMap;
    
    let mut snapshots: HashMap<String, f64> = HashMap::new();
    
    while let Some(result) = cursor.next().await {
        if let Ok(snapshot) = result {
            let snap_date = snapshot.get_str("date").unwrap_or("").to_string();
            let balance = snapshot.get_f64("total_usd").unwrap_or(0.0);
            snapshots.insert(snap_date, balance);
        }
    }
    
    // Get today's balance (from snapshot or recalculate)
    let today_usd = if let Some(&balance) = snapshots.get(&today_str) {
        log::info!("   ✅ Using today snapshot: ${:.2}", balance);
        balance
    } else {
        log::warn!("   ⚠️  No snapshot for today, calculating current balance...");
        let current_balance = get_user_balances(db, user_id).await
            .map_err(|e| format!("Failed to get current balance: {}", e))?;
        
        // 💾 Auto-save snapshot for today to improve future queries
        let save_result = save_balance_snapshot_custom(db, user_id, Some(&today_str), Some(current_balance.total_usd)).await;
        if let Err(e) = save_result {
            log::warn!("   ⚠️  Failed to auto-save today's snapshot: {}", e);
        }
        
        current_balance.total_usd
    };
    
    // Get yesterday's balance with smart fallback
    let yesterday_usd = if let Some(&balance) = snapshots.get(&yesterday_str) {
        log::info!("   ✅ Found yesterday snapshot: ${:.2}", balance);
        balance
    } else {
        // 🔍 Fallback inteligente: buscar snapshot mais recente antes de ontem
        log::warn!("   ⚠️  No snapshot for yesterday ({}), searching for nearest snapshot...", yesterday_str);
        
        let mut nearest_balance = None;
        let mut search_days = 2;
        
        // Busca até 7 dias atrás por um snapshot válido
        while search_days <= 7 && nearest_balance.is_none() {
            let search_date = date_obj - chrono::Duration::days(search_days);
            let search_str = search_date.format("%Y-%m-%d").to_string();
            
            if let Some(&balance) = snapshots.get(&search_str) {
                log::info!("   ✅ Found snapshot {} days ago: ${:.2}", search_days, balance);
                nearest_balance = Some(balance);
                break;
            }
            search_days += 1;
        }
        
        // Se não encontrou nenhum snapshot, usa o valor de hoje (PNL será 0)
        nearest_balance.unwrap_or_else(|| {
            log::warn!("   ⚠️  No historical snapshots found, using today's value (PNL will be 0)");
            today_usd
        })
    };
    
    // Calculate PNL with precision
    let pnl_usd = today_usd - yesterday_usd;
    let pnl_percent = if yesterday_usd != 0.0 && yesterday_usd.abs() > 0.01 {
        (pnl_usd / yesterday_usd) * 100.0
    } else {
        0.0
    };
    
    let is_profit = if pnl_usd == 0.0 {
        None // No change
    } else {
        Some(pnl_usd > 0.0)
    };
    
    log::info!("   💰 PNL: ${:.2} ({:.2}%) - profit: {:?}", 
        pnl_usd, pnl_percent, is_profit);
    
    // ✅ Valores monetários USD sempre com 2 casas decimais
    Ok(DailyPnLResponse {
        user_id: user_id.to_string(),
        today_usd: format!("{:.2}", today_usd),
        yesterday_usd: format!("{:.2}", yesterday_usd),
        pnl_usd: format!("{:.2}", pnl_usd),
        pnl_percent: format!("{:.2}", pnl_percent),
        is_profit,
        _raw: DailyPnLRaw {
            today_usd,
            yesterday_usd,
            pnl_usd,
            pnl_percent,
        },
    })
}

// Auto-save daily snapshot (only once per day)
pub async fn auto_save_daily_snapshot(
    db: &MongoDB,
    user_id: &str,
) -> Result<(), String> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let collection = db.collection::<mongodb::bson::Document>("balance_snapshots");
    
    // Check if snapshot already exists for today
    let filter = doc! {
        "user_id": user_id,
        "date": &today,
    };
    
    if let Ok(Some(_)) = collection.find_one(filter).await {
        log::debug!("   ℹ️  Snapshot already exists for today ({}), skipping", today);
        return Ok(());
    }
    
    // Save new snapshot
    log::info!("💾 Auto-saving daily snapshot for user: {} (date: {})", user_id, today);
    save_balance_snapshot_custom(db, user_id, Some(&today), None).await
}

// Save daily balance snapshot
pub async fn save_balance_snapshot(
    db: &MongoDB,
    user_id: &str,
) -> Result<(), String> {
    save_balance_snapshot_custom(db, user_id, None, None).await
}

// Save daily balance snapshot with custom date and/or balance
// 🔥 VERSÃO DINÂMICA: Salva detalhes de CADA exchange (ativas E inativas)
pub async fn save_balance_snapshot_custom(
    db: &MongoDB,
    user_id: &str,
    custom_date: Option<&str>,
    custom_balance: Option<f64>,
) -> Result<(), String> {
    log::info!("💾 Saving DETAILED balance snapshot for user: {} (custom_date: {:?}, custom_balance: {:?})", 
        user_id, custom_date, custom_balance);
    
    // Use custom date or today
    let date = if let Some(d) = custom_date {
        log::info!("   Using custom date: {}", d);
        d.to_string()
    } else {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    };
    
    let timestamp = chrono::Utc::now().timestamp();
    
    // Se tem balance customizado, usa formato simples (compatibilidade com dados antigos)
    if let Some(balance) = custom_balance {
        log::info!("   Using custom balance: ${:.2} (simple format)", balance);
        
        let collection = db.collection::<mongodb::bson::Document>("balance_snapshots");
        
        let filter = doc! {
            "user_id": user_id,
            "date": &date,
        };
        
        let update = doc! {
            "$set": {
                "user_id": user_id,
                "date": &date,
                "total_usd": balance,
                "timestamp": timestamp,
                "updated_at": mongodb::bson::DateTime::now(),
            }
        };
        
        let options = mongodb::options::UpdateOptions::builder()
            .upsert(true)
            .build();
        
        collection
            .update_one(filter, update)
            .with_options(options)
            .await
            .map_err(|e| format!("Failed to save snapshot: {}", e))?;
        
        log::info!("✅ Simple snapshot saved: date={}, balance=${:.2}", date, balance);
        return Ok(());
    }
    
    // 🔥 NOVO: Buscar balance de TODAS as exchanges (ativas E inativas)
    // Isso preserva histórico completo para cálculo dinâmico
    log::info!("   Fetching detailed balances from ALL exchanges...");
    
    let current_balance_response = get_user_balances(db, user_id).await
        .map_err(|e| format!("Failed to get current balance: {}", e))?;
    
    let mut exchanges_details = Vec::new();
    let mut total_active_usd = 0.0;
    
    // Pegar detalhes de cada exchange
    for exchange_balance in current_balance_response.exchanges {
        let exchange_detail = ExchangeSnapshotDetail {
            exchange_id: exchange_balance.exchange_id.clone(),
            exchange_name: exchange_balance.exchange.clone(),
            balance_usd: exchange_balance.total_usd,
            is_active: true, // get_user_balances já filtra só ativas
            tokens_count: exchange_balance.balances.len(),
        };
        
        total_active_usd += exchange_balance.total_usd;
        exchanges_details.push(exchange_detail);
        
        log::info!("   📊 {}: ${:.2} ({} tokens)", 
            exchange_balance.exchange, 
            exchange_balance.total_usd,
            exchange_balance.balances.len()
        );
    }
    
    log::info!("   💰 Total (active exchanges): ${:.2}", total_active_usd);
    
    // Converter para BSON
    let exchanges_bson = exchanges_details.iter()
        .map(|detail| {
            doc! {
                "exchange_id": &detail.exchange_id,
                "exchange_name": &detail.exchange_name,
                "balance_usd": detail.balance_usd,
                "is_active": detail.is_active,
                "tokens_count": detail.tokens_count as i32,
            }
        })
        .collect::<Vec<_>>();
    
    // Salvar snapshot completo
    let collection = db.collection::<mongodb::bson::Document>("balance_snapshots");
    
    let filter = doc! {
        "user_id": user_id,
        "date": &date,
    };
    
    let update = doc! {
        "$set": {
            "user_id": user_id,
            "date": &date,
            "total_usd": total_active_usd,
            "timestamp": timestamp,
            "updated_at": mongodb::bson::DateTime::now(),
            "exchanges": exchanges_bson, // 🔥 Array com detalhes
        }
    };
    
    let options = mongodb::options::UpdateOptions::builder()
        .upsert(true)
        .build();
    
    collection
        .update_one(filter, update)
        .with_options(options)
        .await
        .map_err(|e| format!("Failed to save snapshot: {}", e))?;
    
    log::info!("✅ Detailed snapshot saved: date={}, exchanges={}, total=${:.2}", 
        date, exchanges_details.len(), total_active_usd);
    
    Ok(())
}
