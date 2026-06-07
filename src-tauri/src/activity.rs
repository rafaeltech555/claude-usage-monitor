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

/// Active if a hint says so, else if the last token landed within ACTIVE_WINDOW.
pub fn is_active(last_ts: Option<DateTime<Local>>, now: DateTime<Local>, hint_fresh: bool) -> bool {
    if hint_fresh {
        return true;
    }
    match last_ts {
        Some(t) => (now - t).num_seconds() <= ACTIVE_WINDOW_SECS,
        None => false,
    }
}

/// Estimate minutes until the 5h window reaches 100%, via a least-squares fit of
/// `(time, used_%)` samples. None if <2 samples, non-positive slope, or full.
pub fn mins_to_empty(samples: &[(DateTime<Local>, f64)], current_pct: f64) -> Option<f64> {
    if samples.len() < 2 || current_pct >= 100.0 {
        return None;
    }
    let t0 = samples[0].0;
    let xs: Vec<f64> = samples
        .iter()
        .map(|(t, _)| (*t - t0).num_seconds() as f64 / 60.0)
        .collect();
    let ys: Vec<f64> = samples.iter().map(|(_, p)| *p).collect();
    let n = xs.len() as f64;
    let sx: f64 = xs.iter().sum();
    let sy: f64 = ys.iter().sum();
    let sxx: f64 = xs.iter().map(|x| x * x).sum();
    let sxy: f64 = xs.iter().zip(&ys).map(|(x, y)| x * y).sum();
    let denom = n * sxx - sx * sx;
    if denom == 0.0 {
        return None;
    }
    let slope = (n * sxy - sx * sy) / denom; // %/min
    if slope <= 0.0 {
        return None;
    }
    Some((100.0 - current_pct) / slope)
}

/// True if the window resets before the projected empty time (i.e. you won't
/// run out this window). `reset_secs` = seconds until the 5h window resets.
pub fn beats_reset(mins_to_empty: Option<f64>, reset_secs: i64) -> bool {
    match mins_to_empty {
        Some(m) => m * 60.0 > reset_secs as f64,
        None => false,
    }
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

    #[test]
    fn is_active_by_silence_or_hint() {
        let now = Local::now();
        assert!(is_active(Some(t(60, now)), now, false)); // recent
        assert!(!is_active(Some(t(300, now)), now, false)); // too old
        assert!(is_active(Some(t(300, now)), now, true)); // hint overrides
        assert!(!is_active(None, now, false)); // never seen
    }

    #[test]
    fn mins_to_empty_linear_fit() {
        let now = Local::now();
        // 2%/min slope: 40% at t-10min, 60% now
        let samples = vec![(t(600, now), 40.0), (now, 60.0)];
        let m = mins_to_empty(&samples, 60.0).unwrap();
        assert!((m - 20.0).abs() < 1e-6); // (100-60)/2 = 20 min
    }

    #[test]
    fn mins_to_empty_none_when_flat_or_sparse() {
        let now = Local::now();
        assert!(mins_to_empty(&[(now, 50.0)], 50.0).is_none()); // <2 samples
        let flat = vec![(t(600, now), 50.0), (now, 50.0)];
        assert!(mins_to_empty(&flat, 50.0).is_none()); // zero slope
        let full = vec![(t(600, now), 90.0), (now, 100.0)];
        assert!(mins_to_empty(&full, 100.0).is_none()); // already full
    }

    #[test]
    fn beats_reset_compares_minutes_to_seconds() {
        assert!(beats_reset(Some(40.0), 30 * 60)); // empties in 40min, resets in 30min
        assert!(!beats_reset(Some(20.0), 30 * 60)); // empties first
        assert!(!beats_reset(None, 30 * 60));
    }
}
