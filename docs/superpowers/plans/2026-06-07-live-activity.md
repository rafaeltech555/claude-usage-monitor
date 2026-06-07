# Live Activity Status Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show a live "current Claude Code session burn" status in the widget — tok/min, a 10-minute sparkline, cumulative session tokens, and an estimated "5h window empties in N min", across four renderings (A detailed-card block, B idle state, C compact-pill indicator, D standalone big-burn mode).

**Architecture:** A new Rust module `activity.rs` tails `~/.claude/projects/*/*.jsonl` incrementally (byte-offset per file) into a recent-events ring, plus pure functions for burn rate / sparkline buckets / active-detection / time-to-empty. A second tokio ticker (~5s) emits a `LiveActivity` snapshot over a new `activity-update` event, independent of the existing ≥180s quota poll (which additionally records 5h-% samples for the time-to-empty slope). The statusline hook, when fresh, supplies the active session path for immediacy; otherwise mtime+tail is the fallback. The frontend listens to `activity-update` and renders A/B/C/D.

**Tech Stack:** Rust (Tauri v2, chrono, serde, glob), TypeScript (vanilla + Vite), vitest, cargo test.

---

## File Structure

- **Modify** `src-tauri/src/config.rs` — add `show_activity: bool` (default true), allow `"activity"` mode in `normalize`.
- **Create** `src-tauri/src/activity.rs` — `LiveActivity` struct, pure compute functions, `ActivityTracker` (incremental tailer).
- **Modify** `src-tauri/src/statusline.rs` — write/read an `activity-hint.json` (active session path) in the hook.
- **Modify** `src-tauri/src/lib.rs` — register module; `AppState` fields; record 5h-% samples; `build_activity`; `get_activity` command; `spawn_activity_ticker`; `ACTIVITY` window size + `apply_mode`/`set_mode`/tray for the new mode.
- **Modify** `src/format.ts` + `src/format.test.ts` — `fmtRate`, `fmtMinsToEmpty` + tests.
- **Modify** `index.html` + `src/styles.css` — live-block (A/B), pill indicator (C), activity card (D), `--live` color, settings checkbox + mode option.
- **Modify** `src/main.ts` — `LiveActivity` type, `activity-update` listener, `renderActivity`, sparkline drawing, mode/settings wiring.

Reference mockup (visual target, not shipped): `/tmp/activity-mockup.html`.

---

## Task 1: Config — `show_activity` field

**Files:**
- Modify: `src-tauri/src/config.rs`

- [ ] **Step 1: Add a failing test**

Add to the `tests` module in `src-tauri/src/config.rs`:

```rust
    #[test]
    fn show_activity_defaults_on_and_activity_mode_is_valid() {
        let c = Config::default();
        assert!(c.show_activity);

        let mut m = Config { mode: "activity".into(), ..Config::default() };
        m.normalize();
        assert_eq!(m.mode, "activity"); // must survive normalize
    }
```

- [ ] **Step 2: Run it, verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml show_activity_defaults_on`
Expected: FAIL (no field `show_activity`; `activity` mode reset to `compact`).

- [ ] **Step 3: Add the field, default, and normalize allowance**

In the `Config` struct (after `renewal_day`):

```rust
    /// Show the live-activity block / indicators / burn mode.
    pub show_activity: bool,
```

In `impl Default for Config` (after `renewal_day: 0,`):

```rust
            show_activity: true,
```

In `normalize`, change the mode guard:

```rust
        if !matches!(self.mode.as_str(), "compact" | "detailed" | "activity") {
            self.mode = "compact".into();
        }
