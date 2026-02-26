// ═══════════════════════════════════════════════════════════════════
// STRATEGY SERVICE — Engine de processamento de estratégias
// ═══════════════════════════════════════════════════════════════════
//
// Fluxo: tick() -> fetch_price() -> evaluate_rules() -> execute_signals() -> persist()
// Agora opera sobre UserStrategies (1 doc per user, array de StrategyItem)
// Collection: "user_strategy"
//

use crate::{
    ccxt::CCXTClient,
    database::MongoDB,
    models::{
        DecryptedExchange, ExecutionAction, PositionInfo, StrategyItem,
        StrategyExecution, StrategySignal, StrategyStatus, SignalType,
        UserStrategies,
    },
    services::user_exchanges_service,
    utils::thread_pool::spawn_ccxt_blocking,
};
use mongodb::bson::doc;

const COLLECTION: &str = "user_strategy";

// ═══════════════════════════════════════════════════════════════════
// TICK RESULT
// ═══════════════════════════════════════════════════════════════════

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
// FETCH PRICE
// ═══════════════════════════════════════════════════════════════════

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
        let client = CCXTClient::new(&ccxt_id, &api_key, &api_secret, passphrase.as_deref())?;
        let ticker = client.fetch_ticker_sync(&symbol)?;
        ticker.get("last").and_then(|v| v.as_f64())
            .ok_or_else(|| format!("No 'last' price in ticker for {}", symbol))
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}

// ═══════════════════════════════════════════════════════════════════
// TICK — Ciclo principal para UMA estrategia
// ═══════════════════════════════════════════════════════════════════

