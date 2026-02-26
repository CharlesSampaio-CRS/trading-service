// ═══════════════════════════════════════════════════════════════════
// STRATEGY SERVICE — Engine de processamento de estratégias (Fase 3)
// ═══════════════════════════════════════════════════════════════════
//
// Responsabilidades:
// 1. Buscar preço atual via CCXT (fetch_ticker_sync)
// 2. Avaliar regras (TP, SL, Trailing, DCA, Grid)
// 3. Gerar sinais (StrategySignal)
// 4. Executar ações (simulated ou real via CCXT — Fase 5)
// 5. Atualizar posição e gravar no MongoDB
//
// Fluxo:
//   tick() → fetch_price() → evaluate_rules() → [generate_signal()] → [execute_action()] → persist()
//

use crate::{
    ccxt::CCXTClient,
    database::MongoDB,
    models::{
        ExecutionAction, Strategy, StrategyExecution, StrategySignal,
        StrategyStatus, SignalType,
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
        | StrategyStatus::Error
        | StrategyStatus::Idle => {
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
    let executions: Vec<StrategyExecution> = Vec::new();
    let new_status: Option<StrategyStatus> = None;

    match strategy.status {
        StrategyStatus::Monitoring => {
            // Avaliar regra de entrada (Buy signal)
            evaluate_entry_rules(strategy, price, now, &mut signals);
        }
        StrategyStatus::InPosition => {
            // Avaliar TP, SL, Trailing, DCA
            evaluate_exit_rules(strategy, price, now, &mut signals);
            evaluate_dca_rules(strategy, price, now, &mut signals);
        }
        StrategyStatus::BuyPending | StrategyStatus::SellPending => {
            // TODO Fase 5: verificar status da ordem na exchange
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

    // ── 4. Marcar sinais que geram ação (Fase 5: executar de verdade) ──
    // Por agora, apenas registramos as ações simuladas.
    // Na Fase 5, sinais com `acted = true` dispararão ordens reais via CCXT.
    for signal in &mut signals {
        match signal.signal_type {
            SignalType::Buy => {
                // Fase 5: CCXTClient.create_order_sync(symbol, "market", "buy", amount, None)
                signal.acted = false; // Será true na Fase 5
                log::info!(
                    "🟢 [Strategy {}] BUY SIGNAL: {} @ {:.8} — {}",
                    &strategy_id[..8.min(strategy_id.len())],
                    strategy.symbol,
                    price,
                    signal.message
                );
            }
            SignalType::TakeProfit | SignalType::StopLoss | SignalType::TrailingStop => {
                signal.acted = false;
                log::info!(
                    "🔴 [Strategy {}] SELL SIGNAL ({}): {} @ {:.8} — {}",
                    &strategy_id[..8.min(strategy_id.len())],
                    signal.signal_type,
                    strategy.symbol,
                    price,
                    signal.message
                );
            }
            SignalType::DcaBuy => {
                signal.acted = false;
                log::info!(
                    "🔵 [Strategy {}] DCA BUY SIGNAL: {} @ {:.8} — {}",
                    &strategy_id[..8.min(strategy_id.len())],
                    strategy.symbol,
                    price,
                    signal.message
                );
            }
            _ => {}
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
/// Atualiza: last_price, last_checked_at, position.highest_price, signals, executions, status
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

    // Atualizar position.highest_price e position.current_price
    if result.price > 0.0 {
        if let Some(ref position) = strategy.position {
            if result.price > position.highest_price {
                update_set.insert("position.highest_price", result.price);
            }
            if result.price < position.lowest_price || position.lowest_price == 0.0 {
                update_set.insert("position.lowest_price", result.price);
            }
            update_set.insert("position.current_price", result.price);

            // Calcular PNL não realizado
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

    // Construir update com $set e $push
    let mut update_doc = doc! { "$set": update_set };

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
        "💾 [Strategy {}] Tick persisted: price={:.8}, signals={}, execs={}",
        &result.strategy_id[..8.min(result.strategy_id.len())],
        result.price,
        result.signals.len(),
        result.executions.len()
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

    // Buscar estratégias ativas com status processável
    let filter = doc! {
        "is_active": true,
        "status": { "$in": ["monitoring", "in_position", "buy_pending", "sell_pending"] }
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
    };

    log::info!(
        "📊 Strategy processing complete: {} total, {} processed, {} errors, {} signals",
        result.total,
        result.processed,
        result.errors,
        result.signals_generated
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
}
