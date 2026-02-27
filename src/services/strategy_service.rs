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

pub async fn fetch_current_price(
    ccxt_id: &str, api_key: &str, api_secret: &str,
    passphrase: Option<&str>, symbol: &str,
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
            .ok_or_else(|| format!("No 'last' price for {}", symbol))
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}

pub async fn tick(db: &MongoDB, user_id: &str, strategy: &StrategyItem) -> TickResult {
    let strategy_id = strategy.strategy_id.clone();
    let now = chrono::Utc::now().timestamp();

    if !strategy.is_active {
        return TickResult {
            strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
            signals: vec![], executions: vec![], new_status: None,
            error: Some("Strategy is not active".into()),
        };
    }

    match strategy.status {
        StrategyStatus::Paused | StrategyStatus::Completed
        | StrategyStatus::StoppedOut | StrategyStatus::Expired | StrategyStatus::Error => {
            return TickResult {
                strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
                signals: vec![], executions: vec![], new_status: None,
                error: Some(format!("Status '{}' not processable", strategy.status)),
            };
        }
        _ => {}
    }

    if strategy.is_expired() {
        return TickResult {
            strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
            signals: vec![StrategySignal {
                signal_type: SignalType::Expired, price: 0.0,
                message: format!("Strategy expired after {}min", strategy.config.time_execution_min),
                acted: false, price_change_percent: 0.0, created_at: now,
            }],
            executions: vec![], new_status: Some(StrategyStatus::Expired), error: None,
        };
    }

    let decrypted = match user_exchanges_service::get_user_exchanges_decrypted(db, user_id).await {
        Ok(ex) => ex,
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
                error: Some(format!("Exchange {} not found", strategy.exchange_id)),
            };
        }
    };

    let price = match fetch_current_price(
        &exchange.ccxt_id, &exchange.api_key, &exchange.api_secret,
        exchange.passphrase.as_deref(), &strategy.symbol,
    ).await {
        Ok(p) => p,
        Err(e) => {
            return TickResult {
                strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
                signals: vec![], executions: vec![], new_status: None,
                error: Some(format!("Price fetch failed: {}", e)),
            };
        }
    };

    let mut signals: Vec<StrategySignal> = Vec::new();
    let mut executions: Vec<StrategyExecution> = Vec::new();
    let mut new_status: Option<StrategyStatus> = None;

    match strategy.status {
        StrategyStatus::Idle | StrategyStatus::Monitoring => {
            if strategy.status == StrategyStatus::Idle {
                new_status = Some(StrategyStatus::Monitoring);
            }
            evaluate_trigger(strategy, price, now, &mut signals);
        }
        StrategyStatus::InPosition => {
            evaluate_exit(strategy, price, now, &mut signals);
        }
        StrategyStatus::GradualSelling => {
            evaluate_gradual(strategy, price, now, &mut signals);
        }
        _ => {}
    }

    for signal in &mut signals {
        match signal.signal_type {
            SignalType::TakeProfit | SignalType::GradualSell => {
                let sell_amount = calc_sell_amount(strategy, &signal.signal_type);
                if sell_amount <= 0.0 { continue; }

                match execute_order(exchange, &strategy.symbol, "market", "sell", sell_amount, None).await {
                    Ok(order) => {
                        signal.acted = true;
                        let entry = strategy.position.as_ref().map(|p| p.entry_price).unwrap_or(0.0);
                        let filled = order.filled.unwrap_or(sell_amount);
                        let sell_price = order.avg_price.unwrap_or(price);
                        let pnl = (sell_price - entry) * filled;
                        let fee = order.fee.unwrap_or(0.0);
                        let reason = match signal.signal_type {
                            SignalType::GradualSell => "gradual_sell".to_string(),
                            _ => "take_profit".to_string(),
                        };
                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action: ExecutionAction::Sell, reason: reason.clone(),
                            price: sell_price, amount: filled,
                            total: order.cost.unwrap_or(sell_price * filled),
                            fee, pnl_usd: pnl - fee,
                            exchange_order_id: Some(order.order_id),
                            executed_at: now, error_message: None,
                        });

                        if !strategy.config.gradual_sell {
                            new_status = Some(StrategyStatus::Completed);
                        } else {
                            let remaining_lots = strategy.config.gradual_lots.iter().filter(|l| !l.executed).count();
                            if remaining_lots <= 1 {
                                new_status = Some(StrategyStatus::Completed);
                            } else {
                                new_status = Some(StrategyStatus::GradualSelling);
                            }
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
                            executed_at: now, error_message: Some(e),
                        });
                    }
                }
            }
            SignalType::StopLoss => {
                let qty = strategy.position.as_ref().map(|p| p.quantity).unwrap_or(0.0);
                if qty <= 0.0 { continue; }
                match execute_order(exchange, &strategy.symbol, "market", "sell", qty, None).await {
                    Ok(order) => {
                        signal.acted = true;
                        let entry = strategy.position.as_ref().map(|p| p.entry_price).unwrap_or(0.0);
                        let filled = order.filled.unwrap_or(qty);
                        let sell_price = order.avg_price.unwrap_or(price);
                        let pnl = (sell_price - entry) * filled;
                        let fee = order.fee.unwrap_or(0.0);
                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action: ExecutionAction::Sell, reason: "stop_loss".into(),
                            price: sell_price, amount: filled,
                            total: order.cost.unwrap_or(sell_price * filled),
                            fee, pnl_usd: pnl - fee,
                            exchange_order_id: Some(order.order_id),
                            executed_at: now, error_message: None,
                        });
                        new_status = Some(StrategyStatus::StoppedOut);
                    }
                    Err(e) => {
                        signal.acted = false;
                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action: ExecutionAction::SellFailed,
                            reason: format!("stop_loss_failed: {}", e),
                            price, amount: qty, total: qty * price,
                            fee: 0.0, pnl_usd: 0.0, exchange_order_id: None,
                            executed_at: now, error_message: Some(e),
                        });
                    }
                }
            }
            _ => {}
        }
    }

    TickResult { strategy_id, symbol: strategy.symbol.clone(), price, signals, executions, new_status, error: None }
}

