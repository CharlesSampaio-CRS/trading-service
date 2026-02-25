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
            config.set_item("timeout", 30000).map_err(|e| e.to_string())?; // 30 segundos
            
            // 🚀 OTIMIZAÇÃO: HTTP Connection pooling e keepAlive
            // Reutiliza conexões TCP/TLS ao invés de criar novas a cada request
            let http_agent_config = PyDict::new(py);
            http_agent_config.set_item("keepAlive", true).map_err(|e| e.to_string())?;
            http_agent_config.set_item("keepAliveMsecs", 30000).map_err(|e| e.to_string())?; // 30s
            http_agent_config.set_item("maxSockets", 10).map_err(|e| e.to_string())?;
            http_agent_config.set_item("maxFreeSockets", 5).map_err(|e| e.to_string())?;
            config.set_item("agent", http_agent_config).map_err(|e| e.to_string())?;
            
            // ❌ DESABILITA CACHE DO CCXT - Força busca sempre fresca
            let options = PyDict::new(py);
            options.set_item("warnOnFetchOpenOrdersWithoutSymbol", false).map_err(|e| e.to_string())?;
            options.set_item("fetchBalanceCacheTTL", 0).map_err(|e| e.to_string())?;  // 🔥 NO CACHE
            options.set_item("fetchTickersCacheTTL", 0).map_err(|e| e.to_string())?;  // 🔥 NO CACHE
            options.set_item("recvWindow", 10000).map_err(|e| e.to_string())?;  // 🚀 OTIMIZAÇÃO: Janela maior (menos erros de nonce)
            
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
    /// 🔥 REAL-TIME: Usa timestamp para garantir bypass de cache (exceto exchanges restritivas)
    pub fn fetch_tickers_sync(&self) -> Result<HashMap<String, f64>, String> {
        Python::with_gil(|py| {
            log::debug!("🔍 Fetching tickers from {}...", self.exchange_name);
            
            // ⚠️ Algumas exchanges (Binance, MEXC, OKX) não aceitam parâmetros personalizados
            let exchange_lower = self.exchange_name.to_lowercase();
            let is_restrictive = exchange_lower == "binance" || exchange_lower == "mexc" || exchange_lower == "okx" || exchange_lower == "okx";
            
            let tickers_obj = if is_restrictive {
                // Exchanges restritivas: SEM parâmetros
                log::debug!("🔧 [{}] Calling fetch_tickers WITHOUT params (restrictive exchange)", self.exchange_name);
                self.exchange
                    .as_ref(py)
                    .call_method0("fetch_tickers")
                    .map_err(|e| format!("Failed to fetch tickers: {}", e))?
            } else {
                // Outras exchanges: COM timestamp para bypass de cache
                let params_dict = pyo3::types::PyDict::new(py);
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis();
                params_dict.set_item("_t", timestamp)
                    .map_err(|e| format!("Failed to set timestamp param: {}", e))?;
                
                log::debug!("🔧 [{}] Calling fetch_tickers WITH timestamp: {} (NO CACHE)", self.exchange_name, timestamp);
                
                self.exchange
                    .as_ref(py)
                    .call_method1("fetch_tickers", (params_dict,))
                    .map_err(|e| format!("Failed to fetch tickers: {}", e))?
            };
            
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
            // ⚠️ IMPORTANTE: Binance, MEXC e OKX NÃO aceitam parâmetros extras!
            // Outras exchanges aceitam timestamp para bypass de cache
            let exchange_lower = exchange_name.to_lowercase();
            let is_restrictive = exchange_lower == "binance" || exchange_lower == "mexc" || exchange_lower == "okx" || exchange_lower == "okx";
            
            let balance_dict = if is_restrictive {
                // Binance/MEXC: SEM parâmetros (exchanges restritivas)
                log::debug!("🔧 [{}] Chamando fetch_balance SEM parâmetros (exchange restritiva)", exchange_name);
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
            // 🔥 REAL-TIME: Adiciona timestamp para garantir bypass de cache (exceto exchanges restritivas)
            let (tickers, changes) = {
                // ⚠️ Algumas exchanges (Binance, MEXC, OKX) não aceitam parâmetros personalizados
                let exchange_lower = exchange_name.to_lowercase();
                let is_restrictive = exchange_lower == "binance" || exchange_lower == "mexc" || exchange_lower == "okx" || exchange_lower == "okx";
                
                let tickers_result = if is_restrictive {
                    // Exchanges restritivas: SEM parâmetros
                    log::debug!("🔧 [{}] Calling fetch_tickers WITHOUT params (restrictive exchange)", exchange_name);
                    exchange.as_ref(py).call_method0("fetch_tickers")
                } else {
                    // Outras exchanges: COM timestamp para bypass de cache
                    let params_dict = pyo3::types::PyDict::new(py);
                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis();
                    if let Err(e) = params_dict.set_item("_t", timestamp) {
                        log::warn!("⚠️  Could not set timestamp for {}: {}", exchange_name, e);
                    }
                    
                    log::debug!("🔧 [{}] Calling fetch_tickers WITH timestamp: {} (NO CACHE)", exchange_name, timestamp);
                    exchange.as_ref(py).call_method1("fetch_tickers", (params_dict,))
                };
                
                match tickers_result {
                    Ok(tickers_obj) => {
                        let mut prices = HashMap::new();
                        let mut percent_changes = HashMap::new();
                        
                        // Verifica se tickers_obj não é None
                        if tickers_obj.is_none() {
                            log::warn!("⚠️  fetch_tickers returned None for {}", exchange_name);
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
                                                    // Prioriza pares com USDT, depois USDC, USD e BRL
                                                    if symbol_str.ends_with("/USDT") || 
                                                       symbol_str.ends_with("/USDC") || 
                                                       symbol_str.ends_with("/USD") ||
                                                       symbol_str.ends_with("/BRL") {
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
                                                   symbol_str.ends_with("/USD") ||
                                                   symbol_str.ends_with("/BRL") {
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
                        } else {
                            log::warn!("⚠️  Could not downcast tickers to PyDict for {}", exchange_name);
                        }
                        
                        (prices, percent_changes)
                    }
                    Err(e) => {
                        log::warn!("⚠️  Could not fetch tickers from {}: {}", exchange_name, e);
                        (HashMap::new(), HashMap::new())
                    }
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
                            || symbol == "USD"
                        {
                            // Stablecoins e USD = $1.00
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
            // ⚠️ Exchanges restritivas (Binance, MEXC, OKX, Bybit, Kraken) não aceitam parâmetros extras
            let exchange_lower = self.exchange_name.to_lowercase();
            let is_restrictive = exchange_lower == "binance" 
                || exchange_lower == "mexc" 
                || exchange_lower == "okx"
                || exchange_lower == "bybit"
                || exchange_lower == "kraken";
            
            let order = if is_restrictive {
                // Sem parâmetros para exchanges restritivas
                self.exchange
                    .as_ref(py)
                    .call_method("fetch_order", (order_id, symbol), None)
                    .map_err(|e| format!("Failed to fetch order: {}", e))?
            } else {
                // 🔥 Adiciona timestamp para bypass de cache
                let params = pyo3::types::PyDict::new(py);
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis();
                params.set_item("_t", timestamp)
                    .map_err(|e| format!("Failed to set timestamp: {}", e))?;
                
                self.exchange
                    .as_ref(py)
                    .call_method("fetch_order", (order_id, symbol), Some(params))
                    .map_err(|e| format!("Failed to fetch order: {}", e))?
            };
            
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
            
            // ⚠️ Exchanges restritivas (Binance, MEXC, OKX, Bybit, Kraken) não aceitam parâmetros extras
            let exchange_lower = self.exchange_name.to_lowercase();
            let is_restrictive = exchange_lower == "binance" 
                || exchange_lower == "mexc" 
                || exchange_lower == "okx"
                || exchange_lower == "bybit"
                || exchange_lower == "kraken";
            
            let orders = if is_restrictive {
                // Sem parâmetros para exchanges restritivas
                log::debug!("🔧 [{}] Calling {} WITHOUT params (restrictive exchange)", self.exchange_name, method);
                self.exchange
                    .as_ref(py)
                    .call_method(method, (), None)
                    .map_err(|e| format!("Failed to fetch orders: {}", e))?
            } else {
                // 🔥 Adiciona timestamp para bypass de cache
                let params = pyo3::types::PyDict::new(py);
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis();
                params.set_item("_t", timestamp)
                    .map_err(|e| format!("Failed to set timestamp: {}", e))?;
                
                log::debug!("🔧 [{}] Calling {} WITH timestamp: {}", self.exchange_name, method, timestamp);
                self.exchange
                    .as_ref(py)
                    .call_method(method, (), Some(params))
                    .map_err(|e| format!("Failed to fetch orders: {}", e))?
            };
            
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
            // ⚠️ Exchanges restritivas (Binance, MEXC) não aceitam parâmetros extras
            let exchange_lower = self.exchange_name.to_lowercase();
            let is_restrictive = exchange_lower == "binance" || exchange_lower == "mexc" || exchange_lower == "okx";
            
            let ticker = if is_restrictive {
                // Sem parâmetros para exchanges restritivas
                self.exchange
                    .as_ref(py)
                    .call_method("fetch_ticker", (symbol,), None)
                    .map_err(|e| format!("Failed to fetch ticker: {}", e))?
            } else {
                // 🔥 Adiciona timestamp para bypass de cache
                let params = pyo3::types::PyDict::new(py);
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis();
                params.set_item("_t", timestamp)
                    .map_err(|e| format!("Failed to set timestamp: {}", e))?;
                
                self.exchange
                    .as_ref(py)
                    .call_method("fetch_ticker", (symbol,), Some(params))
                    .map_err(|e| format!("Failed to fetch ticker: {}", e))?
            };
            
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
            // ⚠️ Exchanges restritivas (Binance, MEXC) não aceitam parâmetros extras
            let exchange_lower = self.exchange_name.to_lowercase();
            let is_restrictive = exchange_lower == "binance" || exchange_lower == "mexc" || exchange_lower == "okx";
            
            let positions = if is_restrictive {
                // Sem parâmetros para exchanges restritivas
                self.exchange
                    .as_ref(py)
                    .call_method("fetch_positions", (), None)
                    .map_err(|e| format!("Failed to fetch positions: {}", e))?
            } else {
                // 🔥 Adiciona timestamp para bypass de cache
                let params = pyo3::types::PyDict::new(py);
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis();
                params.set_item("_t", timestamp)
                    .map_err(|e| format!("Failed to set timestamp: {}", e))?;
                
                self.exchange
                    .as_ref(py)
                    .call_method("fetch_positions", (), Some(params))
                    .map_err(|e| format!("Failed to fetch positions: {}", e))?
            };
            
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
            // ⚠️ Exchanges restritivas (Binance, MEXC) não aceitam parâmetros extras
            let exchange_lower = self.exchange_name.to_lowercase();
            let is_restrictive = exchange_lower == "binance" || exchange_lower == "mexc" || exchange_lower == "okx";
            
            let markets = if is_restrictive {
                // Sem parâmetros para exchanges restritivas
                self.exchange
                    .as_ref(py)
                    .call_method("fetch_markets", (), None)
                    .map_err(|e| format!("Failed to fetch markets: {}", e))?
            } else {
                // 🔥 Adiciona timestamp para bypass de cache
                let params = pyo3::types::PyDict::new(py);
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis();
                params.set_item("_t", timestamp)
                    .map_err(|e| format!("Failed to set timestamp: {}", e))?;
                
                self.exchange
                    .as_ref(py)
                    .call_method("fetch_markets", (), Some(params))
                    .map_err(|e| format!("Failed to fetch markets: {}", e))?
            };
            
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
    /// 🔥 REAL-TIME: Usa timestamp para garantir bypass de cache
    pub fn fetch_balance_raw(&self) -> Result<PyObject, String> {
        Python::with_gil(|py| {
            // ⚠️ Exchanges restritivas (Binance, MEXC) não aceitam parâmetros extras
            let exchange_lower = self.exchange_name.to_lowercase();
            let is_restrictive = exchange_lower == "binance" || exchange_lower == "mexc" || exchange_lower == "okx";
            
            let balance = if is_restrictive {
                // Sem parâmetros para exchanges restritivas
                self.exchange
                    .as_ref(py)
                    .call_method("fetch_balance", (), None)
                    .map_err(|e| format!("Failed to fetch balance: {}", e))?
            } else {
                // 🔥 Adiciona timestamp para bypass de cache
                let params = pyo3::types::PyDict::new(py);
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis();
                params.set_item("_t", timestamp)
                    .map_err(|e| format!("Failed to set timestamp: {}", e))?;
                
                self.exchange
                    .as_ref(py)
                    .call_method("fetch_balance", (), Some(params))
                    .map_err(|e| format!("Failed to fetch balance: {}", e))?
            };
            
            Ok(balance.into())
        })
    }
    
    /// Fetch open orders for a specific symbol (used for MEXC)
    /// 🔥 REAL-TIME: Usa timestamp para garantir bypass de cache
    pub fn fetch_open_orders_with_symbol(&self, symbol: &str) -> Result<Vec<PyObject>, String> {
        Python::with_gil(|py| {
            // ⚠️ Exchanges restritivas (Binance, MEXC) não aceitam parâmetros extras
            let exchange_lower = self.exchange_name.to_lowercase();
            let is_restrictive = exchange_lower == "binance" || exchange_lower == "mexc" || exchange_lower == "okx";
            
            let orders = if is_restrictive {
                // Sem parâmetros para exchanges restritivas
                self.exchange
                    .as_ref(py)
                    .call_method("fetch_open_orders", (symbol,), None)
                    .map_err(|e| format!("Failed to fetch open orders for {}: {}", symbol, e))?
            } else {
                // 🔥 Adiciona timestamp para bypass de cache
                let params = pyo3::types::PyDict::new(py);
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis();
                params.set_item("_t", timestamp)
                    .map_err(|e| format!("Failed to set timestamp: {}", e))?;
                
                self.exchange
                    .as_ref(py)
                    .call_method("fetch_open_orders", (symbol,), Some(params))
                    .map_err(|e| format!("Failed to fetch open orders for {}: {}", symbol, e))?
            };
            
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
    
    /// Search market symbols by query string
    /// 🔥 REAL-TIME: Usa timestamp para garantir bypass de cache
    pub fn search_markets_symbols_sync(&self, query: &str, limit: usize) -> Result<Vec<String>, String> {
        Python::with_gil(|py| {
            // ⚠️ Exchanges restritivas (Binance, MEXC) não aceitam parâmetros extras
            let exchange_lower = self.exchange_name.to_lowercase();
            let is_restrictive = exchange_lower == "binance" || exchange_lower == "mexc" || exchange_lower == "okx";
            
            let markets = if is_restrictive {
                // Sem parâmetros para exchanges restritivas
                self.exchange
                    .as_ref(py)
                    .call_method("fetch_markets", (), None)
                    .map_err(|e| format!("Failed to fetch markets: {}", e))?
            } else {
                // 🔥 Adiciona timestamp para bypass de cache
                let params = pyo3::types::PyDict::new(py);
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis();
                params.set_item("_t", timestamp)
                    .map_err(|e| format!("Failed to set timestamp: {}", e))?;
                
                self.exchange
                    .as_ref(py)
                    .call_method("fetch_markets", (), Some(params))
                    .map_err(|e| format!("Failed to fetch markets: {}", e))?
            };

            let query_upper = query.trim().to_uppercase();
            if query_upper.is_empty() {
                return Ok(Vec::new());
            }

            let mut seen = std::collections::HashSet::new();
            let mut symbols = Vec::new();

            if let Ok(markets_list) = markets.downcast::<PyList>() {
                for market in markets_list.iter() {
                    let market_dict = match market.downcast::<PyDict>() {
                        Ok(dict) => dict,
                        Err(_) => continue,
                    };

                    let is_active = market_dict
                        .get_item("active")
                        .ok()
                        .flatten()
                        .and_then(|v| v.extract::<bool>().ok())
                        .unwrap_or(true);

                    if !is_active {
                        continue;
                    }

                    let base_symbol = market_dict
                        .get_item("base")
                        .ok()
                        .flatten()
                        .and_then(|v| v.extract::<String>().ok())
                        .or_else(|| {
                            market_dict
                                .get_item("symbol")
                                .ok()
                                .flatten()
                                .and_then(|v| v.extract::<String>().ok())
                                .and_then(|pair| pair.split('/').next().map(|v| v.to_string()))
                        });

                    let base_symbol = match base_symbol {
                        Some(symbol) if !symbol.trim().is_empty() => symbol.to_uppercase(),
                        _ => continue,
                    };

                    if !base_symbol.contains(&query_upper) {
                        continue;
                    }

                    if seen.insert(base_symbol.clone()) {
                        symbols.push(base_symbol);
                        if symbols.len() >= limit {
                            break;
                        }
                    }
                }
            }

            Ok(symbols)
        })
    }

    /// Verifica as permissões da API key testando operações específicas
    pub fn check_api_permissions(&self) -> Result<crate::services::user_exchanges_service::ApiPermissions, String> {
        Python::with_gil(|py| {
            log::info!("🔐 Checking API key permissions for {}...", self.exchange_name);
            
            let mut permissions = crate::services::user_exchanges_service::ApiPermissions {
                can_read: false,
                can_trade: false,
                can_withdraw: false,
                is_restricted: false,
            };
            
            // 1. Verificar leitura (já validado com fetch_balance, mas vamos confirmar)
            match self.exchange.as_ref(py).call_method0("fetch_balance") {
                Ok(_) => {
                    permissions.can_read = true;
                    log::info!("✅ Read permission confirmed");
                }
                Err(e) => {
                    log::error!("❌ Read permission denied: {}", e);
                    return Ok(permissions);
                }
            }
            
            // 2. Verificar permissões de trade (Spot)
            // Testamos com fetch_open_orders que requer autenticação de trade
            // Se falhar com erro de permissão = key não tem trade
            // Se funcionar ou falhar por outro motivo = key tem trade
            log::info!("🔍 Testing trade permission via fetch_open_orders...");
            match self.exchange.as_ref(py).call_method0("fetch_open_orders") {
                Ok(_) => {
                    permissions.can_trade = true;
                    log::info!("✅ Trade permission confirmed (fetch_open_orders succeeded)");
                }
                Err(e) => {
                    let error_str = e.to_string().to_lowercase();
                    // Se o erro é de permissão, a key não tem trade
                    if error_str.contains("permission") || 
                       error_str.contains("not allowed") ||
                       error_str.contains("unauthorized") ||
                       error_str.contains("forbidden") ||
                       error_str.contains("denied") ||
                       error_str.contains("trade") && error_str.contains("disabled") {
                        permissions.can_trade = false;
                        log::warn!("⚠️ Trade permission denied: {}", error_str);
                    } else {
                        // Outros erros (ex: "symbol required", "no orders", etc.) = tem permissão
                        permissions.can_trade = true;
                        log::info!("✅ Trade permission assumed (error is not permission-related): {}", error_str);
                    }
                }
            }
            
            // 3. Verificar permissões de withdrawal
            // IMPORTANTE: exchange.has['withdraw'] indica que a EXCHANGE suporta saques,
            // NÃO que a API key tem permissão. Para verificar a permissão real da key,
            // tentamos chamar fetchDepositAddress que requer permissão de withdraw.
            // Se funcionar, a key tem permissão de withdraw (inseguro!).
            // Se falhar com erro de permissão, a key não tem (seguro!).
            let has_fetch_deposit_address = self.exchange
                .as_ref(py)
                .getattr("has")
                .ok()
                .and_then(|has_dict| has_dict.downcast::<PyDict>().ok())
                .and_then(|dict| dict.get_item("fetchDepositAddress").ok().flatten())
                .and_then(|v| v.extract::<bool>().ok())
                .unwrap_or(false);
            
            if has_fetch_deposit_address {
                // Tentar operação que requer permissão de withdraw para detectar se a key tem
                log::info!("🔍 Testing withdrawal permission via fetchDepositAddress...");
                let withdraw_test = self.exchange
                    .as_ref(py)
                    .call_method1("fetch_deposit_address", ("BTC",));
                
                match withdraw_test {
                    Ok(_) => {
                        // Se conseguiu buscar endereço de depósito, a key tem permissão de withdraw
                        permissions.can_withdraw = true;
                        log::warn!("⚠️ Withdrawal permission detected - API key can withdraw!");
                    }
                    Err(e) => {
                        let error_str = e.to_string().to_lowercase();
                        if error_str.contains("permission") || 
                           error_str.contains("not allowed") ||
                           error_str.contains("unauthorized") ||
                           error_str.contains("forbidden") ||
                           error_str.contains("denied") ||
                           error_str.contains("apikey") ||
                           error_str.contains("api key") {
                            // Erro de permissão = key não tem withdraw (bom!)
                            permissions.can_withdraw = false;
                            log::info!("✅ No withdrawal permission detected (key is safe)");
                        } else {
                            // Outro tipo de erro (ex: moeda não suportada, rede, etc)
                            // Não conseguimos determinar, assumir que NÃO tem (mais seguro)
                            permissions.can_withdraw = false;
                            log::info!("✅ Withdrawal permission unclear, assuming disabled (safe default): {}", error_str);
                        }
                    }
                }
            } else {
                // Exchange não suporta fetchDepositAddress - não podemos testar
                // Assumir que NÃO tem permissão (default seguro)
                permissions.can_withdraw = false;
                log::info!("✅ Cannot test withdrawal permission (no fetchDepositAddress), assuming disabled");
            }
            
            // 4. Verificar se há restrições de IP
            // Isso é difícil de detectar diretamente, mas podemos inferir de erros específicos
            // Por ora, marcar como false (sem restrições detectadas)
            permissions.is_restricted = false;
            
            log::info!("🔐 Permissions summary: read={}, trade={}, withdraw={}", 
                permissions.can_read, permissions.can_trade, permissions.can_withdraw);
            
            Ok(permissions)
        })
    }
    
    /// Obtém informações sobre rate limits da exchange
    pub fn get_rate_limit_info(&self) -> Result<crate::services::user_exchanges_service::RateLimitInfo, String> {
        Python::with_gil(|py| {
            log::info!("⏱️ Checking rate limits for {}...", self.exchange_name);
            
            // Tentar obter informações de rate limit do objeto exchange
            let rate_limit = self.exchange
                .as_ref(py)
                .getattr("rateLimit")
                .and_then(|v| v.extract::<u32>())
                .ok();
            
            // Tentar obter headers da última requisição
            let last_response_headers = self.exchange
                .as_ref(py)
                .getattr("last_response_headers")
                .ok();
            
            let mut remaining = None;
            let mut limit = None;
            let mut reset_at = None;
            
            if let Some(headers_obj) = last_response_headers {
                if let Ok(headers_dict) = headers_obj.downcast::<PyDict>() {
                    // Tentar diferentes headers de rate limit (varia por exchange)
                    remaining = headers_dict
                        .get_item("X-RateLimit-Remaining")
                        .or_else(|_| headers_dict.get_item("x-ratelimit-remaining"))
                        .or_else(|_| headers_dict.get_item("X-MBX-USED-WEIGHT-1M"))
                        .ok()
                        .flatten()
                        .and_then(|v| v.extract::<String>().ok())
                        .and_then(|s| s.parse::<u32>().ok());
                    
                    limit = headers_dict
                        .get_item("X-RateLimit-Limit")
                        .or_else(|_| headers_dict.get_item("x-ratelimit-limit"))
                        .ok()
                        .flatten()
                        .and_then(|v| v.extract::<String>().ok())
                        .and_then(|s| s.parse::<u32>().ok());
                    
                    reset_at = headers_dict
                        .get_item("X-RateLimit-Reset")
                        .or_else(|_| headers_dict.get_item("x-ratelimit-reset"))
                        .ok()
                        .flatten()
                        .and_then(|v| v.extract::<String>().ok())
                        .and_then(|s| s.parse::<i64>().ok());
                }
            }
            
            if let Some(rl) = rate_limit {
                log::info!("⏱️ Rate limit: {} ms between requests", rl);
            }
            
            if let Some(rem) = remaining {
                log::info!("⏱️ Remaining requests: {}", rem);
                if rem < 10 {
                    log::warn!("⚠️ Rate limit nearly exhausted! Only {} requests remaining", rem);
                }
            }
            
            Ok(crate::services::user_exchanges_service::RateLimitInfo {
                remaining,
                limit,
                reset_at,
            })
        })
    }
}

