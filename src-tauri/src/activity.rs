//! Live "current session burn" status, computed by incrementally tailing
//! `~/.claude/projects/*/*.jsonl` into a recent-events ring. All math lives in
//! pure functions (unit-tested); file IO is confined to `ActivityTracker`.

use chrono::{DateTime, Local};
use serde::Serialize;
use std::collections::VecDeque;

/// Silence (seconds) after which a session counts as inactive.
pub const ACTIVE_WINDOW_SECS: i64 = 120;
/// Window (seconds) over which burn rate is averaged.
pub const RATE_WINDOW_SECS: i64 = 300;
/// Number of per-minute buckets in the sparkline.
pub const SPARK_MINUTES: usize = 10;

#[derive(Debug, Clone, Serialize, Default)]
pub struct LiveActivity {
    pub active: bool,
    pub burn_tpm: f64,
    pub session_tokens: u64,
    pub last_active_secs: u64,
    pub mins_to_empty: Option<f64>,
    pub beats_reset: bool,
    pub spark: Vec<f64>,
    pub source: String, // "statusline" | "jsonl"
}

/// Tokens/min over the last `window_secs`, summed across all events.
pub fn burn_rate(events: &[(DateTime<Local>, u64)], now: DateTime<Local>, window_secs: i64) -> f64 {
    let cutoff = now - chrono::Duration::seconds(window_secs);
    let sum: u64 = events
        .iter()
        .filter(|(t, _)| *t >= cutoff)
        .map(|(_, n)| *n)
        .sum();
    sum as f64 / (window_secs as f64 / 60.0)
}

/// Per-minute token buckets for the last `minutes` minutes; oldest first,
/// newest last (so the sparkline reads left→right in time order).
pub fn spark_buckets(
    events: &[(DateTime<Local>, u64)],
    now: DateTime<Local>,
    minutes: usize,
) -> Vec<f64> {
    let mut buckets = vec![0.0; minutes];
    for (t, n) in events {
        let age_min = (now - *t).num_seconds() / 60; // 0 = current minute
        if age_min >= 0 && (age_min as usize) < minutes {
            let idx = minutes - 1 - age_min as usize;
            buckets[idx] += *n as f64;
        }
    }
    buckets
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(secs_ago: i64, now: DateTime<Local>) -> DateTime<Local> {
        now - chrono::Duration::seconds(secs_ago)
    }

    #[test]
    fn burn_rate_averages_window_only() {
        let now = Local::now();
        let events = vec![
            (t(30, now), 1000),  // in window
            (t(120, now), 2000), // in window
            (t(600, now), 9999), // outside 300s window
        ];
        // (1000 + 2000) over 5 minutes = 600 tok/min
        assert!((burn_rate(&events, now, RATE_WINDOW_SECS) - 600.0).abs() < 1e-6);
    }

    #[test]
    fn spark_buckets_place_by_minute() {
        let now = Local::now();
        let events = vec![
            (t(10, now), 5.0 as u64),  // current minute -> last bucket
            (t(70, now), 7),           // 1 min ago -> second-to-last
            (t(10_000, now), 999),     // far outside -> ignored
        ];
        let b = spark_buckets(&events, now, SPARK_MINUTES);
        assert_eq!(b.len(), SPARK_MINUTES);
        assert_eq!(b[SPARK_MINUTES - 1], 5.0);
        assert_eq!(b[SPARK_MINUTES - 2], 7.0);
        assert_eq!(b[0], 0.0);
    }
}
