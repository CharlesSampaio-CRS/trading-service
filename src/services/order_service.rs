// ==================== ZERO DATABASE ARCHITECTURE ====================
// Orders operations via CCXT - NO MongoDB persistence needed
// Credentials come from frontend (decrypted from IndexedDB/WatermelonDB)

use crate::{
    ccxt::CCXTClient,
    models::{
        Order, OrdersResponse, CreateOrderResponse, CancelOrderResponse,
        DecryptedExchange, OrderFee,
        CreateOrderWithCredsRequest, CancelOrderWithCredsRequest,
    },
};
use futures::future::join_all;
use pyo3::{Python, types::PyDict};

/// Fetch orders from exchanges sent by frontend (with decrypted credentials)
pub async fn fetch_orders_from_exchanges(
    exchanges: Vec<DecryptedExchange>,
) -> Result<OrdersResponse, String> {
    log::info!("📊 Processing {} exchanges from frontend", exchanges.len());
    
    if exchanges.is_empty() {
        return Ok(OrdersResponse {
            success: true,
            orders: vec![],
            count: 0,
        });
    }
    
    // Fetch orders from all exchanges in parallel
    let tasks: Vec<_> = exchanges
        .into_iter()
        .map(|exchange| {
            tokio::spawn(async move {
                fetch_exchange_orders(exchange, "dummy_user", "open").await
            })
        })
        .collect();
    
    let results = join_all(tasks).await;
    
    let mut all_orders = Vec::new();
    let mut success_count = 0;
    let mut error_count = 0;
    
    for result in results {
        match result {
            Ok(Ok(mut orders)) => {
                success_count += 1;
                all_orders.append(&mut orders);
            }
            Ok(Err(e)) => {
                log::debug!("[Orders] Exchange error: {}", e);
                error_count += 1;
            }
            Err(e) => {
                log::error!("[Orders] Task join error: {}", e);
                error_count += 1;
            }
        }
    }
    
    // Sort by timestamp descending
    all_orders.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    
    let count = all_orders.len();
    
    log::info!("[Orders] Fetched {} orders ({} success, {} errors)", 
        count, success_count, error_count);
    
    Ok(OrdersResponse {
        success: true,
        orders: all_orders,
        count,
    })
}