pub async fn tick(
    db: &MongoDB,
    user_id: &str,
    strategy: &StrategyItem,
) -> TickResult {
    let strategy_id = strategy.strategy_id.clone();
    let now = chrono::Utc::now().timestamp();

    // 0. Validar processavel
    if !strategy.is_active {
        return TickResult {
            strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
            signals: vec![], executions: vec![], new_status: None,
            error: Some("Strategy is not active".to_string()),
        };
    }

    match strategy.status {
        StrategyStatus::Paused | StrategyStatus::Completed | StrategyStatus::Error => {
            return TickResult {
                strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
                signals: vec![], executions: vec![], new_status: None,
                error: Some(format!("Strategy status '{}' is not processable", strategy.status)),
            };
        }
        StrategyStatus::Idle => {
            log::info!("[tick] Strategy {} is idle but active — auto-promoting to monitoring", strategy_id);
        }
        _ => {}
    }

    // 1. Buscar credenciais
    let decrypted = match user_exchanges_service::get_user_exchanges_decrypted(db, user_id).await {
        Ok(exchanges) => exchanges,
        Err(e) => {
            return TickResult {
                strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
                signals: vec![], executions: vec![],
                new_status: Some(StrategyStatus::Error),
                error: Some(format!("Failed to decrypt exchanges: {}", e)),
            };
        }
    };

    let exchange = match decrypted.iter().find(|ex| ex.exchange_id == strategy.exchange_id) {
        Some(ex) => ex,
        None => {
            return TickResult {
                strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
                signals: vec![], executions: vec![],
                new_status: Some(StrategyStatus::Error),
                error: Some(format!("Exchange {} not found for user {}", strategy.exchange_id, user_id)),
            };
        }
    };

    // 2. Buscar preco
    let price = match fetch_current_price(
        &exchange.ccxt_id, &exchange.api_key, &exchange.api_secret,
        exchange.passphrase.as_deref(), &strategy.symbol,
    ).await {
        Ok(p) => p,
        Err(e) => {
            return TickResult {
                strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
                signals: vec![], executions: vec![], new_status: None,
                error: Some(format!("Failed to fetch price for {}: {}", strategy.symbol, e)),
            };
        }
    };

    log::debug!("[Strategy {}] {} @ {} price = {:.8}",
        &strategy_id[..8.min(strategy_id.len())], strategy.symbol, exchange.name, price);

    // 3. Avaliar regras
    let mut signals: Vec<StrategySignal> = Vec::new();
    let mut executions: Vec<StrategyExecution> = Vec::new();
    let mut new_status: Option<StrategyStatus> = None;

    match strategy.status {
        StrategyStatus::Monitoring | StrategyStatus::Idle => {
            if strategy.status == StrategyStatus::Idle {
                new_status = Some(StrategyStatus::Monitoring);
            }
            evaluate_entry_rules(strategy, price, now, &mut signals);
        }
        StrategyStatus::InPosition => {
            evaluate_exit_rules(strategy, price, now, &mut signals);
            evaluate_dca_rules(strategy, price, now, &mut signals);
        }
        StrategyStatus::BuyPending | StrategyStatus::SellPending => {
            signals.push(StrategySignal {
                signal_type: SignalType::Info, price,
                message: format!("Pending order check — status: {}", strategy.status),
                acted: false, price_change_percent: 0.0, created_at: now,
            });
        }
        _ => {}
    }

    // 4. Executar sinais via CCXT
    for signal in &mut signals {
        match signal.signal_type {
            SignalType::Buy | SignalType::DcaBuy => {
                let investment = calculate_buy_amount(strategy, &signal.signal_type);
                if investment <= 0.0 { continue; }
                let amount = investment / price;

                log::info!("[Strategy {}] EXECUTING {} ORDER: {} {:.8} @ {:.8} (${:.2})",
                    &strategy_id[..8.min(strategy_id.len())], signal.signal_type,
                    strategy.symbol, amount, price, investment);

                match execute_order(exchange, &strategy.symbol, "market", "buy", amount, None).await {
                    Ok(order_result) => {
                        signal.acted = true;
                        let action = if signal.signal_type == SignalType::DcaBuy { ExecutionAction::DcaBuy } else { ExecutionAction::Buy };
                        let reason = if signal.signal_type == SignalType::DcaBuy {
                            format!("dca_buy_{}", strategy.config.dca.as_ref().map(|d| d.buys_done + 1).unwrap_or(1))
                        } else { "entry_buy".to_string() };

                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action, reason,
                            price: order_result.avg_price.unwrap_or(price),
                            amount: order_result.filled.unwrap_or(amount),
                            total: order_result.cost.unwrap_or(investment),
                            fee: order_result.fee.unwrap_or(0.0),
                            pnl_usd: 0.0,
                            exchange_order_id: Some(order_result.order_id.clone()),
                            executed_at: now, error_message: None,
                        });

                        if signal.signal_type != SignalType::DcaBuy {
                            new_status = Some(StrategyStatus::InPosition);
                        }
                    }
                    Err(e) => {
                        signal.acted = false;
                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action: ExecutionAction::BuyFailed,
                            reason: format!("buy_failed: {}", e),
                            price, amount, total: investment, fee: 0.0, pnl_usd: 0.0,
                            exchange_order_id: None, executed_at: now,
                            error_message: Some(e.clone()),
                        });
                        log::error!("[Strategy {}] BUY FAILED: {}", &strategy_id[..8.min(strategy_id.len())], e);
                    }
                }
            }

            SignalType::TakeProfit | SignalType::StopLoss | SignalType::TrailingStop => {
                let (sell_amount, sell_percent) = calculate_sell_amount(strategy, price, &signal.signal_type);
                if sell_amount <= 0.0 { continue; }

                match execute_order(exchange, &strategy.symbol, "market", "sell", sell_amount, None).await {
                    Ok(order_result) => {
                        signal.acted = true;
                        let entry_price = strategy.position.as_ref().map(|p| p.entry_price).unwrap_or(0.0);
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
                            action: ExecutionAction::Sell, reason,
                            price: sell_price, amount: filled,
                            total: order_result.cost.unwrap_or(sell_price * filled),
                            fee: order_result.fee.unwrap_or(0.0), pnl_usd: pnl,
                            exchange_order_id: Some(order_result.order_id.clone()),
                            executed_at: now, error_message: None,
                        });
                        if sell_percent >= 99.9 || matches!(signal.signal_type, SignalType::StopLoss | SignalType::TrailingStop) {
                            new_status = Some(StrategyStatus::Completed);
                        }
                    }
                    Err(e) => {
                        signal.acted = false;
                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action: ExecutionAction::SellFailed,
                            reason: format!("sell_failed: {}", e),
                            price, amount: sell_amount, total: sell_amount * price,
                            fee: 0.0, pnl_usd: 0.0, exchange_order_id: None,
                            executed_at: now, error_message: Some(e.clone()),
                        });
                        log::error!("[Strategy {}] SELL FAILED: {}", &strategy_id[..8.min(strategy_id.len())], e);
                    }
                }
            }

            SignalType::GridTrade => {
                let is_sell = signal.message.to_lowercase().contains("sell");
                let side = if is_sell { "sell" } else { "buy" };
                let (grid_amount, grid_investment) = if is_sell {
                    let (amt, _pct) = calculate_sell_amount(strategy, price, &SignalType::GridTrade);
                    (amt, amt * price)
                } else {
                    let inv = calculate_buy_amount(strategy, &SignalType::GridTrade);
                    (inv / price, inv)
                };
                if grid_amount <= 0.0 { continue; }

                match execute_order(exchange, &strategy.symbol, "market", side, grid_amount, None).await {
                    Ok(order_result) => {
                        signal.acted = true;
                        let (action, pnl) = if is_sell {
                            let entry = strategy.position.as_ref().map(|p| p.entry_price).unwrap_or(0.0);
                            let filled = order_result.filled.unwrap_or(grid_amount);
                            let sell_px = order_result.avg_price.unwrap_or(price);
                            (ExecutionAction::GridSell, (sell_px - entry) * filled)
                        } else {
                            (ExecutionAction::GridBuy, 0.0)
                        };
                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action, reason: format!("grid_{}", side),
                            price: order_result.avg_price.unwrap_or(price),
                            amount: order_result.filled.unwrap_or(grid_amount),
                            total: order_result.cost.unwrap_or(grid_investment),
                            fee: order_result.fee.unwrap_or(0.0), pnl_usd: pnl,
                            exchange_order_id: Some(order_result.order_id.clone()),
                            executed_at: now, error_message: None,
                        });
                        if !is_sell && strategy.position.is_none() {
                            new_status = Some(StrategyStatus::InPosition);
                        }
                    }
                    Err(e) => {
                        signal.acted = false;
                        let action = if is_sell { ExecutionAction::SellFailed } else { ExecutionAction::BuyFailed };
                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action, reason: format!("grid_{}_failed: {}", side, e),
                            price, amount: grid_amount, total: grid_investment,
                            fee: 0.0, pnl_usd: 0.0, exchange_order_id: None,
                            executed_at: now, error_message: Some(e.clone()),
                        });
                    }
                }
            }

            _ => {} // Info, PriceAlert
        }
    }

    TickResult {
        strategy_id, symbol: strategy.symbol.clone(), price,
        signals, executions, new_status, error: None,
    }
}

