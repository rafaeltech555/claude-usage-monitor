//! User configuration: persisted to ~/.config/claude-usage-monitor/config.json

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const MIN_POLL_SECS: u64 = 180; // /api/oauth/usage throttles hard; never poll faster.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// "compact" | "detailed"
    pub mode: String,
    /// Pinned corner: "tl" | "tr" | "bl" | "br"
    pub corner: String,
    /// Quota poll interval in seconds (clamped to >= 180 on load).
    pub poll_secs: u64,
    /// Utilization (%) at which the UI turns amber.
    pub warn_threshold: f64,
    /// Utilization (%) at which the UI turns red.
    pub crit_threshold: f64,
    /// Window opacity 0.0..=1.0 (applied in the frontend).
    pub opacity: f64,
    /// Launch on login (wired up in a later milestone).
    pub autostart: bool,
    /// Opt-in: register a statusline command in ~/.claude/settings.json.
    /// Default OFF — we never touch the user's settings unless they enable this.
    pub statusline_optin: bool,
    /// Show the flame effect on the tray rings when usage rises.
    pub effects: bool,
    /// Show a prominent pulsing alert on the widget when warn/crit thresholds hit.
    pub alert_effects: bool,
    /// Monthly billing day-of-month (1..=31) for the renewal countdown.
    /// 0 = unset (the renewal line is hidden). Set from your Claude billing page.
    pub renewal_day: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: "compact".into(),
            corner: "tr".into(),
            poll_secs: MIN_POLL_SECS,
            warn_threshold: 75.0,
            crit_threshold: 90.0,
            opacity: 0.96,
            autostart: false,
            statusline_optin: false,
            effects: true,
            alert_effects: true,
            renewal_day: 0,
        }
    }
}

impl Config {
    pub fn dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("claude-usage-monitor")
    }

    pub fn path() -> PathBuf {
        Self::dir().join("config.json")
    }

    pub fn load() -> Self {
        let mut cfg = match std::fs::read_to_string(Self::path()) {
            Ok(s) => serde_json::from_str::<Config>(&s).unwrap_or_default(),
            Err(_) => Config::default(),
        };
        cfg.normalize();
        cfg
    }

    pub fn save(&self) -> Result<(), String> {
        let dir = Self::dir();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(Self::path(), json).map_err(|e| e.to_string())
    }

    fn normalize(&mut self) {
        if self.poll_secs < MIN_POLL_SECS {
            self.poll_secs = MIN_POLL_SECS;
        }
        if !matches!(self.mode.as_str(), "compact" | "detailed") {
            self.mode = "compact".into();
        }
        if !matches!(self.corner.as_str(), "tl" | "tr" | "bl" | "br") {
            self.corner = "tr".into();
        }
        self.opacity = self.opacity.clamp(0.3, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_valid() {
        let c = Config::default();
        assert_eq!(c.mode, "compact");
        assert_eq!(c.corner, "tr");
        assert_eq!(c.poll_secs, MIN_POLL_SECS);
        assert!(c.effects);
        assert!(!c.statusline_optin);
    }

    #[test]
    fn normalize_clamps_invalid_values() {
        let mut c = Config {
            poll_secs: 5,
            mode: "weird".into(),
            corner: "zz".into(),
            opacity: 9.0,
            ..Config::default()
        };
        c.normalize();
        assert!(c.poll_secs >= MIN_POLL_SECS);
        assert_eq!(c.mode, "compact");
        assert_eq!(c.corner, "tr");
        assert!(c.opacity <= 1.0 && c.opacity >= 0.3);
    }

    #[test]
    fn normalize_keeps_valid_values() {
        let mut c = Config {
            poll_secs: 300,
            mode: "detailed".into(),
            corner: "bl".into(),
            ..Config::default()
        };
        c.normalize();
        assert_eq!(c.poll_secs, 300);
        assert_eq!(c.mode, "detailed");
        assert_eq!(c.corner, "bl");
    }
}