/// Helper: Fetch orders from a single exchange
async fn fetch_exchange_orders(
    exchange: DecryptedExchange,
    user_id: &str,
    status_filter: &str,
) -> Result<Vec<Order>, String> {
    let exchange_name = exchange.name.clone();
    let ccxt_id = exchange.ccxt_id.clone();
    
    log::info!("🔄 [Orders] Fetching {} orders from {} ({})", 
        status_filter, &exchange_name, &ccxt_id);
    
    let user_id_clone = user_id.to_string();
    let exchange_id_clone = exchange.exchange_id.clone();
    let exchange_name_clone = exchange_name.clone();
    let ccxt_id_clone = ccxt_id.clone();
    let status = status_filter.to_string();
    
    // Timeout de 10 segundos para cada exchange
    let timeout_duration = std::time::Duration::from_secs(10);
    let exchange_name_for_timeout = exchange_name.clone();
    
    let task = tokio::task::spawn_blocking(move || {
        log::debug!("🔧 [Orders] Creating CCXT client for {}", ccxt_id_clone);
        let client = CCXTClient::new(
            &exchange.ccxt_id,
            &exchange.api_key,
            &exchange.api_secret,
            exchange.passphrase.as_deref(),
        )?;
        
        // Special handling for MEXC: requires symbol for fetch_open_orders
        if ccxt_id_clone.to_lowercase() == "mexc" && status == "open" {
            log::info!("🔍 [MEXC] Special handling: fetching orders per symbol");
            
            // Fetch balance to get active currencies
            match client.fetch_balance_raw() {
                Ok(balance_obj) => {
                    let mut all_orders = Vec::new();
                    let quote_currencies = vec!["USDT", "USDC", "BTC", "ETH"];
                    let max_symbols = 20; // Limit to avoid rate limits
                    let mut symbols_checked = 0;
                    
                    // Get markets to check if symbol exists
                    let markets = client.get_markets().ok();
                    
                    Python::with_gil(|py| {
                        // Extract balance dict
                        if let Ok(balance_dict) = balance_obj.downcast::<PyDict>(py) {
                            // Get 'total' field
                            if let Ok(Some(total)) = balance_dict.get_item("total") {
                                if let Ok(total_dict) = total.downcast::<PyDict>() {
                                    log::debug!("📊 [MEXC] Scanning balance for active currencies...");
                                    
                                    // Iterate through currencies with balance
                                    for (currency, amount) in total_dict.iter() {
                                        if symbols_checked >= max_symbols {
                                            break;
                                        }
                                        
                                        // Check if amount > 0
                                        if let Ok(amt) = amount.extract::<f64>() {
                                            if amt > 0.0 {
                                                if let Ok(curr) = currency.extract::<String>() {
                                                    // Try each quote currency
                                                    for quote in &quote_currencies {
                                                        let symbol = format!("{}/{}", curr, quote);
                                                        
                                                        // Check if market exists
                                                        let market_exists = if let Some(ref mkts) = markets {
                                                            if let Ok(mkts_dict) = mkts.downcast::<PyDict>(py) {
                                                                mkts_dict.contains(&symbol).unwrap_or(false)
                                                            } else {
                                                                false
                                                            }
                                                        } else {
                                                            false
                                                        };
                                                        
                                                        if market_exists {
                                                            log::debug!("  🔍 [MEXC] Trying symbol: {}", symbol);
                                                            symbols_checked += 1;
                                                            
                                                            // Fetch orders for this symbol
                                                            match client.fetch_open_orders_with_symbol(&symbol) {
                                                                Ok(orders) => {
                                                                    if !orders.is_empty() {
                                                                        log::info!("  ✅ [MEXC] {}: {} orders", symbol, orders.len());
                                                                        all_orders.extend(orders);
                                                                    }
                                                                }
                                                                Err(e) => {
                                                                    log::debug!("  ✗ [MEXC] {}: {}", symbol, e);
                                                                }
                                                            }
                                                            
                                                            // Found valid symbol for this currency, skip other quotes
                                                            break;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    });
                    
                    log::info!("✅ [MEXC] Collected {} total open orders from {} symbols", 
                        all_orders.len(), symbols_checked);
                    
                    // Convert to Vec<Order>
                    return all_orders.into_iter()
                        .map(|order| convert_ccxt_order_to_model(order, &user_id_clone, &exchange_id_clone, &exchange_name_clone))
                        .collect();
                }
                Err(e) => {
                    log::warn!("⚠️  [MEXC] Failed to fetch balance: {}", e);
                    log::warn!("⚠️  [MEXC] Falling back to standard fetch (will likely fail)");
                }
            }
        }
        
        // Standard fetch for other exchanges or MEXC fallback
        log::debug!("📞 [Orders] Calling fetch_orders_sync with status: {}", status);
        let orders = client.fetch_orders_sync(&status)?;
        
        log::info!("✅ [Orders] Received {} orders from {}", orders.len(), exchange_name_clone);
        
        // Convert to Vec<Order>
        orders.into_iter()
            .map(|order| convert_ccxt_order_to_model(order, &user_id_clone, &exchange_id_clone, &exchange_name_clone))
            .collect()
    });
    
    let result: Result<Vec<Order>, String> = match tokio::time::timeout(timeout_duration, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => {
            log::error!("❌ [Orders] Task error for {}: {}", exchange_name_for_timeout, e);
            return Err(format!("Task error: {}", e));
        }
        Err(_) => {
            log::warn!("⏱️ [Orders] Timeout fetching from {} after 10s", exchange_name_for_timeout);
            return Err(format!("Timeout fetching orders from {}", exchange_name_for_timeout));
        }
    };
    
    match &result {
        Ok(orders) => log::info!("🎉 [Orders] Successfully converted {} orders from {}", orders.len(), exchange_name),
        Err(e) => log::error!("❌ [Orders] Error fetching from {}: {}", exchange_name, e),
    }
    
    result
}

/// Helper: Convert CCXT PyObject order to Rust Order model
fn convert_ccxt_order_to_model(
    order: pyo3::PyObject,
    user_id: &str,
    exchange_id: &str,
    exchange_name: &str,
) -> Result<Order, String> {
    use pyo3::prelude::*;
    
    Python::with_gil(|py| {
        let order_ref = order.as_ref(py);
        
        // Helper para extrair String com fallback
        let extract_string = |key: &str| -> String {
            order_ref.get_item(key)
                .ok()
                .and_then(|v| {
                    if v.is_none() {
                        None
                    } else {
                        v.extract().ok()
                    }
                })
                .unwrap_or_else(|| String::new())
        };
        
        // Extrai ID da API - NÃO gera fallback
        let order_id = extract_string("id");
        
        if order_id.is_empty() {
            log::warn!("⚠️  Exchange returned order without ID");
        }
        
        Ok(Order {
            _id: None,
            id: order_id,
            user_id: user_id.to_string(),
            exchange: exchange_name.to_string(),
            exchange_id: exchange_id.to_string(),
            symbol: extract_string("symbol"),
            order_type: extract_string("type"),
            side: extract_string("side"),
            price: order_ref.get_item("price").ok().and_then(|v| {
                if v.is_none() { None } else { v.extract().ok() }
            }),
            amount: order_ref.get_item("amount").ok().and_then(|v| {
                if v.is_none() { None } else { v.extract().ok() }
            }).unwrap_or(0.0),
            filled: order_ref.get_item("filled").ok().and_then(|v| {
                if v.is_none() { None } else { v.extract().ok() }
            }).unwrap_or(0.0),
            remaining: order_ref.get_item("remaining").ok().and_then(|v| {
                if v.is_none() { None } else { v.extract().ok() }
            }).unwrap_or(0.0),
            cost: order_ref.get_item("cost").ok().and_then(|v| {
                if v.is_none() { None } else { v.extract().ok() }
            }).unwrap_or(0.0),
            status: extract_string("status"),
            fee: order_ref.get_item("fee").ok().and_then(|fee| {
                if fee.is_none() {
                    return None;
                }
                Some(OrderFee {
                    currency: fee.get_item("currency").ok()?.extract().ok()?,
                    cost: fee.get_item("cost").ok()?.extract().ok()?,
                })
            }),
            timestamp: order_ref.get_item("timestamp").ok().and_then(|v| {
                if v.is_none() { None } else { v.extract().ok() }
            }).unwrap_or(0),
            datetime: extract_string("datetime"),
            created_at: Some(chrono::Utc::now()),
            updated_at: Some(chrono::Utc::now()),
        })
    }).map_err(|e: pyo3::PyErr| format!("Failed to convert order: {}", e))
}

/// Create order com credenciais do frontend (sem MongoDB)
pub async fn create_order_with_creds(
    request: &CreateOrderWithCredsRequest,
) -> Result<CreateOrderResponse, String> {
    log::info!("Creating {} {} order for {} on {} (with frontend creds)", 
        request.side, request.order_type, request.symbol, request.exchange_name);
    
    let order_type_clone = request.order_type.clone();
    let side_clone = request.side.clone();
    let symbol_clone = request.symbol.clone();
    let amount_clone = request.amount;
    let price_clone = request.price;
    let exchange_name_clone = request.exchange_name.clone();
    let ccxt_id_clone = request.ccxt_id.clone();
    let api_key_clone = request.api_key.clone();
    let api_secret_clone = request.api_secret.clone();
    let passphrase_clone = request.passphrase.clone();
    
    let result = tokio::task::spawn_blocking(move || {
        let client = CCXTClient::new(
            &ccxt_id_clone,
            &api_key_clone,
            &api_secret_clone,
            passphrase_clone.as_deref(),
        )?;
        
        let order = client.create_order_sync(
            &symbol_clone,
            &order_type_clone,
            &side_clone,
            amount_clone,
            price_clone,
        )?;
        
        convert_ccxt_order_to_model(order, "no_user", "no_exchange_id", &exchange_name_clone)
    }).await.map_err(|e| format!("Task error: {}", e))??;
    
    if result.id.is_empty() {
        log::error!("❌ Order created but exchange returned empty ID");
        return Err("Exchange returned order with empty ID".to_string());
    }
    
    log::info!("✅ Order created with ID: {}", result.id);
    
    Ok(CreateOrderResponse {
        success: true,
        order: Some(result),
        error: None,
    })
}

/// Cancel order com credenciais do frontend (sem MongoDB)
pub async fn cancel_order_with_creds(
    request: &CancelOrderWithCredsRequest,
) -> Result<CancelOrderResponse, String> {
    log::info!("Canceling order {} on {} (with frontend creds)", request.order_id, request.exchange_name);
    
    let order_id_clone = request.order_id.clone();
    let symbol_clone = request.symbol.clone();
    let ccxt_id_clone = request.ccxt_id.clone();
    let api_key_clone = request.api_key.clone();
    let api_secret_clone = request.api_secret.clone();
    let passphrase_clone = request.passphrase.clone();
    
    tokio::task::spawn_blocking(move || {
        let client = CCXTClient::new(
            &ccxt_id_clone,
            &api_key_clone,
            &api_secret_clone,
            passphrase_clone.as_deref(),
        )?;
        
        client.cancel_order_sync(&order_id_clone, symbol_clone.as_deref())
    }).await.map_err(|e| format!("Task error: {}", e))??;
    
    log::info!("Order {} canceled successfully", request.order_id);
    
    Ok(CancelOrderResponse {
        success: true,
        message: format!("Order {} canceled successfully", request.order_id),
        order_id: Some(request.order_id.clone()),
        error: None,
    })
}
