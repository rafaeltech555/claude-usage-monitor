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
use std::io::Read;
use std::path::{Path, PathBuf};

pub fn data_path() -> PathBuf {
    Config::dir().join("statusline.json")
}

fn settings_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude/settings.json"))
}

fn win_from(v: &serde_json::Value) -> Option<QuotaWindow> {
    let u = v.get("used_percentage").and_then(|x| x.as_f64())?;
    let r = v
        .get("resets_at")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    Some(QuotaWindow {
        utilization: u,
        resets_at: r,
    })
}

/// Invoked as `<exe> --statusline`: read stdin, persist quota, echo a line.
pub fn run_hook() {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let v: serde_json::Value = serde_json::from_str(&input).unwrap_or(serde_json::json!({}));
    let rl = &v["rate_limits"];

    let usage = QuotaUsage {
        five_hour: win_from(&rl["five_hour"]),
        seven_day: win_from(&rl["seven_day"]),
        seven_day_opus: win_from(&rl["seven_day_opus"]),
        seven_day_sonnet: win_from(&rl["seven_day_sonnet"]),
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

    let fmt = |w: &Option<QuotaWindow>| {
        w.as_ref()
            .map(|x| format!("{:.0}%", x.utilization))
            .unwrap_or_else(|| "—".into())
    };
    print!("⚡ {} · 7d {}", fmt(&usage.five_hour), fmt(&usage.seven_day));
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
    format!("{exe} --statusline")
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
    let path = settings_path().ok_or("找不到家目錄")?;
    let mut obj: serde_json::Value = if path.exists() {
        let s = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
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
        let _ = std::fs::copy(&path, path.with_extension("json.cum-backup"));
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
    std::fs::write(&path, out + "\n").map_err(|e| e.to_string())
}

/// Remove our statusLine entry (only if it is ours).
pub fn disable() -> Result<(), String> {
    let path = settings_path().ok_or("找不到家目錄")?;
    if !path.exists() {
        return Ok(());
    }
    let s = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut obj: serde_json::Value = serde_json::from_str(&s).map_err(|e| e.to_string())?;
    if obj.get("statusLine").map(is_ours).unwrap_or(false) {
        if let Some(m) = obj.as_object_mut() {
            m.remove("statusLine");
        }
        let out = serde_json::to_string_pretty(&obj).map_err(|e| e.to_string())?;
        std::fs::write(&path, out + "\n").map_err(|e| e.to_string())?;
    }
    Ok(())
}
