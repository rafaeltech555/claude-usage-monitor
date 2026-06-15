# Statusline Context % Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Append the current session's context-window utilization (`· ctx N%`) to the statusline string this app prints for Claude Code.

**Architecture:** All changes live in `src-tauri/src/statusline.rs`. Four small pure functions (sum a usage object, pick the context-window denominator, find the last usage in a transcript tail, format the percent segment) plus a thin tail-reading IO wrapper, wired into `run_hook()`. `activity.rs` is untouched — its `session_tokens` is cumulative burn, a different quantity.

**Tech Stack:** Rust, `serde_json`, std `fs`/`io` (`Seek`/`Read`). Tests are inline `#[cfg(test)] mod tests` in `statusline.rs`, run with `cargo test`.

**Spec:** `docs/superpowers/specs/2026-06-15-statusline-context-pct-design.md`

**Test command (used throughout):**
```bash
cargo test --manifest-path /home/finn/sideproject/claude-usage-monitor/src-tauri/Cargo.toml statusline
```

---

### Task 1: `context_fill` — sum a usage object into current context size

**Files:**
- Modify: `src-tauri/src/statusline.rs` (add fn near `win_from`, add test in `mod tests`)

- [ ] **Step 1: Write the failing test**

Add inside `mod tests`:

```rust
#[test]
fn context_fill_sums_input_and_cache() {
    let u = serde_json::json!({
        "input_tokens": 2,
        "cache_creation_input_tokens": 1693,
        "cache_read_input_tokens": 48565,
        "output_tokens": 9999
    });
    assert_eq!(context_fill(&u), 50260); // output is NOT part of context fill
    // missing keys count as 0
    assert_eq!(context_fill(&serde_json::json!({"input_tokens": 10})), 10);
    assert_eq!(context_fill(&serde_json::json!({})), 0);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path /home/finn/sideproject/claude-usage-monitor/src-tauri/Cargo.toml statusline::tests::context_fill_sums_input_and_cache`
Expected: FAIL — `cannot find function context_fill in this scope`.

- [ ] **Step 3: Write minimal implementation**

Add near the top of `statusline.rs` (after `win_from`):

