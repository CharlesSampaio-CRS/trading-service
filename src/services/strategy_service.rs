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

    // ── Guard: inactive strategy ────────────────────────────────────
    if !strategy.is_active {
        return TickResult {
            strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
            signals: vec![], executions: vec![], new_status: None,
            error: Some(format!("Strategy '{}' is not active. Activate it to resume monitoring.", strategy.name)),
        };
    }

    // ── Guard: terminal status ──────────────────────────────────────
    match strategy.status {
        StrategyStatus::Paused => {
            return TickResult {
                strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
                signals: vec![], executions: vec![], new_status: None,
                error: Some(format!("Strategy '{}' is paused. Activate it to resume.", strategy.name)),
            };
        }
        StrategyStatus::Completed => {
            return TickResult {
                strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
                signals: vec![], executions: vec![], new_status: None,
                error: Some(format!("Strategy '{}' already completed with PnL ${:.2}.", strategy.name, strategy.total_pnl_usd)),
            };
        }
        StrategyStatus::StoppedOut => {
            return TickResult {
                strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
                signals: vec![], executions: vec![], new_status: None,
                error: Some(format!("Strategy '{}' was stopped out (stop loss triggered).", strategy.name)),
            };
        }
        StrategyStatus::Expired => {
            return TickResult {
                strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
                signals: vec![], executions: vec![], new_status: None,
                error: Some(format!("Strategy '{}' expired after {} minutes.", strategy.name, strategy.config.time_execution_min)),
            };
        }
        StrategyStatus::Error => {
            return TickResult {
                strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
                signals: vec![], executions: vec![], new_status: None,
                error: Some(format!("Strategy '{}' is in error state: {}. Fix the issue and reactivate.",
                    strategy.name, strategy.error_message.as_deref().unwrap_or("unknown error"))),
            };
        }
        _ => {}
    }

    // ── Guard: config validation ────────────────────────────────────
    if strategy.config.base_price <= 0.0 {
        return TickResult {
            strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
            signals: vec![], executions: vec![],
            new_status: Some(StrategyStatus::Error),
            error: Some("Invalid configuration: base_price must be greater than 0. Update the strategy config.".into()),
        };
    }

    // ── Guard: expiration ───────────────────────────────────────────
    if strategy.is_expired() {
        let elapsed_min = (now - strategy.started_at) / 60;
        return TickResult {
            strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
            signals: vec![StrategySignal {
                signal_type: SignalType::Expired, price: 0.0,
                message: format!(
                    "Strategy '{}' expired. Ran for {} minutes (limit: {} min). No position was opened.",
                    strategy.name, elapsed_min, strategy.config.time_execution_min
                ),
                acted: false, price_change_percent: 0.0, created_at: now, source: None,
            }],
            executions: vec![], new_status: Some(StrategyStatus::Expired), error: None,
        };
    }

    // ── Decrypt exchange credentials ────────────────────────────────
    let decrypted = match user_exchanges_service::get_user_exchanges_decrypted(db, user_id).await {
        Ok(ex) => ex,
        Err(e) => {
            log::error!("❌ [{}] Failed to decrypt exchanges: {}", strategy_id, e);
            return TickResult {
                strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
                signals: vec![], executions: vec![],
                new_status: Some(StrategyStatus::Error),
                error: Some("Failed to access exchange credentials. Please reconnect your exchange.".into()),
            };
        }
    };

    let exchange = match decrypted.iter().find(|ex| ex.exchange_id == strategy.exchange_id) {
        Some(ex) => ex,
        None => {
            log::error!("❌ [{}] Exchange '{}' not found for user {}", strategy_id, strategy.exchange_id, user_id);
            return TickResult {
                strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
                signals: vec![], executions: vec![],
                new_status: Some(StrategyStatus::Error),
                error: Some(format!(
                    "Exchange '{}' not found or disconnected. Reconnect your exchange and reactivate the strategy.",
                    strategy.exchange_name
                )),
            };
        }
    };

    // ── Fetch current price ─────────────────────────────────────────
    let price = match fetch_current_price(
        &exchange.ccxt_id, &exchange.api_key, &exchange.api_secret,
        exchange.passphrase.as_deref(), &strategy.symbol,
    ).await {
        Ok(p) if p <= 0.0 => {
            return TickResult {
                strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
                signals: vec![], executions: vec![], new_status: None,
                error: Some(format!(
                    "Received invalid price ({}) for {}. The market may be closed or the pair delisted.",
                    p, strategy.symbol
                )),
            };
        }
        Ok(p) => p,
        Err(e) => {
            let friendly = if e.contains("NetworkError") || e.contains("timeout") {
                format!("Network error fetching {} price. Will retry on next tick.", strategy.symbol)
            } else if e.contains("BadSymbol") || e.contains("not found") {
                format!("Trading pair '{}' not available on {}. Check if the pair is correct.",
                    strategy.symbol, strategy.exchange_name)
            } else if e.contains("AuthenticationError") || e.contains("invalid api") {
                format!("Exchange authentication failed for {}. Check your API keys.",
                    strategy.exchange_name)
            } else if e.contains("RateLimitExceeded") || e.contains("rate limit") {
                format!("Rate limited by {}. Will retry on next tick.", strategy.exchange_name)
            } else {
                format!("Failed to fetch price for {}: {}", strategy.symbol, e)
            };
            log::warn!("⚠️ [{}] Price fetch: {}", strategy_id, friendly);
            return TickResult {
                strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
                signals: vec![], executions: vec![], new_status: None,
                error: Some(friendly),
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

                // ── Double-check: se invested_amount preenchido, verificar lucro real ──
                // Garante que o valor atual do investimento é REALMENTE maior que o investido.
                // Protege contra slippage, fees inesperadas, ou pequenas variações de preço.
                let config = &strategy.config;
                if config.invested_amount > 0.0 && config.base_price > 0.0 {
                    let estimated_qty = config.invested_amount / config.base_price;
                    let current_value = estimated_qty * price;
                    let estimated_pnl = current_value - config.invested_amount;
                    if estimated_pnl <= 0.0 {
                        log::warn!(
                            "⚠️ [{}] DOUBLE-CHECK: trigger atingido mas investimento NÃO dá lucro real! \
                            Investido: ${:.2}, Valor atual: ${:.2}, PnL estimado: ${:.2}. Venda BLOQUEADA — aguardando preço melhor.",
                            strategy.strategy_id, config.invested_amount, current_value, estimated_pnl
                        );
                        signal.acted = false;
                        signal.message = format!(
                            "⚠️ DOUBLE-CHECK: Trigger atingido (preço {:.2}) mas investimento de ${:.2} \
                            valeria ${:.2} agora (PnL: {:+.2}$). Venda bloqueada — aguardando lucro real.",
                            price, config.invested_amount, current_value, estimated_pnl
                        );
                        signal.signal_type = SignalType::Info;
                        continue;
                    }
                    log::info!(
                        "✅ [{}] DOUBLE-CHECK OK: Investido ${:.2} → atual ${:.2} (lucro: +${:.2}). Prosseguindo com venda.",
                        strategy.strategy_id, config.invested_amount, current_value, estimated_pnl
                    );
                }

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
                        log::info!("✅ [{}] {} executed: {:.6} {} @ {:.4} | PnL: ${:.2}",
                            strategy.strategy_id, reason, filled, strategy.symbol, sell_price, pnl - fee);
                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action: ExecutionAction::Sell, reason: reason.clone(),
                            price: sell_price, amount: filled,
                            total: order.cost.unwrap_or(sell_price * filled),
                            fee, pnl_usd: pnl - fee,
                            exchange_order_id: Some(order.order_id),
                            executed_at: now, error_message: None, source: None,
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
                        let friendly = classify_order_error(&e, &strategy.symbol, &strategy.exchange_name);
                        log::error!("❌ [{}] Sell failed: {} | raw: {}", strategy.strategy_id, friendly, e);
                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action: ExecutionAction::SellFailed,
                            reason: format!("sell_failed: {}", friendly),
                            price, amount: sell_amount, total: sell_amount * price,
                            fee: 0.0, pnl_usd: 0.0, exchange_order_id: None,
                            executed_at: now, error_message: Some(friendly), source: None,
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
                        log::warn!("🛑 [{}] STOP LOSS executed: {:.6} {} @ {:.4} | Loss: ${:.2}",
                            strategy.strategy_id, filled, strategy.symbol, sell_price, pnl - fee);
                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action: ExecutionAction::Sell, reason: "stop_loss".into(),
                            price: sell_price, amount: filled,
                            total: order.cost.unwrap_or(sell_price * filled),
                            fee, pnl_usd: pnl - fee,
                            exchange_order_id: Some(order.order_id),
                            executed_at: now, error_message: None, source: None,
                        });
                        new_status = Some(StrategyStatus::StoppedOut);
                    }
                    Err(e) => {
                        signal.acted = false;
                        let friendly = classify_order_error(&e, &strategy.symbol, &strategy.exchange_name);
                        log::error!("❌ [{}] Stop loss SELL FAILED: {} | raw: {}", strategy.strategy_id, friendly, e);
                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action: ExecutionAction::SellFailed,
                            reason: format!("stop_loss_failed: {}", friendly),
                            price, amount: qty, total: qty * price,
                            fee: 0.0, pnl_usd: 0.0, exchange_order_id: None,
                            executed_at: now, error_message: Some(friendly), source: None,
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

    // PnL estimado em $ (se invested_amount preenchido)
    let pnl_info = match config.estimated_pnl(price) {
        Some(pnl) => format!(" Investimento: ${:.2} → atual ${:.2} (PnL: {:+.2}$)",
            config.invested_amount,
            config.invested_amount + pnl,
            pnl
        ),
        None => String::new(),
    };

    if strategy.position.is_some() {
        if price >= trigger {
            if config.gradual_sell && !config.gradual_lots.is_empty() {
                let lot = config.gradual_lots.iter().find(|l| !l.executed);
                if let Some(lot) = lot {
                    signals.push(StrategySignal {
                        signal_type: SignalType::TakeProfit, price,
                        message: format!(
                            "🎯 TRIGGER ATINGIDO! Preço {:.2} >= trigger {:.2} ({:+.2}%).{} Iniciando venda gradual — lote {} de {:.0}%.",
                            price, trigger, pct, pnl_info, lot.lot_number, lot.sell_percent
                        ),
                        acted: false, price_change_percent: pct, created_at: now, source: None,
                    });
                }
            } else {
                signals.push(StrategySignal {
                    signal_type: SignalType::TakeProfit, price,
                    message: format!(
                        "🎯 TRIGGER ATINGIDO! Preço {:.2} >= trigger {:.2} ({:+.2}%).{} Executando venda total.",
                        price, trigger, pct, pnl_info
                    ),
                    acted: false, price_change_percent: pct, created_at: now, source: None,
                });
            }
        } else if config.stop_loss_enabled && price <= sl_price {
            signals.push(StrategySignal {
                signal_type: SignalType::StopLoss, price,
                message: format!(
                    "🛑 STOP LOSS ATINGIDO! Preço {:.2} <= stop {:.2} ({:+.2}%).{} Vendendo tudo para limitar perda.",
                    price, sl_price, pct, pnl_info
                ),
                acted: false, price_change_percent: pct, created_at: now, source: None,
            });
        } else {
            let diff_trigger = trigger - price;
            let diff_trigger_pct = (diff_trigger / price) * 100.0;
            let diff_sl = price - sl_price;
            let diff_sl_pct = (diff_sl / price) * 100.0;
            signals.push(StrategySignal {
                signal_type: SignalType::Info, price,
                message: format!(
                    "👁️ Monitorando: preço {:.2} ({:+.2}% do base). Faltam {:.2} ({:.2}%) para trigger {:.2}.{}{}",
                    price, pct, diff_trigger, diff_trigger_pct, trigger,
                    if config.stop_loss_enabled {
                        format!(" Margem até stop: {:.2} ({:.2}%) acima de {:.2}.", diff_sl, diff_sl_pct, sl_price)
                    } else {
                        " Stop loss desativado.".to_string()
                    },
                    pnl_info
                ),
                acted: false, price_change_percent: pct, created_at: now, source: None,
            });
        }
    } else {
        let diff_trigger = trigger - price;
        let diff_trigger_pct = if price > 0.0 { (diff_trigger / price) * 100.0 } else { 0.0 };
        let sl_info = if config.stop_loss_enabled {
            format!(" Stop loss em {:.2}.", sl_price)
        } else {
            " Stop loss desativado.".to_string()
        };
        signals.push(StrategySignal {
            signal_type: SignalType::Info, price,
            message: format!(
                "⏳ Sem posição aberta. Preço atual: {:.2} ({:+.2}% do base {:.2}). Trigger em {:.2} (faltam {:.2}, {:.2}%).{}{} Aguardando entrada manual ou via exchange.",
                price, pct, config.base_price, trigger, diff_trigger, diff_trigger_pct, sl_info, pnl_info
            ),
            acted: false, price_change_percent: pct, created_at: now, source: None,
        });
    }
}

