//! Live "current session burn" status, computed by incrementally tailing
//! `~/.claude/projects/*/*.jsonl` into a recent-events ring. All math lives in
//! pure functions (unit-tested); file IO is confined to `ActivityTracker`.

use chrono::{DateTime, Local};
use serde::Serialize;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

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

/// Parse one transcript line; return (timestamp, input+output tokens) for
/// assistant messages carrying usage, else None.
pub fn parse_assistant_tokens(line: &str) -> Option<(DateTime<Local>, u64)> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let ts = v.get("timestamp")?.as_str()?;
    let dt = DateTime::parse_from_rfc3339(ts).ok()?.with_timezone(&Local);
    let usage = v.get("message")?.get("usage")?;
    let g = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    Some((dt, g("input_tokens") + g("output_tokens")))
}

struct FileState {
    offset: u64,
    session_total: u64,
    last_ts: Option<DateTime<Local>>,
}

/// Incremental tailer: remembers a byte offset per transcript so each tick only
/// reads appended bytes, and keeps a recent-events ring for rate/sparkline.
pub struct ActivityTracker {
    files: HashMap<PathBuf, FileState>,
    events: VecDeque<(DateTime<Local>, u64)>,
}

impl ActivityTracker {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            events: VecDeque::new(),
        }
    }

    /// Tail transcripts under `~/.claude/projects`. `force_path` (from a fresh
    /// statusline hint) is read even if its mtime is borderline.
    pub fn tick(&mut self, now: DateTime<Local>, force_path: Option<PathBuf>) {
        let Some(home) = dirs::home_dir() else { return };
        self.tick_in(&home.join(".claude/projects"), now, force_path);
    }

    /// Testable core: tail `*/*.jsonl` under `base`.
    pub fn tick_in(&mut self, base: &Path, now: DateTime<Local>, force_path: Option<PathBuf>) {
        let pattern = base.join("*/*.jsonl");
        let Ok(paths) = glob::glob(&pattern.to_string_lossy()) else { return };
        let recent_cutoff = chrono::Duration::minutes(SPARK_MINUTES as i64 + 2);

        for path in paths.flatten() {
            let forced = force_path.as_deref() == Some(path.as_path());
            // mtime prefilter (cheap): skip files untouched recently, unless forced.
            if !forced {
                let fresh = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .map(|mt| {
                        let mt: DateTime<Local> = mt.into();
                        now - mt <= recent_cutoff
                    })
                    .unwrap_or(false);
                if !fresh {
                    continue;
                }
            }
            self.ingest_file(&path, now);
        }
        self.prune(now);
    }

    fn ingest_file(&mut self, path: &Path, now: DateTime<Local>) {
        let Ok(mut f) = std::fs::File::open(path) else { return };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        let fs = self.files.entry(path.to_path_buf()).or_insert(FileState {
            offset: 0,
            session_total: 0,
            last_ts: None,
        });
        if len < fs.offset {
            // File truncated/rotated — start over.
            fs.offset = 0;
            fs.session_total = 0;
            fs.last_ts = None;
        }
        if f.seek(SeekFrom::Start(fs.offset)).is_err() {
            return;
        }
        let mut buf = String::new();
        if f.read_to_string(&mut buf).is_err() {
            return;
        }
        // Only consume up to the last complete line; keep a partial tail for next time.
        let consumed = buf.rfind('\n').map(|i| i + 1).unwrap_or(0);
        fs.offset += consumed as u64;
        let recent_cutoff = chrono::Duration::minutes(SPARK_MINUTES as i64 + 1);
        for line in buf[..consumed].lines() {
            if let Some((ts, tokens)) = parse_assistant_tokens(line) {
                fs.session_total += tokens;
                fs.last_ts = Some(ts);
                if now - ts <= recent_cutoff && ts <= now {
                    self.events.push_back((ts, tokens));
                }
            }
        }
    }

    fn prune(&mut self, now: DateTime<Local>) {
        let cutoff = now - chrono::Duration::minutes(SPARK_MINUTES as i64 + 1);
        while let Some((t, _)) = self.events.front() {
            if *t < cutoff {
                self.events.pop_front();
            } else {
                break;
            }
        }
    }

    /// Build the snapshot. `quota_samples`/`five_pct`/`reset_secs` drive the
    /// time-to-empty estimate; they come from the quota poller.
    pub fn snapshot(
        &self,
        now: DateTime<Local>,
        hint_fresh: bool,
        source: &str,
        quota_samples: &[(DateTime<Local>, f64)],
        five_pct: Option<f64>,
        reset_secs: Option<i64>,
    ) -> LiveActivity {
        let ev: Vec<(DateTime<Local>, u64)> = self.events.iter().cloned().collect();
        let last_ts = ev.iter().map(|(t, _)| *t).max();
        let active = is_active(last_ts, now, hint_fresh);
        let burn_tpm = burn_rate(&ev, now, RATE_WINDOW_SECS);
        let spark = spark_buckets(&ev, now, SPARK_MINUTES);
        let session_tokens = self
            .files
            .values()
            .filter(|f| {
                f.last_ts
                    .map_or(false, |t| (now - t).num_seconds() <= ACTIVE_WINDOW_SECS)
            })
            .map(|f| f.session_total)
            .sum();
        let last_active_secs = last_ts.map_or(0, |t| (now - t).num_seconds().max(0) as u64);
        let mins = if active {
            mins_to_empty(quota_samples, five_pct.unwrap_or(100.0))
        } else {
            None
        };
        let beats = beats_reset(mins, reset_secs.unwrap_or(0));
        LiveActivity {
            active,
            burn_tpm,
            session_tokens,
            last_active_secs,
            mins_to_empty: mins,
            beats_reset: beats,
            spark,
            source: source.to_string(),
        }
    }
}

