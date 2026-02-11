use serde::{Deserialize, Serialize};
use reqwest;
use std::collections::HashMap;

const COINGECKO_API_BASE: &str = "https://api.coingecko.com/api/v3";

#[derive(Debug, Serialize, Deserialize)]
pub struct CoinGeckoTokenInfo {
    pub id: String,
    pub symbol: String,
    pub name: String,
    #[serde(default)]
    pub image: Option<CoinGeckoImage>,
    #[serde(default)]
    pub market_data: Option<CoinGeckoMarketData>,
    #[serde(default)]
    pub description: Option<HashMap<String, String>>,
    #[serde(default)]
    pub links: Option<CoinGeckoLinks>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoinGeckoImage {
    pub thumb: Option<String>,
    pub small: Option<String>,
    pub large: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoinGeckoMarketData {
    #[serde(default)]
    pub current_price: Option<HashMap<String, f64>>,
    #[serde(default)]
    pub market_cap: Option<HashMap<String, f64>>,
    #[serde(default)]
    pub total_volume: Option<HashMap<String, f64>>,
    #[serde(default)]
    pub price_change_percentage_24h: Option<f64>,
    #[serde(default)]
    pub price_change_percentage_7d: Option<f64>,
    #[serde(default)]
    pub price_change_percentage_30d: Option<f64>,
    #[serde(default)]
    pub ath: Option<HashMap<String, f64>>,
    #[serde(default)]
    pub atl: Option<HashMap<String, f64>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoinGeckoLinks {
    #[serde(default)]
    pub homepage: Option<Vec<String>>,
    #[serde(default)]
    pub whitepaper: Option<String>,
    #[serde(default)]
    pub blockchain_site: Option<Vec<String>>,
    #[serde(default)]
    pub official_forum_url: Option<Vec<String>>,
    #[serde(default)]
    pub subreddit_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TokenInfoResponse {
    pub success: bool,
    pub source: String,
    pub symbol: String,
    pub name: String,
    pub coingecko_id: Option<String>,
    pub image: Option<String>,
    pub current_price_usd: Option<f64>,
    pub market_cap_usd: Option<f64>,
    pub volume_24h_usd: Option<f64>,
    pub price_change_24h: Option<f64>,
    pub price_change_7d: Option<f64>,
    pub price_change_30d: Option<f64>,
    pub ath_usd: Option<f64>,
    pub atl_usd: Option<f64>,
    pub description: Option<String>,
    pub website: Option<String>,
    pub whitepaper: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoinGeckoSimplePrice {
    #[serde(flatten)]
    pub prices: HashMap<String, CoinPrice>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoinPrice {
    pub usd: f64,
    #[serde(default)]
    pub usd_24h_change: Option<f64>,
}

/// Busca informações detalhadas de um token no CoinGecko
pub async fn get_token_info_from_coingecko(
    coingecko_id: &str,
) -> Result<TokenInfoResponse, String> {
    log::info!("🦎 Fetching token info from CoinGecko: {}", coingecko_id);

    let url = format!("{}/coins/{}?localization=false&tickers=false&market_data=true&community_data=false&developer_data=false&sparkline=false", 
        COINGECKO_API_BASE, coingecko_id);

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch from CoinGecko: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("CoinGecko API error: {}", response.status()));
    }

    let coin_data: CoinGeckoTokenInfo = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse CoinGecko response: {}", e))?;

    // Extract data
    let current_price_usd = coin_data.market_data
        .as_ref()
        .and_then(|md| md.current_price.as_ref())
        .and_then(|prices| prices.get("usd"))
        .copied();

    let market_cap_usd = coin_data.market_data
        .as_ref()
        .and_then(|md| md.market_cap.as_ref())
        .and_then(|mc| mc.get("usd"))
        .copied();

    let volume_24h_usd = coin_data.market_data
        .as_ref()
        .and_then(|md| md.total_volume.as_ref())
        .and_then(|vol| vol.get("usd"))
        .copied();

    let price_change_24h = coin_data.market_data
        .as_ref()
        .and_then(|md| md.price_change_percentage_24h);

    let price_change_7d = coin_data.market_data
        .as_ref()
        .and_then(|md| md.price_change_percentage_7d);

    let price_change_30d = coin_data.market_data
        .as_ref()
        .and_then(|md| md.price_change_percentage_30d);

    let ath_usd = coin_data.market_data
        .as_ref()
        .and_then(|md| md.ath.as_ref())
        .and_then(|ath| ath.get("usd"))
        .copied();

    let atl_usd = coin_data.market_data
        .as_ref()
        .and_then(|md| md.atl.as_ref())
        .and_then(|atl| atl.get("usd"))
        .copied();

    let image = coin_data.image
        .and_then(|img| img.large.or(img.small).or(img.thumb));

    let description = coin_data.description
        .and_then(|desc| desc.get("en").cloned());

    let website = coin_data.links
        .as_ref()
        .and_then(|links| links.homepage.as_ref())
        .and_then(|sites| sites.first().cloned());

    let whitepaper = coin_data.links
        .as_ref()
        .and_then(|links| links.whitepaper.clone());

    log::info!("✅ CoinGecko data retrieved for {}: ${:?}", 
        coin_data.symbol.to_uppercase(), current_price_usd);

    Ok(TokenInfoResponse {
        success: true,
        source: "coingecko".to_string(),
        symbol: coin_data.symbol.to_uppercase(),
        name: coin_data.name,
        coingecko_id: Some(coin_data.id),
        image,
        current_price_usd,
        market_cap_usd,
        volume_24h_usd,
        price_change_24h,
        price_change_7d,
        price_change_30d,
        ath_usd,
        atl_usd,
        description,
        website,
        whitepaper,
    })
}

/// Busca preços de múltiplos tokens no CoinGecko (batch)
pub async fn get_prices_from_coingecko(
    coingecko_ids: Vec<String>,
) -> Result<HashMap<String, f64>, String> {
    if coingecko_ids.is_empty() {
        return Ok(HashMap::new());
    }

    log::info!("🦎 Fetching prices from CoinGecko for {} tokens", coingecko_ids.len());

    let ids_string = coingecko_ids.join(",");
    let url = format!("{}/simple/price?ids={}&vs_currencies=usd&include_24hr_change=true", 
        COINGECKO_API_BASE, ids_string);

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch prices from CoinGecko: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("CoinGecko API error: {}", response.status()));
    }

    let prices_data: HashMap<String, CoinPrice> = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse CoinGecko prices: {}", e))?;

    let mut result = HashMap::new();
    for (id, price_data) in prices_data {
        result.insert(id, price_data.usd);
    }

    log::info!("✅ Retrieved {} prices from CoinGecko", result.len());

    Ok(result)
}

/// Busca informações de um token por símbolo (tenta encontrar o coingecko_id primeiro)
pub async fn search_token_by_symbol(
    symbol: &str,
) -> Result<Vec<CoinGeckoSearchResult>, String> {
    log::info!("🔍 Searching CoinGecko for symbol: {}", symbol);

    let url = format!("{}/search?query={}", COINGECKO_API_BASE, symbol);

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Failed to search CoinGecko: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("CoinGecko API error: {}", response.status()));
    }

    let search_response: CoinGeckoSearchResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse CoinGecko search: {}", e))?;

    log::info!("✅ Found {} results for '{}'", search_response.coins.len(), symbol);

    Ok(search_response.coins)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoinGeckoSearchResponse {
    pub coins: Vec<CoinGeckoSearchResult>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CoinGeckoSearchResult {
    pub id: String,
    pub name: String,
    pub symbol: String,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub large: Option<String>,
    #[serde(default)]
    pub market_cap_rank: Option<u32>,
}