// ═══════════════════════════════════════════════════════════════════
// ORDER EXECUTION via CCXT
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct OrderResult {
    pub order_id: String,
    pub status: String,
    pub filled: Option<f64>,
    pub avg_price: Option<f64>,
    pub cost: Option<f64>,
    pub fee: Option<f64>,
}

async fn execute_order(
    exchange: &DecryptedExchange,
    symbol: &str, order_type: &str, side: &str,
    amount: f64, price: Option<f64>,
) -> Result<OrderResult, String> {
    let ccxt_id = exchange.ccxt_id.clone();
    let api_key = exchange.api_key.clone();
    let api_secret = exchange.api_secret.clone();
    let passphrase = exchange.passphrase.clone();
    let symbol = symbol.to_string();
    let order_type = order_type.to_string();
    let side = side.to_string();

    spawn_ccxt_blocking(move || {
        let client = CCXTClient::new(&ccxt_id, &api_key, &api_secret, passphrase.as_deref())?;
        let order_obj = client.create_order_sync(&symbol, &order_type, &side, amount, price)?;

        use pyo3::prelude::*;
        Python::with_gil(|py| {
            let order_ref = order_obj.as_ref(py);
            let extract_string = |key: &str| -> String {
                order_ref.get_item(key).ok()
                    .and_then(|v| if v.is_none() { None } else { v.extract().ok() })
                    .unwrap_or_default()
            };
            let extract_f64 = |key: &str| -> Option<f64> {
                order_ref.get_item(key).ok()
                    .and_then(|v| if v.is_none() { None } else { v.extract().ok() })
            };
            let fee_cost: Option<f64> = order_ref.get_item("fee").ok()
                .and_then(|fee| {
                    if fee.is_none() { return None; }
                    fee.get_item("cost").ok()?.extract().ok()
                });
            let avg_price = extract_f64("average").or_else(|| extract_f64("price"));
            Ok(OrderResult {
                order_id: extract_string("id"),
                status: extract_string("status"),
                filled: extract_f64("filled"),
                avg_price, cost: extract_f64("cost"), fee: fee_cost,
            })
        })
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}

fn calculate_buy_amount(strategy: &StrategyItem, signal_type: &SignalType) -> f64 {
    let config = &strategy.config;
    match signal_type {
        SignalType::DcaBuy => {
            config.dca.as_ref().and_then(|d| d.amount_per_buy)
                .or(config.min_investment).unwrap_or(0.0)
        }
        SignalType::GridTrade => {
            let levels = config.grid.as_ref().and_then(|g| g.levels).unwrap_or(1) as f64;
            let total = config.min_investment.unwrap_or(0.0);
            total / levels
        }
        _ => config.min_investment.unwrap_or(0.0),
    }
}

fn calculate_sell_amount(strategy: &StrategyItem, _price: f64, signal_type: &SignalType) -> (f64, f64) {
    let position = match &strategy.position {
        Some(pos) if pos.quantity > 0.0 => pos,
        _ => return (0.0, 0.0),
    };
    match signal_type {
        SignalType::TakeProfit => {
            let tp = strategy.config.take_profit_levels.iter().find(|tp| !tp.executed);
            match tp {
                Some(tp_level) => {
                    let pct = tp_level.sell_percent / 100.0;
                    (position.quantity * pct, tp_level.sell_percent)
                }
                None => (position.quantity, 100.0),
            }
        }
        SignalType::StopLoss | SignalType::TrailingStop => (position.quantity, 100.0),
        SignalType::GridTrade => {
            let levels = strategy.config.grid.as_ref().and_then(|g| g.levels).unwrap_or(1) as f64;
            let sell_pct = 100.0 / levels;
            (position.quantity * (sell_pct / 100.0), sell_pct)
        }
        _ => (0.0, 0.0),
    }
}

// ═══════════════════════════════════════════════════════════════════
// EVALUATE ENTRY RULES
// ═══════════════════════════════════════════════════════════════════

fn evaluate_entry_rules(strategy: &StrategyItem, price: f64, now: i64, signals: &mut Vec<StrategySignal>) {
    let config = &strategy.config;
    match strategy.strategy_type.as_str() {
        "buy_and_hold" => {
            if strategy.position.is_none() {
                signals.push(StrategySignal {
                    signal_type: SignalType::Buy, price,
                    message: format!("Buy and Hold: entrada em {} @ {:.8}", strategy.symbol, price),
                    acted: false, price_change_percent: 0.0, created_at: now,
                });
            }
        }
        "dca" => {
            if let Some(dca) = &config.dca {
                if dca.enabled {
                    let should_buy = match (dca.max_buys, dca.buys_done) {
                        (Some(max), done) if done >= max => false,
                        _ => true,
                    };
                    if should_buy {
                        let last_buy_time = strategy.executions.iter()
                            .filter(|e| matches!(e.action, ExecutionAction::Buy | ExecutionAction::DcaBuy))
                            .last().map(|e| e.executed_at).unwrap_or(0);
                        let interval = dca.interval_seconds.unwrap_or(86400);
                        if now - last_buy_time >= interval {
                            signals.push(StrategySignal {
                                signal_type: SignalType::DcaBuy, price,
                                message: format!("DCA Buy #{}: {} @ {:.8}", dca.buys_done + 1, strategy.symbol, price),
                                acted: false, price_change_percent: 0.0, created_at: now,
                            });
                        }
                    }
                }
            }
        }
        "swing_trade" | "day_trade" | "scalping" => {
            let change = if let Some(last) = strategy.last_price {
                if last > 0.0 { ((price - last) / last) * 100.0 } else { 0.0 }
            } else { 0.0 };
            signals.push(StrategySignal {
                signal_type: SignalType::Info, price,
                message: format!("{} monitoring: {} @ {:.8} (delta {:.2}%)", strategy.strategy_type, strategy.symbol, price, change),
                acted: false, price_change_percent: change, created_at: now,
            });
        }
        "grid" => {
            if let Some(grid) = &config.grid {
                if grid.enabled {
                    if let (Some(center), Some(spacing), Some(levels)) = (grid.center_price, grid.spacing_percent, grid.levels) {
                        for i in 1..=levels {
                            let buy_level = center * (1.0 - (spacing / 100.0) * i as f64);
                            let sell_level = center * (1.0 + (spacing / 100.0) * i as f64);
                            if price <= buy_level {
                                signals.push(StrategySignal {
                                    signal_type: SignalType::GridTrade, price,
                                    message: format!("Grid Buy Level {}: {} @ {:.8}", i, strategy.symbol, price),
                                    acted: false, price_change_percent: ((price - center) / center) * 100.0, created_at: now,
                                });
                                break;
                            }
                            if price >= sell_level && strategy.position.is_some() {
                                signals.push(StrategySignal {
                                    signal_type: SignalType::GridTrade, price,
                                    message: format!("Grid Sell Level {}: {} @ {:.8}", i, strategy.symbol, price),
                                    acted: false, price_change_percent: ((price - center) / center) * 100.0, created_at: now,
                                });
                                break;
                            }
                        }
                    }
                }
            }
        }
        _ => {
            signals.push(StrategySignal {
                signal_type: SignalType::Info, price,
                message: format!("Unknown type '{}': monitoring {} @ {:.8}", strategy.strategy_type, strategy.symbol, price),
                acted: false, price_change_percent: 0.0, created_at: now,
            });
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// EVALUATE EXIT RULES (TP/SL/Trailing)
// ═══════════════════════════════════════════════════════════════════

fn evaluate_exit_rules(strategy: &StrategyItem, price: f64, now: i64, signals: &mut Vec<StrategySignal>) {
    let config = &strategy.config;
    let position = match &strategy.position {
        Some(pos) => pos,
        None => return,
    };
    let entry_price = position.entry_price;
    if entry_price <= 0.0 { return; }
    let price_change_pct = ((price - entry_price) / entry_price) * 100.0;
    let highest = position.highest_price.max(price);

    // Take Profit
    for (i, tp) in config.take_profit_levels.iter().enumerate() {
        if !tp.executed && price_change_pct >= tp.percent {
            signals.push(StrategySignal {
                signal_type: SignalType::TakeProfit, price,
                message: format!("TP{} atingido: {:.2}% >= {:.2}% (sell {:.0}%)", i + 1, price_change_pct, tp.percent, tp.sell_percent),
                acted: false, price_change_percent: price_change_pct, created_at: now,
            });
            break;
        }
    }

    // Stop Loss
    if let Some(sl) = &config.stop_loss {
        if sl.enabled {
            let sl_threshold = -(sl.percent);
            if sl.trailing {
                let trailing_distance = sl.trailing_distance.unwrap_or(sl.percent);
                let trailing_threshold = highest * (1.0 - trailing_distance / 100.0);
                if price <= trailing_threshold {
                    let drop_from_high = ((price - highest) / highest) * 100.0;
                    signals.push(StrategySignal {
                        signal_type: SignalType::TrailingStop, price,
                        message: format!("Trailing Stop: preco {:.8} caiu {:.2}% do maximo {:.8}", price, drop_from_high, highest),
                        acted: false, price_change_percent: price_change_pct, created_at: now,
                    });
                }
            } else if price_change_pct <= sl_threshold {
                signals.push(StrategySignal {
                    signal_type: SignalType::StopLoss, price,
                    message: format!("Stop Loss: {:.2}% <= {:.2}%", price_change_pct, sl_threshold),
                    acted: false, price_change_percent: price_change_pct, created_at: now,
                });
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// EVALUATE DCA RULES
// ═══════════════════════════════════════════════════════════════════

fn evaluate_dca_rules(strategy: &StrategyItem, price: f64, now: i64, signals: &mut Vec<StrategySignal>) {
    let config = &strategy.config;
    let position = match &strategy.position {
        Some(pos) => pos,
        None => return,
    };

    if let Some(dca) = &config.dca {
        if !dca.enabled { return; }
        if let Some(max) = dca.max_buys {
            if dca.buys_done >= max { return; }
        }
        let last_buy_time = strategy.executions.iter()
            .filter(|e| matches!(e.action, ExecutionAction::Buy | ExecutionAction::DcaBuy))
            .last().map(|e| e.executed_at).unwrap_or(0);
        let interval = dca.interval_seconds.unwrap_or(86400);
        if now - last_buy_time < interval { return; }

        if let Some(dip_pct) = dca.dip_percent {
            let entry_price = position.entry_price;
            if entry_price > 0.0 {
                let price_change = ((price - entry_price) / entry_price) * 100.0;
                if price_change <= -dip_pct {
                    signals.push(StrategySignal {
                        signal_type: SignalType::DcaBuy, price,
                        message: format!("DCA Buy (dip {:.2}%): {} @ {:.8} — #{}", price_change.abs(), strategy.symbol, price, dca.buys_done + 1),
                        acted: false, price_change_percent: price_change, created_at: now,
                    });
                }
            }
        } else {
            signals.push(StrategySignal {
                signal_type: SignalType::DcaBuy, price,
                message: format!("DCA Buy (scheduled): {} @ {:.8} — #{}", strategy.symbol, price, dca.buys_done + 1),
                acted: false, price_change_percent: 0.0, created_at: now,
            });
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// PERSIST TICK RESULT — Grava no MongoDB via array filters
// ═══════════════════════════════════════════════════════════════════

pub async fn persist_tick_result(
    db: &MongoDB,
    user_id: &str,
    strategy: &StrategyItem,
    result: &TickResult,
) -> Result<(), String> {
    let collection = db.collection::<UserStrategies>(COLLECTION);
    let now = chrono::Utc::now().timestamp();
    let p = "strategies.$[elem]";

    let mut update_set = doc! {
        format!("{}.last_checked_at", p): now,
        format!("{}.updated_at", p): now,
        "updated_at": now,
    };

    if result.price > 0.0 {
        update_set.insert(format!("{}.last_price", p), result.price);
    }

    if let Some(ref new_status) = result.new_status {
        update_set.insert(format!("{}.status", p), mongodb::bson::to_bson(new_status).unwrap_or_default());
        match new_status {
            StrategyStatus::Paused | StrategyStatus::Completed | StrategyStatus::Error => {
                update_set.insert(format!("{}.is_active", p), false);
            }
            _ => {}
        }
    }

    if let Some(ref error) = result.error {
        update_set.insert(format!("{}.error_message", p), error.as_str());
    } else {
        update_set.insert(format!("{}.error_message", p), mongodb::bson::Bson::Null);
    }

    // Processar execucoes para atualizar posicao
    let mut current_position = strategy.position.clone();
    let mut accumulated_pnl: f64 = 0.0;
    let mut dca_buys_increment: i32 = 0;
    let mut tp_indices_executed: Vec<usize> = Vec::new();

    for exec in &result.executions {
        match exec.action {
            ExecutionAction::Buy | ExecutionAction::DcaBuy | ExecutionAction::GridBuy => {
                if let Some(ref mut pos) = current_position {
                    let old_cost = pos.entry_price * pos.quantity;
                    let new_cost = exec.price * exec.amount;
                    let new_qty = pos.quantity + exec.amount;
                    if new_qty > 0.0 {
                        pos.entry_price = (old_cost + new_cost) / new_qty;
                        pos.quantity = new_qty;
                        pos.total_cost = old_cost + new_cost;
                    }
                    pos.current_price = result.price;
                    if result.price > pos.highest_price { pos.highest_price = result.price; }
                    if result.price < pos.lowest_price || pos.lowest_price == 0.0 { pos.lowest_price = result.price; }
                    if pos.entry_price > 0.0 {
                        pos.unrealized_pnl = (result.price - pos.entry_price) * pos.quantity;
                        pos.unrealized_pnl_percent = ((result.price - pos.entry_price) / pos.entry_price) * 100.0;
                    }
                } else {
                    current_position = Some(PositionInfo {
                        entry_price: exec.price, quantity: exec.amount, total_cost: exec.total,
                        current_price: result.price, unrealized_pnl: 0.0, unrealized_pnl_percent: 0.0,
                        highest_price: result.price, lowest_price: result.price, opened_at: now,
                    });
                }
                if exec.action == ExecutionAction::DcaBuy { dca_buys_increment += 1; }
            }
            ExecutionAction::Sell | ExecutionAction::GridSell => {
                accumulated_pnl += exec.pnl_usd;
                if let Some(ref mut pos) = current_position {
                    pos.quantity -= exec.amount;
                    if pos.quantity > 0.0001 {
                        pos.total_cost = pos.entry_price * pos.quantity;
                        pos.current_price = result.price;
                        if pos.entry_price > 0.0 {
                            pos.unrealized_pnl = (result.price - pos.entry_price) * pos.quantity;
                            pos.unrealized_pnl_percent = ((result.price - pos.entry_price) / pos.entry_price) * 100.0;
                        }
                    }
                }
                if exec.reason.starts_with("take_profit") {
                    for (i, tp) in strategy.config.take_profit_levels.iter().enumerate() {
                        if !tp.executed && !tp_indices_executed.contains(&i) {
                            tp_indices_executed.push(i);
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Posicao
    let position_closed = current_position.as_ref().map(|p| p.quantity <= 0.0001).unwrap_or(false);
    if position_closed {
        update_set.insert(format!("{}.position", p), mongodb::bson::Bson::Null);
    } else if let Some(ref pos) = current_position {
        if let Ok(pos_bson) = mongodb::bson::to_bson(pos) {
            update_set.insert(format!("{}.position", p), pos_bson);
        }
    } else if result.price > 0.0 {
        if let Some(ref position) = strategy.position {
            if result.price > position.highest_price {
                update_set.insert(format!("{}.position.highest_price", p), result.price);
            }
            if result.price < position.lowest_price || position.lowest_price == 0.0 {
                update_set.insert(format!("{}.position.lowest_price", p), result.price);
            }
            update_set.insert(format!("{}.position.current_price", p), result.price);
            if position.entry_price > 0.0 {
                let unrealized_pnl = (result.price - position.entry_price) * position.quantity;
                let unrealized_pnl_pct = ((result.price - position.entry_price) / position.entry_price) * 100.0;
                update_set.insert(format!("{}.position.unrealized_pnl", p), unrealized_pnl);
                update_set.insert(format!("{}.position.unrealized_pnl_percent", p), unrealized_pnl_pct);
            }
        }
    }

    // $inc PNL e contadores
    let mut update_inc = doc! {};
    if accumulated_pnl.abs() > 0.0001 {
        update_inc.insert(format!("{}.total_pnl_usd", p), accumulated_pnl);
    }
    let new_exec_count = result.executions.iter()
        .filter(|e| !matches!(e.action, ExecutionAction::BuyFailed | ExecutionAction::SellFailed))
        .count() as i32;
    if new_exec_count > 0 {
        update_inc.insert(format!("{}.total_executions", p), new_exec_count);
    }
    if dca_buys_increment > 0 {
        update_inc.insert(format!("{}.config.dca.buys_done", p), dca_buys_increment);
    }

    // TPs executados
    for idx in &tp_indices_executed {
        update_set.insert(format!("{}.config.take_profit_levels.{}.executed", p, idx), true);
        update_set.insert(format!("{}.config.take_profit_levels.{}.executed_at", p, idx), now);
    }

    let mut update_doc = doc! { "$set": update_set };
    if !update_inc.is_empty() {
        update_doc.insert("$inc", update_inc);
    }

    // $push signals e executions no array da strategy
    // Nota: MongoDB nao suporta $push com array filters no mesmo nível
    // Entao faremos um segundo update para push
    let array_filter = doc! { "elem.strategy_id": &strategy.strategy_id };

    collection.update_one(
        doc! { "user_id": user_id },
        update_doc,
    ).array_filters(vec![array_filter.clone()]).await
        .map_err(|e| format!("Failed to persist tick result: {}", e))?;

    // Push signals (limitado a 100)
    if !result.signals.is_empty() {
        let signals_bson: Vec<mongodb::bson::Bson> = result.signals.iter()
            .filter_map(|s| mongodb::bson::to_bson(s).ok()).collect();
        if !signals_bson.is_empty() {
            let push_doc = doc! {
                "$push": {
                    format!("{}.signals", p): {
                        "$each": signals_bson,
                        "$slice": -100
                    }
                }
            };
            let _ = collection.update_one(
                doc! { "user_id": user_id },
                push_doc,
            ).array_filters(vec![array_filter.clone()]).await;
        }
    }

    // Push executions
    if !result.executions.is_empty() {
        let executions_bson: Vec<mongodb::bson::Bson> = result.executions.iter()
            .filter_map(|e| mongodb::bson::to_bson(e).ok()).collect();
        if !executions_bson.is_empty() {
            let push_doc = doc! {
                "$push": {
                    format!("{}.executions", p): {
                        "$each": executions_bson
                    }
                }
            };
            let _ = collection.update_one(
                doc! { "user_id": user_id },
                push_doc,
            ).array_filters(vec![array_filter]).await;
        }
    }

    log::debug!("[Strategy {}] Tick persisted: price={:.8}, signals={}, execs={}",
        &result.strategy_id[..8.min(result.strategy_id.len())],
        result.price, result.signals.len(), result.executions.len());

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════
// ACTIVATE / PAUSE — via array filter
// ═══════════════════════════════════════════════════════════════════

pub async fn activate_strategy(
    db: &MongoDB,
    strategy_id: &str,
    user_id: &str,
) -> Result<StrategyItem, String> {
    let collection = db.collection::<UserStrategies>(COLLECTION);
    let now = chrono::Utc::now().timestamp();
    let p = "strategies.$[elem]";

    collection.update_one(
        doc! { "user_id": user_id },
        doc! {
            "$set": {
                format!("{}.status", p): "monitoring",
                format!("{}.is_active", p): true,
                format!("{}.error_message", p): mongodb::bson::Bson::Null,
                format!("{}.updated_at", p): now,
                "updated_at": now,
            }
        },
    ).array_filters(vec![doc! { "elem.strategy_id": strategy_id }]).await
        .map_err(|e| format!("Failed to activate: {}", e))?;

    // Buscar estrategia atualizada
    let user_doc = collection.find_one(doc! { "user_id": user_id }).await
        .map_err(|e| format!("Failed to fetch: {}", e))?
        .ok_or_else(|| "User strategies not found".to_string())?;

    user_doc.strategies.into_iter()
        .find(|s| s.strategy_id == strategy_id)
        .ok_or_else(|| "Strategy not found".to_string())
}

pub async fn pause_strategy(
    db: &MongoDB,
    strategy_id: &str,
    user_id: &str,
) -> Result<StrategyItem, String> {
    let collection = db.collection::<UserStrategies>(COLLECTION);
    let now = chrono::Utc::now().timestamp();
    let p = "strategies.$[elem]";

    collection.update_one(
        doc! { "user_id": user_id },
        doc! {
            "$set": {
                format!("{}.status", p): "paused",
                format!("{}.is_active", p): false,
                format!("{}.updated_at", p): now,
                "updated_at": now,
            }
        },
    ).array_filters(vec![doc! { "elem.strategy_id": strategy_id }]).await
        .map_err(|e| format!("Failed to pause: {}", e))?;

    let user_doc = collection.find_one(doc! { "user_id": user_id }).await
        .map_err(|e| format!("Failed to fetch: {}", e))?
        .ok_or_else(|| "User strategies not found".to_string())?;

    user_doc.strategies.into_iter()
        .find(|s| s.strategy_id == strategy_id)
        .ok_or_else(|| "Strategy not found".to_string())
}

// ═══════════════════════════════════════════════════════════════════
// PROCESS ALL — Processa todas estrategias ativas de todos usuarios
// ═══════════════════════════════════════════════════════════════════

pub async fn process_active_strategies(db: &MongoDB) -> Result<ProcessResult, String> {
    let collection = db.collection::<UserStrategies>(COLLECTION);
    let now = chrono::Utc::now().timestamp();

    // Buscar todos os documentos que possuem ao menos 1 estrategia ativa
    let filter = doc! {
        "strategies": {
            "$elemMatch": {
                "is_active": true,
                "status": { "$in": ["idle", "monitoring", "in_position", "buy_pending", "sell_pending"] }
            }
        }
    };

    let mut cursor = collection.find(filter).await
        .map_err(|e| format!("Failed to query user_strategy: {}", e))?;

    use futures::stream::StreamExt;
    let mut total = 0;
    let mut processed = 0;
    let mut errors = 0;
    let mut signals_generated = 0;
    let mut orders_executed = 0;

    while let Some(result) = cursor.next().await {
        match result {
            Ok(user_doc) => {
                let user_id = user_doc.user_id.clone();
                for strategy in &user_doc.strategies {
                    if !strategy.is_active { continue; }
                    match strategy.status {
                        StrategyStatus::Idle | StrategyStatus::Monitoring
                        | StrategyStatus::InPosition | StrategyStatus::BuyPending
                        | StrategyStatus::SellPending => {}
                        _ => continue,
                    }

                    total += 1;

                    // Verificar intervalo
                    let last_checked = strategy.last_checked_at.unwrap_or(0);
                    if now - last_checked < strategy.check_interval_secs {
                        continue;
                    }

                    let tick_result = tick(db, &user_id, strategy).await;
                    signals_generated += tick_result.signals.len();
                    orders_executed += tick_result.executions.len();

                    match persist_tick_result(db, &user_id, strategy, &tick_result).await {
                        Ok(_) => processed += 1,
                        Err(e) => {
                            log::error!("[Strategy {}] Failed to persist: {}", tick_result.strategy_id, e);
                            errors += 1;
                        }
                    }

                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
            Err(e) => {
                log::error!("Error reading user_strategy doc: {}", e);
                errors += 1;
            }
        }
    }

    let result = ProcessResult { total, processed, errors, signals_generated, orders_executed };
    log::info!("Strategy processing: {} total, {} processed, {} errors, {} signals, {} orders",
        result.total, result.processed, result.errors, result.signals_generated, result.orders_executed);
    Ok(result)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProcessResult {
    pub total: usize,
    pub processed: usize,
    pub errors: usize,
    pub signals_generated: usize,
    pub orders_executed: usize,
}
