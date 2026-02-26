// ═══════════════════════════════════════════════════════════════════
// STRATEGY SERVICE — Engine de processamento de estratégias (Fase 3+5)
// ═══════════════════════════════════════════════════════════════════
//
// Responsabilidades:
// 1. Buscar preço atual via CCXT (fetch_ticker_sync)
// 2. Avaliar regras (TP, SL, Trailing, DCA, Grid)
// 3. Gerar sinais (StrategySignal)
// 4. Executar ordens reais via CCXT (Fase 5)
// 5. Atualizar posição, PNL e gravar no MongoDB
//
// Fluxo:
//   tick() → fetch_price() → evaluate_rules() → execute_signals() → persist()
//

use crate::{
    ccxt::CCXTClient,
    database::MongoDB,
    models::{
        DecryptedExchange, ExecutionAction, PositionInfo, Strategy,
        StrategyExecution, StrategySignal, StrategyStatus, SignalType,
    },
    services::user_exchanges_service,
    utils::thread_pool::spawn_ccxt_blocking,
};
use mongodb::bson::{doc, oid::ObjectId};

// ═══════════════════════════════════════════════════════════════════
// TICK RESULT — O que o engine produz a cada ciclo
// ═══════════════════════════════════════════════════════════════════

/// Resultado de um ciclo de processamento
#[derive(Debug)]
pub struct TickResult {
    pub strategy_id: String,
    pub symbol: String,
    pub price: f64,
    pub signals: Vec<StrategySignal>,
    pub executions: Vec<StrategyExecution>,
    pub new_status: Option<StrategyStatus>,
    pub error: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════
// FETCH PRICE — Busca preço via CCXT
// ═══════════════════════════════════════════════════════════════════

/// Busca preço atual de um symbol na exchange (via CCXT, blocking)
pub async fn fetch_current_price(
    ccxt_id: &str,
    api_key: &str,
    api_secret: &str,
    passphrase: Option<&str>,
    symbol: &str,
) -> Result<f64, String> {
    let ccxt_id = ccxt_id.to_string();
    let api_key = api_key.to_string();
    let api_secret = api_secret.to_string();
    let passphrase = passphrase.map(|s| s.to_string());
    let symbol = symbol.to_string();

    spawn_ccxt_blocking(move || {
        let client = CCXTClient::new(
            &ccxt_id,
            &api_key,
            &api_secret,
            passphrase.as_deref(),
        )?;

        let ticker = client.fetch_ticker_sync(&symbol)?;

        ticker
            .get("last")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| format!("No 'last' price in ticker for {}", symbol))
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}

// ═══════════════════════════════════════════════════════════════════
// TICK — Ciclo principal do engine para UMA estratégia
// ═══════════════════════════════════════════════════════════════════

/// Processa um ciclo completo para uma estratégia
///
/// 1. Valida que a estratégia está ativa e processável
/// 2. Busca credenciais da exchange do usuário
/// 3. Busca preço atual via CCXT
/// 4. Avalia regras e gera sinais
/// 5. Persiste alterações no MongoDB
pub async fn tick(
    db: &MongoDB,
    strategy: &Strategy,
) -> TickResult {
    let strategy_id = strategy
        .id
        .map(|id| id.to_hex())
        .unwrap_or_default();

    let now = chrono::Utc::now().timestamp();

    // ── 0. Validar que a estratégia é processável ──
    if !strategy.is_active {
        return TickResult {
            strategy_id,
            symbol: strategy.symbol.clone(),
            price: 0.0,
            signals: vec![],
            executions: vec![],
            new_status: None,
            error: Some("Strategy is not active".to_string()),
        };
    }

    match strategy.status {
        StrategyStatus::Paused
        | StrategyStatus::Completed
        | StrategyStatus::Error => {
            return TickResult {
                strategy_id,
                symbol: strategy.symbol.clone(),
                price: 0.0,
                signals: vec![],
                executions: vec![],
                new_status: None,
                error: Some(format!("Strategy status '{}' is not processable", strategy.status)),
            };
        }
        StrategyStatus::Idle => {
            // Auto-promote: idle + is_active → monitoring, then proceed
            log::info!("[tick] Strategy {} is idle but active — auto-promoting to monitoring", strategy_id);
            // Will be persisted as Monitoring after tick completes
        }
        _ => {} // Monitoring, InPosition, BuyPending, SellPending → prosseguir
    }

    // ── 1. Buscar credenciais da exchange ──
    let decrypted = match user_exchanges_service::get_user_exchanges_decrypted(db, &strategy.user_id).await {
        Ok(exchanges) => exchanges,
        Err(e) => {
            return TickResult {
                strategy_id,
                symbol: strategy.symbol.clone(),
                price: 0.0,
                signals: vec![],
                executions: vec![],
                new_status: Some(StrategyStatus::Error),
                error: Some(format!("Failed to decrypt exchanges: {}", e)),
            };
        }
    };

    let exchange = match decrypted
        .iter()
        .find(|ex| ex.exchange_id == strategy.exchange_id)
    {
        Some(ex) => ex,
        None => {
            return TickResult {
                strategy_id,
                symbol: strategy.symbol.clone(),
                price: 0.0,
                signals: vec![],
                executions: vec![],
                new_status: Some(StrategyStatus::Error),
                error: Some(format!(
                    "Exchange {} not found for user {}",
                    strategy.exchange_id, strategy.user_id
                )),
            };
        }
    };

    // ── 2. Buscar preço atual ──
    let price = match fetch_current_price(
        &exchange.ccxt_id,
        &exchange.api_key,
        &exchange.api_secret,
        exchange.passphrase.as_deref(),
        &strategy.symbol,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            return TickResult {
                strategy_id,
                symbol: strategy.symbol.clone(),
                price: 0.0,
                signals: vec![],
                executions: vec![],
                new_status: None,
                error: Some(format!("Failed to fetch price for {}: {}", strategy.symbol, e)),
            };
        }
    };

    log::debug!(
        "📊 [Strategy {}] {} @ {} price = {:.8}",
        &strategy_id[..8.min(strategy_id.len())],
        strategy.symbol,
        exchange.name,
        price
    );

    // ── 3. Avaliar regras e gerar sinais ──
    let mut signals: Vec<StrategySignal> = Vec::new();
    let mut executions: Vec<StrategyExecution> = Vec::new();
    let mut new_status: Option<StrategyStatus> = None;

    match strategy.status {
        StrategyStatus::Monitoring | StrategyStatus::Idle => {
            // Idle + is_active auto-promotes to Monitoring behavior
            if strategy.status == StrategyStatus::Idle {
                new_status = Some(StrategyStatus::Monitoring);
            }
            // Avaliar regra de entrada (Buy signal)
            evaluate_entry_rules(strategy, price, now, &mut signals);
        }
        StrategyStatus::InPosition => {
            // Avaliar TP, SL, Trailing, DCA
            evaluate_exit_rules(strategy, price, now, &mut signals);
            evaluate_dca_rules(strategy, price, now, &mut signals);
        }
        StrategyStatus::BuyPending | StrategyStatus::SellPending => {
            signals.push(StrategySignal {
                signal_type: SignalType::Info,
                price,
                message: format!("Pending order check — status: {}", strategy.status),
                acted: false,
                price_change_percent: 0.0,
                created_at: now,
            });
        }
        _ => {}
    }

    // ── 4. Executar sinais via CCXT (Fase 5) ──
    for signal in &mut signals {
        match signal.signal_type {
            SignalType::Buy | SignalType::DcaBuy => {
                // Calcular amount a comprar
                let investment = calculate_buy_amount(strategy, &signal.signal_type);
                if investment <= 0.0 {
                    log::warn!(
                        "⚠️  [Strategy {}] Skipping {} — investment amount is 0",
                        &strategy_id[..8.min(strategy_id.len())],
                        signal.signal_type
                    );
                    continue;
                }

                let amount = investment / price;
                log::info!(
                    "🟢 [Strategy {}] EXECUTING {} ORDER: {} {:.8} @ {:.8} (${:.2})",
                    &strategy_id[..8.min(strategy_id.len())],
                    signal.signal_type,
                    strategy.symbol,
                    amount,
                    price,
                    investment
                );

                match execute_order(
                    exchange,
                    &strategy.symbol,
                    "market",
                    "buy",
                    amount,
                    None, // market order — sem price
                )
                .await
                {
                    Ok(order_result) => {
                        signal.acted = true;

                        let action = match signal.signal_type {
                            SignalType::DcaBuy => ExecutionAction::DcaBuy,
                            _ => ExecutionAction::Buy,
                        };

                        let reason = match signal.signal_type {
                            SignalType::DcaBuy => format!(
                                "dca_buy_{}",
                                strategy.config.dca.as_ref().map(|d| d.buys_done + 1).unwrap_or(1)
                            ),
                            _ => "entry_buy".to_string(),
                        };

                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action,
                            reason,
                            price: order_result.avg_price.unwrap_or(price),
                            amount: order_result.filled.unwrap_or(amount),
                            total: order_result.cost.unwrap_or(investment),
                            fee: order_result.fee.unwrap_or(0.0),
                            pnl_usd: 0.0,
                            exchange_order_id: Some(order_result.order_id.clone()),
                            executed_at: now,
                            error_message: None,
                        });

                        // Transição de status
                        if signal.signal_type == SignalType::DcaBuy {
                            // DCA: permanece InPosition
                        } else {
                            new_status = Some(StrategyStatus::InPosition);
                        }

                        log::info!(
                            "✅ [Strategy {}] BUY FILLED: order_id={}, filled={:.8}, cost={:.2}",
                            &strategy_id[..8.min(strategy_id.len())],
                            order_result.order_id,
                            order_result.filled.unwrap_or(amount),
                            order_result.cost.unwrap_or(investment)
                        );
                    }
                    Err(e) => {
                        signal.acted = false;

                        let action = match signal.signal_type {
                            SignalType::DcaBuy => ExecutionAction::BuyFailed,
                            _ => ExecutionAction::BuyFailed,
                        };

                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action,
                            reason: format!("buy_failed: {}", e),
                            price,
                            amount,
                            total: investment,
                            fee: 0.0,
                            pnl_usd: 0.0,
                            exchange_order_id: None,
                            executed_at: now,
                            error_message: Some(e.clone()),
                        });

                        log::error!(
                            "❌ [Strategy {}] BUY FAILED: {} — {}",
                            &strategy_id[..8.min(strategy_id.len())],
                            strategy.symbol,
                            e
                        );
                    }
                }
            }

            SignalType::TakeProfit | SignalType::StopLoss | SignalType::TrailingStop => {
                // Calcular amount a vender
                let (sell_amount, sell_percent) =
                    calculate_sell_amount(strategy, price, &signal.signal_type);
                if sell_amount <= 0.0 {
                    log::warn!(
                        "⚠️  [Strategy {}] Skipping {} — no position to sell",
                        &strategy_id[..8.min(strategy_id.len())],
                        signal.signal_type
                    );
                    continue;
                }

                log::info!(
                    "� [Strategy {}] EXECUTING {} ORDER: SELL {} {:.8} @ {:.8} ({:.0}% of position)",
                    &strategy_id[..8.min(strategy_id.len())],
                    signal.signal_type,
                    strategy.symbol,
                    sell_amount,
                    price,
                    sell_percent
                );

                match execute_order(
                    exchange,
                    &strategy.symbol,
                    "market",
                    "sell",
                    sell_amount,
                    None,
                )
                .await
                {
                    Ok(order_result) => {
                        signal.acted = true;

                        // Calcular PNL
                        let entry_price = strategy
                            .position
                            .as_ref()
                            .map(|p| p.entry_price)
                            .unwrap_or(0.0);
                        let filled = order_result.filled.unwrap_or(sell_amount);
                        let sell_price = order_result.avg_price.unwrap_or(price);
                        let pnl = (sell_price - entry_price) * filled;

                        let reason = match signal.signal_type {
                            SignalType::TakeProfit => "take_profit".to_string(),
                            SignalType::StopLoss => "stop_loss".to_string(),
                            SignalType::TrailingStop => "trailing_stop".to_string(),
                            _ => "sell".to_string(),
                        };

                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action: ExecutionAction::Sell,
                            reason,
                            price: sell_price,
                            amount: filled,
                            total: order_result.cost.unwrap_or(sell_price * filled),
                            fee: order_result.fee.unwrap_or(0.0),
                            pnl_usd: pnl,
                            exchange_order_id: Some(order_result.order_id.clone()),
                            executed_at: now,
                            error_message: None,
                        });

                        // Status: se vendeu 100%, completa; senão permanece InPosition
                        if sell_percent >= 99.9 {
                            new_status = Some(StrategyStatus::Completed);
                        }
                        // SL / Trailing sempre fecha 100%
                        if matches!(
                            signal.signal_type,
                            SignalType::StopLoss | SignalType::TrailingStop
                        ) {
                            new_status = Some(StrategyStatus::Completed);
                        }

                        log::info!(
                            "✅ [Strategy {}] SELL FILLED: order_id={}, filled={:.8}, pnl=${:.2}",
                            &strategy_id[..8.min(strategy_id.len())],
                            order_result.order_id,
                            filled,
                            pnl
                        );
                    }
                    Err(e) => {
                        signal.acted = false;

                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action: ExecutionAction::SellFailed,
                            reason: format!("sell_failed: {}", e),
                            price,
                            amount: sell_amount,
                            total: sell_amount * price,
                            fee: 0.0,
                            pnl_usd: 0.0,
                            exchange_order_id: None,
                            executed_at: now,
                            error_message: Some(e.clone()),
                        });

                        log::error!(
                            "❌ [Strategy {}] SELL FAILED: {} — {}",
                            &strategy_id[..8.min(strategy_id.len())],
                            strategy.symbol,
                            e
                        );
                    }
                }
            }

            SignalType::GridTrade => {
                // Grid: determinar se é buy ou sell pelo contexto da mensagem
                let is_sell = signal.message.to_lowercase().contains("sell");
                let side = if is_sell { "sell" } else { "buy" };

                let (grid_amount, grid_investment) = if is_sell {
                    let (amt, _pct) =
                        calculate_sell_amount(strategy, price, &SignalType::GridTrade);
                    (amt, amt * price)
                } else {
                    let inv = calculate_buy_amount(strategy, &SignalType::GridTrade);
                    (inv / price, inv)
                };

                if grid_amount <= 0.0 {
                    continue;
                }

                log::info!(
                    "� [Strategy {}] GRID {} ORDER: {} {:.8} @ {:.8}",
                    &strategy_id[..8.min(strategy_id.len())],
                    side.to_uppercase(),
                    strategy.symbol,
                    grid_amount,
                    price
                );

                match execute_order(exchange, &strategy.symbol, "market", side, grid_amount, None)
                    .await
                {
                    Ok(order_result) => {
                        signal.acted = true;

                        let (action, pnl) = if is_sell {
                            let entry = strategy
                                .position
                                .as_ref()
                                .map(|p| p.entry_price)
                                .unwrap_or(0.0);
                            let filled = order_result.filled.unwrap_or(grid_amount);
                            let sell_px = order_result.avg_price.unwrap_or(price);
                            (ExecutionAction::GridSell, (sell_px - entry) * filled)
                        } else {
                            (ExecutionAction::GridBuy, 0.0)
                        };

                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action,
                            reason: format!("grid_{}", side),
                            price: order_result.avg_price.unwrap_or(price),
                            amount: order_result.filled.unwrap_or(grid_amount),
                            total: order_result.cost.unwrap_or(grid_investment),
                            fee: order_result.fee.unwrap_or(0.0),
                            pnl_usd: pnl,
                            exchange_order_id: Some(order_result.order_id.clone()),
                            executed_at: now,
                            error_message: None,
                        });

                        // Grid buy sem posição → InPosition
                        if !is_sell && strategy.position.is_none() {
                            new_status = Some(StrategyStatus::InPosition);
                        }
                    }
                    Err(e) => {
                        signal.acted = false;
                        let action = if is_sell {
                            ExecutionAction::SellFailed
                        } else {
                            ExecutionAction::BuyFailed
                        };
                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action,
                            reason: format!("grid_{}_failed: {}", side, e),
                            price,
                            amount: grid_amount,
                            total: grid_investment,
                            fee: 0.0,
                            pnl_usd: 0.0,
                            exchange_order_id: None,
                            executed_at: now,
                            error_message: Some(e.clone()),
                        });

                        log::error!(
                            "❌ [Strategy {}] GRID {} FAILED: {}",
                            &strategy_id[..8.min(strategy_id.len())],
                            side.to_uppercase(),
                            e
                        );
                    }
                }
            }

            _ => {
                // Info, PriceAlert — sem execução
            }
        }
    }

    TickResult {
        strategy_id,
        symbol: strategy.symbol.clone(),
        price,
        signals,
        executions,
        new_status,
        error: None,
    }
}

