use crate::{database::MongoDB, services::strategy_service};
use tokio::time::{interval, Duration};
use std::env;

const DEFAULT_INTERVAL_SECS: u64 = 30;

fn now_str() -> String {
    let now = chrono::Utc::now();
    now.format("%d/%m/%Y %H:%M:%S UTC").to_string()
}

pub async fn start_strategy_monitor(db: MongoDB) {
    let enabled = env::var("STRATEGY_MONITOR_ENABLED").unwrap_or_else(|_| "true".to_string());
    if enabled.to_lowercase() != "true" && enabled != "1" {
        log::info!("📴 Strategy monitor DISABLED (STRATEGY_MONITOR_ENABLED != true)");
        return;
    }

    let interval_secs: u64 = env::var("STRATEGY_MONITOR_INTERVAL_SECS")
        .ok().and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_INTERVAL_SECS).max(5);

    log::info!(
        "🚀 Strategy monitor iniciado | intervalo: {}s | hora: {}",
        interval_secs, now_str()
    );

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let mut tick_interval = interval(Duration::from_secs(interval_secs));
        let mut cycle: u64 = 0;

        loop {
            tick_interval.tick().await;
            cycle += 1;
            let start = std::time::Instant::now();
            let cycle_time = now_str();
            let next_secs = interval_secs;

            match strategy_service::process_active_strategies(&db).await {
                Ok(r) => {
                    let elapsed = start.elapsed().as_millis();
                    if r.processed > 0 || r.errors > 0 {
                        log::info!(
                            "✅ Monitor #{} [{}] | {} estratégias | {} verificadas | {} ordens | {} erros | {:.0}ms | próxima em {}s",
                            cycle, cycle_time, r.total, r.processed, r.orders_executed, r.errors, elapsed, next_secs
                        );
                    } else if cycle % 10 == 0 {
                        // A cada 10 ciclos (~5min com 30s) loga mesmo sem ação
                        log::info!(
                            "💤 Monitor #{} [{}] | {} estratégia(s) ativa(s), nenhuma ação | {:.0}ms | próxima em {}s",
                            cycle, cycle_time, r.total, elapsed, next_secs
                        );
                    }
                }
                Err(e) => {
                    log::error!(
                        "❌ Monitor #{} [{}] FALHOU: {} | próxima tentativa em {}s",
                        cycle, cycle_time, e, next_secs
                    );
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    });
}
