use anyhow::Result;
use trading_service::{CCXTClient, MongoDBClient};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logger
    env_logger::init();

    // Load environment variables
    dotenv::dotenv().ok();

    log::info!("Starting Trading Service...");

    // Initialize MongoDB connection
    let mongo_client = MongoDBClient::new().await?;
    log::info!("MongoDB connection established");

    // Test MongoDB health
    if mongo_client.health_check().await? {
        log::info!("MongoDB health check passed");
    }

    // Initialize CCXT client
    let ccxt_client = CCXTClient::new()?;
    log::info!("CCXT client initialized");

    // Example: Fetch ticker for BTC/USDT
    match ccxt_client.fetch_ticker("BTC/USDT") {
        Ok(ticker) => {
            log::info!("Ticker data for BTC/USDT:");
            log::info!("  Symbol: {}", ticker.symbol);
            if let Some(last) = ticker.last {
                log::info!("  Last Price: ${:.2}", last);
            }
            if let Some(bid) = ticker.bid {
                log::info!("  Bid: ${:.2}", bid);
            }
            if let Some(ask) = ticker.ask {
                log::info!("  Ask: ${:.2}", ask);
            }
            if let Some(volume) = ticker.volume {
                log::info!("  Volume: {:.2}", volume);
            }
        }
        Err(e) => {
            log::error!("Failed to fetch ticker: {}", e);
        }
    }

    // Example: Get available markets (first 10)
    match ccxt_client.get_markets() {
        Ok(markets) => {
            log::info!("Available markets (first 10):");
            for market in markets.iter().take(10) {
                log::info!("  - {}", market);
            }
            log::info!("  ... and {} more", markets.len().saturating_sub(10));
        }
        Err(e) => {
            log::error!("Failed to get markets: {}", e);
        }
    }

    // Example: Fetch balance (if API credentials are configured)
    match ccxt_client.fetch_balance() {
        Ok(balances) => {
            log::info!("Account balances:");
            for balance in balances {
                log::info!(
                    "  {}: Free={:.8}, Used={:.8}, Total={:.8}",
                    balance.currency,
                    balance.free,
                    balance.used,
                    balance.total
                );
            }
        }
        Err(e) => {
            log::warn!(
                "Could not fetch balance (API credentials may not be configured): {}",
                e
            );
        }
    }

    log::info!("Trading Service demonstration complete");
    Ok(())
}
