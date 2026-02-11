use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use std::collections::HashMap;
use crate::models::Balance;

pub struct CCXTClient {
    exchange: Py<PyAny>,
    exchange_name: String,
}

impl CCXTClient {
    pub fn new(
        exchange_name: &str,
        api_key: &str,
        secret: &str,
        passphrase: Option<&str>,
    ) -> Result<Self, String> {
        Python::with_gil(|py| {
            // Import ccxt
            let ccxt = py
                .import("ccxt")
                .map_err(|e| format!("Failed to import ccxt: {}", e))?;
            
            // Get exchange class
            let exchange_class = ccxt
                .getattr(exchange_name)
                .map_err(|e| format!("Exchange {} not found: {}", exchange_name, e))?;
            
            // Create configuration dict with correct CCXT parameter names
            let config = PyDict::new(py);
            config.set_item("apiKey", api_key).map_err(|e| e.to_string())?;
            config.set_item("secret", secret).map_err(|e| e.to_string())?;
            config.set_item("enableRateLimit", true).map_err(|e| e.to_string())?;
            config.set_item("timeout", 60000).map_err(|e| e.to_string())?; // 60 segundos (aumentado de 8s)
            
            // ❌ DESABILITA CACHE DO CCXT - Força busca sempre fresca
            let options = PyDict::new(py);
            options.set_item("warnOnFetchOpenOrdersWithoutSymbol", false).map_err(|e| e.to_string())?;
            options.set_item("fetchBalanceCacheTTL", 0).map_err(|e| e.to_string())?;  // 🔥 NO CACHE
            options.set_item("fetchTickersCacheTTL", 0).map_err(|e| e.to_string())?;  // 🔥 NO CACHE
            
            if let Some(pass) = passphrase {
                config.set_item("password", pass).map_err(|e| e.to_string())?;
            }
            
            // Bybit specific configuration for Unified Trading Account
            if exchange_name.to_lowercase() == "bybit" {
                options.set_item("defaultType", "spot").map_err(|e| e.to_string())?;
                options.set_item("accountType", "UNIFIED").map_err(|e| e.to_string())?;
                log::info!("🔧 [Bybit] Configured with Unified Trading Account (spot market)");
            }
            
            config.set_item("options", options).map_err(|e| e.to_string())?;
            
            // Instantiate exchange - pass config as first positional argument
            let exchange = exchange_class
                .call1((config,))
                .map_err(|e| format!("Failed to create exchange: {}", e))?;
            
            Ok(Self {
                exchange: exchange.into(),
                exchange_name: exchange_name.to_string(),
            })
        })
    }
    
