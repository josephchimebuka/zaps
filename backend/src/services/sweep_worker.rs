use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

use crate::db::r#yield::{list_auto_sweep_candidates, process_internal_sweep_deposit};

const DEFAULT_SWEEP_INTERVAL_SECS: u64 = 300;
const DEFAULT_MIN_IDLE_AMOUNT: i64 = 100_000;
const BATCH_SIZE: i64 = 50;

pub struct SweepWorkerConfig {
    pub poll_interval: Duration,
    pub min_idle_amount: i64,
}

impl SweepWorkerConfig {
    pub fn from_env() -> Self {
        let poll_secs = std::env::var("SWEEP_POLL_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_SWEEP_INTERVAL_SECS);
        let min_idle = std::env::var("SWEEP_MIN_IDLE_AMOUNT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_MIN_IDLE_AMOUNT);

        Self {
            poll_interval: Duration::from_secs(poll_secs),
            min_idle_amount: min_idle,
        }
    }
}

/// BE-029: Periodically sweep idle available stablecoin balances into the
/// yield vault for users with auto-earn enabled.
pub async fn run(pool: PgPool, config: SweepWorkerConfig) {
    tracing::info!(
        "Starting auto-sweep worker (interval={:?}, min_idle={})",
        config.poll_interval,
        config.min_idle_amount
    );

    let mut interval = tokio::time::interval(config.poll_interval);

    loop {
        interval.tick().await;

        if let Err(err) = sweep_once(&pool, config.min_idle_amount).await {
            tracing::error!("Auto-sweep cycle failed: {err:?}");
        }
    }
}

async fn sweep_once(pool: &PgPool, min_idle_amount: i64) -> Result<(), sqlx::Error> {
    let candidates = list_auto_sweep_candidates(pool, min_idle_amount, BATCH_SIZE).await?;

    if candidates.is_empty() {
        tracing::debug!("Auto-sweep: no eligible users this cycle");
        return Ok(());
    }

    let mut swept = 0usize;
    for balance in candidates {
        let amount = balance.available_balance;
        if amount < min_idle_amount {
            continue;
        }

        let tx_hash = format!("zaps-auto-sweep-{}", Uuid::new_v4());
        match process_internal_sweep_deposit(pool, balance.user_id, amount, &tx_hash).await {
            Ok(()) => {
                swept += 1;
                tracing::info!(
                    user_id = %balance.user_id,
                    amount,
                    "Auto-swept idle balance into yield vault"
                );
            }
            Err(sqlx::Error::RowNotFound) => {
                tracing::debug!(
                    user_id = %balance.user_id,
                    "Auto-sweep skipped: insufficient available balance"
                );
            }
            Err(err) => {
                tracing::warn!(
                    user_id = %balance.user_id,
                    error = ?err,
                    "Auto-sweep deposit failed"
                );
            }
        }
    }

    tracing::debug!("Auto-sweep cycle complete: swept {} user(s)", swept);
    Ok(())
}
