//! Opt-in statusline source (default OFF).
//!
//! When enabled, we register `<this-exe> --statusline` as Claude Code's
//! statusLine command. Claude Code pipes session JSON (including `rate_limits`
//! for Pro/Max) to its stdin on every render; our hook extracts the quota,
//! writes it to a 0600 file the app reads, and echoes a short status line back.
//!
//! Enabling backs up `~/.claude/settings.json` and refuses to overwrite an
//! existing user statusLine. The OAuth token is never involved here.

use crate::config::Config;
use crate::quota::{QuotaUsage, QuotaWindow};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub fn data_path() -> PathBuf {
    Config::dir().join("statusline.json")
}

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

fn settings_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude/settings.json"))
}

/// Pure: tokens occupying the model's context window for one request, i.e. the
/// total input sent that turn. Output is excluded (it is not yet context).
pub fn context_fill(usage: &serde_json::Value) -> u64 {
    let g = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    g("input_tokens") + g("cache_creation_input_tokens") + g("cache_read_input_tokens")
}

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

/// Read at most the last 128 KiB of a transcript and return the latest context
/// fill. Bounds work regardless of transcript size; a partial leading line is
/// harmlessly skipped by the parser. The tail seek may land mid-codepoint when
/// the file contains multi-byte UTF-8 characters; we use a lossy conversion so
/// that a single garbled boundary byte does not discard the entire read.
pub fn read_context_fill(path: &Path) -> Option<u64> {
    const TAIL_BYTES: u64 = 128 * 1024;
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut raw = Vec::new();
    f.read_to_end(&mut raw).ok()?;
    let buf = String::from_utf8_lossy(&raw);
    parse_last_context_fill(&buf)
}

/// Pure: render a percentage with an ANSI 256-color by danger level
/// (<50 green 114, 50–79 yellow 221, >=80 red 203). Claude Code renders
/// ANSI escapes in statusline output.
pub fn paint_pct(pct: f64) -> String {
    let color = if pct >= 80.0 {
        203
    } else if pct >= 50.0 {
        221
    } else {
        114
    };
    format!("\x1b[38;5;{color}m{pct:.0}%\x1b[0m")
}

/// Pure: " · <label> <pct>" segments for model-scoped limits ("" when none).
pub fn model_segment(limits: &[crate::quota::ModelLimit]) -> String {
    limits
        .iter()
        .map(|l| format!(" · {} {}", l.label, paint_pct(l.percent)))
        .collect()
}

/// Pure: the " · ctx N%" suffix, or "" when fill is unknown or window is invalid.
pub fn ctx_segment(fill: Option<u64>, window: u64) -> String {
    match fill {
        Some(f) if window > 0 => {
            let pct = ((f as f64 / window as f64) * 100.0).round().min(100.0);
            format!(" · ctx {}", paint_pct(pct))
        }
        _ => String::new(),
    }
}

fn win_from(v: &serde_json::Value) -> Option<QuotaWindow> {
    // The percentage key has varied across Claude Code versions; accept the known
    // spellings. (statusline-raw.json reveals the actual one if none of these hit.)
    let u = ["used_percentage", "utilization", "percent_used", "percentage"]
        .iter()
        .find_map(|k| v.get(*k).and_then(|x| x.as_f64()))?;
    let r = ["resets_at", "reset_at", "resetsAt"]
        .iter()
        .filter_map(|k| v.get(*k))
        .find(|x| !x.is_null())
        .and_then(crate::quota::normalize_reset);
    Some(QuotaWindow {
        utilization: u,
        resets_at: r,
    })
}

/// Invoked as `<exe> --statusline`: read stdin, persist quota, echo a line.
pub fn run_hook() {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);

    // Diagnostic: persist the raw payload Claude Code sends so we can verify the
    // exact rate-limit schema (it varies by version/plan, and the parsed file
    // alone can't tell us why a window came back null). Owner-only, local file.
    {
        let p = Config::dir().join("statusline-raw.json");
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(&p, &input).is_ok() {
            set_owner_only(&p);
        }
    }

    let v: serde_json::Value = serde_json::from_str(&input).unwrap_or(serde_json::json!({}));
    let rl = &v["rate_limits"];

    let usage = QuotaUsage {
        five_hour: win_from(&rl["five_hour"]),
        seven_day: win_from(&rl["seven_day"]),
        seven_day_opus: win_from(&rl["seven_day_opus"]),
        seven_day_sonnet: win_from(&rl["seven_day_sonnet"]),
        ..Default::default()
    };

    if let Ok(json) = serde_json::to_string(&usage) {
        let p = data_path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(&p, json).is_ok() {
            set_owner_only(&p);
        }
    }

    // Persist the active-session hint (best-effort) for the live-activity ticker.
    let hint = parse_hint(&v);
    if let Ok(json) = serde_json::to_string(&hint) {
        let p = hint_path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(&p, json).is_ok() {
            set_owner_only(&p);
        }
    }

    let fmt = |w: &Option<QuotaWindow>| {
        w.as_ref()
            .map(|x| paint_pct(x.utilization))
            .unwrap_or_else(|| "—".into())
    };
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
}

