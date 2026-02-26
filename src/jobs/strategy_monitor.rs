// ═══════════════════════════════════════════════════════════════════
// STRATEGY MONITOR — Background job que processa estratégias (Fase 4)
// ═══════════════════════════════════════════════════════════════════
//
// Padrão: mesmo do snapshot_scheduler.rs
// - Spawna tokio::spawn em background
// - Roda em loop com intervalo configurável
// - Chama strategy_service::process_active_strategies()
// - Logs de cada ciclo
//
// Configuração via env:
//   STRATEGY_MONITOR_INTERVAL_SECS  — intervalo do loop (default: 30s)
//   STRATEGY_MONITOR_ENABLED        — "true" para ativar (default: true)
//

use crate::{
    database::MongoDB,
    services::strategy_service,
};
use tokio::time::{interval, Duration};
use std::env;

/// Intervalo padrão do monitor em segundos
const DEFAULT_INTERVAL_SECS: u64 = 30;

/// Inicia o monitor de estratégias em background
///
/// O monitor roda a cada N segundos (configurável via STRATEGY_MONITOR_INTERVAL_SECS)
/// e chama `process_active_strategies()` que:
/// 1. Busca todas as estratégias ativas com status processável
/// 2. Respeita o `check_interval_secs` individual de cada estratégia
/// 3. Executa tick() → evaluate → persist para cada uma
pub async fn start_strategy_monitor(db: MongoDB) {
    // Verificar se está habilitado
    let enabled = env::var("STRATEGY_MONITOR_ENABLED")
        .unwrap_or_else(|_| "true".to_string());

    if enabled.to_lowercase() != "true" && enabled != "1" {
        log::info!("⏸️  Strategy monitor DISABLED (STRATEGY_MONITOR_ENABLED={})", enabled);
        return;
    }

    // Ler intervalo do env ou usar default
    let interval_secs: u64 = env::var("STRATEGY_MONITOR_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_INTERVAL_SECS)
        .max(5); // Mínimo 5 segundos para evitar sobrecarga

    log::info!(
        "🎯 Starting strategy monitor (interval: {}s, enabled: {})",
        interval_secs, enabled
    );

    // Spawn task em background
    tokio::spawn(async move {
        // Delay inicial de 10s para permitir que o servidor inicie completamente
        log::info!("🎯 Strategy monitor: waiting 10s for server warmup...");
        tokio::time::sleep(Duration::from_secs(10)).await;

        // Primeira execução imediata
        log::info!("🎯 Strategy monitor: running initial cycle...");
        run_monitor_cycle(&db, 0).await;

        // Loop principal
        let mut tick_interval = interval(Duration::from_secs(interval_secs));
        let mut cycle_count: u64 = 1;

        loop {
            tick_interval.tick().await;
            cycle_count += 1;
            run_monitor_cycle(&db, cycle_count).await;
        }
    });

    log::info!("✅ Strategy monitor started successfully");
}

/// Executa um ciclo do monitor
async fn run_monitor_cycle(db: &MongoDB, cycle: u64) {
    let start = std::time::Instant::now();

    // Log verboso a cada 10 ciclos, debug nos demais
    if cycle % 10 == 0 {
        log::info!("🎯 Strategy monitor cycle #{} starting...", cycle);
    } else {
        log::debug!("🎯 Strategy monitor cycle #{} starting...", cycle);
    }

    match strategy_service::process_active_strategies(db).await {
        Ok(result) => {
            let elapsed = start.elapsed();

            if result.processed > 0 || result.errors > 0 {
                // Só loga como info se processou algo
                log::info!(
                    "🎯 Monitor cycle #{}: {} total, {} processed, {} errors, {} signals ({:.1}ms)",
                    cycle,
                    result.total,
                    result.processed,
                    result.errors,
                    result.signals_generated,
                    elapsed.as_millis()
                );
            } else if cycle % 10 == 0 {
                // A cada 10 ciclos, loga mesmo que não tenha processado
                log::info!(
                    "🎯 Monitor cycle #{}: {} strategies found, none due for processing ({:.1}ms)",
                    cycle,
                    result.total,
                    elapsed.as_millis()
                );
            } else {
                log::debug!(
                    "🎯 Monitor cycle #{}: {} strategies, {} processed ({:.1}ms)",
                    cycle,
                    result.total,
                    result.processed,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) => {
            log::error!("❌ Strategy monitor cycle #{} failed: {}", cycle, e);

            // Em caso de erro, espera um pouco mais antes do próximo ciclo
            // para não ficar bombardeando em caso de erro persistente
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
}