```rust
/// Pure: tokens occupying the model's context window for one request, i.e. the
/// total input sent that turn. Output is excluded (it is not yet context).
pub fn context_fill(usage: &serde_json::Value) -> u64 {
    let g = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    g("input_tokens") + g("cache_creation_input_tokens") + g("cache_read_input_tokens")
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path /home/finn/sideproject/claude-usage-monitor/src-tauri/Cargo.toml statusline::tests::context_fill_sums_input_and_cache`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/statusline.rs
git commit -m "feat(statusline): add context_fill to sum context-window tokens"
```

---

### Task 2: `context_window` — pick the denominator (200k vs 1M)

**Files:**
- Modify: `src-tauri/src/statusline.rs`

- [ ] **Step 1: Write the failing test**

Add inside `mod tests`:

```rust
#[test]
fn context_window_detects_1m() {
    // model id carrying the 1m marker -> 1M
    assert_eq!(context_window("claude-opus-4-8[1m]", false), 1_000_000);
    assert_eq!(context_window("CLAUDE-OPUS-4-8-1M", false), 1_000_000);
    // no marker, but already past 200k -> must be a 1M session
    assert_eq!(context_window("claude-opus-4-8", true), 1_000_000);
    // no marker, not over 200k -> default 200k
    assert_eq!(context_window("claude-opus-4-8", false), 200_000);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path /home/finn/sideproject/claude-usage-monitor/src-tauri/Cargo.toml statusline::tests::context_window_detects_1m`
Expected: FAIL — `cannot find function context_window`.

- [ ] **Step 3: Write minimal implementation**

Add to `statusline.rs`:

```rust
/// Pure: context-window size for the denominator. The statusline payload's
/// model id may or may not carry a "1m" marker (the transcript's never does), so
/// we also treat `exceeds_200k_tokens` as proof of a 1M session.
pub fn context_window(model_id: &str, exceeds_200k: bool) -> u64 {
    if model_id.to_lowercase().contains("1m") || exceeds_200k {
        1_000_000
    } else {
        200_000
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path /home/finn/sideproject/claude-usage-monitor/src-tauri/Cargo.toml statusline::tests::context_window_detects_1m`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/statusline.rs
git commit -m "feat(statusline): add context_window denominator detection"
```

---

### Task 3: `parse_last_context_fill` — last usage from transcript text

**Files:**
- Modify: `src-tauri/src/statusline.rs`

Note: each transcript line is one JSON object. A truncated first line (from a mid-file tail read) simply fails `serde_json::from_str` and is skipped — no explicit "drop first line" step needed.

- [ ] **Step 1: Write the failing test**

Add inside `mod tests`:

```rust
#[test]
fn parse_last_context_fill_takes_last_assistant_usage() {
    let content = concat!(
        "{\"type\":\"user\"}\n",
        "{\"type\":\"assistant\",\"message\":{\"usage\":{\"input_tokens\":1,\"cache_read_input_tokens\":100}}}\n",
        "{\"type\":\"assistant\",\"message\":{\"usage\":{\"input_tokens\":5,\"cache_creation_input_tokens\":20,\"cache_read_input_tokens\":300}}}\n"
    );
    // last assistant usage: 5 + 20 + 300 = 325
    assert_eq!(parse_last_context_fill(content), Some(325));

    // a truncated leading line is ignored; the valid line below still counts
    let partial = concat!(
        "ut_tokens\":1,\"cache_read_input_tokens\":100}}}\n",
        "{\"type\":\"assistant\",\"message\":{\"usage\":{\"input_tokens\":7}}}\n"
    );
    assert_eq!(parse_last_context_fill(partial), Some(7));

    // no assistant-with-usage -> None
    assert_eq!(parse_last_context_fill("{\"type\":\"user\"}\nnot json\n"), None);
    assert_eq!(parse_last_context_fill(""), None);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path /home/finn/sideproject/claude-usage-monitor/src-tauri/Cargo.toml statusline::tests::parse_last_context_fill_takes_last_assistant_usage`
Expected: FAIL — `cannot find function parse_last_context_fill`.

- [ ] **Step 3: Write minimal implementation**

Add to `statusline.rs`:

```rust
/// Pure: scan complete transcript lines, return the context fill of the LAST
/// assistant message that carries usage. Lines that fail to parse (e.g. a
/// truncated tail line) are skipped.
pub fn parse_last_context_fill(content: &str) -> Option<u64> {
    let mut last: Option<u64> = None;
    for line in content.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        if let Some(usage) = v.get("message").and_then(|m| m.get("usage")) {
            last = Some(context_fill(usage));
        }
    }
    last
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path /home/finn/sideproject/claude-usage-monitor/src-tauri/Cargo.toml statusline::tests::parse_last_context_fill_takes_last_assistant_usage`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/statusline.rs
git commit -m "feat(statusline): parse last context fill from transcript text"
```

---

### Task 4: `read_context_fill` — tail-read the transcript file

**Files:**
- Modify: `src-tauri/src/statusline.rs`

- [ ] **Step 1: Write the failing test**

Add inside `mod tests`:

```rust
#[test]
fn read_context_fill_tails_a_file() {
    let dir = std::env::temp_dir().join(format!("cum-ctx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.jsonl");
    std::fs::write(
        &path,
        concat!(
            "{\"type\":\"assistant\",\"message\":{\"usage\":{\"input_tokens\":1}}}\n",
            "{\"type\":\"assistant\",\"message\":{\"usage\":{\"input_tokens\":9,\"cache_read_input_tokens\":11}}}\n"
        ),
    )
    .unwrap();

    assert_eq!(read_context_fill(&path), Some(20)); // last line: 9 + 11

    // missing file -> None
    assert_eq!(read_context_fill(&dir.join("nope.jsonl")), None);

    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path /home/finn/sideproject/claude-usage-monitor/src-tauri/Cargo.toml statusline::tests::read_context_fill_tails_a_file`
Expected: FAIL — `cannot find function read_context_fill`.

- [ ] **Step 3: Write minimal implementation**

`std::io::{Read, Seek, SeekFrom}` are needed. `Read` is already imported at the top of `statusline.rs`; add `Seek` and `SeekFrom`. Change the existing `use std::io::Read;` line to:

```rust
use std::io::{Read, Seek, SeekFrom};
```

Then add:

```rust
/// Read at most the last 128 KiB of a transcript and return the latest context
/// fill. Bounds work regardless of transcript size; a partial leading line is
/// harmlessly skipped by the parser.
pub fn read_context_fill(path: &Path) -> Option<u64> {
    const TAIL_BYTES: u64 = 128 * 1024;
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = String::new();
    f.read_to_string(&mut buf).ok()?;
    parse_last_context_fill(&buf)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path /home/finn/sideproject/claude-usage-monitor/src-tauri/Cargo.toml statusline::tests::read_context_fill_tails_a_file`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/statusline.rs
git commit -m "feat(statusline): tail-read transcript for latest context fill"
```

---

### Task 5: `ctx_segment` + wire into `run_hook`

**Files:**
- Modify: `src-tauri/src/statusline.rs` (add `ctx_segment`, edit `run_hook`'s final `print!`)

- [ ] **Step 1: Write the failing test**

Add inside `mod tests`:

```rust
#[test]
fn ctx_segment_formats_and_clamps() {
    assert_eq!(ctx_segment(Some(86_000), 200_000), " · ctx 43%");
    assert_eq!(ctx_segment(Some(500_000), 1_000_000), " · ctx 50%");
    // over-100 (wrong denominator) clamps to 100
    assert_eq!(ctx_segment(Some(250_000), 200_000), " · ctx 100%");
    // unknown fill -> empty segment (omitted entirely)
    assert_eq!(ctx_segment(None, 200_000), "");
    // guard against zero denominator
    assert_eq!(ctx_segment(Some(10), 0), "");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path /home/finn/sideproject/claude-usage-monitor/src-tauri/Cargo.toml statusline::tests::ctx_segment_formats_and_clamps`
Expected: FAIL — `cannot find function ctx_segment`.

- [ ] **Step 3: Write minimal implementation**

Add to `statusline.rs`:

```rust
/// Pure: the " · ctx N%" suffix, or "" when fill is unknown or window is invalid.
pub fn ctx_segment(fill: Option<u64>, window: u64) -> String {
    match fill {
        Some(f) if window > 0 => {
            let pct = ((f as f64 / window as f64) * 100.0).round().min(100.0) as u64;
            format!(" · ctx {pct}%")
        }
        _ => String::new(),
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path /home/finn/sideproject/claude-usage-monitor/src-tauri/Cargo.toml statusline::tests::ctx_segment_formats_and_clamps`
Expected: PASS.

- [ ] **Step 5: Wire into `run_hook`**

In `run_hook()`, replace the final line:

```rust
    print!("⚡ {} · 7d {}", fmt(&usage.five_hour), fmt(&usage.seven_day));
```

with:

```rust
    let ctx = v
        .get("transcript_path")
        .and_then(|p| p.as_str())
        .and_then(|p| read_context_fill(Path::new(p)));
    let window = context_window(
        v.get("model").and_then(|m| m.get("id")).and_then(|x| x.as_str()).unwrap_or(""),
        v.get("exceeds_200k_tokens").and_then(|x| x.as_bool()).unwrap_or(false),
    );
    print!(
        "⚡ {} · 7d {}{}",
        fmt(&usage.five_hour),
        fmt(&usage.seven_day),
        ctx_segment(ctx, window)
    );
```

- [ ] **Step 6: Run the full statusline test suite**

Run: `cargo test --manifest-path /home/finn/sideproject/claude-usage-monitor/src-tauri/Cargo.toml statusline`
Expected: PASS — all existing tests plus the 5 new ones.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/statusline.rs
git commit -m "feat(statusline): show current-session context % in statusline"
```

---

### Task 6: Manual smoke check (no code)

- [ ] **Step 1: Build and exercise the hook with a real transcript**

Run (pipe a minimal payload pointing at a real transcript; substitute an existing `*.jsonl` path):

```bash
cargo build --manifest-path /home/finn/sideproject/claude-usage-monitor/src-tauri/Cargo.toml
TP=$(ls -t ~/.claude/projects/*/*.jsonl | head -1)
printf '{"transcript_path":"%s","model":{"id":"claude-opus-4-8"},"rate_limits":{},"exceeds_200k_tokens":false}' "$TP" \
  | /home/finn/sideproject/claude-usage-monitor/src-tauri/target/debug/claude-usage-monitor --statusline; echo
```

Expected: a line ending with `· ctx N%` where N is a plausible percent (e.g. the ~50k/200k ≈ 25% seen during design). If the transcript has no assistant-with-usage, the `· ctx` segment is correctly absent.

- [ ] **Step 2: Confirm the raw payload dump (Open Item)**

After a real Claude Code render with this build registered, inspect `~/.config/claude-usage-monitor/statusline-raw.json` to confirm whether the statusline `model.id` carries a `[1m]` marker. Record the finding; if it does, the `exceeds_200k_tokens` fallback in `context_window` becomes belt-and-suspenders rather than the primary 1M signal. (This ties into the separately-paused dev-build-shadowing issue.)

---

## Self-Review

- **Spec coverage:** context_fill (Task 1) ✓ formula; context_window (Task 2) ✓ 3-rule detection; tail read 128 KiB + last usage (Tasks 3–4) ✓; display `· ctx N%` + omit-when-unknown + clamp 100% (Task 5) ✓; only `statusline.rs` touched, `activity.rs` untouched ✓; edge cases (missing file, no usage, >100%, partial first line) ✓ covered across Tasks 3–5 tests; Open Item on model.id (Task 6 Step 2) ✓.
- **Placeholder scan:** none — every code/test step has complete content.
- **Type consistency:** `context_fill(&Value)->u64`, `context_window(&str,bool)->u64`, `parse_last_context_fill(&str)->Option<u64>`, `read_context_fill(&Path)->Option<u64>`, `ctx_segment(Option<u64>,u64)->String` — names and signatures consistent across all tasks and the `run_hook` wiring.