fn evaluate_trigger(strategy: &StrategyItem, price: f64, now: i64, signals: &mut Vec<StrategySignal>) {
    let config = &strategy.config;
    if config.base_price <= 0.0 { return; }

    let trigger = config.trigger_price();
    let sl_price = config.stop_loss_price();
    let pct = ((price - config.base_price) / config.base_price) * 100.0;

    if strategy.position.is_some() {
        if price >= trigger {
            if config.gradual_sell && !config.gradual_lots.is_empty() {
                let lot = config.gradual_lots.iter().find(|l| !l.executed);
                if let Some(lot) = lot {
                    signals.push(StrategySignal {
                        signal_type: SignalType::TakeProfit, price,
                        message: format!("Trigger {:.2} hit! Gradual lot 1 ({:.0}%)", trigger, lot.sell_percent),
                        acted: false, price_change_percent: pct, created_at: now,
                    });
                }
            } else {
                signals.push(StrategySignal {
                    signal_type: SignalType::TakeProfit, price,
                    message: format!("Trigger {:.2} hit! Full sell", trigger),
                    acted: false, price_change_percent: pct, created_at: now,
                });
            }
        } else if price <= sl_price {
            signals.push(StrategySignal {
                signal_type: SignalType::StopLoss, price,
                message: format!("Stop loss {:.2} hit! Price {:.2}", sl_price, price),
                acted: false, price_change_percent: pct, created_at: now,
            });
        } else {
            signals.push(StrategySignal {
                signal_type: SignalType::Info, price,
                message: format!("Monitoring: {:.2} | trigger={:.2} sl={:.2} ({:+.2}%)", price, trigger, sl_price, pct),
                acted: false, price_change_percent: pct, created_at: now,
            });
        }
    } else {
        signals.push(StrategySignal {
            signal_type: SignalType::Info, price,
            message: format!("No position. Base={:.2} trigger={:.2} sl={:.2}", config.base_price, trigger, sl_price),
            acted: false, price_change_percent: pct, created_at: now,
        });
    }
}