fn set_owner_only(p: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = p;
}

/// Read the statusline-provided quota if the file was updated recently.
pub fn read_fresh(max_age_secs: u64) -> Option<QuotaUsage> {
    let p = data_path();
    let modified = std::fs::metadata(&p).ok()?.modified().ok()?;
    let age = std::time::SystemTime::now().duration_since(modified).ok()?;
    if age.as_secs() > max_age_secs {
        return None;
    }
    serde_json::from_str(&std::fs::read_to_string(&p).ok()?).ok()
}

fn our_command() -> String {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "claude-usage-monitor".into());
    // Quote the path: Claude Code runs this via a shell, and the macOS app lives
    // at "/Applications/Claude Usage Monitor.app/..." — the spaces would otherwise
    // split the command and the hook would never fire (so statusline.json never
    // gets written). Quoting is harmless on space-free Linux paths too.
    format!("\"{exe}\" --statusline")
}

fn is_ours(sl: &serde_json::Value) -> bool {
    sl.get("command")
        .and_then(|c| c.as_str())
        .map(|c| c.contains("--statusline") && c.contains("claude-usage-monitor"))
        .unwrap_or(false)
}

/// Register our statusLine command, backing up settings.json and refusing to
/// clobber an existing user statusLine.
pub fn enable() -> Result<(), String> {
    enable_at(&settings_path().ok_or("找不到家目錄")?)
}

fn enable_at(path: &std::path::Path) -> Result<(), String> {
    let mut obj: serde_json::Value = if path.exists() {
        let s = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&s).map_err(|e| format!("settings.json 解析失敗: {e}"))?
    } else {
        serde_json::json!({})
    };

    if let Some(existing) = obj.get("statusLine") {
        if !is_ours(existing) {
            return Err("偵測到你已有自訂 statusLine，為避免覆蓋未做更動。請先移除既有設定再啟用。".into());
        }
    }

    if path.exists() {
        let _ = std::fs::copy(path, path.with_extension("json.cum-backup"));
    }
    obj["statusLine"] = serde_json::json!({
        "type": "command",
        "command": our_command(),
        "padding": 0
    });
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let out = serde_json::to_string_pretty(&obj).map_err(|e| e.to_string())?;
    std::fs::write(path, out + "\n").map_err(|e| e.to_string())
}

/// Remove our statusLine entry (only if it is ours).
pub fn disable() -> Result<(), String> {
    disable_at(&settings_path().ok_or("找不到家目錄")?)
}