fn evaluate_exit(strategy: &StrategyItem, price: f64, now: i64, signals: &mut Vec<StrategySignal>) {
    let config = &strategy.config;
    let position = match &strategy.position {
        Some(pos) if pos.quantity > 0.0 => pos,
        _ => {
            signals.push(StrategySignal {
                signal_type: SignalType::Info, price,
                message: "⚠️ Status 'in_position' mas sem quantidade aberta. Verifique o estado da estratégia.".into(),
                acted: false, price_change_percent: 0.0, created_at: now, source: None,
            });
            return;
        }
    };

    let entry = position.entry_price;
    if entry <= 0.0 {
        signals.push(StrategySignal {
            signal_type: SignalType::Info, price,
            message: "⚠️ Preço de entrada é 0. Não é possível calcular PnL. Verifique a posição.".into(),
            acted: false, price_change_percent: 0.0, created_at: now, source: None,
        });
        return;
    }
    let pct = ((price - entry) / entry) * 100.0;
    let trigger = config.trigger_price();
    let sl_price = config.stop_loss_price();
    let unrealized_pnl = (price - entry) * position.quantity;

    if price >= trigger {
        if config.gradual_sell && !config.gradual_lots.is_empty() {
            let lot = config.gradual_lots.iter().find(|l| !l.executed);
            if let Some(lot) = lot {
                signals.push(StrategySignal {
                    signal_type: SignalType::TakeProfit, price,
                    message: format!(
                        "🎯 TAKE PROFIT! Preço {:.2} >= trigger {:.2} ({:+.2}%). PnL não realizado: ${:.2}. Iniciando venda gradual — lote {} ({:.0}%).",
                        price, trigger, pct, unrealized_pnl, lot.lot_number, lot.sell_percent
                    ),
                    acted: false, price_change_percent: pct, created_at: now, source: None,
                });
            } else {
                signals.push(StrategySignal {
                    signal_type: SignalType::TakeProfit, price,
                    message: format!(
                        "🎯 Todos os lotes graduais executados. Vendendo posição restante ({:.6} unidades).",
                        position.quantity
                    ),
                    acted: false, price_change_percent: pct, created_at: now, source: None,
                });
            }
        } else {
            signals.push(StrategySignal {
                signal_type: SignalType::TakeProfit, price,
                message: format!(
                    "🎯 TAKE PROFIT! Preço {:.2} >= trigger {:.2} ({:+.2}%). PnL não realizado: ${:.2}. Vendendo tudo.",
                    price, trigger, pct, unrealized_pnl
                ),
                acted: false, price_change_percent: pct, created_at: now, source: None,
            });
        }
    } else if config.stop_loss_enabled && price <= sl_price {
        signals.push(StrategySignal {
            signal_type: SignalType::StopLoss, price,
            message: format!(
                "🛑 STOP LOSS! Preço {:.2} <= stop {:.2} ({:+.2}%). Perda estimada: ${:.2}. Vendendo tudo para limitar perda.",
                price, sl_price, pct, unrealized_pnl
            ),
            acted: false, price_change_percent: pct, created_at: now, source: None,
        });
    } else {
        let diff_trigger = trigger - price;
        let diff_trigger_pct = (diff_trigger / price) * 100.0;
        let diff_sl = price - sl_price;
        let diff_sl_pct = (diff_sl / price) * 100.0;
        let highest = position.highest_price;
        let drawdown = if highest > 0.0 { ((highest - price) / highest) * 100.0 } else { 0.0 };
        let sl_msg = if config.stop_loss_enabled {
            format!(" Margem até SL: {:.2} ({:.2}%).", diff_sl, diff_sl_pct)
        } else {
            " SL desativado.".to_string()
        };
        signals.push(StrategySignal {
            signal_type: SignalType::Info, price,
            message: format!(
                "📊 Em posição: {:.6} unidades, entrada {:.2}. Preço {:.2} ({:+.2}%). PnL: ${:.2}. Faltam {:.2} ({:.2}%) para TP {:.2}.{} Máxima: {:.2} (drawdown: {:.2}%).",
                position.quantity, entry, price, pct, unrealized_pnl,
                diff_trigger, diff_trigger_pct, trigger,
                sl_msg,
                highest, drawdown
            ),
            acted: false, price_change_percent: pct, created_at: now, source: None,
        });
    }
}