```

- [ ] **Step 4: Run tests, verify pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml config`
Expected: PASS (all config tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/config.rs
git commit -m "feat(config): add show_activity flag and allow activity mode"
```

---

## Task 2: `activity.rs` — struct + `burn_rate` + `spark_buckets`

**Files:**
- Create: `src-tauri/src/activity.rs`

- [ ] **Step 1: Create the module with the struct, constants, and the two pure functions, plus failing tests**

Create `src-tauri/src/activity.rs`:

```rust
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
```

- [ ] **Step 2: Register the module so it compiles**

In `src-tauri/src/lib.rs`, add to the `mod` declarations at the top (after `mod config;`):

```rust
mod activity;
```

- [ ] **Step 3: Run tests, verify they fail then pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml activity::`
Expected: PASS (both tests). If the module didn't compile before adding code, that's the "fail" state; with the code above it should pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/activity.rs src-tauri/src/lib.rs
git commit -m "feat(activity): LiveActivity struct + burn_rate/spark_buckets"
```

---

## Task 3: `activity.rs` — `is_active` + `mins_to_empty` + `beats_reset`

**Files:**
- Modify: `src-tauri/src/activity.rs`

- [ ] **Step 1: Add failing tests**

Add to the `tests` module in `src-tauri/src/activity.rs`:

```rust
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
```

- [ ] **Step 2: Run, verify fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml activity::`
Expected: FAIL (functions not defined).

- [ ] **Step 3: Implement the three functions**

Add to `src-tauri/src/activity.rs` (after `spark_buckets`, before `#[cfg(test)]`):

```rust
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
```

- [ ] **Step 4: Run, verify pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml activity::`
Expected: PASS (all activity tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/activity.rs
git commit -m "feat(activity): is_active, mins_to_empty, beats_reset"
```

---

## Task 4: `activity.rs` — `parse_assistant_tokens` + `ActivityTracker`

**Files:**
- Modify: `src-tauri/src/activity.rs`

- [ ] **Step 1: Add failing tests (line parser + temp-file integration)**

Add to the `tests` module in `src-tauri/src/activity.rs`:

```rust
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
```

- [ ] **Step 2: Run, verify fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml activity::tests::tracker_tails`
Expected: FAIL (no `ActivityTracker` / `parse_assistant_tokens`).

- [ ] **Step 3: Implement the parser and tracker**

Add imports at the top of `src-tauri/src/activity.rs` (below the existing `use` lines):

```rust
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
```

Add (after `beats_reset`, before `#[cfg(test)]`):

```rust
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
```

- [ ] **Step 4: Run, verify pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml activity::`
Expected: PASS (all activity tests, including the temp-file integration).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/activity.rs
git commit -m "feat(activity): incremental ActivityTracker tailer + parser"
```

---

## Task 5: statusline hint — active session path

**Files:**
- Modify: `src-tauri/src/statusline.rs`

- [ ] **Step 1: Add failing tests**

Add to the `tests` module in `src-tauri/src/statusline.rs`:

```rust
    #[test]
    fn parse_hint_pulls_session_fields() {
        let v = serde_json::json!({
            "transcript_path": "/home/u/.claude/projects/p/s.jsonl",
            "session_id": "abc-123",
            "rate_limits": {}
        });
        let h = parse_hint(&v);
        assert_eq!(h.transcript_path.as_deref(), Some("/home/u/.claude/projects/p/s.jsonl"));
        assert_eq!(h.session_id.as_deref(), Some("abc-123"));

        let empty = parse_hint(&serde_json::json!({}));
        assert!(empty.transcript_path.is_none());
    }
```

- [ ] **Step 2: Run, verify fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml statusline::tests::parse_hint`
Expected: FAIL (`parse_hint` / `ActivityHint` not defined).

- [ ] **Step 3: Implement the hint type, parser, writer, and reader**

In `src-tauri/src/statusline.rs`, add to the imports (it already has `use serde_json`; add serde derive import):

```rust
use serde::{Deserialize, Serialize};
```

Add after `pub fn data_path()`:

```rust
pub fn hint_path() -> PathBuf {
    Config::dir().join("activity-hint.json")
}

/// Which Claude Code session is currently rendering a statusline.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActivityHint {
    pub transcript_path: Option<String>,
    pub session_id: Option<String>,
}

/// Pure: extract the active-session hint from Claude Code's statusline stdin.
pub fn parse_hint(v: &serde_json::Value) -> ActivityHint {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
    ActivityHint {
        transcript_path: s("transcript_path"),
        session_id: s("session_id"),
    }
}