fn evaluate_exit(strategy: &StrategyItem, price: f64, now: i64, signals: &mut Vec<StrategySignal>) {
    let config = &strategy.config;
    let position = match &strategy.position {
        Some(pos) if pos.quantity > 0.0 => pos,
        _ => return,
    };

    let entry = position.entry_price;
    if entry <= 0.0 { return; }
    let pct = ((price - entry) / entry) * 100.0;
    let trigger = config.trigger_price();
    let sl_price = config.stop_loss_price();

    if price >= trigger {
        if config.gradual_sell && !config.gradual_lots.is_empty() {
            let lot = config.gradual_lots.iter().find(|l| !l.executed);
            if let Some(lot) = lot {
                signals.push(StrategySignal {
                    signal_type: SignalType::TakeProfit, price,
                    message: format!("TP {:.2} hit! Start gradual lot 1 ({:.0}%)", trigger, lot.sell_percent),
                    acted: false, price_change_percent: pct, created_at: now,
                });
            } else {
                signals.push(StrategySignal {
                    signal_type: SignalType::TakeProfit, price,
                    message: "All lots done. Sell remaining".into(),
                    acted: false, price_change_percent: pct, created_at: now,
                });
            }
        } else {
            signals.push(StrategySignal {
                signal_type: SignalType::TakeProfit, price,
                message: format!("TP {:.2} hit! Full sell ({:+.2}%)", trigger, pct),
                acted: false, price_change_percent: pct, created_at: now,
            });
        }
    } else if price <= sl_price {
        signals.push(StrategySignal {
            signal_type: SignalType::StopLoss, price,
            message: format!("SL {:.2} hit! Sell all ({:+.2}%)", sl_price, pct),
            acted: false, price_change_percent: pct, created_at: now,
        });
    } else {
        signals.push(StrategySignal {
            signal_type: SignalType::Info, price,
            message: format!("Holding: {:.2} ({:+.2}%) trigger={:.2} sl={:.2}", price, pct, trigger, sl_price),
            acted: false, price_change_percent: pct, created_at: now,
        });
    }
}

fn evaluate_gradual(strategy: &StrategyItem, price: f64, now: i64, signals: &mut Vec<StrategySignal>) {
    let config = &strategy.config;
    let position = match &strategy.position {
        Some(pos) if pos.quantity > 0.0 => pos,
        _ => return,
    };

    let entry = position.entry_price;
    if entry <= 0.0 { return; }
    let pct = ((price - entry) / entry) * 100.0;
    let sl_price = config.stop_loss_price();

    if price <= sl_price {
        signals.push(StrategySignal {
            signal_type: SignalType::StopLoss, price,
            message: format!("SL {:.2} during gradual! Sell all ({:+.2}%)", sl_price, pct),
            acted: false, price_change_percent: pct, created_at: now,
        });
        return;
    }

    let timer_secs = config.timer_gradual_min * 60;
    let last_sell = strategy.last_gradual_sell_at.unwrap_or(0);
    if now - last_sell < timer_secs {
        let remaining = timer_secs - (now - last_sell);
        signals.push(StrategySignal {
            signal_type: SignalType::Info, price,
            message: format!("Gradual timer: {}s remaining", remaining),
            acted: false, price_change_percent: pct, created_at: now,
        });
        return;
    }

    let next_lot_idx = config.gradual_lots.iter().position(|l| !l.executed);
    match next_lot_idx {
        Some(idx) => {
            let lot = &config.gradual_lots[idx];
            let gradual_trigger = config.gradual_trigger_price(idx);
            if price >= gradual_trigger {
                signals.push(StrategySignal {
                    signal_type: SignalType::GradualSell, price,
                    message: format!(
                        "Gradual lot {} ({:.0}%): {:.2} >= {:.2}",
                        lot.lot_number, lot.sell_percent, price, gradual_trigger
                    ),
                    acted: false, price_change_percent: pct, created_at: now,
                });
            } else {
                signals.push(StrategySignal {
                    signal_type: SignalType::Info, price,
                    message: format!(
                        "Gradual wait: lot {} needs {:.2}, price={:.2}",
                        lot.lot_number, gradual_trigger, price
                    ),
                    acted: false, price_change_percent: pct, created_at: now,
                });
            }
        }
        None => {
            signals.push(StrategySignal {
                signal_type: SignalType::TakeProfit, price,
                message: "All gradual lots done. Sell remaining".into(),
                acted: false, price_change_percent: pct, created_at: now,
            });
        }
    }
}

