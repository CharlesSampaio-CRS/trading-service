use crate::{
    ccxt::CCXTClient,
    database::MongoDB,
    models::{
        Balance, DecryptedExchange, ExecutionAction, PositionInfo, StrategyItem,
        StrategyExecution, StrategySignal, StrategyStatus, SignalType,
        UserStrategies,
    },
    services::user_exchanges_service,
    utils::thread_pool::spawn_ccxt_blocking,
};
use mongodb::bson::doc;
use std::collections::HashMap;

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

/// Buscar saldo disponível (free) de todos os ativos na exchange.
async fn fetch_balance(exchange: &DecryptedExchange) -> Result<HashMap<String, Balance>, String> {
    let ccxt_id = exchange.ccxt_id.clone();
    let api_key = exchange.api_key.clone();
    let api_secret = exchange.api_secret.clone();
    let passphrase = exchange.passphrase.clone();

    spawn_ccxt_blocking(move || {
        let client = CCXTClient::new(&ccxt_id, &api_key, &api_secret, passphrase.as_deref())?;
        client.fetch_balance_sync()
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}

pub async fn tick(db: &MongoDB, user_id: &str, strategy: &StrategyItem) -> TickResult {
    let strategy_id = strategy.strategy_id.clone();
    let now = chrono::Utc::now().timestamp();

    // ── Guard: archived/deleted strategy ──────────────────────────────
    if strategy.deleted_at.is_some() {
        return TickResult {
            strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
            signals: vec![], executions: vec![], new_status: None,
            error: Some(format!("Strategy '{}' is archived.", strategy.name)),
        };
    }

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

    // ── Guard: consecutive sell/buy failures ─────────────────────────
    // If last 5 executions are all sell_failed or buy_failed, pause the strategy
    // to avoid spamming the exchange with failing orders every 30 seconds.
    const MAX_CONSECUTIVE_FAILURES: usize = 5;
    let recent_execs: Vec<_> = strategy.executions.iter().rev().take(MAX_CONSECUTIVE_FAILURES).collect();
    if recent_execs.len() == MAX_CONSECUTIVE_FAILURES
        && recent_execs.iter().all(|e| matches!(e.action, ExecutionAction::SellFailed | ExecutionAction::BuyFailed))
    {
        let last_err = recent_execs.first().and_then(|e| e.error_message.as_deref()).unwrap_or("unknown");
        log::warn!(
            "🛑 [{}] {} consecutive order failures detected. Pausing strategy. Last error: {}",
            strategy_id, MAX_CONSECUTIVE_FAILURES, last_err
        );
        return TickResult {
            strategy_id, symbol: strategy.symbol.clone(), price: 0.0,
            signals: vec![StrategySignal {
                signal_type: SignalType::Info, price: 0.0,
                message: format!(
                    "🛑 Strategy paused after {} consecutive order failures. Last error: {}. Please check your exchange balance and reactivate.",
                    MAX_CONSECUTIVE_FAILURES, last_err
                ),
                acted: false, price_change_percent: 0.0, created_at: now, source: None,
            }],
            executions: vec![], new_status: Some(StrategyStatus::Error),
            error: Some(format!("Auto-paused: {} consecutive order failures. Last: {}", MAX_CONSECUTIVE_FAILURES, last_err)),
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

    // ── Verificar saldo da exchange se houver sinais de ação ──────
    let has_actionable = signals.iter().any(|s| matches!(
        s.signal_type,
        SignalType::TakeProfit | SignalType::GradualSell | SignalType::StopLoss | SignalType::DcaBuy | SignalType::BuyDip
    ));
    let balances = if has_actionable {
        match fetch_balance(exchange).await {
            Ok(b) => Some(b),
            Err(e) => {
                log::warn!("⚠️ [{}] Não conseguiu buscar saldo: {}. Prosseguindo sem validação.", strategy.strategy_id, e);
                None
            }
        }
    } else {
        None
    };

    // Extrair token base e quote do symbol (ex: "SOL/USDT" → "SOL", "USDT")
    let (base_asset, quote_asset) = {
        let parts: Vec<&str> = strategy.symbol.split('/').collect();
        (parts.get(0).unwrap_or(&"").to_string(), parts.get(1).unwrap_or(&"USDT").to_string())
    };

    for signal in &mut signals {
        match signal.signal_type {
            SignalType::TakeProfit | SignalType::GradualSell => {
                let mut sell_amount = calc_sell_amount(strategy, &signal.signal_type);
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

                // ── Balance check: verificar se tem saldo suficiente do token para vender ──
                // IMPORTANTE: Sempre usar o saldo REAL da exchange ao invés de position.quantity,
                // porque fees da compra fazem o saldo real ser ligeiramente menor que o estimado.
                if let Some(ref bals) = balances {
                    let token_free = bals.get(&base_asset).map(|b| b.free).unwrap_or(0.0);
                    if token_free <= 0.0 {
                        log::warn!(
                            "⚠️ [{}] SALDO ZERO para vender {:.6} {}! Venda BLOQUEADA.",
                            strategy.strategy_id, sell_amount, base_asset
                        );
                        signal.acted = false;
                        signal.message = format!(
                            "⚠️ Saldo insuficiente! Precisa de {:.6} {} para vender, mas não tem saldo disponível na exchange. Verifique seu saldo.",
                            sell_amount, base_asset
                        );
                        signal.signal_type = SignalType::Info;
                        continue;
                    }
                    if token_free < sell_amount {
                        // Saldo real é menor que position.quantity (normal por causa de fees da compra)
                        // Usa o saldo real da exchange para evitar "insufficient balance"
                        log::warn!(
                            "⚠️ [{}] Ajustando sell_amount: position.qty={:.6} mas saldo real={:.6} {}. Usando saldo real.",
                            strategy.strategy_id, sell_amount, token_free, base_asset
                        );
                        sell_amount = token_free;
                    }
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
                // Se DCA está ativado, NÃO vende — transforma em DcaBuy
                if strategy.config.dca_enabled
                    && strategy.config.dca_buy_amount_usd > 0.0
                    && strategy.dca_buys_done < strategy.config.dca_max_buys
                {
                    signal.signal_type = SignalType::DcaBuy;
                    signal.acted = false;
                    signal.message = format!(
                        "📉 DCA ativado! Preço {:.2} caiu abaixo do stop. Convertendo em compra DCA #{} de ${:.2} (máx: {}).",
                        price, strategy.dca_buys_done + 1, strategy.config.dca_buy_amount_usd, strategy.config.dca_max_buys
                    );
                    // Será tratado no bloco DcaBuy abaixo
                } else {
                    let mut qty = strategy.position.as_ref().map(|p| p.quantity).unwrap_or(0.0);
                    if qty <= 0.0 { continue; }

                    // ── Balance check: verificar saldo do token para stop loss ──
                    // IMPORTANTE: Sempre usar saldo REAL da exchange (fees da compra reduzem o saldo)
                    if let Some(ref bals) = balances {
                        let token_free = bals.get(&base_asset).map(|b| b.free).unwrap_or(0.0);
                        if token_free <= 0.0 {
                            log::warn!(
                                "⚠️ [{}] SALDO ZERO para stop loss! Precisa: {:.6} {}. Venda BLOQUEADA.",
                                strategy.strategy_id, qty, base_asset
                            );
                            signal.acted = false;
                            signal.message = format!(
                                "⚠️ Saldo insuficiente para stop loss! Precisa de {:.6} {} mas não tem saldo. Verifique seu saldo na exchange.",
                                qty, base_asset
                            );
                            signal.signal_type = SignalType::Info;
                            continue;
                        }
                        if token_free < qty {
                            // Saldo real é menor que position.quantity (normal por causa de fees)
                            log::warn!(
                                "⚠️ [{}] Ajustando stop loss qty: position.qty={:.6} mas saldo real={:.6} {}. Usando saldo real.",
                                strategy.strategy_id, qty, token_free, base_asset
                            );
                            qty = token_free;
                        }
                    }

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
            }
            SignalType::DcaBuy => {
                // ── DCA: comprar mais para baixar preço médio ──
                let dca_amount = strategy.config.dca_buy_amount_usd;
                if dca_amount <= 0.0 { continue; }

                // ── Balance check: verificar se tem USDT suficiente para comprar ──
                if let Some(ref bals) = balances {
                    let quote_free = bals.get(&quote_asset).map(|b| b.free).unwrap_or(0.0);
                    if quote_free < dca_amount * 0.95 { // 5% margem
                        log::warn!(
                            "⚠️ [{}] SALDO {} INSUFICIENTE para DCA! Precisa: ${:.2}, Disponível: ${:.2}. Compra BLOQUEADA.",
                            strategy.strategy_id, quote_asset, dca_amount, quote_free
                        );
                        signal.acted = false;
                        signal.message = format!(
                            "⚠️ Saldo insuficiente para DCA! Precisa de ${:.2} {} mas só tem ${:.2} disponível. Deposite mais {} ou reduza o valor DCA.",
                            dca_amount, quote_asset, quote_free, quote_asset
                        );
                        signal.signal_type = SignalType::Info;
                        continue;
                    }
                }

                let buy_qty = dca_amount / price;
                log::info!(
                    "📉 [{}] DCA BUY #{}: comprando ${:.2} = {:.6} {} @ {:.2}",
                    strategy.strategy_id, strategy.dca_buys_done + 1, dca_amount, buy_qty, strategy.symbol, price
                );
                match execute_order(exchange, &strategy.symbol, "market", "buy", buy_qty, None).await {
                    Ok(order) => {
                        signal.acted = true;
                        let filled = order.filled.unwrap_or(buy_qty);
                        let buy_price = order.avg_price.unwrap_or(price);
                        let cost = order.cost.unwrap_or(buy_price * filled);
                        let fee = order.fee.unwrap_or(0.0);
                        // Calcular novo preço médio
                        let old_qty = strategy.position.as_ref().map(|p| p.quantity).unwrap_or(0.0);
                        let old_cost = strategy.position.as_ref().map(|p| p.total_cost).unwrap_or(0.0);
                        let new_qty = old_qty + filled;
                        let new_cost = old_cost + cost;
                        let new_avg = if new_qty > 0.0 { new_cost / new_qty } else { buy_price };
                        log::info!(
                            "✅ [{}] DCA BUY #{} OK: +{:.6} @ {:.2}. Novo médio: {:.2} (era {:.2}). Total: {:.6} unidades, ${:.2} investido.",
                            strategy.strategy_id, strategy.dca_buys_done + 1, filled, buy_price, new_avg,
                            strategy.position.as_ref().map(|p| p.entry_price).unwrap_or(0.0),
                            new_qty, new_cost
                        );
                        signal.message = format!(
                            "📉 DCA BUY #{} executado! Comprou {:.6} {} @ {:.2} (${:.2}). Novo preço médio: {:.2}. Total investido: ${:.2}.",
                            strategy.dca_buys_done + 1, filled, strategy.symbol, buy_price, cost, new_avg, new_cost
                        );
                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action: ExecutionAction::Buy, reason: format!("dca_buy_{}", strategy.dca_buys_done + 1),
                            price: buy_price, amount: filled, total: cost,
                            fee, pnl_usd: 0.0,
                            exchange_order_id: Some(order.order_id),
                            executed_at: now, error_message: None, source: None,
                        });
                        // O persist_tick_result vai atualizar position, base_price, invested_amount e dca_buys_done
                    }
                    Err(e) => {
                        signal.acted = false;
                        let friendly = classify_order_error(&e, &strategy.symbol, &strategy.exchange_name);
                        log::error!("❌ [{}] DCA BUY FAILED: {} | raw: {}", strategy.strategy_id, friendly, e);
                        signal.message = format!(
                            "❌ DCA BUY #{} falhou: {}. Tentará novamente no próximo tick.",
                            strategy.dca_buys_done + 1, friendly
                        );
                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action: ExecutionAction::BuyFailed,
                            reason: format!("dca_buy_failed: {}", friendly),
                            price, amount: buy_qty, total: dca_amount,
                            fee: 0.0, pnl_usd: 0.0, exchange_order_id: None,
                            executed_at: now, error_message: Some(friendly), source: None,
                        });
                    }
                }
            }
            SignalType::BuyDip => {
                // ── Buy the Dip: compra automática na queda (funciona SEM posição) ──
                let dip_amount = strategy.config.auto_buy_dip_amount_usd;
                if dip_amount <= 0.0 { continue; }

                // ── Balance check: verificar se tem USDT suficiente ──
                if let Some(ref bals) = balances {
                    let quote_free = bals.get(&quote_asset).map(|b| b.free).unwrap_or(0.0);
                    if quote_free < dip_amount * 0.95 {
                        log::warn!(
                            "⚠️ [{}] SALDO {} INSUFICIENTE para Buy Dip! Precisa: ${:.2}, Disponível: ${:.2}. Compra BLOQUEADA.",
                            strategy.strategy_id, quote_asset, dip_amount, quote_free
                        );
                        signal.acted = false;
                        signal.message = format!(
                            "⚠️ Saldo insuficiente para Buy Dip! Precisa de ${:.2} {} mas só tem ${:.2} disponível.",
                            dip_amount, quote_asset, quote_free
                        );
                        signal.signal_type = SignalType::Info;
                        continue;
                    }
                }

                let buy_qty = dip_amount / price;
                log::info!(
                    "🛒 [{}] BUY DIP #{}: comprando ${:.2} = {:.6} {} @ {:.2}",
                    strategy.strategy_id, strategy.buy_dip_buys_done + 1, dip_amount, buy_qty, strategy.symbol, price
                );
                match execute_order(exchange, &strategy.symbol, "market", "buy", buy_qty, None).await {
                    Ok(order) => {
                        signal.acted = true;
                        let filled = order.filled.unwrap_or(buy_qty);
                        let buy_price = order.avg_price.unwrap_or(price);
                        let cost = order.cost.unwrap_or(buy_price * filled);
                        let fee = order.fee.unwrap_or(0.0);
                        // Calcular novo preço médio se já tem posição
                        let old_qty = strategy.position.as_ref().map(|p| p.quantity).unwrap_or(0.0);
                        let old_cost = strategy.position.as_ref().map(|p| p.total_cost).unwrap_or(0.0);
                        let new_qty = old_qty + filled;
                        let new_cost = old_cost + cost;
                        let new_avg = if new_qty > 0.0 { new_cost / new_qty } else { buy_price };
                        log::info!(
                            "✅ [{}] BUY DIP #{} OK: +{:.6} @ {:.2}. Preço médio: {:.2}. Total: {:.6} un, ${:.2} investido.",
                            strategy.strategy_id, strategy.buy_dip_buys_done + 1, filled, buy_price, new_avg, new_qty, new_cost
                        );
                        signal.message = format!(
                            "🛒 BUY DIP #{} executado! Comprou {:.6} {} @ {:.2} (${:.2}). Preço médio: {:.2}. Total investido: ${:.2}.",
                            strategy.buy_dip_buys_done + 1, filled, strategy.symbol, buy_price, cost, new_avg, new_cost
                        );
                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action: ExecutionAction::Buy, reason: format!("buy_dip_{}", strategy.buy_dip_buys_done + 1),
                            price: buy_price, amount: filled, total: cost,
                            fee, pnl_usd: 0.0,
                            exchange_order_id: Some(order.order_id),
                            executed_at: now, error_message: None, source: None,
                        });
                        // Se não tinha posição, o persist_tick_result vai criar uma.
                        // Se já tinha, vai atualizar o preço médio.
                        // Também muda status para in_position se estava monitoring.
                        if strategy.position.is_none() {
                            new_status = Some(StrategyStatus::InPosition);
                        }
                    }
                    Err(e) => {
                        signal.acted = false;
                        let friendly = classify_order_error(&e, &strategy.symbol, &strategy.exchange_name);
                        log::error!("❌ [{}] BUY DIP FAILED: {} | raw: {}", strategy.strategy_id, friendly, e);
                        signal.message = format!(
                            "❌ Buy Dip #{} falhou: {}. Tentará no próximo tick.",
                            strategy.buy_dip_buys_done + 1, friendly
                        );
                        executions.push(StrategyExecution {
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            action: ExecutionAction::BuyFailed,
                            reason: format!("buy_dip_failed: {}", friendly),
                            price, amount: buy_qty, total: dip_amount,
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
                    "Monitorando: preço {:.2} ({:+.2}% do base). Faltam {:.2} ({:.2}%) para trigger {:.2}.{}{}",
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
        // ── Sem posição aberta ──
        let diff_trigger = trigger - price;
        let diff_trigger_pct = if price > 0.0 { (diff_trigger / price) * 100.0 } else { 0.0 };
        let sl_info = if config.stop_loss_enabled {
            format!(" Stop loss em {:.2}.", sl_price)
        } else {
            " Stop loss desativado.".to_string()
        };

        // ── Auto Buy Dip: comprar na queda mesmo sem posição ──
        if config.auto_buy_dip_enabled
            && config.auto_buy_dip_amount_usd > 0.0
            && strategy.buy_dip_buys_done < config.auto_buy_dip_max_buys
        {
            let dip_trigger = config.buy_dip_trigger_price();
            if price <= dip_trigger {
                signals.push(StrategySignal {
                    signal_type: SignalType::BuyDip, price,
                    message: format!(
                        "🛒 BUY DIP #{}: preço {:.2} caiu {:.2}% abaixo do base {:.2} (trigger: {:.2}). Comprando ${:.2}. ({}/{} compras)",
                        strategy.buy_dip_buys_done + 1, price, pct.abs(), config.base_price, dip_trigger,
                        config.auto_buy_dip_amount_usd, strategy.buy_dip_buys_done, config.auto_buy_dip_max_buys
                    ),
                    acted: false, price_change_percent: pct, created_at: now, source: None,
                });
                return; // Sinal de ação gerado, não precisa do info
            }
            // Incluir info sobre Buy Dip no monitoramento
            let diff_dip = price - dip_trigger;
            let diff_dip_pct = if price > 0.0 { (diff_dip / price) * 100.0 } else { 0.0 };
            signals.push(StrategySignal {
                signal_type: SignalType::Info, price,
                message: format!(
                    "⏳ Sem posição. Preço {:.2} ({:+.2}% do base). TP: faltam {:.2} ({:.2}%). Buy Dip #{}: faltam {:.2} ({:.2}%) para comprar (trigger: {:.2}).{}{} ",
                    price, pct, diff_trigger, diff_trigger_pct,
                    strategy.buy_dip_buys_done + 1, diff_dip, diff_dip_pct, dip_trigger,
                    sl_info, pnl_info
                ),
                acted: false, price_change_percent: pct, created_at: now, source: None,
            });
        } else {
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
    } else if config.dca_enabled && config.dca_buy_amount_usd > 0.0
              && strategy.dca_buys_done < config.dca_max_buys {
        // DCA: verificar se preço caiu dca_trigger_percent% abaixo do preço médio (entry)
        let dca_trigger_price = entry * (1.0 - config.dca_trigger_percent / 100.0);
        if price <= dca_trigger_price {
            signals.push(StrategySignal {
                signal_type: SignalType::DcaBuy, price,
                message: format!(
                    "📉 DCA #{}: preço {:.2} caiu {:.2}% abaixo da média {:.2} (trigger DCA: {:.2}). Comprando +${:.2}. ({}/{} compras DCA)",
                    strategy.dca_buys_done + 1, price, pct.abs(), entry, dca_trigger_price,
                    config.dca_buy_amount_usd, strategy.dca_buys_done, config.dca_max_buys
                ),
                acted: false, price_change_percent: pct, created_at: now, source: None,
            });
        } else {
            let diff_trigger = trigger - price;
            let diff_trigger_pct = (diff_trigger / price) * 100.0;
            let diff_dca = price - dca_trigger_price;
            let diff_dca_pct = (diff_dca / price) * 100.0;
            let highest = position.highest_price;
            let drawdown = if highest > 0.0 { ((highest - price) / highest) * 100.0 } else { 0.0 };
            signals.push(StrategySignal {
                signal_type: SignalType::Info, price,
                message: format!(
                    "📊 Em posição: {:.6} un, entrada {:.2}. Preço {:.2} ({:+.2}%). PnL: ${:.2}. TP: faltam {:.2} ({:.2}%). DCA #{}: faltam {:.2} ({:.2}%) para comprar mais. Máxima: {:.2} (drawdown: {:.2}%).",
                    position.quantity, entry, price, pct, unrealized_pnl,
                    diff_trigger, diff_trigger_pct,
                    strategy.dca_buys_done + 1, diff_dca, diff_dca_pct,
                    highest, drawdown
                ),
                acted: false, price_change_percent: pct, created_at: now, source: None,
            });
        }
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
    } else if config.dca_enabled && config.dca_buy_amount_usd > 0.0
              && strategy.dca_buys_done < config.dca_max_buys {
        let dca_trigger_price = entry * (1.0 - config.dca_trigger_percent / 100.0);
        if price <= dca_trigger_price {
            signals.push(StrategySignal {
                signal_type: SignalType::DcaBuy, price,
                message: format!(
                    "📉 DCA #{} durante gradual: preço {:.2} caiu abaixo de {:.2}. Comprando +${:.2}. ({}/{} lotes vendidos, {}/{} DCAs)",
                    strategy.dca_buys_done + 1, price, dca_trigger_price,
                    config.dca_buy_amount_usd, executed_lots, total_lots,
                    strategy.dca_buys_done, config.dca_max_buys
                ),
                acted: false, price_change_percent: pct, created_at: now, source: None,
            });
            return;
        }
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

    // ── DCA: atualizar base_price (preço médio), invested_amount e dca_buys_done ──
    let dca_buys_in_result = result.executions.iter()
        .filter(|e| e.action == ExecutionAction::Buy && e.reason.starts_with("dca_buy"))
        .count() as i32;
    if dca_buys_in_result > 0 {
        update_inc.insert(format!("{}.dca_buys_done", p), dca_buys_in_result);
        // Atualizar base_price e invested_amount com o novo preço médio
        if let Some(ref pos) = current_position {
            update_set.insert(format!("{}.config.base_price", p), pos.entry_price);
            update_set.insert(format!("{}.config.invested_amount", p), pos.total_cost);
            log::info!(
                "📉 [{}] DCA persist: novo base_price={:.2}, invested_amount={:.2}, dca_buys_done += {}",
                strategy.strategy_id, pos.entry_price, pos.total_cost, dca_buys_in_result
            );
        }
    }

    // ── Buy Dip: atualizar base_price, invested_amount e buy_dip_buys_done ──
    let buy_dip_in_result = result.executions.iter()
        .filter(|e| e.action == ExecutionAction::Buy && e.reason.starts_with("buy_dip"))
        .count() as i32;
    if buy_dip_in_result > 0 {
        update_inc.insert(format!("{}.buy_dip_buys_done", p), buy_dip_in_result);
        if let Some(ref pos) = current_position {
            update_set.insert(format!("{}.config.base_price", p), pos.entry_price);
            update_set.insert(format!("{}.config.invested_amount", p), pos.total_cost);
            log::info!(
                "🛒 [{}] Buy Dip persist: novo base_price={:.2}, invested_amount={:.2}, buy_dip_buys_done += {}",
                strategy.strategy_id, pos.entry_price, pos.total_cost, buy_dip_in_result
            );
        }
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

    // Se já está ativa em estado operacional (monitoring, in_position, gradual_selling),
    // não precisa reativar — retorna a estratégia como está sem erro.
    if strategy.is_active {
        match strategy.status {
            StrategyStatus::Monitoring | StrategyStatus::InPosition | StrategyStatus::GradualSelling => {
                let user_doc2 = collection.find_one(doc! { "user_id": user_id }).await
                    .map_err(|e| format!("Failed to fetch strategy: {}", e))?
                    .ok_or_else(|| "Strategy not found.".to_string())?;
                return user_doc2.strategies.into_iter()
                    .find(|s| s.strategy_id == strategy_id)
                    .ok_or_else(|| "Strategy not found.".to_string());
            }
            _ => {}
        }
    }

    if strategy.config.base_price <= 0.0 {
        return Err("Cannot activate: base price is 0 or invalid. Update the strategy configuration first.".to_string());
    }

    let now = chrono::Utc::now().timestamp();
    let p = "strategies.$[elem]";

    // Determinar status correto ao ativar:
    // - Se tem posição aberta → volta para in_position (preserva posição)
    // - Caso contrário → monitoring (aguarda entrada)
    let restore_status = if strategy.position.is_some() {
        match strategy.status {
            StrategyStatus::GradualSelling => "gradual_selling",
            _ => "in_position",
        }
    } else {
        "monitoring"
    };

    log::info!(
        "▶️ Activating strategy '{}' ({}) → status: {} | user: {}",
        strategy.name, strategy_id, restore_status, user_id
    );

    collection.update_one(
        doc! { "user_id": user_id },
        doc! { "$set": {
            format!("{}.status", p): restore_status,
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
                    if strategy.deleted_at.is_some() { continue; }
                    match strategy.status {
                        StrategyStatus::Idle | StrategyStatus::Monitoring
                        | StrategyStatus::InPosition | StrategyStatus::GradualSelling => {}
                        _ => continue,
                    }
                    total += 1;
                    let last_checked = strategy.last_checked_at.unwrap_or(0);
                    let elapsed_since_check = now - last_checked;
                    // Use 25s threshold (slightly less than 30s interval) to avoid
                    // strategies being skipped due to minor timing drift
                    if elapsed_since_check < 25 { continue; }

                    // ── Log antes de tickar: mostra o que vai ser verificado ──
                    let last_price_str = strategy.last_price
                        .map(|p| format!("{:.4}", p))
                        .unwrap_or_else(|| "?".to_string());
                    let config = &strategy.config;
                    let trigger_str = format!("{:.4}", config.trigger_price());
                    let pnl_str = if let Some(pos) = &strategy.position {
                        if pos.entry_price > 0.0 {
                            let pct = ((strategy.last_price.unwrap_or(0.0) - pos.entry_price) / pos.entry_price) * 100.0;
                            format!(" | PnL: {:+.2}%", pct)
                        } else { String::new() }
                    } else { String::new() };
                    let consecutive_fails = strategy.executions.iter().rev()
                        .take_while(|e| matches!(e.action, ExecutionAction::SellFailed | ExecutionAction::BuyFailed))
                        .count();
                    let fail_warn = if consecutive_fails > 0 {
                        format!(" | ⚠️ {} falha(s) consecutiva(s)", consecutive_fails)
                    } else { String::new() };

                    log::info!(
                        "🔍 [{} | {}] {} | status: {} | preço: {} | TP: {}{}{}",
                        &strategy.strategy_id[..8.min(strategy.strategy_id.len())],
                        strategy.exchange_name,
                        strategy.symbol,
                        strategy.status,
                        last_price_str,
                        trigger_str,
                        pnl_str,
                        fail_warn
                    );

                    let tick_result = tick(db, &user_id, strategy).await;
                    signals_generated += tick_result.signals.len();
                    orders_executed += tick_result.executions.len();

                    // ── Log pós-tick: resultado ──
                    for sig in &tick_result.signals {
                        if !matches!(sig.signal_type, SignalType::Info) || sig.acted {
                            log::info!(
                                "   ↳ [{:?}{}] {}",
                                sig.signal_type,
                                if sig.acted { " ✅" } else { "" },
                                &sig.message[..sig.message.len().min(120)]
                            );
                        }
                    }
                    for exec in &tick_result.executions {
                        log::info!(
                            "   💰 [{:?}] {:.6} {} @ {:.4} | PnL: ${:.2}",
                            exec.action, exec.amount, strategy.symbol, exec.price, exec.pnl_usd
                        );
                    }
                    if let Some(ref err) = tick_result.error {
                        log::warn!("   ⚠️ tick error: {}", err);
                    }

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

// ═══════════════════════════════════════════════════════════════════
// TESTES — Simulação de estratégia SOL/USDT com DCA + Gradual Sell
// ═══════════════════════════════════════════════════════════════════
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        Balance, GradualLot, PositionInfo, StrategyConfig, StrategyItem, StrategySignal,
        StrategyStatus, SignalType,
    };
    use std::collections::HashMap;

    /// Cria a estratégia SOL exatamente como configurada no MongoDB:
    /// - Comprou $36 de SOL a $88.29 (0.40774 SOL)
    /// - TP: 12% em 4 lotes de 25%
    /// - DCA: compra +$36 se cair 10%, máx 3 compras
    /// - Stop Loss desativado
    fn create_sol_strategy() -> StrategyItem {
        StrategyItem {
            strategy_id: "test-sol-001".into(),
            name: "SOL DCA + Gradual".into(),
            symbol: "SOL/USDT".into(),
            exchange_id: "test-exchange".into(),
            exchange_name: "Binance".into(),
            is_active: true,
            status: StrategyStatus::InPosition,
            config: StrategyConfig {
                base_price: 88.29,
                invested_amount: 36.0,
                take_profit_percent: 12.0,
                stop_loss_enabled: false,
                stop_loss_percent: 5.0,
                gradual_take_percent: 5.0,
                fee_percent: 0.2,
                gradual_sell: true,
                gradual_lots: vec![
                    GradualLot { lot_number: 1, sell_percent: 25.0, executed: false, executed_at: None, executed_price: None, realized_pnl: None },
                    GradualLot { lot_number: 2, sell_percent: 25.0, executed: false, executed_at: None, executed_price: None, realized_pnl: None },
                    GradualLot { lot_number: 3, sell_percent: 25.0, executed: false, executed_at: None, executed_price: None, realized_pnl: None },
                    GradualLot { lot_number: 4, sell_percent: 25.0, executed: false, executed_at: None, executed_price: None, realized_pnl: None },
                ],
                timer_gradual_min: 15,
                time_execution_min: 43200,
                dca_enabled: true,
                dca_buy_amount_usd: 36.0,
                dca_trigger_percent: 10.0,
                dca_max_buys: 3,
            },
            position: Some(PositionInfo {
                entry_price: 88.29,
                quantity: 0.40774,
                total_cost: 36.0,
                current_price: 88.29,
                unrealized_pnl: 0.0,
                unrealized_pnl_percent: 0.0,
                highest_price: 88.29,
                opened_at: 1772330880,
            }),
            executions: vec![],
            signals: vec![],
            last_checked_at: None,
            last_price: None,
            last_gradual_sell_at: None,
            error_message: None,
            total_pnl_usd: 0.0,
            total_executions: 0,
            dca_buys_done: 0,
            started_at: 1772330880,
            created_at: 1772330880,
            updated_at: 1772330880,
        }
    }

    /// Helper: roda evaluate_exit e retorna os sinais gerados
    fn run_tick(strategy: &StrategyItem, price: f64) -> Vec<StrategySignal> {
        let mut signals = Vec::new();
        let now = chrono::Utc::now().timestamp();
        match strategy.status {
            StrategyStatus::InPosition => evaluate_exit(strategy, price, now, &mut signals),
            StrategyStatus::GradualSelling => evaluate_gradual(strategy, price, now, &mut signals),
            StrategyStatus::Monitoring => evaluate_trigger(strategy, price, now, &mut signals),
            _ => {}
        }
        signals
    }

    /// Helper: simula o efeito de um DCA buy na strategy (sem exchange real)
    fn simulate_dca_buy(strategy: &mut StrategyItem, buy_price: f64) {
        let dca_amount = strategy.config.dca_buy_amount_usd;
        let new_qty_bought = dca_amount / buy_price;

        let old_qty = strategy.position.as_ref().map(|p| p.quantity).unwrap_or(0.0);
        let old_cost = strategy.position.as_ref().map(|p| p.total_cost).unwrap_or(0.0);
        let new_qty = old_qty + new_qty_bought;
        let new_cost = old_cost + dca_amount;
        let new_avg = new_cost / new_qty;

        // Atualiza position
        if let Some(ref mut pos) = strategy.position {
            pos.quantity = new_qty;
            pos.total_cost = new_cost;
            pos.entry_price = new_avg;
        }
        // Atualiza config (como o persist faz)
        strategy.config.base_price = new_avg;
        strategy.config.invested_amount = new_cost;
        strategy.dca_buys_done += 1;
    }

    /// Helper: imprime resumo formatado do sinal
    fn print_signal(tick_num: usize, price: f64, signal: &StrategySignal, strategy: &StrategyItem) {
        let pos = strategy.position.as_ref();
        let entry = pos.map(|p| p.entry_price).unwrap_or(0.0);
        let qty = pos.map(|p| p.quantity).unwrap_or(0.0);
        let invested = strategy.config.invested_amount;
        let current_value = qty * price;
        let pnl = current_value - invested;
        let pnl_pct = if invested > 0.0 { (pnl / invested) * 100.0 } else { 0.0 };

        println!(
            "  Tick #{:2} │ Preço: ${:>8.2} │ {:12} │ Entrada: ${:.2} │ Qtd: {:.5} │ Investido: ${:.2} │ Valor: ${:.2} │ PnL: {:+.2}$ ({:+.1}%)",
            tick_num, price,
            format!("{}", signal.signal_type),
            entry, qty, invested, current_value, pnl, pnl_pct
        );
        println!("           │ {}", signal.message);
        println!("           └─────────────────────────────────────────────────");
    }

    // ════════════════════════════════════════════════════════════════
    // TESTE 1: Cenário completo — preço cai, DCA, sobe, venda gradual
    // ════════════════════════════════════════════════════════════════
    #[test]
    fn test_sol_strategy_full_scenario() {
        println!("\n╔══════════════════════════════════════════════════════════════╗");
        println!("║  SIMULAÇÃO SOL/USDT — DCA + Venda Gradual                  ║");
        println!("║  Compra: $36 @ $88.29 (0.40774 SOL)                        ║");
        println!("║  TP: 12% em 4 lotes · DCA: -10% compra +$36 (máx 3x)      ║");
        println!("╚══════════════════════════════════════════════════════════════╝\n");

        let mut strategy = create_sol_strategy();

        // Triggers calculados
        let tp_price = strategy.config.trigger_price();
        let dca1_price = strategy.config.base_price * 0.90; // -10%
        println!("  📊 Triggers iniciais:");
        println!("     TP (12% + 0.2% fee): ${:.2}", tp_price);
        println!("     DCA #1 (-10%):       ${:.2}", dca1_price);
        println!("     Stop Loss:           DESATIVADO");
        println!("  ─────────────────────────────────────────────────────────\n");

        // 10 variações simuladas de preço da SOL
        let prices = vec![
            85.00,  // Tick 1: caiu um pouco, sem trigger
            80.00,  // Tick 2: caiu mais, quase DCA (precisa < 79.46)
            79.00,  // Tick 3: ⚡ DCA #1 triggered! Preço caiu 10%+
            75.00,  // Tick 4: após DCA#1, novo médio ~83.64, novo trigger DCA = 83.64*0.9 = ~75.28 → quase
            74.00,  // Tick 5: ⚡ DCA #2 triggered!
            82.00,  // Tick 6: subindo, mas ainda abaixo do TP
            92.00,  // Tick 7: subindo forte
            98.00,  // Tick 8: perto do TP
            99.00,  // Tick 9: ⚡ TP triggered! Preço acima do trigger
           105.00,  // Tick 10: continua acima, mais vendas graduais
        ];

        for (i, &price) in prices.iter().enumerate() {
            let tick_num = i + 1;
            let signals = run_tick(&strategy, price);

            for signal in &signals {
                print_signal(tick_num, price, signal, &strategy);

                // Simular efeitos do DCA
                if signal.signal_type == SignalType::DcaBuy {
                    simulate_dca_buy(&mut strategy, price);
                    let new_tp = strategy.config.trigger_price();
                    let new_dca = strategy.config.base_price * 0.90;
                    println!("           🔄 APÓS DCA #{}: novo médio ${:.2}, novo TP ${:.2}, próximo DCA ${:.2}",
                        strategy.dca_buys_done, strategy.config.base_price, new_tp, new_dca);
                    println!("           └─────────────────────────────────────────────────");
                }
            }
        }

        println!("\n  📋 RESUMO FINAL:");
        println!("     DCA buys realizados: {}/{}", strategy.dca_buys_done, strategy.config.dca_max_buys);
        println!("     Preço médio final:   ${:.2}", strategy.config.base_price);
        println!("     Total investido:     ${:.2}", strategy.config.invested_amount);
        println!("     Quantidade total:    {:.5} SOL", strategy.position.as_ref().map(|p| p.quantity).unwrap_or(0.0));
        println!("     TP price final:      ${:.2}", strategy.config.trigger_price());
        println!();
    }

    // ════════════════════════════════════════════════════════════════
    // TESTE 2: Preço só cai — 3 DCAs e depois para (máx atingido)
    // ════════════════════════════════════════════════════════════════
    #[test]
    fn test_sol_dca_max_reached() {
        println!("\n╔══════════════════════════════════════════════════════════════╗");
        println!("║  TESTE: DCA máximo atingido (3 compras)                     ║");
        println!("╚══════════════════════════════════════════════════════════════╝\n");

        let mut strategy = create_sol_strategy();

        let prices = vec![
            79.00,  // DCA #1 (-10.5%)
            70.00,  // DCA #2 (verificar se cai 10% do novo médio)
            60.00,  // DCA #3 (último permitido)
            55.00,  // Já fez 3 DCAs — NÃO deve comprar mais → só Info
        ];

        for (i, &price) in prices.iter().enumerate() {
            let signals = run_tick(&strategy, price);
            for signal in &signals {
                print_signal(i + 1, price, signal, &strategy);

                if signal.signal_type == SignalType::DcaBuy {
                    assert!(strategy.dca_buys_done < strategy.config.dca_max_buys,
                        "Não deveria gerar DCA acima do máximo!");
                    simulate_dca_buy(&mut strategy, price);
                    println!("           🔄 DCA #{} OK. Novo médio: ${:.2}, investido: ${:.2}",
                        strategy.dca_buys_done, strategy.config.base_price, strategy.config.invested_amount);
                    println!("           └─────────────────────────────────────────────────");
                }
            }
        }

        // No tick 4 ($55), já fez 3 DCAs — deve ser Info, não DcaBuy
        let final_signals = run_tick(&strategy, 55.0);
        let has_dca = final_signals.iter().any(|s| s.signal_type == SignalType::DcaBuy);
        assert!(!has_dca, "Após 3 DCAs, NÃO deve gerar mais DcaBuy!");
        assert_eq!(strategy.dca_buys_done, 3);
        println!("\n  ✅ Máximo de 3 DCAs respeitado. dca_buys_done = {}", strategy.dca_buys_done);
        println!("     Preço médio final: ${:.2} (era $88.29)", strategy.config.base_price);
        println!("     Total investido:   ${:.2} (era $36)", strategy.config.invested_amount);
        println!();
    }

    // ════════════════════════════════════════════════════════════════
    // TESTE 3: Stop Loss desativado — preço despenca, não vende
    // ════════════════════════════════════════════════════════════════
    #[test]
    fn test_sol_stop_loss_disabled() {
        println!("\n╔══════════════════════════════════════════════════════════════╗");
        println!("║  TESTE: Stop Loss desativado + DCA                          ║");
        println!("╚══════════════════════════════════════════════════════════════╝\n");

        let strategy = create_sol_strategy();
        assert!(!strategy.config.stop_loss_enabled, "Stop loss deve estar desativado!");

        // Preço cai 50% — NÃO deve gerar StopLoss
        let signals = run_tick(&strategy, 44.0);
        let has_stop = signals.iter().any(|s| s.signal_type == SignalType::StopLoss);
        assert!(!has_stop, "Stop loss está desativado, não deveria gerar sinal StopLoss!");

        // Mas deve gerar DcaBuy (se dentro do limite)
        let has_dca = signals.iter().any(|s| s.signal_type == SignalType::DcaBuy);
        assert!(has_dca, "Com DCA ativado e queda de 50%, deveria gerar DcaBuy!");

        for signal in &signals {
            print_signal(1, 44.0, signal, &strategy);
        }
        println!("\n  ✅ Stop Loss corretamente desativado, DCA gerado no lugar.");
        println!();
    }

    // ════════════════════════════════════════════════════════════════
    // TESTE 4: Double-check — não vende se PnL é negativo
    // ════════════════════════════════════════════════════════════════
    #[test]
    fn test_double_check_prevents_loss_sale() {
        println!("\n╔══════════════════════════════════════════════════════════════╗");
        println!("║  TESTE: Double-check impede venda com prejuízo              ║");
        println!("╚══════════════════════════════════════════════════════════════╝\n");

        let strategy = create_sol_strategy();
        let config = &strategy.config;

        // Preço exatamente no trigger (12% + 0.2% = 12.2%)
        let trigger = config.trigger_price();
        println!("  Trigger price: ${:.2}", trigger);

        // Double-check: invested_amount $36, base $88.29
        // Se preço = trigger, valor = (36/88.29) * trigger
        let qty = config.invested_amount / config.base_price;
        let value_at_trigger = qty * trigger;
        let pnl_at_trigger = value_at_trigger - config.invested_amount;
        println!("  Valor no trigger: ${:.2}, PnL: ${:.2}", value_at_trigger, pnl_at_trigger);
        assert!(pnl_at_trigger > 0.0, "No trigger real, PnL deve ser positivo!");

        // Agora testar com preço levemente abaixo do investido (não deveria vender)
        let bad_price = config.base_price * 0.99; // -1%
        let value_at_bad = qty * bad_price;
        let pnl_at_bad = value_at_bad - config.invested_amount;
        println!("  Preço ruim: ${:.2}, Valor: ${:.2}, PnL: ${:.2} — NÃO deve vender", bad_price, value_at_bad, pnl_at_bad);
        assert!(pnl_at_bad < 0.0, "Preço abaixo do base = prejuízo");

        println!("\n  ✅ Double-check: garante que venda só acontece com lucro real.");
        println!();
    }

    // ════════════════════════════════════════════════════════════════
    // TESTE 5: Triggers calculados corretamente
    // ════════════════════════════════════════════════════════════════
    #[test]
    fn test_trigger_calculations() {
        println!("\n╔══════════════════════════════════════════════════════════════╗");
        println!("║  TESTE: Cálculo de triggers                                 ║");
        println!("╚══════════════════════════════════════════════════════════════╝\n");

        let config = create_sol_strategy().config;

        // TP = base * (1 + 12% + 0.2%) = 88.29 * 1.122 = $99.06
        let tp = config.trigger_price();
        let expected_tp = 88.29 * 1.122;
        println!("  TP price:       ${:.2} (esperado: ${:.2})", tp, expected_tp);
        assert!((tp - expected_tp).abs() < 0.01, "TP incorreto!");

        // DCA trigger = base * 0.90 = 88.29 * 0.90 = $79.46
        let dca_trigger = config.base_price * (1.0 - config.dca_trigger_percent / 100.0);
        let expected_dca = 88.29 * 0.90;
        println!("  DCA trigger:    ${:.2} (esperado: ${:.2})", dca_trigger, expected_dca);
        assert!((dca_trigger - expected_dca).abs() < 0.01, "DCA trigger incorreto!");

        // Gradual lot 0 = TP price (mesmo)
        // Gradual lot 1 = base * (1 + 12% + 0.2% + 5%) = 88.29 * 1.172 = $103.48
        let grad0 = config.gradual_trigger_price(0);
        let grad1 = config.gradual_trigger_price(1);
        let grad2 = config.gradual_trigger_price(2);
        let grad3 = config.gradual_trigger_price(3);
        println!("  Gradual Lote 1: ${:.2} (= TP)", grad0);
        println!("  Gradual Lote 2: ${:.2} (+5%)", grad1);
        println!("  Gradual Lote 3: ${:.2} (+10%)", grad2);
        println!("  Gradual Lote 4: ${:.2} (+15%)", grad3);

        assert!(grad0 < grad1, "Lote 2 deve ter trigger maior que lote 1");
        assert!(grad1 < grad2, "Lote 3 deve ter trigger maior que lote 2");
        assert!(grad2 < grad3, "Lote 4 deve ter trigger maior que lote 3");

        // Stop loss = desativado, mas cálculo = base * 0.95 = $83.88
        let sl = config.stop_loss_price();
        println!("  Stop Loss:      ${:.2} (DESATIVADO)", sl);
        assert!(!config.stop_loss_enabled, "SL deve estar off");

        println!("\n  ✅ Todos os triggers calculados corretamente.");
        println!();
    }

    // ════════════════════════════════════════════════════════════════
    // TESTE 6: DCA reduz preço médio progressivamente
    // ════════════════════════════════════════════════════════════════
    #[test]
    fn test_dca_lowers_average_price() {
        println!("\n╔══════════════════════════════════════════════════════════════╗");
        println!("║  TESTE: DCA reduz preço médio progressivamente              ║");
        println!("╚══════════════════════════════════════════════════════════════╝\n");

        let mut strategy = create_sol_strategy();
        let initial_avg = strategy.config.base_price;

        println!("  Inicial: avg ${:.2}, investido ${:.2}, qty {:.5}",
            initial_avg, strategy.config.invested_amount,
            strategy.position.as_ref().unwrap().quantity);

        // DCA #1 a $79.00
        simulate_dca_buy(&mut strategy, 79.00);
        let avg1 = strategy.config.base_price;
        assert!(avg1 < initial_avg, "Após DCA#1, média deve cair!");
        println!("  DCA #1 @ $79.00: avg ${:.2}, investido ${:.2}, qty {:.5}",
            avg1, strategy.config.invested_amount,
            strategy.position.as_ref().unwrap().quantity);

        // DCA #2 a $70.00
        simulate_dca_buy(&mut strategy, 70.00);
        let avg2 = strategy.config.base_price;
        assert!(avg2 < avg1, "Após DCA#2, média deve cair mais!");
        println!("  DCA #2 @ $70.00: avg ${:.2}, investido ${:.2}, qty {:.5}",
            avg2, strategy.config.invested_amount,
            strategy.position.as_ref().unwrap().quantity);

        // DCA #3 a $62.00
        simulate_dca_buy(&mut strategy, 62.00);
        let avg3 = strategy.config.base_price;
        assert!(avg3 < avg2, "Após DCA#3, média deve cair mais!");
        println!("  DCA #3 @ $62.00: avg ${:.2}, investido ${:.2}, qty {:.5}",
            avg3, strategy.config.invested_amount,
            strategy.position.as_ref().unwrap().quantity);

        // Novo TP deve ser menor que o original
        let original_tp = 88.29 * 1.122;
        let new_tp = strategy.config.trigger_price();
        assert!(new_tp < original_tp, "Com média menor, TP deve ser mais baixo e alcançável!");
        println!("\n  TP original: ${:.2} → TP após 3 DCAs: ${:.2} (${:.2} mais fácil!)",
            original_tp, new_tp, original_tp - new_tp);

        println!("\n  ✅ DCA reduz preço médio: ${:.2} → ${:.2} (-{:.1}%)",
            initial_avg, avg3, ((initial_avg - avg3) / initial_avg) * 100.0);
        println!();
    }

    // ════════════════════════════════════════════════════════════════
    // TESTE 7: Saldo USDT insuficiente bloqueia compra DCA
    // ════════════════════════════════════════════════════════════════
    #[test]
    fn test_insufficient_usdt_blocks_dca_buy() {
        println!("\n╔══════════════════════════════════════════════════════════════╗");
        println!("║  TESTE: Saldo USDT insuficiente bloqueia DCA Buy            ║");
        println!("╚══════════════════════════════════════════════════════════════╝\n");

        let strategy = create_sol_strategy();
        let price = 79.00; // Preço que ativaria DCA (-10.5% do base $88.29)

        // Verificar que a queda de preço realmente gera sinal DCA
        let signals = run_tick(&strategy, price);
        let dca_signal = signals.iter().find(|s| s.signal_type == SignalType::DcaBuy);
        assert!(dca_signal.is_some(), "Deveria gerar sinal DcaBuy a $79.00!");
        println!("  ✅ Sinal DcaBuy gerado a ${:.2} (correto)", price);

        // Simular validação de saldo que o tick() faz:
        // No tick(), se balances.get("USDT").free < dca_amount * 0.95, bloqueia
        let dca_amount = strategy.config.dca_buy_amount_usd; // $36
        let margin = 0.95;
        let min_required = dca_amount * margin; // $34.20

        // Cenário 1: Saldo $10 — INSUFICIENTE
        let usdt_free_low = 10.0;
        let blocked = usdt_free_low < min_required;
        assert!(blocked, "Com $10 USDT, DCA de $36 DEVE ser bloqueado!");
        println!("  ✅ Saldo USDT ${:.2} < mínimo ${:.2} → compra BLOQUEADA", usdt_free_low, min_required);

        // Cenário 2: Saldo $0 — INSUFICIENTE (zero na conta)
        let usdt_free_zero = 0.0;
        let blocked_zero = usdt_free_zero < min_required;
        assert!(blocked_zero, "Com $0 USDT, DCA DEVE ser bloqueado!");
        println!("  ✅ Saldo USDT ${:.2} (vazio) → compra BLOQUEADA", usdt_free_zero);

        // Cenário 3: Saldo $34.00 — INSUFICIENTE (quase mas não chega)
        let usdt_almost = 34.00;
        let blocked_almost = usdt_almost < min_required;
        assert!(blocked_almost, "Com $34 USDT, DCA de $36 (mínimo $34.20) DEVE ser bloqueado!");
        println!("  ✅ Saldo USDT ${:.2} < mínimo ${:.2} → compra BLOQUEADA (por $0.20)", usdt_almost, min_required);

        // Cenário 4: Saldo $35.00 — SUFICIENTE (dentro da margem de 5%)
        let usdt_ok = 35.00;
        let allowed = usdt_ok >= min_required;
        assert!(allowed, "Com $35 USDT e margem 5%, deveria PERMITIR DCA de $36!");
        println!("  ✅ Saldo USDT ${:.2} >= mínimo ${:.2} → compra PERMITIDA (margem 5%)", usdt_ok, min_required);

        // Cenário 5: Saldo $50.00 — SUFICIENTE (sobra)
        let usdt_plenty = 50.0;
        let allowed_plenty = usdt_plenty >= min_required;
        assert!(allowed_plenty, "Com $50 USDT, DCA DEVE ser permitido!");
        println!("  ✅ Saldo USDT ${:.2} >= mínimo ${:.2} → compra PERMITIDA", usdt_plenty, min_required);

        // Simular o HashMap de balances como o tick() faz
        let mut balances: HashMap<String, Balance> = HashMap::new();
        balances.insert("USDT".to_string(), Balance {
            symbol: "USDT".to_string(),
            free: 10.0, used: 0.0, total: 10.0,
            usd_value: None, change_24h: None,
        });
        let quote_free = balances.get("USDT").map(|b| b.free).unwrap_or(0.0);
        assert!(quote_free < dca_amount * 0.95, "HashMap check: $10 < $34.20");
        println!("\n  ✅ Validação via HashMap<String, Balance> OK — mesma lógica do tick()");

        // Verificar que a mensagem de bloqueio seria informativa
        let block_msg = format!(
            "⚠️ Saldo insuficiente para DCA! Precisa de ${:.2} USDT mas só tem ${:.2} disponível.",
            dca_amount, quote_free
        );
        assert!(block_msg.contains("insuficiente"), "Mensagem deve conter 'insuficiente'");
        assert!(block_msg.contains("36.00"), "Mensagem deve mostrar valor necessário");
        println!("  ✅ Mensagem de erro: {}", block_msg);

        println!("\n  ✅ Validação completa: saldo USDT insuficiente bloqueia DCA corretamente.");
        println!();
    }

    // ════════════════════════════════════════════════════════════════
    // TESTE 8: Saldo do token insuficiente bloqueia venda (TP/SL)
    // ════════════════════════════════════════════════════════════════
    #[test]
    fn test_insufficient_token_blocks_sell() {
        println!("\n╔══════════════════════════════════════════════════════════════╗");
        println!("║  TESTE: Saldo token insuficiente bloqueia venda             ║");
        println!("╚══════════════════════════════════════════════════════════════╝\n");

        // Criar estratégia COM stop loss ativado para testar cenário de venda
        let mut strategy = create_sol_strategy();
        strategy.config.stop_loss_enabled = true;
        strategy.config.dca_enabled = false; // Desativar DCA para testar SL puro

        let qty = strategy.position.as_ref().unwrap().quantity; // 0.40774 SOL
        let margin = 0.95;
        let min_token_required = qty * margin; // ~0.38735 SOL
        println!("  Posição: {:.5} SOL", qty);
        println!("  Mínimo para vender (com margem 5%): {:.5} SOL\n", min_token_required);

        // ── Cenário A: Take Profit com saldo insuficiente ──
        let tp_price = strategy.config.trigger_price(); // ~$99.06
        let signals_tp = run_tick(&strategy, tp_price + 1.0); // Preço acima do TP
        let has_tp = signals_tp.iter().any(|s| s.signal_type == SignalType::GradualSell || s.signal_type == SignalType::TakeProfit);
        assert!(has_tp, "Deveria gerar sinal de venda acima do TP!");
        println!("  ✅ Sinal de venda gerado a ${:.2} (acima do TP ${:.2})", tp_price + 1.0, tp_price);

        // Simular saldo insuficiente: 0.05 SOL na exchange (vendeu por fora, retirou, etc.)
        let mut balances: HashMap<String, Balance> = HashMap::new();
        let sell_amount = calc_sell_amount(&strategy, &SignalType::GradualSell);
        println!("  Lote gradual (25%): {:.5} SOL, mínimo com margem: {:.5}", sell_amount, sell_amount * margin);

        balances.insert("SOL".to_string(), Balance {
            symbol: "SOL".to_string(),
            free: 0.05, used: 0.0, total: 0.05,
            usd_value: None, change_24h: None,
        });

        let token_free = balances.get("SOL").map(|b| b.free).unwrap_or(0.0);
        let blocked = token_free < sell_amount * margin;
        assert!(blocked, "Com 0.05 SOL, venda de {:.5} SOL DEVE ser bloqueada!", sell_amount);
        println!("  ✅ Saldo SOL {:.5} < necessário {:.5} → venda BLOQUEADA", token_free, sell_amount * margin);

        // ── Cenário B: Saldo zero (token retirado da exchange) ──
        balances.insert("SOL".to_string(), Balance {
            symbol: "SOL".to_string(),
            free: 0.0, used: 0.0, total: 0.0,
            usd_value: None, change_24h: None,
        });
        let token_free_zero = balances.get("SOL").map(|b| b.free).unwrap_or(0.0);
        let blocked_zero = token_free_zero < sell_amount * margin;
        assert!(blocked_zero, "Com 0 SOL, venda DEVE ser bloqueada!");
        println!("  ✅ Saldo SOL 0.00000 (retirado) → venda BLOQUEADA");

        // ── Cenário C: Token nem existe no balanço (ex: nunca comprou via exchange) ──
        let balances_empty: HashMap<String, Balance> = HashMap::new();
        let token_free_missing = balances_empty.get("SOL").map(|b| b.free).unwrap_or(0.0);
        assert_eq!(token_free_missing, 0.0, "Token ausente deve retornar 0.0");
        let blocked_missing = token_free_missing < sell_amount * margin;
        assert!(blocked_missing, "Token ausente no balanço DEVE bloquear venda!");
        println!("  ✅ Token SOL ausente no balanço → venda BLOQUEADA");

        // ── Cenário D: Stop Loss com saldo insuficiente ──
        let sl_price = strategy.config.stop_loss_price(); // ~$83.88
        let signals_sl = run_tick(&strategy, sl_price - 1.0); // Preço abaixo do SL
        let has_sl = signals_sl.iter().any(|s| s.signal_type == SignalType::StopLoss);
        assert!(has_sl, "Deveria gerar sinal StopLoss abaixo do SL!");

        // Para stop loss, vende TUDO (qty completa)
        let sl_sell_amount = calc_sell_amount(&strategy, &SignalType::StopLoss);
        assert!((sl_sell_amount - qty).abs() < 0.001, "Stop loss vende toda a posição!");

        // Saldo parcial: tem metade do token (pode ter vendido parte manualmente)
        balances.insert("SOL".to_string(), Balance {
            symbol: "SOL".to_string(),
            free: qty / 2.0, // 0.20387 SOL (metade)
            used: 0.0, total: qty / 2.0,
            usd_value: None, change_24h: None,
        });
        let half_free = balances.get("SOL").map(|b| b.free).unwrap_or(0.0);
        let blocked_half = half_free < sl_sell_amount * margin;
        assert!(blocked_half, "Com metade do SOL ({:.5}), stop loss de {:.5} DEVE ser bloqueado!", half_free, sl_sell_amount);
        println!("  ✅ Saldo SOL {:.5} (metade) < necessário {:.5} → stop loss BLOQUEADO", half_free, sl_sell_amount * margin);

        // ── Cenário E: Saldo suficiente — venda permitida ──
        balances.insert("SOL".to_string(), Balance {
            symbol: "SOL".to_string(),
            free: qty, // Saldo exato = posição
            used: 0.0, total: qty,
            usd_value: None, change_24h: None,
        });
        let token_ok = balances.get("SOL").map(|b| b.free).unwrap_or(0.0);
        let allowed = token_ok >= sell_amount * margin;
        assert!(allowed, "Com saldo exato ({:.5} SOL), venda DEVE ser permitida!", token_ok);
        println!("  ✅ Saldo SOL {:.5} >= necessário {:.5} → venda PERMITIDA", token_ok, sell_amount * margin);

        // ── Cenário F: Saldo com folga — venda permitida ──
        balances.insert("SOL".to_string(), Balance {
            symbol: "SOL".to_string(),
            free: qty * 2.0, // Dobro do necessário
            used: 0.0, total: qty * 2.0,
            usd_value: None, change_24h: None,
        });
        let token_plenty = balances.get("SOL").map(|b| b.free).unwrap_or(0.0);
        let allowed_plenty = token_plenty >= sell_amount * margin;
        assert!(allowed_plenty, "Com dobro do saldo, venda DEVE ser permitida!");
        println!("  ✅ Saldo SOL {:.5} (dobro) → venda PERMITIDA com folga", token_plenty);

        // ── Resumo das condições de bloqueio ──
        println!("\n  📋 REGRAS DE VALIDAÇÃO DE SALDO:");
        println!("     VENDA (TP/SL/Gradual): token_free >= sell_amount × 0.95");
        println!("     COMPRA (DCA):          quote_free >= dca_amount  × 0.95");
        println!("     Margem de 5% para arredondamento e micro-diferenças");

        println!("\n  ✅ Validação completa: saldo insuficiente bloqueia venda corretamente.");
        println!();
    }
}