/// Read the hint if its file was written within `max_age_secs`.
pub fn read_hint_fresh(max_age_secs: u64) -> Option<ActivityHint> {
    let p = hint_path();
    let modified = std::fs::metadata(&p).ok()?.modified().ok()?;
    let age = std::time::SystemTime::now().duration_since(modified).ok()?;
    if age.as_secs() > max_age_secs {
        return None;
    }
    serde_json::from_str(&std::fs::read_to_string(&p).ok()?).ok()
}
```

In `run_hook`, after the existing block that writes the quota `data_path()` file (right before the `let fmt = |w: &Option<QuotaWindow>| {` line), add:

```rust
    // Persist the active-session hint (best-effort) for the live-activity ticker.
    let hint = parse_hint(&v);
    if let Ok(json) = serde_json::to_string(&hint) {
        let p = hint_path();
        if std::fs::write(&p, json).is_ok() {
            set_owner_only(&p);
        }
    }
```

- [ ] **Step 4: Run, verify pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml statusline::`
Expected: PASS (all statusline tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/statusline.rs
git commit -m "feat(statusline): write/read active-session activity hint"
```

---

## Task 6: lib.rs — wiring (state, samples, ticker, command, mode)

**Files:**
- Modify: `src-tauri/src/lib.rs`

> No new automated test here (pure functions are tested in Tasks 2–4); this task is integration glue, verified by `cargo build` + manual run in Task 10.

- [ ] **Step 1: Extend `AppState`**

Add fields to `struct AppState` (after `anim_gen: AtomicU64,`):

```rust
    activity: Mutex<activity::ActivityTracker>,
    quota_samples: Mutex<std::collections::VecDeque<(chrono::DateTime<chrono::Local>, f64)>>,
```

In the `.manage(AppState { ... })` initializer (after `anim_gen: AtomicU64::new(0),`):

```rust
            activity: Mutex::new(activity::ActivityTracker::new()),
            quota_samples: Mutex::new(std::collections::VecDeque::new()),
```

- [ ] **Step 2: Register the command and start the ticker**

In `.invoke_handler(tauri::generate_handler![ ... ])`, add `get_activity,` to the list (after `get_snapshot,`).

In the `.setup(...)` closure, after `spawn_poller(app.handle().clone());`:

```rust
            spawn_activity_ticker(app.handle().clone());
```

- [ ] **Step 3: Record 5h-% samples in `poll_once`**

In `poll_once`, after `*app.state::<AppState>().latest.lock().unwrap() = snap.clone();`:

```rust
    // Sample the 5h utilization for the time-to-empty slope (keep ~last 10,
    // drop anything older than 20 min so a slope never spans a window reset).
    if let Some(w) = &snap.quota.five_hour {
        let now = chrono::Local::now();
        let mut s = app.state::<AppState>().quota_samples.lock().unwrap();
        s.push_back((now, w.utilization));
        let cutoff = now - chrono::Duration::minutes(20);
        while s.front().map_or(false, |(t, _)| *t < cutoff) {
            s.pop_front();
        }
        while s.len() > 10 {
            s.pop_front();
        }
    }
```

- [ ] **Step 4: Add the command, builder, reset-parser, and ticker**

Add after the `get_snapshot` command:

```rust
#[tauri::command]
fn get_activity(app: AppHandle) -> activity::LiveActivity {
    build_activity(&app)
}
```

Add near the polling section (e.g. after `spawn_poller`):

```rust
fn parse_reset_secs(rfc3339: &str) -> Option<i64> {
    let dt = chrono::DateTime::parse_from_rfc3339(rfc3339).ok()?;
    let secs = (dt.with_timezone(&chrono::Local) - chrono::Local::now()).num_seconds();
    if secs > 0 {
        Some(secs)
    } else {
        None
    }
}

/// Tail transcripts and assemble the current LiveActivity snapshot.
fn build_activity(app: &AppHandle) -> activity::LiveActivity {
    let now = chrono::Local::now();
    let state = app.state::<AppState>();

    let optin = state.config.lock().unwrap().statusline_optin;
    let hint = if optin {
        statusline::read_hint_fresh(15)
    } else {
        None
    };
    let hint_fresh = hint.is_some();
    let force = hint
        .as_ref()
        .and_then(|h| h.transcript_path.clone())
        .map(std::path::PathBuf::from);
    let source = if hint_fresh { "statusline" } else { "jsonl" };

    state.activity.lock().unwrap().tick(now, force);

    let samples: Vec<_> = state.quota_samples.lock().unwrap().iter().cloned().collect();
    let (five_pct, reset_secs) = {
        let snap = state.latest.lock().unwrap();
        let five = snap.quota.five_hour.clone();
        (
            five.as_ref().map(|w| w.utilization),
            five.as_ref()
                .and_then(|w| w.resets_at.as_deref())
                .and_then(parse_reset_secs),
        )
    };

    state
        .activity
        .lock()
        .unwrap()
        .snapshot(now, hint_fresh, source, &samples, five_pct, reset_secs)
}

fn spawn_activity_ticker(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let show = app.state::<AppState>().config.lock().unwrap().show_activity;
            if show {
                let app2 = app.clone();
                if let Ok(act) =
                    tauri::async_runtime::spawn_blocking(move || build_activity(&app2)).await
                {
                    let _ = app.emit("activity-update", &act);
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}
```

- [ ] **Step 5: Support the `activity` window mode**

Add a size constant near `COMPACT`/`DETAILED`/`SETTINGS`:

```rust
const ACTIVITY: (f64, f64) = (210.0, 160.0);
```

In `apply_mode`, extend the size match:

```rust
    let (w, h) = match mode {
        "detailed" => DETAILED,
        "settings" => SETTINGS,
        "activity" => ACTIVITY,
        _ => COMPACT,
    };
```

In `set_mode`, allow persisting the new mode:

```rust
        if mode == "compact" || mode == "detailed" || mode == "activity" {
            c.mode = mode.clone();
            let _ = c.save();
        }
```

- [ ] **Step 6: Add a tray menu entry for the burn mode**

In `build_tray`, after the `detailed` MenuItem:

```rust
    let activity = MenuItem::with_id(app, "activity", "即時燒速", true, None::<&str>)?;
```

Add `&activity` to the `Menu::with_items` array (after `&detailed,`), and add a match arm in `on_menu_event` (after the `"detailed"` arm):

```rust
            "activity" => apply_mode_persist(app, "activity"),
```

- [ ] **Step 7: Build and run the Rust test suite**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: builds clean (no errors).
Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS (all tests).

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(lib): activity ticker, get_activity command, burn mode wiring"
```

---

## Task 7: Frontend format helpers

**Files:**
- Modify: `src/format.ts`, `src/format.test.ts`

- [ ] **Step 1: Add failing tests**

Append to `src/format.test.ts`:

```ts
import { fmtRate, fmtMinsToEmpty } from "./format";

describe("fmtRate", () => {
  it("abbreviates tok/min", () => {
    expect(fmtRate(12400)).toBe("12.4k");
    expect(fmtRate(940)).toBe("940");
    expect(fmtRate(0)).toBe("0");
  });
});

describe("fmtMinsToEmpty", () => {
  it("prefers the beats-reset message", () => {
    expect(fmtMinsToEmpty(120, true)).toBe("✓ 重置前不會見底");
  });
  it("formats minutes and hours with the ≈ marker", () => {
    expect(fmtMinsToEmpty(25, false)).toBe("≈ 25 分見底");
    expect(fmtMinsToEmpty(95, false)).toBe("≈ 1時35分見底");
  });
  it("returns empty for unknown / non-positive", () => {
    expect(fmtMinsToEmpty(null, false)).toBe("");
    expect(fmtMinsToEmpty(0, false)).toBe("");
  });
});
```

- [ ] **Step 2: Run, verify fail**

Run: `npm test -- format`
Expected: FAIL (`fmtRate` / `fmtMinsToEmpty` not exported).

- [ ] **Step 3: Implement the helpers**

Append to `src/format.ts`:

```ts
// tokens/min, abbreviated like fmtTokens but rounded for small values.
export function fmtRate(tpm: number): string {
  if (tpm >= 1000) return (tpm / 1000).toFixed(1) + "k";
  return String(Math.round(tpm));
}

// "5h empties in N" line. beatsReset wins; null/≤0 -> "" (caller hides it).
export function fmtMinsToEmpty(mins: number | null, beatsReset: boolean): string {
  if (beatsReset) return "✓ 重置前不會見底";
  if (mins == null || !isFinite(mins) || mins <= 0) return "";
  if (mins >= 60) {
    const h = Math.floor(mins / 60);
    const m = Math.round(mins % 60);
    return `≈ ${h}時${m}分見底`;
  }
  return `≈ ${Math.round(mins)} 分見底`;
}
```

- [ ] **Step 4: Run, verify pass**

Run: `npm test`
Expected: PASS (all vitest suites).

- [ ] **Step 5: Commit**

```bash
git add src/format.ts src/format.test.ts
git commit -m "feat(format): fmtRate and fmtMinsToEmpty helpers"
```

---

## Task 8: HTML + CSS (A/B/C/D markup + styles)

**Files:**
- Modify: `index.html`, `src/styles.css`

- [ ] **Step 1: Add the live-block to the detailed card (A/B)**

In `index.html`, inside `<div class="card-live">`, immediately after the closing `</div>` of the second `.meter` (the weekly meter) and before `<div class="foot">`, insert:

```html
        <div id="live-block" class="live-block" hidden>
          <div class="live-head">
            <span class="live-tag"><span class="dot"></span><span id="la-state">活動中</span>
              <span id="la-proj" class="live-proj"></span></span>
          </div>
          <div class="rate-row la-active">
            <span id="la-rate" class="rate-big">—</span><span class="rate-unit">tok/min</span>
          </div>
          <div id="la-empty" class="la-active la-emptyline"></div>
          <svg id="la-spark" class="spark la-active" viewBox="0 0 240 26" preserveAspectRatio="none"></svg>
          <div id="la-sess" class="sess"></div>
        </div>
```

- [ ] **Step 2: Add the compact-pill indicator (C)**

In `index.html`, inside `<span class="live">` of `#compact`, after the `<span id="c-reset" ...>` element, add:

```html
        <span id="c-livedot" class="livedot" hidden></span>
        <span id="c-liverate" class="liverate" hidden></span>
```

- [ ] **Step 3: Add the 🔥 button to the detailed header**

In `index.html`, inside the detailed card's `<span class="head-actions">`, before `<button id="btn-collapse" ...>`, add:

```html
          <button id="btn-activity" class="iconbtn" title="即時燒速">🔥</button>
```

- [ ] **Step 4: Add the standalone activity card (D)**

In `index.html`, after the closing `</div>` of `#detailed` and before `<!-- SETTINGS panel -->`, add:

```html
    <!-- ACTIVITY: standalone big burn-rate mode -->
    <div id="activity" class="widget card actcard">
      <div class="card-head" data-tauri-drag-region>
        <span class="title">🔥 即時燒速</span>
        <span class="head-actions">
          <button id="btn-act-back" class="iconbtn" title="返回詳細">←</button>
          <button id="btn-act-hide" class="iconbtn" title="收回系統匣">✕</button>
        </span>
      </div>
      <div class="actbody">
        <div class="live-tag"><span class="dot"></span><span id="act-state">活動中</span></div>
        <div id="act-rate" class="act-rate">—</div>
        <div class="act-unit">tok / 分鐘</div>
        <svg id="act-spark" class="spark" viewBox="0 0 180 26" preserveAspectRatio="none"></svg>
        <div id="act-empty" class="la-emptyline"></div>
      </div>
    </div>
```

- [ ] **Step 5: Add the settings option + checkbox**

In `index.html`, in the settings `<select id="s-mode">`, add a third option after `詳細`:

```html
          <option value="activity">即時燒速</option>
```

After the `s-alerts` checkbox `<label>` block, add:

```html
      <label class="srow check">
        <input id="s-activity" type="checkbox" /> 顯示即時活動
      </label>
```

- [ ] **Step 6: Add CSS**

In `src/styles.css`, add `--live` to `:root` (after `--ice: ...;`):

```css
  --live: #3ddc84;   /* live / active */
```

Add the new mode display rule (after the `body.mode-settings #settings` line):

```css
body.mode-activity #activity { display: flex; }
```

Append to the end of `src/styles.css`:

```css
/* ---------- live activity (A/B detailed block) ---------- */
.live-block {
  border-top: 1px solid var(--track);
  padding-top: 7px;
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.live-head { display: flex; align-items: center; justify-content: space-between; font-size: 11.5px; }
.live-tag { display: flex; align-items: center; gap: 6px; color: var(--live); font-weight: 600; }
.live-proj { color: var(--muted); font-weight: 400; }
.dot {
  width: 8px; height: 8px; border-radius: 50%; background: var(--live);
  box-shadow: 0 0 0 0 rgba(61, 220, 132, 0.6); animation: ping 1.6s ease-out infinite;
}
@keyframes ping {
  0% { box-shadow: 0 0 0 0 rgba(61, 220, 132, 0.55); }
  70% { box-shadow: 0 0 0 7px rgba(61, 220, 132, 0); }
  100% { box-shadow: 0 0 0 0 rgba(61, 220, 132, 0); }
}
.rate-row { display: flex; align-items: baseline; gap: 6px; }
.rate-big { font-size: 20px; font-weight: 700; color: var(--fg); font-variant-numeric: tabular-nums; }
.rate-unit { font-size: 11px; color: var(--muted); }
.la-emptyline { font-size: 11px; color: var(--live); }
.spark { width: 100%; height: 26px; display: block; }
.live-block .sess { display: flex; justify-content: space-between; font-size: 11px; color: var(--muted); }
.live-block .sess b { color: var(--fg); font-weight: 600; }

/* idle: grey, no pulse, hide the active-only rows */
.live-block.idle .dot { background: var(--muted); animation: none; box-shadow: none; }
.live-block.idle .live-tag { color: var(--muted); }
.live-block.idle .la-active { display: none; }

/* ---------- compact pill indicator (C) ---------- */
.pill .livedot {
  width: 7px; height: 7px; border-radius: 50%; background: var(--live);
  box-shadow: 0 0 0 0 rgba(61, 220, 132, 0.6); animation: ping 1.6s ease-out infinite; margin-left: auto;
}
.pill .liverate { color: var(--live); font-size: 11px; font-weight: 600; }

/* ---------- standalone burn mode (D) ---------- */
.actcard .actbody {
  flex: 1; display: flex; flex-direction: column; align-items: center;
  justify-content: center; text-align: center; gap: 4px;
}
.act-rate { font-size: 30px; font-weight: 800; font-variant-numeric: tabular-nums; }
.act-unit { font-size: 11px; color: var(--muted); }
.actcard .spark { width: 170px; }
```

- [ ] **Step 7: Commit (markup/styles compile via the build in Task 9)**

```bash
git add index.html src/styles.css
git commit -m "feat(ui): live-activity markup and styles (A/B/C/D)"
```

---

## Task 9: main.ts — types, listener, rendering, wiring

**Files:**
- Modify: `src/main.ts`

- [ ] **Step 1: Add the type and config field**

In `src/main.ts`, add after the `Snapshot` type:

```ts
type LiveActivity = {
  active: boolean;
  burn_tpm: number;
  session_tokens: number;
  last_active_secs: number;
  mins_to_empty: number | null;
  beats_reset: boolean;
  spark: number[];
  source: string;
};
```

Add `show_activity: boolean;` to the `Config` type (after `alert_effects: boolean;`).

Update the format import line:

```ts
import { fmtTokens, fmtCountdown, nextRenewal, fmtRate, fmtMinsToEmpty } from "./format";
```

Add module-level state after `let cfg: Config;`:

```ts
let latestActivity: LiveActivity | null = null;
let staleNow = false;
```

- [ ] **Step 2: Track stale + update setMode for the new mode**

In `render(s)`, after `const stale = ...; document.body.classList.toggle("stale", stale);`, add:

```ts
  staleNow = stale;
```

Replace the `setMode` function body with:

```ts
function setMode(mode: string) {
  document.body.classList.remove("mode-compact", "mode-detailed", "mode-settings", "mode-activity");
  const m = ["detailed", "settings", "activity"].includes(mode) ? mode : "compact";
  document.body.classList.add("mode-" + m);
}
```

- [ ] **Step 3: Add the sparkline drawer and `renderActivity`**

Add these functions (e.g. after `render`):

```ts
function drawSpark(svg: SVGElement, w: number, data: number[]) {
  const max = Math.max(...data, 1);
  const n = data.length;
  if (n < 2) {
    svg.innerHTML = "";
    return;
  }
  const step = w / (n - 1);
  const line = data
    .map((v, i) => `${i ? "L" : "M"}${(i * step).toFixed(1)},${(26 - (v / max) * 24).toFixed(1)}`)
    .join(" ");
  const area = `${line} L${w},26 L0,26 Z`;
  svg.innerHTML =
    `<defs><linearGradient id="lg" x1="0" x2="0" y1="0" y2="1">` +
    `<stop offset="0" stop-color="#3ddc84" stop-opacity=".35"/>` +
    `<stop offset="1" stop-color="#3ddc84" stop-opacity="0"/></linearGradient></defs>` +
    `<path d="${area}" fill="url(#lg)"/>` +
    `<path d="${line}" fill="none" stroke="#3ddc84" stroke-width="1.6" stroke-linejoin="round"/>`;
}

function idleAgo(secs: number): string {
  if (secs <= 0) return "—";
  const m = Math.round(secs / 60);
  return m < 1 ? "剛剛" : `${m} 分鐘前`;
}

function renderActivity(a: LiveActivity) {
  const block = $("live-block");
  const dot = $("c-livedot");
  const rate = $("c-liverate");

  // Master toggle off: hide everything related.
  if (!cfg.show_activity) {
    block.hidden = true;
    dot.hidden = true;
    rate.hidden = true;
    return;
  }

  // ----- A/B: detailed live-block -----
  block.hidden = false;
  block.classList.toggle("idle", !a.active);
  if (a.active) {
    $("la-state").textContent = "活動中";
    $("la-proj").textContent = a.source === "statusline" ? "· session 進行中" : "";
    $("la-rate").textContent = fmtRate(a.burn_tpm);
    $("la-empty").textContent = fmtMinsToEmpty(a.mins_to_empty, a.beats_reset);
    $("la-empty").hidden = $("la-empty").textContent === "";
    $("la-sess").textContent = `本次 session ${fmtTokens(a.session_tokens)} tok`;
    drawSpark($("la-spark") as unknown as SVGElement, 240, a.spark);
  } else {
    $("la-state").textContent = "💤 無活動 session";
    $("la-proj").textContent = "";
    $("la-sess").textContent = `最後活動 ${idleAgo(a.last_active_secs)}`;
  }

  // ----- C: compact pill indicator (auto-hidden by .live when stale) -----
  dot.hidden = !a.active;
  rate.hidden = !a.active;
  if (a.active) rate.textContent = fmtRate(a.burn_tpm) + "/m";

  // ----- D: standalone burn card -----
  if (staleNow) {
    $("act-state").textContent = "❄ token 已過期";
    $("act-rate").textContent = "—";
    $("act-empty").textContent = "請開啟 Claude Code 重新登入";
    ($("act-spark") as unknown as SVGElement).innerHTML = "";
  } else if (a.active) {
    $("act-state").textContent = "活動中";
    $("act-rate").textContent = fmtRate(a.burn_tpm);
    $("act-empty").textContent = fmtMinsToEmpty(a.mins_to_empty, a.beats_reset);
    drawSpark($("act-spark") as unknown as SVGElement, 180, a.spark);
  } else {
    $("act-state").textContent = "💤 無活動 session";
    $("act-rate").textContent = "0";
    $("act-empty").textContent = `最後活動 ${idleAgo(a.last_active_secs)}`;
    ($("act-spark") as unknown as SVGElement).innerHTML = "";
  }
}

// Reflect show_activity changes immediately (hide block, leave burn mode).
function applyActivityVisibility() {
  const opt = document.querySelector('#s-mode option[value="activity"]') as HTMLOptionElement | null;
  if (opt) opt.hidden = !cfg.show_activity;
  $("btn-activity").hidden = !cfg.show_activity;
  if (!cfg.show_activity && cfg.mode === "activity") {
    cfg.mode = "detailed";
    setMode("detailed");
    invoke("set_mode", { mode: "detailed" });
  }
  if (latestActivity) renderActivity(latestActivity);
}
```

- [ ] **Step 4: Wire settings, buttons, and the event listener**

In `wireSettings`, after the `s-alerts` handler, add:

```ts
  on("s-activity", "change", (el) => {
    cfg.show_activity = (el as HTMLInputElement).checked;
    saveCfg();
    applyActivityVisibility();
  });
```

In `populateSettings`, after the `s-alerts` line, add:

```ts
  (document.getElementById("s-activity") as HTMLInputElement).checked = cfg.show_activity;
```

In the `DOMContentLoaded` handler, after the `btn-hide` click handler, add the new button handlers:

```ts
  $("btn-activity").addEventListener("click", (e) => {
    e.stopPropagation();
    setMode("activity");
    invoke("set_mode", { mode: "activity" });
  });
  $("btn-act-back").addEventListener("click", (e) => {
    e.stopPropagation();
    setMode("detailed");
    invoke("set_mode", { mode: "detailed" });
  });
  $("btn-act-hide").addEventListener("click", (e) => {
    e.stopPropagation();
    invoke("hide_window");
  });
```

After the existing `await listen<Snapshot>("usage-update", ...)` block, add:

```ts
  await listen<LiveActivity>("activity-update", (ev) => {
    latestActivity = ev.payload;
    renderActivity(latestActivity);
  });
```

At the end of the `DOMContentLoaded` handler, after `invoke("refresh_now");`, add:

```ts
  applyActivityVisibility();
  invoke<LiveActivity>("get_activity").then((a) => {
    latestActivity = a;
    renderActivity(a);
  });
```

- [ ] **Step 5: Type-check and build the frontend**

Run: `npm run build`
Expected: `tsc` passes (no type errors) and Vite build succeeds.

- [ ] **Step 6: Run the frontend tests**

Run: `npm test`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/main.ts
git commit -m "feat(ui): render live activity (A/B/C/D) + mode/settings wiring"
```

---

## Task 10: Full build, test, and manual verification

**Files:** none (verification only)

- [ ] **Step 1: Full Rust test + build**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS (config, activity, statusline, usage, quota, icon).
Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: clean build.

- [ ] **Step 2: Full frontend test + build**

Run: `npm test && npm run build`
Expected: vitest PASS, tsc + Vite build clean.

- [ ] **Step 3: Manual smoke test (dev)**

Run: `npm run tauri dev`
Verify, with a Claude Code session actively running:
- Detailed mode shows the green pulsing dot, a tok/min number, a sparkline, and "本次 session … tok" (A).
- Stop the session ~2 min; the block collapses to grey "💤 無活動 session · 最後活動 N 分鐘前" (B).
- Compact mode shows the green dot + `Nk/m` while active, and hides both when idle (C).
- Click 🔥 in the detailed header → standalone burn card with the big number + sparkline; ← returns to detailed (D).
- Settings → uncheck 「顯示即時活動」 → block/indicator disappear, the 即時燒速 mode option/button hide.
- Let the token go stale (or simulate): the detailed block hides with the frozen card; the burn card shows "❄ token 已過期".

- [ ] **Step 4: Update README**

Add a bullet to the 功能 list in `README.md` describing the live-activity feature (burn rate tok/min, sparkline, session total, 5h time-to-empty estimate, statusline-priority/jsonl-fallback, 即時燒速 mode, `show_activity` toggle). Add `show_activity` to the 設定檔 line.

```bash
git add README.md
git commit -m "docs: document live activity status feature"
```

- [ ] **Step 5: Final commit (if any uncommitted changes remain)**

```bash
git status   # expect clean
```

---

## Self-Review Notes

- **Spec coverage:** data model (T2/T4) ✓; engine + incremental tail (T4) ✓; pure fns + tests (T2/T3) ✓; statusline-priority/jsonl-fallback (T5/T6) ✓; mins_to_empty via %-slope + beats_reset (T3/T6) ✓; two cadences + samples (T6) ✓; A/B/C/D render (T8/T9) ✓; show_activity setting + off-behavior incl. removing the mode option/button and leaving burn mode (T1/T8/T9 `applyActivityVisibility`) ✓; stale interaction (T9 + existing `.card-live` CSS) ✓; Rust + vitest tests (T2–T5,T7) ✓.
- **Token semantics:** burn rate, sparkline, and session total all use input+output (consistent with the "今日 tok" headline), per spec.
- **Naming consistency:** `LiveActivity`, `ActivityTracker`, `tick`/`tick_in`/`snapshot`, `build_activity`, `get_activity`, `activity-update`, `show_activity`, `fmtRate`, `fmtMinsToEmpty`, `renderActivity`, `applyActivityVisibility` used identically across tasks.
- **IO/TDD boundary:** all math is unit-tested pure functions; the tracker has a temp-file integration test (T4); lib.rs glue is verified by build + manual smoke test (T6/T10), an honest boundary for Tauri runtime wiring.