    /// Fetch all ticker prices from exchange in a single optimized call
    pub fn fetch_tickers_sync(&self) -> Result<HashMap<String, f64>, String> {
        Python::with_gil(|py| {
            log::debug!("🔍 Fetching tickers from {}...", self.exchange_name);
            
            // Call exchange.fetch_tickers()
            let tickers_obj = self.exchange
                .as_ref(py)
                .call_method0("fetch_tickers")
                .map_err(|e| format!("Failed to fetch tickers: {}", e))?;
            
            let mut prices = HashMap::new();
            
            // Parse response: {symbol: {last: price, ...}}
            if let Ok(tickers_dict) = tickers_obj.downcast::<PyDict>() {
                for (symbol_obj, ticker_obj) in tickers_dict.iter() {
                    if let Ok(symbol_str) = symbol_obj.extract::<String>() {
                        if let Ok(ticker_dict) = ticker_obj.downcast::<PyDict>() {
                            // Get last price from ticker
                            if let Some(last) = ticker_dict.get_item("last").ok().flatten() {
                                if let Ok(price) = last.extract::<f64>() {
                                    // Extract base currency: "BTC/USDT" -> "BTC"
                                    if let Some(base) = symbol_str.split('/').next() {
                                        prices.insert(base.to_string(), price);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            
            log::info!("✅ Fetched {} ticker prices from {}", prices.len(), self.exchange_name);
            Ok(prices)
        })
    }
    
    pub async fn fetch_balance(&self) -> Result<HashMap<String, Balance>, String> {
        // This method is kept for compatibility but wraps the sync version
        let exchange = self.exchange.clone();
        let exchange_name = self.exchange_name.clone();
        tokio::task::spawn_blocking(move || {
            Self::fetch_balance_internal(&exchange, &exchange_name)
        })
        .await
        .map_err(|e| format!("Task error: {}", e))?
    }
    
    pub fn fetch_balance_sync(&self) -> Result<HashMap<String, Balance>, String> {
        Self::fetch_balance_internal(&self.exchange, &self.exchange_name)
    }
    
    fn fetch_balance_internal(exchange: &Py<PyAny>, exchange_name: &str) -> Result<HashMap<String, Balance>, String> {
        Python::with_gil(|py| {
            log::info!("🔍 [{}] Fetching fresh balance from CCXT (NO CACHE)...", exchange_name);
            
            // 1. Fetch balance 
            // ⚠️ IMPORTANTE: Binance NÃO aceita parâmetros extras!
            // Outras exchanges aceitam timestamp para bypass de cache
            let balance_dict = if exchange_name.to_lowercase() == "binance" {
                // Binance: SEM parâmetros (muito restritiva)
                log::debug!("🔧 [Binance] Chamando fetch_balance SEM parâmetros (exchange restritiva)");
                exchange
                    .as_ref(py)
                    .call_method0("fetch_balance")
                    .map_err(|e| format!("Failed to fetch balance: {}", e))?
            } else {
                // Outras exchanges: COM timestamp para bypass de cache
                let params_dict = pyo3::types::PyDict::new(py);
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis();
                params_dict.set_item("_t", timestamp)
                    .map_err(|e| format!("Failed to set timestamp param: {}", e))?;
                
                log::debug!("🔧 [{}] Chamando fetch_balance COM timestamp: {}", exchange_name, timestamp);
                exchange
                    .as_ref(py)
                    .call_method1("fetch_balance", (params_dict,))
                    .map_err(|e| format!("Failed to fetch balance: {}", e))?
            };
            
            log::debug!("✅ [{}] Balance fetched from CCXT (no cache)", exchange_name);
            
            // 2. Fetch tickers (prices AND change_24h) - non-blocking if fails
            // 🔥 IMPORTANTE: Não passa parâmetros para fetch_tickers para evitar erro do CCXT
            // O fetch_balance já força cache bypass, tickers pode usar comportamento padrão
            let (tickers, changes) = match exchange.as_ref(py).call_method0("fetch_tickers") {
                Ok(tickers_obj) => {
                    let mut prices = HashMap::new();
                    let mut percent_changes = HashMap::new();
                    
                    // Verifica se tickers_obj não é None
                    if tickers_obj.is_none() {
                        log::warn!("⚠️  fetch_tickers returned None for {}", exchange_name);
                        (HashMap::new(), HashMap::new())
                    } else if let Ok(tickers_dict) = tickers_obj.downcast::<PyDict>() {
                        for (symbol_obj, ticker_obj) in tickers_dict.iter() {
                            // Verifica se symbol não é None antes de extrair
                            if symbol_obj.is_none() {
                                continue;
                            }
                            
                            if let Ok(symbol_str) = symbol_obj.extract::<String>() {
                                // Verifica se symbol_str não está vazio
                                if symbol_str.is_empty() {
                                    continue;
                                }
                                
                                // Verifica se ticker não é None
                                if ticker_obj.is_none() {
                                    continue;
                                }
                                
                                if let Ok(ticker_dict) = ticker_obj.downcast::<PyDict>() {
                                    // Extract price (last)
                                    if let Some(last) = ticker_dict.get_item("last").ok().flatten() {
                                        if let Ok(price) = last.extract::<f64>() {
                                            if price > 0.0 {  // Ignora preços zero ou negativos
                                                if let Some(base) = symbol_str.split('/').next() {
                                                    // 🔍 Busca preço em USDT para tokens que não sejam stablecoins
                                                    // Prioriza pares com USDT, depois USDC, depois USD
                                                    if symbol_str.ends_with("/USDT") || 
                                                       symbol_str.ends_with("/USDC") || 
                                                       symbol_str.ends_with("/USD") {
                                                        // Sobrescreve apenas se ainda não tiver preço ou se for mais específico
                                                        if !prices.contains_key(base) || symbol_str.ends_with("/USDT") {
                                                            prices.insert(base.to_string(), price);
                                                            log::debug!("💱 {}: ${:.6} (from {})", base, price, symbol_str);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    // Extract percentage change (change_24h)
                                    if let Some(percentage) = ticker_dict.get_item("percentage").ok().flatten() {
                                        if let Ok(change) = percentage.extract::<f64>() {
                                            if let Some(base) = symbol_str.split('/').next() {
                                                if symbol_str.ends_with("/USDT") || 
                                                   symbol_str.ends_with("/USDC") || 
                                                   symbol_str.ends_with("/USD") {
                                                    if !percent_changes.contains_key(base) || symbol_str.ends_with("/USDT") {
                                                        percent_changes.insert(base.to_string(), change);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        log::info!("✅ Fetched {} ticker prices (USDT pairs) and {} changes from {}", 
                            prices.len(), percent_changes.len(), exchange_name);
                        (prices, percent_changes)
                    } else {
                        log::warn!("⚠️  Could not downcast tickers to PyDict for {}", exchange_name);
                        (HashMap::new(), HashMap::new())
                    }
                }
                Err(e) => {
                    log::warn!("⚠️  Could not fetch tickers from {}: {}", exchange_name, e);
                    (HashMap::new(), HashMap::new())
                }
            };
            
            let total = balance_dict
                .get_item("total")
                .map_err(|e| format!("Failed to get total: {}", e))?;
            
            let free = balance_dict
                .get_item("free")
                .map_err(|e| format!("Failed to get free: {}", e))?;
            
            let used = balance_dict
                .get_item("used")
                .map_err(|e| format!("Failed to get used: {}", e))?;
            
            let mut balances = HashMap::new();
            
            // Convert Python dict to Rust HashMap
            if let Ok(total_dict) = total.downcast::<PyDict>() {
                for (key, value) in total_dict.iter() {
                    let symbol: String = key.extract().unwrap_or_default();
                    let total_amount: f64 = value.extract().unwrap_or(0.0);
                    
                    if total_amount > 0.0 {
                        let free_amount: f64 = free
                            .downcast::<PyDict>()
                            .ok()
                            .and_then(|d| d.get_item(&symbol).ok())
                            .and_then(|opt| opt.and_then(|v| v.extract().ok()))
                            .unwrap_or(0.0);
                        
                        let used_amount: f64 = used
                            .downcast::<PyDict>()
                            .ok()
                            .and_then(|d| d.get_item(&symbol).ok())
                            .and_then(|opt| opt.and_then(|v| v.extract().ok()))
                            .unwrap_or(0.0);
                        
                        // 3. Calculate USD value
                        let price_usd = if symbol == "USDT" 
                            || symbol == "USDC" 
                            || symbol == "DAI" 
                            || symbol == "BUSD"
                            || symbol == "FDUSD"
                        {
                            // Stablecoins = $1.00
                            Some(1.0)
                        } else if let Some(&price) = tickers.get(&symbol) {
                            // Use ticker price
                            Some(price)
                        } else {
                            // No price available - log warning
                            log::warn!("⚠️  [{}] No USDT price found for {}: {} units (check if {}/USDT pair exists)", 
                                exchange_name, symbol, total_amount, symbol);
                            None
                        };
                        
                        let usd_value = price_usd.map(|p| total_amount * p);
                        
                        // Get change_24h from tickers if available
                        let change_24h = changes.get(&symbol).copied();
                        
                        if usd_value.is_some() && price_usd.is_some() {
                            log::debug!(
                                "💰 [{}] {}: {} × ${:.6} = ${:.2} (change: {:+.2}%)",
                                exchange_name,
                                symbol,
                                total_amount,
                                price_usd.unwrap(),
                                usd_value.unwrap(),
                                change_24h.unwrap_or(0.0)
                            );
                        } else {
                            log::debug!(
                                "💰 [{}] {}: {} units (NO USD VALUE - price not available)",
                                exchange_name,
                                symbol,
                                total_amount
                            );
                        }
                        
                        balances.insert(
                            symbol.clone(),
                            Balance {
                                symbol,
                                free: free_amount,
                                used: used_amount,
                                total: total_amount,
                                usd_value,
                                change_24h,  // ✅ NOW HAS CHANGE VALUE!
                            },
                        );
                    }
                }
            }
            
            Ok(balances)
        })
    }
    
    pub async fn cancel_order(&self, order_id: &str, symbol: &str) -> Result<bool, String> {
        Python::with_gil(|py| {
            self.exchange
                .as_ref(py)
                .call_method1("cancel_order", (order_id, symbol))
                .map_err(|e| format!("Failed to cancel order: {}", e))?;
            
            Ok(true)
        })
    }
    
    pub fn cancel_order_sync(&self, order_id: &str, symbol: Option<&str>) -> Result<bool, String> {
        Python::with_gil(|py| {
            if let Some(sym) = symbol {
                self.exchange
                    .as_ref(py)
                    .call_method1("cancel_order", (order_id, sym))
                    .map_err(|e| format!("Failed to cancel order: {}", e))?;
            } else {
                self.exchange
                    .as_ref(py)
                    .call_method1("cancel_order", (order_id,))
                    .map_err(|e| format!("Failed to cancel order: {}", e))?;
            }
            
            Ok(true)
        })
    }
    
    pub fn cancel_all_orders_sync(&self, symbol: Option<&str>) -> Result<usize, String> {
        Python::with_gil(|py| {
            let result = if let Some(sym) = symbol {
                self.exchange
                    .as_ref(py)
                    .call_method1("cancel_all_orders", (sym,))
                    .map_err(|e| format!("Failed to cancel all orders: {}", e))?
            } else {
                self.exchange
                    .as_ref(py)
                    .call_method0("cancel_all_orders")
                    .map_err(|e| format!("Failed to cancel all orders: {}", e))?
            };
            
            // Result can be a list of canceled orders or None
            if let Ok(list) = result.downcast::<PyList>() {
                Ok(list.len())
            } else {
                Ok(0)
            }
        })
    }
    
    pub fn fetch_order_sync(&self, order_id: &str, symbol: &str) -> Result<PyObject, String> {
        Python::with_gil(|py| {
            let order = self.exchange
                .as_ref(py)
                .call_method1("fetch_order", (order_id, symbol))
                .map_err(|e| format!("Failed to fetch order: {}", e))?;
            
            Ok(order.into())
        })
    }
    
    pub fn fetch_orders_sync(&self, status: &str) -> Result<Vec<PyObject>, String> {
        Python::with_gil(|py| {
            let method = match status {
                "open" => "fetch_open_orders",
                "closed" => "fetch_closed_orders",
                _ => "fetch_orders",
            };
            
            let orders = self.exchange
                .as_ref(py)
                .call_method0(method)
                .map_err(|e| format!("Failed to fetch orders: {}", e))?;
            
            let mut result = Vec::new();
            
            if let Ok(orders_list) = orders.downcast::<PyList>() {
                for order in orders_list.iter() {
                    result.push(order.into());
                }
            }
            
            Ok(result)
        })
    }
    
    pub fn create_order_sync(
        &self,
        symbol: &str,
        order_type: &str,
        side: &str,
        amount: f64,
        price: Option<f64>,
    ) -> Result<PyObject, String> {
        Python::with_gil(|py| {
            let order = if let Some(p) = price {
                self.exchange
                    .as_ref(py)
                    .call_method1("create_order", (symbol, order_type, side, amount, p))
                    .map_err(|e| format!("Failed to create order: {}", e))?
            } else {
                self.exchange
                    .as_ref(py)
                    .call_method1("create_order", (symbol, order_type, side, amount))
                    .map_err(|e| format!("Failed to create order: {}", e))?
            };
            
            Ok(order.into())
        })
    }
    
    pub async fn fetch_ticker(&self, symbol: &str) -> Result<HashMap<String, f64>, String> {
        Python::with_gil(|py| {
            let ticker = self
                .exchange
                .as_ref(py)
                .call_method1("fetch_ticker", (symbol,))
                .map_err(|e| format!("Failed to fetch ticker: {}", e))?;
            
            let mut result = HashMap::new();
            
            if let Ok(ticker_dict) = ticker.downcast::<PyDict>() {
                if let Ok(Some(last)) = ticker_dict.get_item("last") {
                    if let Ok(price) = last.extract::<f64>() {
                        result.insert("last".to_string(), price);
                    }
                }
                
                if let Ok(Some(change)) = ticker_dict.get_item("percentage") {
                    if let Ok(pct) = change.extract::<f64>() {
                        result.insert("change_24h".to_string(), pct);
                    }
                }
            }
            
            Ok(result)
        })
    }
    
    pub fn fetch_ticker_sync(&self, symbol: &str) -> Result<serde_json::Value, String> {
        Python::with_gil(|py| {
            let ticker = self.exchange
                .as_ref(py)
                .call_method1("fetch_ticker", (symbol,))
                .map_err(|e| format!("Failed to fetch ticker: {}", e))?;
            
            // Convert Python dict to JSON string
            let json_module = py.import("json")
                .map_err(|e| format!("Failed to import json: {}", e))?;
            
            let json_str: String = json_module
                .call_method1("dumps", (ticker,))
                .and_then(|s| s.extract())
                .map_err(|e| format!("Failed to serialize ticker: {}", e))?;
            
            // Parse JSON string to serde_json::Value
            serde_json::from_str(&json_str)
                .map_err(|e| format!("Failed to parse JSON: {}", e))
        })
    }
    
    pub fn fetch_positions_sync(&self) -> Result<Vec<PyObject>, String> {
        Python::with_gil(|py| {
            let positions = self.exchange
                .as_ref(py)
                .call_method0("fetch_positions")
                .map_err(|e| format!("Failed to fetch positions: {}", e))?;
            
            let mut result = Vec::new();
            
            if let Ok(positions_list) = positions.downcast::<PyList>() {
                for position in positions_list.iter() {
                    result.push(position.into());
                }
            }
            
            Ok(result)
        })
    }
    
    pub fn fetch_markets_sync(&self) -> Result<Vec<PyObject>, String> {
        Python::with_gil(|py| {
            let markets = self.exchange
                .as_ref(py)
                .call_method0("fetch_markets")
                .map_err(|e| format!("Failed to fetch markets: {}", e))?;
            
            let mut result = Vec::new();
            
            if let Ok(markets_list) = markets.downcast::<PyList>() {
                for market in markets_list.iter() {
                    result.push(market.into());
                }
            }
            
            Ok(result)
        })
    }
    
    /// Fetch raw balance from exchange (for MEXC special handling)
    pub fn fetch_balance_raw(&self) -> Result<PyObject, String> {
        Python::with_gil(|py| {
            let balance = self.exchange
                .as_ref(py)
                .call_method0("fetch_balance")
                .map_err(|e| format!("Failed to fetch balance: {}", e))?;
            
            Ok(balance.into())
        })
    }
    
    /// Fetch open orders for a specific symbol (used for MEXC)
    pub fn fetch_open_orders_with_symbol(&self, symbol: &str) -> Result<Vec<PyObject>, String> {
        Python::with_gil(|py| {
            let orders = self.exchange
                .as_ref(py)
                .call_method1("fetch_open_orders", (symbol,))
                .map_err(|e| format!("Failed to fetch open orders for {}: {}", symbol, e))?;
            
            let mut result = Vec::new();
            
            if let Ok(orders_list) = orders.downcast::<PyList>() {
                for order in orders_list.iter() {
                    result.push(order.into());
                }
            }
            
            Ok(result)
        })
    }
    
    /// Get exchange markets (for checking if symbol exists)
    pub fn get_markets(&self) -> Result<PyObject, String> {
        Python::with_gil(|py| {
            let markets = self.exchange
                .as_ref(py)
                .getattr("markets")
                .map_err(|e| format!("Failed to get markets: {}", e))?;
            
            Ok(markets.into())
        })
    }
}

