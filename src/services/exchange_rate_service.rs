use serde::{Deserialize, Serialize};
use reqwest;
use std::collections::HashMap;

// ExchangeRate-API (Free tier: 1,500 requests/month)
const EXCHANGERATE_API_BASE: &str = "https://api.exchangerate-api.com/v4/latest";

// Alternativa: Open Exchange Rates (Free tier: 1,000 requests/month)
// const OPEN_EXCHANGE_RATES_BASE: &str = "https://open.er-api.com/v6/latest";

// Alternativa 2: Fixer.io (requer API key)
// const FIXER_API_BASE: &str = "https://api.fixer.io/latest";

#[derive(Debug, Serialize, Deserialize)]
pub struct ExchangeRatesResponse {
    pub base: String,
    pub date: String,
    pub rates: HashMap<String, f64>,
}

#[derive(Debug, Serialize)]
pub struct ConversionResponse {
    pub success: bool,
    pub from: String,
    pub to: String,
    pub rate: f64,
    pub amount: Option<f64>,
    pub converted: Option<f64>,
    pub last_updated: String,
}

#[derive(Debug, Serialize)]
pub struct AllRatesResponse {
    pub success: bool,
    pub base: String,
    pub rates: HashMap<String, f64>,
    pub last_updated: String,
}

/// Busca taxa de câmbio entre duas moedas
pub async fn get_exchange_rate(
    from: &str,
    to: &str,
) -> Result<f64, String> {
    log::info!("💱 Fetching exchange rate: {} -> {}", from, to);

    // Se from e to são iguais, retorna 1.0
    if from.to_uppercase() == to.to_uppercase() {
        return Ok(1.0);
    }

    let url = format!("{}/{}", EXCHANGERATE_API_BASE, from.to_uppercase());

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch exchange rate: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Exchange rate API error: {}", response.status()));
    }

    let rates_data: ExchangeRatesResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse exchange rates: {}", e))?;

    let to_upper = to.to_uppercase();
    let rate = rates_data.rates
        .get(&to_upper)
        .copied()
        .ok_or_else(|| format!("Currency '{}' not found in rates", to))?;

    log::info!("✅ Exchange rate {}/{}: {:.4}", from, to, rate);

    Ok(rate)
}

/// 🚀 OTIMIZAÇÃO: Busca múltiplas taxas de câmbio em uma única chamada
/// Reduz N chamadas para 1 chamada (muito mais rápido!)
pub async fn get_batch_exchange_rates(
    from_currencies: Vec<&str>,
    to: &str,
) -> Result<HashMap<String, f64>, String> {
    log::info!("💱 Fetching batch exchange rates: {:?} -> {}", from_currencies, to);

    if from_currencies.is_empty() {
        return Ok(HashMap::new());
    }

    // Busca todas as taxas a partir da moeda destino
    let url = format!("{}/{}", EXCHANGERATE_API_BASE, to.to_uppercase());

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch batch exchange rates: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Exchange rate API error: {}", response.status()));
    }

    let rates_data: ExchangeRatesResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse exchange rates: {}", e))?;

    // Extrai apenas as moedas solicitadas e inverte a taxa (FROM/TO ao invés de TO/FROM)
    let mut result = HashMap::new();
    for currency in from_currencies {
        let currency_upper = currency.to_uppercase();
        
        // Se moeda de origem == moeda destino, taxa = 1.0
        if currency_upper == to.to_uppercase() {
            result.insert(currency_upper.clone(), 1.0);
            continue;
        }
        
        if let Some(&rate) = rates_data.rates.get(&currency_upper) {
            // Inverte a taxa: se 1 USD = 5.5 BRL, então 1 BRL = 1/5.5 USD
            let inverted_rate = 1.0 / rate;
            result.insert(currency_upper.clone(), inverted_rate);
            log::debug!("💱 {}/{}: {:.6}", currency, to, inverted_rate);
        }
    }

    log::info!("✅ Fetched {} batch exchange rates to {}", result.len(), to);

    Ok(result)
}

/// Converte um valor de uma moeda para outra
pub async fn convert_currency(
    from: &str,
    to: &str,
    amount: f64,
) -> Result<ConversionResponse, String> {
    log::info!("💱 Converting {:.2} {} to {}", amount, from, to);

    let rate = get_exchange_rate(from, to).await?;
    let converted = amount * rate;

    log::info!("✅ Converted: {:.2} {} = {:.2} {} (rate: {:.4})", 
        amount, from, converted, to, rate);

    Ok(ConversionResponse {
        success: true,
        from: from.to_uppercase(),
        to: to.to_uppercase(),
        rate,
        amount: Some(amount),
        converted: Some(converted),
        last_updated: chrono::Utc::now().to_rfc3339(),
    })
}

/// Busca todas as taxas de câmbio baseadas em uma moeda
pub async fn get_all_rates(
    base: &str,
) -> Result<AllRatesResponse, String> {
    log::info!("💱 Fetching all exchange rates for base: {}", base);

    let url = format!("{}/{}", EXCHANGERATE_API_BASE, base.to_uppercase());

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch exchange rates: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Exchange rate API error: {}", response.status()));
    }

    let rates_data: ExchangeRatesResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse exchange rates: {}", e))?;

    log::info!("✅ Retrieved {} exchange rates for {}", rates_data.rates.len(), base);

    Ok(AllRatesResponse {
        success: true,
        base: rates_data.base,
        rates: rates_data.rates,
        last_updated: rates_data.date,
    })
}

/// Converte BRL para USD (uso comum no sistema)
pub async fn brl_to_usd(amount_brl: f64) -> Result<f64, String> {
    let rate = get_exchange_rate("BRL", "USD").await?;
    Ok(amount_brl * rate)
}

/// Converte USD para BRL (uso comum no sistema)
pub async fn usd_to_brl(amount_usd: f64) -> Result<f64, String> {
    let rate = get_exchange_rate("USD", "BRL").await?;
    Ok(amount_usd * rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_usd_to_brl() {
        let rate = get_exchange_rate("USD", "BRL").await;
        assert!(rate.is_ok());
        let rate_value = rate.unwrap();
        assert!(rate_value > 4.0 && rate_value < 7.0); // Range esperado
    }

    #[tokio::test]
    async fn test_convert_currency() {
        let result = convert_currency("USD", "BRL", 100.0).await;
        assert!(result.is_ok());
        let conversion = result.unwrap();
        assert_eq!(conversion.from, "USD");
        assert_eq!(conversion.to, "BRL");
        assert!(conversion.converted.unwrap() > 400.0); // ~R$5/USD
    }

    #[tokio::test]
    async fn test_same_currency() {
        let rate = get_exchange_rate("USD", "USD").await;
        assert!(rate.is_ok());
        assert_eq!(rate.unwrap(), 1.0);
    }
}
