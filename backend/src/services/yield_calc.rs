use chrono::{DateTime, NaiveDateTime, Utc};
use crate::db::models::UserYieldBalance;

/// Stellar mainnet/testnet approximate ledger cadence (~5 seconds).
pub const SECONDS_PER_YEAR: i64 = 31_536_000;
const DEFAULT_APY_BPS: i32 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YieldEstimate {
    /// Interest accrued since the last sync point (micro-units).
    pub accrued_interest: i64,
    /// Earning balance including accrued but not yet checkpointed interest.
    pub total_earning_balance: i64,
}

/// Linear off-chain yield estimate from the user's earning balance, APY, and
/// elapsed time since the last blockchain/indexer sync.
pub fn estimate_accrued_yield(
    earning_balance: i64,
    apy_bps: i32,
    last_sync_at: NaiveDateTime,
    now: DateTime<Utc>,
) -> YieldEstimate {
    if earning_balance <= 0 || apy_bps <= 0 {
        return YieldEstimate {
            accrued_interest: 0,
            total_earning_balance: earning_balance.max(0),
        };
    }

    let sync_utc = last_sync_at.and_utc();
    let elapsed_secs = (now - sync_utc).num_seconds().max(0);

    let accrued = earning_balance
        .saturating_mul(apy_bps as i64)
        .saturating_mul(elapsed_secs)
        / (10_000 * SECONDS_PER_YEAR);

    YieldEstimate {
        accrued_interest: accrued,
        total_earning_balance: earning_balance.saturating_add(accrued),
    }
}

/// Convenience wrapper for API handlers and background jobs.
pub fn estimate_for_balance(
    balance: &UserYieldBalance,
    apy_bps: Option<i32>,
    now: DateTime<Utc>,
) -> YieldEstimate {
    estimate_accrued_yield(
        balance.earning_balance,
        apy_bps.unwrap_or(DEFAULT_APY_BPS),
        balance.last_yield_sync_at,
        now,
    )
}

/// Helper for feed-style responses that surface live yield totals.
pub fn format_yield_feed_fields(
    balance: &UserYieldBalance,
    apy_bps: Option<i32>,
) -> (i64, i64, f64) {
    let estimate = estimate_for_balance(balance, apy_bps, Utc::now());
    let apy_pct = apy_bps.unwrap_or(DEFAULT_APY_BPS) as f64 / 100.0;
    (
        estimate.accrued_interest,
        estimate.total_earning_balance,
        apy_pct,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn zero_balance_yields_no_interest() {
        let sync = NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let now = DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let est = estimate_accrued_yield(0, 500, sync, now);
        assert_eq!(est.accrued_interest, 0);
        assert_eq!(est.total_earning_balance, 0);
    }

    #[test]
    fn linear_interest_scales_with_time_and_balance() {
        let sync = NaiveDate::from_ymd_opt(2026, 6, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let now = DateTime::parse_from_rfc3339("2026-06-02T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        // 1_000_000 micro-units at 5% APY for 1 day
        let est = estimate_accrued_yield(1_000_000, 500, sync, now);
        let expected = 1_000_000 * 500 * 86_400 / (10_000 * SECONDS_PER_YEAR);
        assert_eq!(est.accrued_interest, expected);
        assert_eq!(est.total_earning_balance, 1_000_000 + expected);
    }

    #[test]
    fn negative_elapsed_is_clamped_to_zero() {
        let sync = NaiveDate::from_ymd_opt(2026, 6, 2)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let now = DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let est = estimate_accrued_yield(500_000, 500, sync, now);
        assert_eq!(est.accrued_interest, 0);
    }
}