fn calc_sell_amount(strategy: &StrategyItem, signal_type: &SignalType) -> f64 {
    let position = match &strategy.position {
        Some(pos) if pos.quantity > 0.0 => pos,
        _ => return 0.0,
    };
    match signal_type {
        SignalType::TakeProfit => {
            if strategy.config.gradual_sell && !strategy.config.gradual_lots.is_empty() {
                let lot = strategy.config.gradual_lots.iter().find(|l| !l.executed);
                match lot {
                    Some(lot) => {
                        let original_qty = if position.entry_price > 0.0 {
                            position.total_cost / position.entry_price
                        } else {
                            position.quantity
                        };
                        (original_qty * lot.sell_percent / 100.0).min(position.quantity)
                    }
                    None => position.quantity,
                }
            } else {
                position.quantity
            }
        }
        SignalType::GradualSell => {
            let lot = strategy.config.gradual_lots.iter().find(|l| !l.executed);
            match lot {
                Some(lot) => {
                    let original_qty = if position.entry_price > 0.0 {
                        position.total_cost / position.entry_price
                    } else {
                        position.quantity
                    };
                    (original_qty * lot.sell_percent / 100.0).min(position.quantity)
                }
                None => position.quantity,
            }
        }
        SignalType::StopLoss => position.quantity,
        _ => 0.0,
    }
}

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
    exchange: &DecryptedExchange, symbol: &str,
    order_type: &str, side: &str, amount: f64, price: Option<f64>,
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
            let s = |key: &str| -> String {
                order_ref.get_item(key).ok()
                    .and_then(|v| if v.is_none() { None } else { v.extract().ok() })
                    .unwrap_or_default()
            };
            let f = |key: &str| -> Option<f64> {
                order_ref.get_item(key).ok()
                    .and_then(|v| if v.is_none() { None } else { v.extract().ok() })
            };
            let fee_cost: Option<f64> = order_ref.get_item("fee").ok()
                .and_then(|fee| {
                    if fee.is_none() { return None; }
                    fee.get_item("cost").ok()?.extract().ok()
                });
            Ok(OrderResult {
                order_id: s("id"), status: s("status"),
                filled: f("filled"), avg_price: f("average").or_else(|| f("price")),
                cost: f("cost"), fee: fee_cost,
            })
        })
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}