fn evaluate_gradual(strategy: &StrategyItem, price: f64, now: i64, signals: &mut Vec<StrategySignal>) {
    let config = &strategy.config;
    let position = match &strategy.position {
        Some(pos) if pos.quantity > 0.0 => pos,
        _ => {
            signals.push(StrategySignal {
                signal_type: SignalType::Info, price,
                message: "⚠️ Status 'gradual_selling' mas sem posição aberta. Todos os lotes podem já ter sido vendidos.".into(),
                acted: false, price_change_percent: 0.0, created_at: now, source: None,
            });
            return;
        }
    };

    let entry = position.entry_price;
    if entry <= 0.0 {
        signals.push(StrategySignal {
            signal_type: SignalType::Info, price,
            message: "⚠️ Preço de entrada é 0 durante venda gradual. Verifique a posição.".into(),
            acted: false, price_change_percent: 0.0, created_at: now, source: None,
        });
        return;
    }
    let pct = ((price - entry) / entry) * 100.0;
    let sl_price = config.stop_loss_price();
    let unrealized_pnl = (price - entry) * position.quantity;
    let executed_lots = config.gradual_lots.iter().filter(|l| l.executed).count();
    let total_lots = config.gradual_lots.len();

    if config.stop_loss_enabled && price <= sl_price {
        signals.push(StrategySignal {
            signal_type: SignalType::StopLoss, price,
            message: format!(
                "🛑 STOP LOSS durante venda gradual! Preço {:.2} <= stop {:.2} ({:+.2}%). {}/{} lotes vendidos. Vendendo posição restante ({:.6}) para limitar perda.",
                price, sl_price, pct, executed_lots, total_lots, position.quantity
            ),
            acted: false, price_change_percent: pct, created_at: now, source: None,
        });
        return;
    }

    let timer_secs = config.timer_gradual_min * 60;
    let last_sell = strategy.last_gradual_sell_at.unwrap_or(0);
    if now - last_sell < timer_secs {
        let remaining_secs = timer_secs - (now - last_sell);
        let remaining_min = remaining_secs / 60;
        let remaining_sec = remaining_secs % 60;
        signals.push(StrategySignal {
            signal_type: SignalType::Info, price,
            message: format!(
                "⏱️ Timer gradual ativo: próximo lote em {}min {}s. Preço {:.2} ({:+.2}%). PnL: ${:.2}. Progresso: {}/{} lotes vendidos.",
                remaining_min, remaining_sec, price, pct, unrealized_pnl, executed_lots, total_lots
            ),
            acted: false, price_change_percent: pct, created_at: now, source: None,
        });
        return;
    }

    let next_lot_idx = config.gradual_lots.iter().position(|l| !l.executed);
    match next_lot_idx {
        Some(idx) => {
            let lot = &config.gradual_lots[idx];
            let gradual_trigger = config.gradual_trigger_price(idx);
            if price >= gradual_trigger {
                let sell_qty = (position.total_cost / position.entry_price * lot.sell_percent / 100.0).min(position.quantity);
                signals.push(StrategySignal {
                    signal_type: SignalType::GradualSell, price,
                    message: format!(
                        "📈 VENDA GRADUAL! Lote {} de {}: preço {:.2} >= trigger gradual {:.2}. Vendendo {:.0}% ({:.6} unidades). Progresso: {}/{} lotes.",
                        lot.lot_number, total_lots, price, gradual_trigger, lot.sell_percent, sell_qty, executed_lots, total_lots
                    ),
                    acted: false, price_change_percent: pct, created_at: now, source: None,
                });
            } else {
                let diff = gradual_trigger - price;
                let diff_pct = (diff / price) * 100.0;
                signals.push(StrategySignal {
                    signal_type: SignalType::Info, price,
                    message: format!(
                        "⏳ Aguardando lote {} de {}: preço {:.2} < trigger gradual {:.2}. Faltam {:.2} ({:.2}%) para acionar. PnL: ${:.2}. Timer: pronto. Progresso: {}/{} lotes.",
                        lot.lot_number, total_lots, price, gradual_trigger, diff, diff_pct, unrealized_pnl, executed_lots, total_lots
                    ),
                    acted: false, price_change_percent: pct, created_at: now, source: None,
                });
            }
        }
        None => {
            signals.push(StrategySignal {
                signal_type: SignalType::TakeProfit, price,
                message: format!(
                    "✅ Todos os {} lotes graduais executados! Vendendo posição restante ({:.6} unidades) a {:.2}.",
                    total_lots, position.quantity, price
                ),
                acted: false, price_change_percent: pct, created_at: now, source: None,
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

/// Classify raw CCXT/exchange errors into user-friendly messages
fn classify_order_error(raw: &str, symbol: &str, exchange_name: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("insufficient") || lower.contains("balance") || lower.contains("not enough") {
        format!("Insufficient balance on {} to sell {}. Check your exchange balance.", exchange_name, symbol)
    } else if lower.contains("minimum") || lower.contains("min order") || lower.contains("too small") {
        format!("Order amount too small for {} on {}. Minimum order size not met.", symbol, exchange_name)
    } else if lower.contains("authentication") || lower.contains("invalid api") || lower.contains("apikey") {
        format!("API authentication failed on {}. Your API keys may be expired or invalid.", exchange_name)
    } else if lower.contains("permission") || lower.contains("not allowed") || lower.contains("restricted") {
        format!("API key lacks trade permission on {}. Enable spot trading in your API settings.", exchange_name)
    } else if lower.contains("rate limit") || lower.contains("too many") {
        format!("Rate limited by {}. Will retry on next tick.", exchange_name)
    } else if lower.contains("network") || lower.contains("timeout") || lower.contains("connection") {
        format!("Network error connecting to {}. Will retry on next tick.", exchange_name)
    } else if lower.contains("not found") || lower.contains("bad symbol") || lower.contains("invalid symbol") {
        format!("Trading pair '{}' not found on {}. It may have been delisted.", symbol, exchange_name)
    } else if lower.contains("market closed") || lower.contains("maintenance") {
        format!("{} market is closed or under maintenance. Will retry when available.", exchange_name)
    } else if lower.contains("ip") || lower.contains("whitelist") {
        format!("IP not whitelisted on {} API. Add the server IP to your API key whitelist.", exchange_name)
    } else {
        format!("Order failed on {}: {}", exchange_name, raw)
    }
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
    db: &MongoDB, user_id: &str, strategy: &StrategyItem, result: &TickResult, manual: bool,
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

    // ── Persist signals ─────────────────────────────────────────────
    // When automatic (monitor), only save actionable signals (TP, SL, GradualSell, Expired)
    // to avoid inflating MongoDB with "monitoring..." info logs every 30s.
    // When manual (user clicked Tick), save ALL signals including Info.
    if !result.signals.is_empty() {
        let source = if manual { "user" } else { "system" };
        let signals_to_save: Vec<StrategySignal> = if manual {
            result.signals.iter().map(|s| {
                let mut sig = s.clone();
                sig.source = Some(source.to_string());
                sig
            }).collect()
        } else {
            result.signals.iter()
                .filter(|s| !matches!(s.signal_type, SignalType::Info))
                .map(|s| {
                    let mut sig = s.clone();
                    sig.source = Some(source.to_string());
                    sig
                }).collect()
        };
        let signals_bson: Vec<mongodb::bson::Bson> = signals_to_save.iter()
            .filter_map(|s| mongodb::bson::to_bson(s).ok()).collect();
        if !signals_bson.is_empty() {
            let _ = collection.update_one(
                doc! { "user_id": user_id },
                doc! { "$push": { format!("{}.signals", p): { "$each": signals_bson, "$slice": -100 } } },
            ).array_filters(vec![array_filter.clone()]).await;
        }
    }

    if !result.executions.is_empty() {
        let source = if manual { "user" } else { "system" };
        let execs_with_source: Vec<StrategyExecution> = result.executions.iter().map(|e| {
            let mut exec = e.clone();
            exec.source = Some(source.to_string());
            exec
        }).collect();
        let execs_bson: Vec<mongodb::bson::Bson> = execs_with_source.iter()
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

    // ── Pre-check: find the strategy and validate ───────────────────
    let user_doc = collection.find_one(doc! { "user_id": user_id }).await
        .map_err(|e| format!("Failed to access database: {}", e))?
        .ok_or_else(|| "No strategies found for your account.".to_string())?;

    let strategy = user_doc.strategies.iter()
        .find(|s| s.strategy_id == strategy_id)
        .ok_or_else(|| "Strategy not found. It may have been deleted.".to_string())?;

    if strategy.is_active && strategy.status == StrategyStatus::Monitoring {
        return Err(format!("Strategy '{}' is already active and monitoring.", strategy.name));
    }

    if strategy.config.base_price <= 0.0 {
        return Err("Cannot activate: base price is 0 or invalid. Update the strategy configuration first.".to_string());
    }

    let now = chrono::Utc::now().timestamp();
    let p = "strategies.$[elem]";

    log::info!("▶️ Activating strategy '{}' ({}) for user {} — resetting started_at for new expiration cycle", strategy.name, strategy_id, user_id);

    // Reset started_at so the expiration timer starts fresh.
    // Without this, a previously expired strategy would immediately expire again
    // because is_expired() checks (now - started_at) >= time_execution_min * 60.
    collection.update_one(
        doc! { "user_id": user_id },
        doc! { "$set": {
            format!("{}.status", p): "monitoring",
            format!("{}.is_active", p): true,
            format!("{}.started_at", p): now,
            format!("{}.error_message", p): mongodb::bson::Bson::Null,
            format!("{}.updated_at", p): now,
            "updated_at": now,
        }},
    ).array_filters(vec![doc! { "elem.strategy_id": strategy_id }]).await
        .map_err(|e| format!("Failed to activate strategy: {}", e))?;

    let user_doc = collection.find_one(doc! { "user_id": user_id }).await
        .map_err(|e| format!("Failed to fetch updated strategy: {}", e))?
        .ok_or_else(|| "Strategy activated but failed to retrieve updated data.".to_string())?;

    user_doc.strategies.into_iter()
        .find(|s| s.strategy_id == strategy_id)
        .ok_or_else(|| "Strategy activated but not found in response.".to_string())
}

pub async fn pause_strategy(db: &MongoDB, strategy_id: &str, user_id: &str) -> Result<StrategyItem, String> {
    let collection = db.collection::<UserStrategies>(COLLECTION);

    // ── Pre-check ───────────────────────────────────────────────────
    let user_doc = collection.find_one(doc! { "user_id": user_id }).await
        .map_err(|e| format!("Failed to access database: {}", e))?
        .ok_or_else(|| "No strategies found for your account.".to_string())?;

    let strategy = user_doc.strategies.iter()
        .find(|s| s.strategy_id == strategy_id)
        .ok_or_else(|| "Strategy not found. It may have been deleted.".to_string())?;

    if !strategy.is_active || strategy.status == StrategyStatus::Paused {
        return Err(format!("Strategy '{}' is already paused.", strategy.name));
    }

    match strategy.status {
        StrategyStatus::Completed | StrategyStatus::StoppedOut | StrategyStatus::Expired => {
            return Err(format!(
                "Cannot pause strategy '{}' — it is already in terminal state '{}'. Create a new strategy instead.",
                strategy.name, strategy.status
            ));
        }
        _ => {}
    }

    let now = chrono::Utc::now().timestamp();
    let p = "strategies.$[elem]";

    log::info!("⏸️ Pausing strategy '{}' ({}) for user {}", strategy.name, strategy_id, user_id);

    collection.update_one(
        doc! { "user_id": user_id },
        doc! { "$set": {
            format!("{}.status", p): "paused",
            format!("{}.is_active", p): false,
            format!("{}.updated_at", p): now,
            "updated_at": now,
        }},
    ).array_filters(vec![doc! { "elem.strategy_id": strategy_id }]).await
        .map_err(|e| format!("Failed to pause strategy: {}", e))?;

    let user_doc = collection.find_one(doc! { "user_id": user_id }).await
        .map_err(|e| format!("Failed to fetch updated strategy: {}", e))?
        .ok_or_else(|| "Strategy paused but failed to retrieve updated data.".to_string())?;

    user_doc.strategies.into_iter()
        .find(|s| s.strategy_id == strategy_id)
        .ok_or_else(|| "Strategy paused but not found in response.".to_string())
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

                    match persist_tick_result(db, &user_id, strategy, &tick_result, false).await {
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