fn disable_at(path: &std::path::Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let s = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut obj: serde_json::Value = serde_json::from_str(&s).map_err(|e| e.to_string())?;
    if obj.get("statusLine").map(is_ours).unwrap_or(false) {
        if let Some(m) = obj.as_object_mut() {
            m.remove("statusLine");
        }
        let out = serde_json::to_string_pretty(&obj).map_err(|e| e.to_string())?;
        std::fs::write(path, out + "\n").map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_command_is_quoted_and_detected_as_ours() {
        let cmd = our_command();
        assert!(cmd.starts_with('"'), "exe path must be quoted: {cmd}");
        assert!(cmd.ends_with("--statusline"));
        // A quoted command with spaces in the path must still be recognized as ours.
        assert!(is_ours(&serde_json::json!({ "command": cmd })));
    }

    #[test]
    fn win_from_accepts_alternate_keys() {
        let a = win_from(&serde_json::json!({"used_percentage": 14.0, "resets_at": "t"})).unwrap();
        assert_eq!(a.utilization, 14.0);
        assert_eq!(a.resets_at.as_deref(), Some("t"));
        // alternate spelling still parses
        let b = win_from(&serde_json::json!({"utilization": 9.0})).unwrap();
        assert_eq!(b.utilization, 9.0);
        assert!(b.resets_at.is_none());
        // no recognized key -> None
        assert!(win_from(&serde_json::json!({"foo": 1})).is_none());
    }

    #[test]
    fn win_from_converts_epoch_resets_at() {
        // Claude Code's statusline now reports rate_limits.*.resets_at as a Unix
        // epoch (seconds), not an RFC3339 string. We must normalize it to a string
        // so the frontend (which does `new Date(resets_at)`) can parse it — matching
        // the OAuth path. A bare integer used to slip through .as_str() as None.
        let w = win_from(&serde_json::json!({"used_percentage": 33, "resets_at": 1782200400}))
            .unwrap();
        assert_eq!(w.utilization, 33.0);
        assert_eq!(w.resets_at.as_deref(), Some("2026-06-23T07:40:00+00:00"));
        // a null reset stays None
        let n = win_from(&serde_json::json!({"used_percentage": 1, "resets_at": null})).unwrap();
        assert!(n.resets_at.is_none());
    }

    #[test]
    fn is_ours_detects() {
        assert!(is_ours(
            &serde_json::json!({"command":"/x/claude-usage-monitor --statusline"})
        ));
        assert!(!is_ours(&serde_json::json!({"command":"my-bar --foo"})));
        assert!(!is_ours(&serde_json::json!({})));
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cum-test-{}-{}", std::process::id(), name));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("settings.json")
    }

    #[test]
    fn enable_disable_roundtrip_preserves_other_keys() {
        let path = tmp("roundtrip");
        std::fs::write(&path, r#"{"theme":"dark","x":1}"#).unwrap();

        enable_at(&path).unwrap();
        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(after.get("statusLine").is_some());
        assert_eq!(after["theme"], "dark");

        disable_at(&path).unwrap();
        let restored: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(restored.get("statusLine").is_none());
        assert_eq!(restored["theme"], "dark");
        assert_eq!(restored["x"], 1);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn enable_refuses_existing_user_statusline() {
        let path = tmp("refuse");
        std::fs::write(&path, r#"{"statusLine":{"command":"other --bar"}}"#).unwrap();
        assert!(enable_at(&path).is_err());
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn ctx_segment_formats_and_clamps() {
        assert_eq!(ctx_segment(Some(86_000), 200_000), format!(" · ctx {}", paint_pct(43.0)));
        assert_eq!(ctx_segment(Some(500_000), 1_000_000), format!(" · ctx {}", paint_pct(50.0)));
        // over-100 (wrong denominator) clamps to 100
        assert_eq!(ctx_segment(Some(250_000), 200_000), format!(" · ctx {}", paint_pct(100.0)));
        // unknown fill -> empty segment (omitted entirely)
        assert_eq!(ctx_segment(None, 200_000), "");
        // guard against zero denominator
        assert_eq!(ctx_segment(Some(10), 0), "");
    }

    #[test]
    fn paint_pct_colors_by_danger() {
        assert_eq!(paint_pct(4.0), "\x1b[38;5;114m4%\x1b[0m");    // <50 綠
        assert_eq!(paint_pct(50.0), "\x1b[38;5;221m50%\x1b[0m");  // 50-79 黃
        assert_eq!(paint_pct(79.4), "\x1b[38;5;221m79%\x1b[0m");
        assert_eq!(paint_pct(80.0), "\x1b[38;5;203m80%\x1b[0m");  // >=80 紅
        assert_eq!(paint_pct(17.6), "\x1b[38;5;114m18%\x1b[0m");  // {:.0} 四捨五入
    }

    #[test]
    fn model_segment_renders_each_limit() {
        use crate::quota::ModelLimit;
        let limits = vec![ModelLimit { label: "Fable".into(), percent: 17.0, resets_at: None }];
        assert_eq!(model_segment(&limits), format!(" · Fable {}", paint_pct(17.0)));
        assert_eq!(model_segment(&[]), "");
        let two = vec![
            ModelLimit { label: "A".into(), percent: 1.0, resets_at: None },
            ModelLimit { label: "B".into(), percent: 2.0, resets_at: None },
        ];
        assert_eq!(
            model_segment(&two),
            format!(" · A {} · B {}", paint_pct(1.0), paint_pct(2.0))
        );
    }

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

    #[test]
    fn read_context_fill_survives_utf8_boundary() {
        // Build a transcript larger than 128 KiB (TAIL_BYTES) so the tail-seek
        // path (start > 0) is exercised. The bulk padding contains multi-byte
        // UTF-8 codepoints (Chinese characters) so the 128 KiB seek boundary
        // can land mid-codepoint. With the old read_to_string implementation
        // that would yield an InvalidData error and silently return None.
        let dir = std::env::temp_dir().join(format!("cum-utf8-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("large.jsonl");

        // Each Chinese char is 3 bytes in UTF-8; build a padding line > 128 KiB.
        // "安安你好" repeated ~12 000 times ≈ 192 KiB (well over the 128 KiB tail).
        let filler: String = "安安你好".repeat(12_000);
        let padding_line = format!(
            "{{\"type\":\"assistant\",\"message\":{{\"content\":\"{filler}\",\"usage\":{{\"input_tokens\":1}}}}}}\n"
        );
        // Final valid line that should be the last parsed usage.
        let last_line =
            "{\"type\":\"assistant\",\"message\":{\"usage\":{\"input_tokens\":42,\"cache_read_input_tokens\":8}}}\n";

        let mut content = padding_line;
        content.push_str(last_line);

        // Sanity-check: the file must exceed 128 KiB so start > 0.
        assert!(
            content.len() > 128 * 1024,
            "test file too small: {} bytes",
            content.len()
        );

        std::fs::write(&path, &content).unwrap();

        // Expected: last line's usage = 42 + 8 = 50.
        assert_eq!(read_context_fill(&path), Some(50));

        std::fs::remove_dir_all(&dir).ok();
    }

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
}
