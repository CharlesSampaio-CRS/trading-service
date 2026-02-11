use anyhow::{anyhow, Result};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyModule};
use serde::{Deserialize, Serialize};
use std::env;

/// CCXT wrapper for cryptocurrency trading operations
pub struct CCXTClient {
    exchange_name: String,
    api_key: Option<String>,
    api_secret: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Ticker {
    pub symbol: String,
    pub last: Option<f64>,
    pub bid: Option<f64>,
    pub ask: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub volume: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Balance {
    pub currency: String,
    pub free: f64,
    pub used: f64,
    pub total: f64,
}

impl CCXTClient {
    /// Create a new CCXT client
    pub fn new() -> Result<Self> {
        let exchange_name = env::var("CCXT_EXCHANGE").unwrap_or_else(|_| "binance".to_string());
        let api_key = env::var("CCXT_API_KEY").ok();
        let api_secret = env::var("CCXT_API_SECRET").ok();

        log::info!("Initializing CCXT client for exchange: {}", exchange_name);

        Ok(Self {
            exchange_name,
            api_key,
            api_secret,
        })
    }

    /// Fetch ticker information for a trading pair
    pub fn fetch_ticker(&self, symbol: &str) -> Result<Ticker> {
        Python::with_gil(|py| {
            let ccxt = PyModule::import_bound(py, "ccxt").map_err(|e| {
                anyhow!(
                    "Failed to import ccxt: {}. Make sure ccxt is installed via pip.",
                    e
                )
            })?;

            // Create exchange instance
            let exchange_class = ccxt
                .getattr(&*self.exchange_name)
                .map_err(|e| anyhow!("Exchange '{}' not found: {}", self.exchange_name, e))?;

            let config = PyDict::new_bound(py);
            if let Some(ref key) = self.api_key {
                config.set_item("apiKey", key)?;
            }
            if let Some(ref secret) = self.api_secret {
                config.set_item("secret", secret)?;
            }

            let exchange = exchange_class.call1((config,))?;

            // Fetch ticker
            let ticker = exchange
                .call_method1("fetch_ticker", (symbol,))
                .map_err(|e| anyhow!("Failed to fetch ticker for {}: {}", symbol, e))?;

            // Extract ticker data
            let symbol_str = ticker
                .getattr("symbol")
                .and_then(|v| v.extract::<String>())
                .unwrap_or_else(|_| symbol.to_string());

            let last = ticker.getattr("last").and_then(|v| v.extract::<f64>()).ok();

            let bid = ticker.getattr("bid").and_then(|v| v.extract::<f64>()).ok();

            let ask = ticker.getattr("ask").and_then(|v| v.extract::<f64>()).ok();

            let high = ticker.getattr("high").and_then(|v| v.extract::<f64>()).ok();

            let low = ticker.getattr("low").and_then(|v| v.extract::<f64>()).ok();

            let volume = ticker
                .getattr("baseVolume")
                .and_then(|v| v.extract::<f64>())
                .ok();

            Ok(Ticker {
                symbol: symbol_str,
                last,
                bid,
                ask,
                high,
                low,
                volume,
            })
        })
    }

    /// Fetch account balance (requires API credentials)
    pub fn fetch_balance(&self) -> Result<Vec<Balance>> {
        if self.api_key.is_none() || self.api_secret.is_none() {
            return Err(anyhow!("API credentials not configured"));
        }

        Python::with_gil(|py| {
            let ccxt = PyModule::import_bound(py, "ccxt")
                .map_err(|e| anyhow!("Failed to import ccxt: {}", e))?;

            // Create exchange instance
            let exchange_class = ccxt
                .getattr(&*self.exchange_name)
                .map_err(|e| anyhow!("Exchange '{}' not found: {}", self.exchange_name, e))?;

            let config = PyDict::new_bound(py);
            config.set_item("apiKey", self.api_key.as_ref().unwrap())?;
            config.set_item("secret", self.api_secret.as_ref().unwrap())?;

            let exchange = exchange_class.call1((config,))?;

            // Fetch balance
            let balance = exchange
                .call_method0("fetch_balance")
                .map_err(|e| anyhow!("Failed to fetch balance: {}", e))?;

            let mut balances = Vec::new();

            // Get the balance dict (usually under 'total', 'free', 'used' keys)
            if let Ok(total_attr) = balance.getattr("total") {
                if let Ok(total_dict) = total_attr.downcast::<PyDict>() {
                    for (currency, total_value) in total_dict.iter() {
                        if let (Ok(currency_str), Ok(total_val)) =
                            (currency.extract::<String>(), total_value.extract::<f64>())
                        {
                            if total_val > 0.0 {
                                // Get free balance
                                let free_val = if let Ok(free_attr) = balance.getattr("free") {
                                    if let Ok(free_dict) = free_attr.downcast::<PyDict>() {
                                        free_dict
                                            .get_item(&currency_str)
                                            .ok()
                                            .flatten()
                                            .and_then(|v| v.extract::<f64>().ok())
                                            .unwrap_or(0.0)
                                    } else {
                                        0.0
                                    }
                                } else {
                                    0.0
                                };

                                // Get used balance
                                let used_val = if let Ok(used_attr) = balance.getattr("used") {
                                    if let Ok(used_dict) = used_attr.downcast::<PyDict>() {
                                        used_dict
                                            .get_item(&currency_str)
                                            .ok()
                                            .flatten()
                                            .and_then(|v| v.extract::<f64>().ok())
                                            .unwrap_or(0.0)
                                    } else {
                                        0.0
                                    }
                                } else {
                                    0.0
                                };

                                balances.push(Balance {
                                    currency: currency_str,
                                    free: free_val,
                                    used: used_val,
                                    total: total_val,
                                });
                            }
                        }
                    }
                }
            }

            Ok(balances)
        })
    }

    /// Get list of supported markets
    pub fn get_markets(&self) -> Result<Vec<String>> {
        Python::with_gil(|py| {
            let ccxt = PyModule::import_bound(py, "ccxt")
                .map_err(|e| anyhow!("Failed to import ccxt: {}", e))?;

            let exchange_class = ccxt
                .getattr(&*self.exchange_name)
                .map_err(|e| anyhow!("Exchange '{}' not found: {}", self.exchange_name, e))?;

            let config = PyDict::new_bound(py);
            let exchange = exchange_class.call1((config,))?;

            // Load markets
            let markets = exchange
                .call_method0("load_markets")
                .map_err(|e| anyhow!("Failed to load markets: {}", e))?;

            let markets_dict = markets
                .downcast::<PyDict>()
                .map_err(|e| anyhow!("Markets is not a dict: {}", e))?;

            let mut market_list = Vec::new();
            for (symbol, _) in markets_dict.iter() {
                if let Ok(symbol_str) = symbol.extract::<String>() {
                    market_list.push(symbol_str);
                }
            }

            Ok(market_list)
        })
    }
}

impl Default for CCXTClient {
    fn default() -> Self {
        Self::new().expect("Failed to create CCXT client")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires Python and CCXT to be installed
    fn test_ccxt_client_creation() {
        dotenv::dotenv().ok();
        let client = CCXTClient::new();
        assert!(client.is_ok());
    }

    #[test]
    #[ignore] // Requires Python, CCXT, and network access
    fn test_fetch_ticker() {
        dotenv::dotenv().ok();
        let client = CCXTClient::new().unwrap();
        let ticker = client.fetch_ticker("BTC/USDT");

        if ticker.is_ok() {
            println!("Ticker: {:?}", ticker.unwrap());
        }
    }
}