// ═══════════════════════════════════════════════════════════════════
// ORDER EXECUTION — Executa ordens reais via CCXT (Fase 5)
// ═══════════════════════════════════════════════════════════════════

/// Resultado parsed de uma ordem executada na exchange
#[derive(Debug, Clone)]
pub struct OrderResult {
    pub order_id: String,
    pub status: String,
    pub filled: Option<f64>,
    pub avg_price: Option<f64>,
    pub cost: Option<f64>,
    pub fee: Option<f64>,
}

/// Executa uma ordem na exchange via CCXT (blocking → async via spawn_ccxt_blocking)
async fn execute_order(
    exchange: &DecryptedExchange,
    symbol: &str,
    order_type: &str,
    side: &str,
    amount: f64,
    price: Option<f64>,
) -> Result<OrderResult, String> {
    let ccxt_id = exchange.ccxt_id.clone();
    let api_key = exchange.api_key.clone();
    let api_secret = exchange.api_secret.clone();
    let passphrase = exchange.passphrase.clone();
    let symbol = symbol.to_string();
    let order_type = order_type.to_string();
    let side = side.to_string();

    spawn_ccxt_blocking(move || {
        let client = CCXTClient::new(
            &ccxt_id,
            &api_key,
            &api_secret,
            passphrase.as_deref(),
        )?;

        let order_obj = client.create_order_sync(&symbol, &order_type, &side, amount, price)?;

        // Parse PyObject response
        use pyo3::prelude::*;
        Python::with_gil(|py| {
            let order_ref = order_obj.as_ref(py);

            let extract_string = |key: &str| -> String {
                order_ref
                    .get_item(key)
                    .ok()
                    .and_then(|v| if v.is_none() { None } else { v.extract().ok() })
                    .unwrap_or_default()
            };

            let extract_f64 = |key: &str| -> Option<f64> {
                order_ref
                    .get_item(key)
                    .ok()
                    .and_then(|v| if v.is_none() { None } else { v.extract().ok() })
            };

            let fee_cost: Option<f64> = order_ref
                .get_item("fee")
                .ok()
                .and_then(|fee| {
                    if fee.is_none() {
                        return None;
                    }
                    fee.get_item("cost").ok()?.extract().ok()
                });

            // average → price médio de execução
            let avg_price = extract_f64("average").or_else(|| extract_f64("price"));

            Ok(OrderResult {
                order_id: extract_string("id"),
                status: extract_string("status"),
                filled: extract_f64("filled"),
                avg_price,
                cost: extract_f64("cost"),
                fee: fee_cost,
            })
        })
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}

/// Calcula o valor em USDT a investir numa compra
fn calculate_buy_amount(strategy: &Strategy, signal_type: &SignalType) -> f64 {
    let config = &strategy.config;

    match signal_type {
        SignalType::DcaBuy => {
            // DCA: usar amount_per_buy do config
            config
                .dca
                .as_ref()
                .and_then(|d| d.amount_per_buy)
                .or(config.min_investment)
                .unwrap_or(0.0)
        }
        SignalType::GridTrade => {
            // Grid: usar min_investment dividido pelo número de levels
            let levels = config
                .grid
                .as_ref()
                .and_then(|g| g.levels)
                .unwrap_or(1) as f64;
            let total = config.min_investment.unwrap_or(0.0);
            total / levels
        }
        _ => {
            // Buy normal: usar min_investment inteiro
            config.min_investment.unwrap_or(0.0)
        }
    }
}

/// Calcula a quantidade a vender e o percentual da posição
/// Retorna (amount_to_sell, sell_percent)
fn calculate_sell_amount(
    strategy: &Strategy,
    _price: f64,
    signal_type: &SignalType,
) -> (f64, f64) {
    let position = match &strategy.position {
        Some(pos) if pos.quantity > 0.0 => pos,
        _ => return (0.0, 0.0),
    };

    match signal_type {
        SignalType::TakeProfit => {
            // Procurar o próximo TP não executado e usar sell_percent
            let tp = strategy
                .config
                .take_profit_levels
                .iter()
                .find(|tp| !tp.executed);

            match tp {
                Some(tp_level) => {
                    let pct = tp_level.sell_percent / 100.0;
                    let amount = position.quantity * pct;
                    (amount, tp_level.sell_percent)
                }
                None => {
                    // Todos TPs executados → vender 100% restante
                    (position.quantity, 100.0)
                }
            }
        }
        SignalType::StopLoss | SignalType::TrailingStop => {
            // SL / Trailing: vender 100% da posição
            (position.quantity, 100.0)
        }
        SignalType::GridTrade => {
            // Grid: vender fração por level
            let levels = strategy
                .config
                .grid
                .as_ref()
                .and_then(|g| g.levels)
                .unwrap_or(1) as f64;
            let sell_pct = 100.0 / levels;
            let amount = position.quantity * (sell_pct / 100.0);
            (amount, sell_pct)
        }
        _ => (0.0, 0.0),
    }
}

// ═══════════════════════════════════════════════════════════════════
// EVALUATE ENTRY RULES — Avalia condições de entrada (Buy)
// ═══════════════════════════════════════════════════════════════════

fn evaluate_entry_rules(
    strategy: &Strategy,
    price: f64,
    now: i64,
    signals: &mut Vec<StrategySignal>,
) {
    let config = &strategy.config;

    // Regra simples: se tem min_investment e preço está dentro da faixa, gera sinal de compra
    // Lógica mais sofisticada pode ser implementada por strategy_type
    match strategy.strategy_type.as_str() {
        "buy_and_hold" => {
            // Buy and Hold: compra imediatamente se não tem posição
            if strategy.position.is_none() {
                signals.push(StrategySignal {
                    signal_type: SignalType::Buy,
                    price,
                    message: format!(
                        "Buy and Hold: entrada imediata em {} @ {:.8}",
                        strategy.symbol, price
                    ),
                    acted: false,
                    price_change_percent: 0.0,
                    created_at: now,
                });
            }
        }
        "dca" => {
            // DCA: compra no intervalo programado
            if let Some(dca) = &config.dca {
                if dca.enabled {
                    let should_buy = match (dca.max_buys, dca.buys_done) {
                        (Some(max), done) if done >= max => false,
                        _ => true,
                    };

                    if should_buy {
                        // Verificar se já passou o intervalo desde última compra
                        let last_buy_time = strategy
                            .executions
                            .iter()
                            .filter(|e| {
                                matches!(
                                    e.action,
                                    ExecutionAction::Buy | ExecutionAction::DcaBuy
                                )
                            })
                            .last()
                            .map(|e| e.executed_at)
                            .unwrap_or(0);

                        let interval = dca.interval_seconds.unwrap_or(86400); // default: 1 dia
                        if now - last_buy_time >= interval {
                            signals.push(StrategySignal {
                                signal_type: SignalType::DcaBuy,
                                price,
                                message: format!(
                                    "DCA Buy #{}: {} @ {:.8} (interval: {}s)",
                                    dca.buys_done + 1,
                                    strategy.symbol,
                                    price,
                                    interval
                                ),
                                acted: false,
                                price_change_percent: 0.0,
                                created_at: now,
                            });
                        }
                    }
                }
            }
        }
        "swing_trade" | "day_trade" | "scalping" => {
            // Para swing/day/scalping: gerar sinal Info com dados atuais
            // Lógica de indicadores técnicos será adicionada na Fase 4/5
            let change = if let Some(last) = strategy.last_price {
                if last > 0.0 {
                    ((price - last) / last) * 100.0
                } else {
                    0.0
                }
            } else {
                0.0
            };

            signals.push(StrategySignal {
                signal_type: SignalType::Info,
                price,
                message: format!(
                    "{} monitoring: {} @ {:.8} (Δ {:.2}% since last check)",
                    strategy.strategy_type, strategy.symbol, price, change
                ),
                acted: false,
                price_change_percent: change,
                created_at: now,
            });
        }
        "grid" => {
            // Grid: verificar se preço cruzou algum nível
            if let Some(grid) = &config.grid {
                if grid.enabled {
                    if let (Some(center), Some(spacing), Some(levels)) =
                        (grid.center_price, grid.spacing_percent, grid.levels)
                    {
                        for i in 1..=levels {
                            let buy_level = center * (1.0 - (spacing / 100.0) * i as f64);
                            let sell_level = center * (1.0 + (spacing / 100.0) * i as f64);

                            if price <= buy_level {
                                signals.push(StrategySignal {
                                    signal_type: SignalType::GridTrade,
                                    price,
                                    message: format!(
                                        "Grid Buy Level {}: {} @ {:.8} (level: {:.8})",
                                        i, strategy.symbol, price, buy_level
                                    ),
                                    acted: false,
                                    price_change_percent: ((price - center) / center) * 100.0,
                                    created_at: now,
                                });
                                break; // Apenas um sinal por tick
                            }
                            if price >= sell_level && strategy.position.is_some() {
                                signals.push(StrategySignal {
                                    signal_type: SignalType::GridTrade,
                                    price,
                                    message: format!(
                                        "Grid Sell Level {}: {} @ {:.8} (level: {:.8})",
                                        i, strategy.symbol, price, sell_level
                                    ),
                                    acted: false,
                                    price_change_percent: ((price - center) / center) * 100.0,
                                    created_at: now,
                                });
                                break;
                            }
                        }
                    }
                }
            }
        }
        _ => {
            // Tipo desconhecido: apenas info
            signals.push(StrategySignal {
                signal_type: SignalType::Info,
                price,
                message: format!(
                    "Unknown strategy type '{}': monitoring {} @ {:.8}",
                    strategy.strategy_type, strategy.symbol, price
                ),
                acted: false,
                price_change_percent: 0.0,
                created_at: now,
            });
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// EVALUATE EXIT RULES — Avalia condições de saída (TP/SL/Trailing)
// ═══════════════════════════════════════════════════════════════════

fn evaluate_exit_rules(
    strategy: &Strategy,
    price: f64,
    now: i64,
    signals: &mut Vec<StrategySignal>,
) {
    let config = &strategy.config;

    let position = match &strategy.position {
        Some(pos) => pos,
        None => return, // Sem posição, nada a avaliar
    };

    let entry_price = position.entry_price;
    if entry_price <= 0.0 {
        return;
    }

    let price_change_pct = ((price - entry_price) / entry_price) * 100.0;
    let highest = position.highest_price.max(price);

    // ── Take Profit Levels ──
    for (i, tp) in config.take_profit_levels.iter().enumerate() {
        if !tp.executed && price_change_pct >= tp.percent {
            signals.push(StrategySignal {
                signal_type: SignalType::TakeProfit,
                price,
                message: format!(
                    "TP{} atingido: {:.2}% >= {:.2}% (sell {:.0}% da posição) — entry: {:.8}, current: {:.8}",
                    i + 1,
                    price_change_pct,
                    tp.percent,
                    tp.sell_percent,
                    entry_price,
                    price
                ),
                acted: false,
                price_change_percent: price_change_pct,
                created_at: now,
            });
            break; // Processa um TP por tick
        }
    }

    // ── Stop Loss ──
    if let Some(sl) = &config.stop_loss {
        if sl.enabled {
            let sl_threshold = -(sl.percent);

            if sl.trailing {
                // Trailing Stop Loss
                let trailing_distance = sl.trailing_distance.unwrap_or(sl.percent);
                let trailing_threshold = highest * (1.0 - trailing_distance / 100.0);

                if price <= trailing_threshold {
                    let drop_from_high = ((price - highest) / highest) * 100.0;
                    signals.push(StrategySignal {
                        signal_type: SignalType::TrailingStop,
                        price,
                        message: format!(
                            "Trailing Stop atingido: preço {:.8} caiu {:.2}% do máximo {:.8} (distância: {:.2}%)",
                            price, drop_from_high, highest, trailing_distance
                        ),
                        acted: false,
                        price_change_percent: price_change_pct,
                        created_at: now,
                    });
                }
            } else {
                // Stop Loss fixo
                if price_change_pct <= sl_threshold {
                    signals.push(StrategySignal {
                        signal_type: SignalType::StopLoss,
                        price,
                        message: format!(
                            "Stop Loss atingido: {:.2}% <= {:.2}% — entry: {:.8}, current: {:.8}",
                            price_change_pct, sl_threshold, entry_price, price
                        ),
                        acted: false,
                        price_change_percent: price_change_pct,
                        created_at: now,
                    });
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// EVALUATE DCA RULES — Avalia condições de DCA (Buy the Dip)
// ═══════════════════════════════════════════════════════════════════

fn evaluate_dca_rules(
    strategy: &Strategy,
    price: f64,
    now: i64,
    signals: &mut Vec<StrategySignal>,
) {
    let config = &strategy.config;

    let position = match &strategy.position {
        Some(pos) => pos,
        None => return,
    };

    if let Some(dca) = &config.dca {
        if !dca.enabled {
            return;
        }

        // Verificar se atingiu max_buys
        if let Some(max) = dca.max_buys {
            if dca.buys_done >= max {
                return;
            }
        }

        // Verificar intervalo
        let last_buy_time = strategy
            .executions
            .iter()
            .filter(|e| {
                matches!(
                    e.action,
                    ExecutionAction::Buy | ExecutionAction::DcaBuy
                )
            })
            .last()
            .map(|e| e.executed_at)
            .unwrap_or(0);

        let interval = dca.interval_seconds.unwrap_or(86400);
        if now - last_buy_time < interval {
            return; // Ainda não é hora
        }

        // Verificar dip_percent (se configurado)
        if let Some(dip_pct) = dca.dip_percent {
            let entry_price = position.entry_price;
            if entry_price > 0.0 {
                let price_change = ((price - entry_price) / entry_price) * 100.0;

                if price_change <= -dip_pct {
                    signals.push(StrategySignal {
                        signal_type: SignalType::DcaBuy,
                        price,
                        message: format!(
                            "DCA Buy (dip {:.2}% >= {:.2}%): {} @ {:.8} — buy #{} of {}",
                            price_change.abs(),
                            dip_pct,
                            strategy.symbol,
                            price,
                            dca.buys_done + 1,
                            dca.max_buys.unwrap_or(999)
                        ),
                        acted: false,
                        price_change_percent: price_change,
                        created_at: now,
                    });
                }
            }
        } else {
            // DCA sem dip: comprar no intervalo
            signals.push(StrategySignal {
                signal_type: SignalType::DcaBuy,
                price,
                message: format!(
                    "DCA Buy (scheduled): {} @ {:.8} — buy #{} of {}",
                    strategy.symbol,
                    price,
                    dca.buys_done + 1,
                    dca.max_buys.unwrap_or(999)
                ),
                acted: false,
                price_change_percent: 0.0,
                created_at: now,
            });
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// PERSIST — Grava resultado do tick no MongoDB
// ═══════════════════════════════════════════════════════════════════

/// Persiste os resultados de um tick no MongoDB
///
/// Fase 5: Atualiza posição (cria/atualiza/remove), contadores DCA, TPs executados,
/// total_pnl_usd, total_executions, além de last_price, signals, executions e status.
pub async fn persist_tick_result(
    db: &MongoDB,
    strategy: &Strategy,
    result: &TickResult,
) -> Result<(), String> {
    let strategy_oid = strategy
        .id
        .ok_or_else(|| "Strategy has no ID".to_string())?;

    let collection = db.collection::<Strategy>("strategies");
    let now = chrono::Utc::now().timestamp();

    // ── Base update: sempre atualizar last_checked_at e updated_at ──
    let mut update_set = doc! {
        "last_checked_at": now,
        "updated_at": now,
    };

    // Atualizar last_price se obtido com sucesso
    if result.price > 0.0 {
        update_set.insert("last_price", result.price);
    }

    // Atualizar status se mudou
    if let Some(ref new_status) = result.new_status {
        update_set.insert(
            "status",
            mongodb::bson::to_bson(new_status).unwrap_or_default(),
        );

        // Sync is_active
        match new_status {
            StrategyStatus::Paused | StrategyStatus::Completed | StrategyStatus::Error => {
                update_set.insert("is_active", false);
            }
            _ => {}
        }
    }

    // Atualizar error_message
    if let Some(ref error) = result.error {
        update_set.insert("error_message", error.as_str());
    } else {
        // Limpar erro se não tem
        update_set.insert("error_message", mongodb::bson::Bson::Null);
    }

    // ── Fase 5: Processar execuções para atualizar posição ──
    let mut current_position = strategy.position.clone();
    let mut accumulated_pnl: f64 = 0.0;
    let mut dca_buys_increment: i32 = 0;
    let mut tp_indices_executed: Vec<usize> = Vec::new();

    for exec in &result.executions {
        match exec.action {
            ExecutionAction::Buy | ExecutionAction::DcaBuy | ExecutionAction::GridBuy => {
                // Atualizar ou criar posição
                if let Some(ref mut pos) = current_position {
                    // Posição existente: calcular preço médio ponderado
                    let old_cost = pos.entry_price * pos.quantity;
                    let new_cost = exec.price * exec.amount;
                    let new_qty = pos.quantity + exec.amount;
                    if new_qty > 0.0 {
                        pos.entry_price = (old_cost + new_cost) / new_qty;
                        pos.quantity = new_qty;
                        pos.total_cost = old_cost + new_cost;
                    }
                    pos.current_price = result.price;
                    if result.price > pos.highest_price {
                        pos.highest_price = result.price;
                    }
                    if result.price < pos.lowest_price || pos.lowest_price == 0.0 {
                        pos.lowest_price = result.price;
                    }
                    // Recalcular PNL não realizado
                    if pos.entry_price > 0.0 {
                        pos.unrealized_pnl = (result.price - pos.entry_price) * pos.quantity;
                        pos.unrealized_pnl_percent =
                            ((result.price - pos.entry_price) / pos.entry_price) * 100.0;
                    }
                } else {
                    // Nova posição
                    current_position = Some(PositionInfo {
                        entry_price: exec.price,
                        quantity: exec.amount,
                        total_cost: exec.total,
                        current_price: result.price,
                        unrealized_pnl: 0.0,
                        unrealized_pnl_percent: 0.0,
                        highest_price: result.price,
                        lowest_price: result.price,
                        opened_at: now,
                    });
                }

                // Contar DCA buys
                if exec.action == ExecutionAction::DcaBuy {
                    dca_buys_increment += 1;
                }
            }

            ExecutionAction::Sell | ExecutionAction::GridSell => {
                // Reduzir posição
                accumulated_pnl += exec.pnl_usd;

                if let Some(ref mut pos) = current_position {
                    pos.quantity -= exec.amount;
                    if pos.quantity <= 0.0001 {
                        // Posição fechada (tolerância de poeira)
                        // Será removida abaixo
                    } else {
                        pos.total_cost = pos.entry_price * pos.quantity;
                        pos.current_price = result.price;
                        if pos.entry_price > 0.0 {
                            pos.unrealized_pnl =
                                (result.price - pos.entry_price) * pos.quantity;
                            pos.unrealized_pnl_percent =
                                ((result.price - pos.entry_price) / pos.entry_price) * 100.0;
                        }
                    }
                }

                // Identificar TP executados
                if exec.reason.starts_with("take_profit") {
                    // Marcar o próximo TP não executado
                    for (i, tp) in strategy.config.take_profit_levels.iter().enumerate() {
                        if !tp.executed && !tp_indices_executed.contains(&i) {
                            tp_indices_executed.push(i);
                            break;
                        }
                    }
                }
            }

            _ => {} // BuyFailed / SellFailed — não altera posição
        }
    }

    // ── Gravar posição atualizada ──
    let position_closed = current_position
        .as_ref()
        .map(|p| p.quantity <= 0.0001)
        .unwrap_or(false);

    if position_closed {
        // Posição fechada: remover
        update_set.insert("position", mongodb::bson::Bson::Null);
    } else if let Some(ref pos) = current_position {
        // Posição existe: atualizar
        if let Ok(pos_bson) = mongodb::bson::to_bson(pos) {
            update_set.insert("position", pos_bson);
        }
    } else if result.price > 0.0 {
        // Sem posição mas com preço — atualizar tracking na posição existente se houver
        if let Some(ref position) = strategy.position {
            if result.price > position.highest_price {
                update_set.insert("position.highest_price", result.price);
            }
            if result.price < position.lowest_price || position.lowest_price == 0.0 {
                update_set.insert("position.lowest_price", result.price);
            }
            update_set.insert("position.current_price", result.price);

            if position.entry_price > 0.0 {
                let unrealized_pnl =
                    (result.price - position.entry_price) * position.quantity;
                let unrealized_pnl_pct =
                    ((result.price - position.entry_price) / position.entry_price) * 100.0;
                update_set.insert("position.unrealized_pnl", unrealized_pnl);
                update_set.insert("position.unrealized_pnl_percent", unrealized_pnl_pct);
            }
        }
    }

    // ── Fase 5: Acumular PNL e contadores ──
    let mut update_inc = doc! {};

    if accumulated_pnl.abs() > 0.0001 {
        update_inc.insert("total_pnl_usd", accumulated_pnl);
    }

    let new_exec_count = result.executions.iter().filter(|e| {
        !matches!(e.action, ExecutionAction::BuyFailed | ExecutionAction::SellFailed)
    }).count() as i32;

    if new_exec_count > 0 {
        update_inc.insert("total_executions", new_exec_count);
    }

    if dca_buys_increment > 0 {
        update_inc.insert("config.dca.buys_done", dca_buys_increment);
    }

    // ── Fase 5: Marcar TPs executados ──
    for idx in &tp_indices_executed {
        update_set.insert(
            format!("config.take_profit_levels.{}.executed", idx),
            true,
        );
        update_set.insert(
            format!("config.take_profit_levels.{}.executed_at", idx),
            now,
        );
    }

    // Construir update com $set, $push e $inc
    let mut update_doc = doc! { "$set": update_set };

    if !update_inc.is_empty() {
        update_doc.insert("$inc", update_inc);
    }

    // Push novos sinais (limitar a 100 últimos no array)
    if !result.signals.is_empty() {
        let signals_bson: Vec<mongodb::bson::Bson> = result
            .signals
            .iter()
            .filter_map(|s| mongodb::bson::to_bson(s).ok())
            .collect();

        if !signals_bson.is_empty() {
            update_doc.insert(
                "$push",
                doc! {
                    "signals": {
                        "$each": signals_bson,
                        "$slice": -100  // Manter últimos 100 sinais
                    }
                },
            );
        }
    }

    // Push novas execuções (sem slice — manter histórico completo)
    if !result.executions.is_empty() {
        let executions_bson: Vec<mongodb::bson::Bson> = result
            .executions
            .iter()
            .filter_map(|e| mongodb::bson::to_bson(e).ok())
            .collect();

        if !executions_bson.is_empty() {
            // Se já tem $push de signals, precisa combinar
            if let Some(push_doc) = update_doc.get_mut("$push") {
                if let Some(push_bson) = push_doc.as_document_mut() {
                    push_bson.insert(
                        "executions",
                        doc! {
                            "$each": executions_bson
                        },
                    );
                }
            } else {
                update_doc.insert(
                    "$push",
                    doc! {
                        "executions": {
                            "$each": executions_bson
                        }
                    },
                );
            }
        }
    }

    collection
        .update_one(
            doc! { "_id": strategy_oid },
            update_doc,
        )
        .await
        .map_err(|e| format!("Failed to persist tick result: {}", e))?;

    log::debug!(
        "💾 [Strategy {}] Tick persisted: price={:.8}, signals={}, execs={}, pnl=${:.2}",
        &result.strategy_id[..8.min(result.strategy_id.len())],
        result.price,
        result.signals.len(),
        result.executions.len(),
        accumulated_pnl
    );

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════
// ACTIVATE / DEACTIVATE — Controle de status via API
// ═══════════════════════════════════════════════════════════════════

/// Ativa uma estratégia (muda status para Monitoring)
pub async fn activate_strategy(
    db: &MongoDB,
    strategy_id: &str,
    user_id: &str,
) -> Result<Strategy, String> {
    let oid = ObjectId::parse_str(strategy_id)
        .map_err(|_| "Invalid strategy ID".to_string())?;

    let collection = db.collection::<Strategy>("strategies");
    let now = chrono::Utc::now().timestamp();

    collection
        .update_one(
            doc! { "_id": oid, "user_id": user_id },
            doc! {
                "$set": {
                    "status": "monitoring",
                    "is_active": true,
                    "error_message": mongodb::bson::Bson::Null,
                    "updated_at": now
                }
            },
        )
        .await
        .map_err(|e| format!("Failed to activate: {}", e))?;

    collection
        .find_one(doc! { "_id": oid })
        .await
        .map_err(|e| format!("Failed to fetch: {}", e))?
        .ok_or_else(|| "Strategy not found".to_string())
}

/// Pausa uma estratégia (mantém posição se tiver)
pub async fn pause_strategy(
    db: &MongoDB,
    strategy_id: &str,
    user_id: &str,
) -> Result<Strategy, String> {
    let oid = ObjectId::parse_str(strategy_id)
        .map_err(|_| "Invalid strategy ID".to_string())?;

    let collection = db.collection::<Strategy>("strategies");
    let now = chrono::Utc::now().timestamp();

    collection
        .update_one(
            doc! { "_id": oid, "user_id": user_id },
            doc! {
                "$set": {
                    "status": "paused",
                    "is_active": false,
                    "updated_at": now
                }
            },
        )
        .await
        .map_err(|e| format!("Failed to pause: {}", e))?;

    collection
        .find_one(doc! { "_id": oid })
        .await
        .map_err(|e| format!("Failed to fetch: {}", e))?
        .ok_or_else(|| "Strategy not found".to_string())
}

// ═══════════════════════════════════════════════════════════════════
// PROCESS ALL — Processa todas as estratégias ativas de um usuário
// ═══════════════════════════════════════════════════════════════════

/// Processa todas as estratégias ativas elegíveis para tick
///
/// Usado pelo strategy_monitor (Fase 4) e pode ser chamado via API para trigger manual
pub async fn process_active_strategies(db: &MongoDB) -> Result<ProcessResult, String> {
    let collection = db.collection::<Strategy>("strategies");
    let now = chrono::Utc::now().timestamp();

    // Buscar estratégias ativas com status processável (idle auto-promove para monitoring)
    let filter = doc! {
        "is_active": true,
        "status": { "$in": ["idle", "monitoring", "in_position", "buy_pending", "sell_pending"] }
    };

    let mut cursor = collection
        .find(filter)
        .await
        .map_err(|e| format!("Failed to query strategies: {}", e))?;

    use futures::stream::StreamExt;

    let mut total = 0;
    let mut processed = 0;
    let mut errors = 0;
    let mut signals_generated = 0;
    let mut orders_executed = 0;

    while let Some(result) = cursor.next().await {
        match result {
            Ok(strategy) => {
                total += 1;

                // Verificar intervalo de checagem
                let last_checked = strategy.last_checked_at.unwrap_or(0);
                let interval = strategy.check_interval_secs;

                if now - last_checked < interval {
                    log::debug!(
                        "⏭️  [Strategy {}] Skipping — last checked {}s ago (interval: {}s)",
                        strategy.id.map(|id| id.to_hex()).unwrap_or_default(),
                        now - last_checked,
                        interval
                    );
                    continue;
                }

                // Processar tick
                let tick_result = tick(db, &strategy).await;

                signals_generated += tick_result.signals.len();
                orders_executed += tick_result.executions.len();

                // Persistir resultado
                match persist_tick_result(db, &strategy, &tick_result).await {
                    Ok(_) => {
                        processed += 1;
                    }
                    Err(e) => {
                        log::error!(
                            "❌ [Strategy {}] Failed to persist: {}",
                            tick_result.strategy_id,
                            e
                        );
                        errors += 1;
                    }
                }

                // Delay pequeno entre estratégias para não sobrecarregar
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            Err(e) => {
                log::error!("❌ Error reading strategy: {}", e);
                errors += 1;
            }
        }
    }

    let result = ProcessResult {
        total,
        processed,
        errors,
        signals_generated,
        orders_executed,
    };

    log::info!(
        "📊 Strategy processing complete: {} total, {} processed, {} errors, {} signals, {} orders",
        result.total,
        result.processed,
        result.errors,
        result.signals_generated,
        result.orders_executed
    );

    Ok(result)
}

/// Resultado do processamento batch
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProcessResult {
    pub total: usize,
    pub processed: usize,
    pub errors: usize,
    pub signals_generated: usize,
    pub orders_executed: usize,
}