impl Default for ActivityTracker {
    fn default() -> Self {
        Self::new()
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

    #[test]
    fn parse_assistant_tokens_extracts_io() {
        let line = r#"{"type":"assistant","timestamp":"2026-06-07T10:00:00Z","message":{"usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":9999}}}"#;
        let (_, tok) = parse_assistant_tokens(line).unwrap();
        assert_eq!(tok, 150); // input+output only; cache ignored
        assert!(parse_assistant_tokens(r#"{"type":"user"}"#).is_none());
        assert!(parse_assistant_tokens("not json").is_none());
    }

    #[test]
    fn tracker_tails_and_sums_recent_session() {
        let now = Local::now();
        let stamp = |secs_ago: i64| (now - chrono::Duration::seconds(secs_ago)).to_rfc3339();
        let base = std::env::temp_dir().join(format!("cum-act-{}", std::process::id()));
        let proj = base.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let file = proj.join("session.jsonl");
        let line = |secs: i64, inp: u64, out: u64| {
            format!(
                r#"{{"type":"assistant","timestamp":"{}","message":{{"usage":{{"input_tokens":{},"output_tokens":{}}}}}}}"#,
                stamp(secs), inp, out
            )
        };
        std::fs::write(&file, format!("{}\n{}\n", line(30, 100, 100), line(60, 50, 50))).unwrap();

        let mut tr = ActivityTracker::new();
        tr.tick_in(&base, now, None);
        let snap = tr.snapshot(now, false, "jsonl", &[], None, None);

        assert!(snap.active);
        assert_eq!(snap.session_tokens, 300); // 200 + 100
        // (200 + 100) over 5 min = 60 tok/min
        assert!((snap.burn_tpm - 60.0).abs() < 1e-6);

        // Append a new line; a second tick should only read the new bytes.
        let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        use std::io::Write;
        writeln!(f, "{}", line(10, 10, 10)).unwrap();
        tr.tick_in(&base, now, None);
        let snap2 = tr.snapshot(now, false, "jsonl", &[], None, None);
        assert_eq!(snap2.session_tokens, 320); // +20, not double-counted

        std::fs::remove_dir_all(&base).ok();
    }
}