pub async fn persist_tick_result(
    db: &MongoDB, user_id: &str, strategy: &StrategyItem, result: &TickResult,
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
            StrategyStatus::Completed | StrategyStatus::StoppedOut
            | StrategyStatus::Expired | StrategyStatus::Error | StrategyStatus::Paused => {
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

    let mut current_position = strategy.position.clone();
    let mut accumulated_pnl: f64 = 0.0;
    let mut gradual_lot_indices_executed: Vec<usize> = Vec::new();
    let mut had_gradual_sell = false;

    for exec in &result.executions {
        match exec.action {
            ExecutionAction::Buy => {
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
                } else {
                    current_position = Some(PositionInfo {
                        entry_price: exec.price, quantity: exec.amount, total_cost: exec.total,
                        current_price: result.price, unrealized_pnl: 0.0, unrealized_pnl_percent: 0.0,
                        highest_price: result.price, opened_at: now,
                    });
                }
            }
            ExecutionAction::Sell => {
                accumulated_pnl += exec.pnl_usd;
                if let Some(ref mut pos) = current_position {
                    pos.quantity -= exec.amount;
                    if pos.quantity > 0.0001 {
                        pos.total_cost = pos.entry_price * pos.quantity;
                        pos.current_price = result.price;
                    }
                }
                if exec.reason.contains("gradual") || exec.reason == "take_profit" {
                    had_gradual_sell = true;
                    for (i, lot) in strategy.config.gradual_lots.iter().enumerate() {
                        if !lot.executed && !gradual_lot_indices_executed.contains(&i) {
                            gradual_lot_indices_executed.push(i);
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if had_gradual_sell {
        update_set.insert(format!("{}.last_gradual_sell_at", p), now);
    }

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
            update_set.insert(format!("{}.position.current_price", p), result.price);
            if position.entry_price > 0.0 {
                let unrealized_pnl = (result.price - position.entry_price) * position.quantity;
                let unrealized_pnl_pct = ((result.price - position.entry_price) / position.entry_price) * 100.0;
                update_set.insert(format!("{}.position.unrealized_pnl", p), unrealized_pnl);
                update_set.insert(format!("{}.position.unrealized_pnl_percent", p), unrealized_pnl_pct);
            }
        }
    }

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

    for idx in &gradual_lot_indices_executed {
        update_set.insert(format!("{}.config.gradual_lots.{}.executed", p, idx), true);
        update_set.insert(format!("{}.config.gradual_lots.{}.executed_at", p, idx), now);
        update_set.insert(format!("{}.config.gradual_lots.{}.executed_price", p, idx), result.price);
    }

    if !gradual_lot_indices_executed.is_empty() {
        let all_executed = strategy.config.gradual_lots.iter().enumerate().all(|(i, lot)| {
            lot.executed || gradual_lot_indices_executed.contains(&i)
        });
        if all_executed && position_closed {
            update_set.insert(format!("{}.status", p), mongodb::bson::to_bson(&StrategyStatus::Completed).unwrap_or_default());
            update_set.insert(format!("{}.is_active", p), false);
        }
    }

    let mut update_doc = doc! { "$set": update_set };
    if !update_inc.is_empty() {
        update_doc.insert("$inc", update_inc);
    }

    let array_filter = doc! { "elem.strategy_id": &strategy.strategy_id };

    collection.update_one(
        doc! { "user_id": user_id },
        update_doc,
    ).array_filters(vec![array_filter.clone()]).await
        .map_err(|e| format!("Failed to persist tick: {}", e))?;

    if !result.signals.is_empty() {
        let signals_bson: Vec<mongodb::bson::Bson> = result.signals.iter()
            .filter_map(|s| mongodb::bson::to_bson(s).ok()).collect();
        if !signals_bson.is_empty() {
            let _ = collection.update_one(
                doc! { "user_id": user_id },
                doc! { "$push": { format!("{}.signals", p): { "$each": signals_bson, "$slice": -100 } } },
            ).array_filters(vec![array_filter.clone()]).await;
        }
    }

    if !result.executions.is_empty() {
        let execs_bson: Vec<mongodb::bson::Bson> = result.executions.iter()
            .filter_map(|e| mongodb::bson::to_bson(e).ok()).collect();
        if !execs_bson.is_empty() {
            let _ = collection.update_one(
                doc! { "user_id": user_id },
                doc! { "$push": { format!("{}.executions", p): { "$each": execs_bson } } },
            ).array_filters(vec![array_filter]).await;
        }
    }

    Ok(())
}

pub async fn activate_strategy(db: &MongoDB, strategy_id: &str, user_id: &str) -> Result<StrategyItem, String> {
    let collection = db.collection::<UserStrategies>(COLLECTION);
    let now = chrono::Utc::now().timestamp();
    let p = "strategies.$[elem]";

    collection.update_one(
        doc! { "user_id": user_id },
        doc! { "$set": {
            format!("{}.status", p): "monitoring",
            format!("{}.is_active", p): true,
            format!("{}.error_message", p): mongodb::bson::Bson::Null,
            format!("{}.updated_at", p): now,
            "updated_at": now,
        }},
    ).array_filters(vec![doc! { "elem.strategy_id": strategy_id }]).await
        .map_err(|e| format!("Failed to activate: {}", e))?;

    let user_doc = collection.find_one(doc! { "user_id": user_id }).await
        .map_err(|e| format!("Failed to fetch: {}", e))?
        .ok_or_else(|| "User strategies not found".to_string())?;

    user_doc.strategies.into_iter()
        .find(|s| s.strategy_id == strategy_id)
        .ok_or_else(|| "Strategy not found".to_string())
}

pub async fn pause_strategy(db: &MongoDB, strategy_id: &str, user_id: &str) -> Result<StrategyItem, String> {
    let collection = db.collection::<UserStrategies>(COLLECTION);
    let now = chrono::Utc::now().timestamp();
    let p = "strategies.$[elem]";

    collection.update_one(
        doc! { "user_id": user_id },
        doc! { "$set": {
            format!("{}.status", p): "paused",
            format!("{}.is_active", p): false,
            format!("{}.updated_at", p): now,
            "updated_at": now,
        }},
    ).array_filters(vec![doc! { "elem.strategy_id": strategy_id }]).await
        .map_err(|e| format!("Failed to pause: {}", e))?;

    let user_doc = collection.find_one(doc! { "user_id": user_id }).await
        .map_err(|e| format!("Failed to fetch: {}", e))?
        .ok_or_else(|| "User strategies not found".to_string())?;

    user_doc.strategies.into_iter()
        .find(|s| s.strategy_id == strategy_id)
        .ok_or_else(|| "Strategy not found".to_string())
}

pub async fn process_active_strategies(db: &MongoDB) -> Result<ProcessResult, String> {
    let collection = db.collection::<UserStrategies>(COLLECTION);
    let now = chrono::Utc::now().timestamp();

    let filter = doc! {
        "strategies": {
            "$elemMatch": {
                "is_active": true,
                "status": { "$in": ["idle", "monitoring", "in_position", "gradual_selling"] }
            }
        }
    };

    let mut cursor = collection.find(filter).await
        .map_err(|e| format!("Failed to query: {}", e))?;

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
                        | StrategyStatus::InPosition | StrategyStatus::GradualSelling => {}
                        _ => continue,
                    }
                    total += 1;
                    let last_checked = strategy.last_checked_at.unwrap_or(0);
                    if now - last_checked < 30 { continue; }

                    let tick_result = tick(db, &user_id, strategy).await;
                    signals_generated += tick_result.signals.len();
                    orders_executed += tick_result.executions.len();

                    match persist_tick_result(db, &user_id, strategy, &tick_result).await {
                        Ok(_) => processed += 1,
                        Err(e) => {
                            log::error!("[Strategy {}] Persist failed: {}", tick_result.strategy_id, e);
                            errors += 1;
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
            Err(e) => {
                log::error!("Error reading user_strategy: {}", e);
                errors += 1;
            }
        }
    }

    Ok(ProcessResult { total, processed, errors, signals_generated, orders_executed })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProcessResult {
    pub total: usize,
    pub processed: usize,
    pub errors: usize,
    pub signals_generated: usize,
    pub orders_executed: usize,
}
