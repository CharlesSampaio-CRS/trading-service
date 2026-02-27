use crate::{database::MongoDB, services::strategy_service};
use tokio::time::{interval, Duration};
use std::env;

const DEFAULT_INTERVAL_SECS: u64 = 30;

pub async fn start_strategy_monitor(db: MongoDB) {
    let enabled = env::var("STRATEGY_MONITOR_ENABLED").unwrap_or_else(|_| "true".to_string());
    if enabled.to_lowercase() != "true" && enabled != "1" {
        log::info!("Strategy monitor DISABLED");
        return;
    }

    let interval_secs: u64 = env::var("STRATEGY_MONITOR_INTERVAL_SECS")
        .ok().and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_INTERVAL_SECS).max(5);

    log::info!("Starting strategy monitor (interval: {}s)", interval_secs);

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let mut tick_interval = interval(Duration::from_secs(interval_secs));
        let mut cycle: u64 = 0;

        loop {
            tick_interval.tick().await;
            cycle += 1;
            let start = std::time::Instant::now();

            match strategy_service::process_active_strategies(&db).await {
                Ok(r) => {
                    if r.processed > 0 || r.errors > 0 || cycle % 10 == 0 {
                        log::info!(
                            "Monitor #{}: {} total, {} processed, {} errors ({:.0}ms)",
                            cycle, r.total, r.processed, r.errors, start.elapsed().as_millis()
                        );
                    }
                }
                Err(e) => {
                    log::error!("Monitor #{} failed: {}", cycle, e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    });
}
