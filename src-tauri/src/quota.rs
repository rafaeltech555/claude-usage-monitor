//! Quota provider: plan utilization (%) + reset times.
//!
//! Default (and only enabled-by-default) source is the unofficial
//! `GET https://api.anthropic.com/api/oauth/usage` endpoint — the same one
//! Claude Code's `/usage` calls. It is wrapped behind [`QuotaProvider`] so a
//! statusline-based or JSONL-approximation source can be swapped in later.
//!
//! Security: the OAuth bearer token is held only in memory, sent only over TLS
//! to the official host, and is never written to disk or logs.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const FALLBACK_VERSION: &str = "2.1.167";

/// A single rate-limit window as reported by the endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QuotaWindow {
    pub utilization: f64,
    pub resets_at: Option<String>,
}

/// Subset of the `/api/oauth/usage` payload we care about.
/// `serde` ignores the many other (unstable) fields the endpoint returns.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QuotaUsage {
    pub five_hour: Option<QuotaWindow>,
    pub seven_day: Option<QuotaWindow>,
    pub seven_day_opus: Option<QuotaWindow>,
    pub seven_day_sonnet: Option<QuotaWindow>,
    /// Raw `limits[]` entries (unstable schema) — parse lazily via
    /// [`model_limits_from`] so an upstream shape change can never poison
    /// the whole payload.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limits: Vec<serde_json::Value>,
}

/// A model-scoped weekly limit (e.g. Fable's own budget) from `limits[]`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelLimit {
    pub label: String,
    pub percent: f64,
    pub resets_at: Option<String>,
}

/// Normalize a reset timestamp to RFC3339. Accepts an RFC3339 string (OAuth)
/// or a Unix epoch-seconds number (Claude Code statusline payload).
pub(crate) fn normalize_reset(v: &serde_json::Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    let secs = v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))?;
    chrono::DateTime::from_timestamp(secs, 0).map(|dt| dt.to_rfc3339())
}

/// Pure: pick the model-scoped weekly limits out of raw `limits[]`.
/// Tolerant by construction: anything malformed is skipped, never an error.
/// Capped at 2 entries to keep the statusline a single short row.
/// `is_active` is deliberately ignored — the API flips it within a day while
/// the percent stays meaningful, and hiding the segment on `false` made the
/// budget "blink" (user ruling 2026-08-09).
pub fn model_limits_from(limits: &[serde_json::Value]) -> Vec<ModelLimit> {
    limits
        .iter()
        .filter(|v| v.get("kind").and_then(|k| k.as_str()) == Some("weekly_scoped"))
        .filter_map(|v| {
            let label = v
                .pointer("/scope/model/display_name")?
                .as_str()
                .filter(|s| !s.is_empty())?
                .to_string();
            let percent = v.get("percent").and_then(|p| p.as_f64())?;
            let resets_at = v.get("resets_at").and_then(normalize_reset);
            Some(ModelLimit { label, percent, resets_at })
        })
        .take(2)
        .collect()
}

/// Abstraction so the data source can be swapped (oauth / statusline / approx).
#[allow(async_fn_in_trait)]
pub trait QuotaProvider {
    async fn fetch(&self) -> Result<QuotaUsage, String>;
}

pub struct OAuthProvider;

impl OAuthProvider {
    pub async fn fetch_timeout(&self, timeout: Duration) -> Result<QuotaUsage, String> {
        let token = read_token()?;
        let ua = format!("claude-code/{}", claude_version());
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| e.to_string())?;
        let resp = client
            .get(USAGE_URL)
            .header("Authorization", format!("Bearer {token}"))
            .header("anthropic-beta", OAUTH_BETA)
            .header("User-Agent", ua)
            .header("Content-Type", "application/json")
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;

        let status = resp.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err("rate limited (429) — backing off".into());
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err("unauthorized (401) — token expired? open Claude Code to refresh".into());
        }
        if !status.is_success() {
            return Err(format!("usage endpoint returned {status}"));
        }
        resp.json::<QuotaUsage>()
            .await
            .map_err(|e| format!("parse failed: {e}"))
    }
}

impl QuotaProvider for OAuthProvider {
    async fn fetch(&self) -> Result<QuotaUsage, String> {
        self.fetch_timeout(Duration::from_secs(10)).await
    }
}

/// Path to the quota cache file the statusline hook can read without a network call.
pub fn cache_path() -> PathBuf {
    crate::config::Config::dir().join("quota-cache.json")
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

/// Persist the latest OAuth quota (percentages + raw limits; never the token)
/// so the statusline hook can show model-scoped budgets without a network call.
pub fn write_cache(u: &QuotaUsage) {
    write_cache_at(&cache_path(), u);
}

pub(crate) fn write_cache_at(p: &Path, u: &QuotaUsage) {
    let Ok(json) = serde_json::to_string(u) else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::write(p, json).is_ok() {
        set_owner_only(p);
    }
}

/// Read the cache if written within `max_age_secs` (freshness = file mtime).
/// `max_age_secs == 0` always misses (there is no meaningful "fresh enough"
/// window of zero, and it lets tests assert a hard-expired cache deterministically).
pub fn read_cache_fresh(max_age_secs: u64) -> Option<QuotaUsage> {
    read_cache_fresh_at(&cache_path(), max_age_secs)
}

pub(crate) fn read_cache_fresh_at(p: &Path, max_age_secs: u64) -> Option<QuotaUsage> {
    if max_age_secs == 0 {
        return None;
    }
    let modified = std::fs::metadata(p).ok()?.modified().ok()?;
    let age = std::time::SystemTime::now().duration_since(modified).ok()?;
    if age.as_secs() > max_age_secs {
        return None;
    }
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

/// Extract the OAuth access token from a credentials JSON blob (the same shape
/// whether it came from `~/.claude/.credentials.json` or the macOS Keychain).
fn parse_access_token(blob: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(blob).ok()?;
    v["claudeAiOauth"]["accessToken"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// macOS stores the Claude Code credentials in the login Keychain as a generic
/// password (the value is the same JSON blob as the Linux credentials file).
#[cfg(target_os = "macos")]
fn read_token_macos() -> Option<String> {
    // NOTE: verify this service name on a real Mac (Keychain Access → "Claude").
    const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
    let out = std::process::Command::new("security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_access_token(String::from_utf8_lossy(&out.stdout).trim())
}

/// Read the OAuth access token: env override first, then (on macOS) the Keychain,
/// then the credentials file. The token never leaves this process except as a
/// TLS Authorization header to the official host.
fn read_token() -> Result<String, String> {
    if let Ok(t) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        if !t.is_empty() {
            return Ok(t);
        }
    }

    #[cfg(target_os = "macos")]
    if let Some(t) = read_token_macos() {
        return Ok(t);
    }

    let path = dirs::home_dir()
        .ok_or("no home dir")?
        .join(".claude/.credentials.json");
    let data = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read credentials: {e}"))?;
    parse_access_token(&data).ok_or_else(|| "no accessToken in credentials".into())
}

/// Detect the installed Claude Code version once (for the required User-Agent),
/// falling back to a recent known version if `claude` isn't on PATH.
fn claude_version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| {
        std::process::Command::new("claude")
            .arg("--version")
            .output()
            .ok()
            .and_then(|o| {
                let s = String::from_utf8_lossy(&o.stdout);
                s.split_whitespace()
                    .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()))
                    .map(|t| t.trim().to_string())
            })
            .unwrap_or_else(|| FALLBACK_VERSION.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_access_token_extracts_or_none() {
        let ok = r#"{"claudeAiOauth":{"accessToken":"sk-abc","refreshToken":"r"}}"#;
        assert_eq!(parse_access_token(ok).as_deref(), Some("sk-abc"));
        assert!(parse_access_token(r#"{"claudeAiOauth":{"accessToken":""}}"#).is_none());
        assert!(parse_access_token(r#"{"other":1}"#).is_none());
        assert!(parse_access_token("not json").is_none());
    }

    #[test]
    fn parses_usage_and_ignores_unknown_fields() {
        let json = r#"{
            "five_hour": {"utilization": 14.0, "resets_at": "2026-06-06T12:30:00+00:00"},
            "seven_day": {"utilization": 3.0, "resets_at": null},
            "seven_day_sonnet": {"utilization": 0.0, "resets_at": null},
            "tangelo": null,
            "extra_usage": {"is_enabled": false}
        }"#;
        let u: QuotaUsage = serde_json::from_str(json).unwrap();
        assert_eq!(u.five_hour.as_ref().unwrap().utilization, 14.0);
        assert_eq!(
            u.five_hour.as_ref().unwrap().resets_at.as_deref(),
            Some("2026-06-06T12:30:00+00:00")
        );
        assert_eq!(u.seven_day.as_ref().unwrap().utilization, 3.0);
        assert!(u.seven_day.as_ref().unwrap().resets_at.is_none());
        assert!(u.seven_day_opus.is_none());
    }

    #[test]
    fn missing_windows_default_to_none() {
        let u: QuotaUsage = serde_json::from_str("{}").unwrap();
        assert!(u.five_hour.is_none());
        assert!(u.seven_day.is_none());
    }

    #[test]
    fn model_limits_from_extracts_weekly_scoped() {
        let limits = vec![
            serde_json::json!({"kind":"session","percent":4,"is_active":true,
                "scope":null}),
            serde_json::json!({"kind":"weekly_scoped","percent":17,"is_active":true,
                "resets_at":"2026-08-11T09:00:00+00:00",
                "scope":{"model":{"id":null,"display_name":"Fable"}}}),
            serde_json::json!({"kind":"weekly_all","percent":10,"is_active":true}),
        ];
        let m = model_limits_from(&limits);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].label, "Fable");
        assert_eq!(m[0].percent, 17.0);
        assert_eq!(m[0].resets_at.as_deref(), Some("2026-08-11T09:00:00+00:00"));
    }

    #[test]
    fn model_limits_from_tolerates_junk_and_caps_at_two() {
        // epoch resets_at、is_active 缺省視為 true、percent 為 int 都要收
        let mk = |name: &str, pct: i64| {
            serde_json::json!({"kind":"weekly_scoped","percent":pct,
                "resets_at":1754899200,
                "scope":{"model":{"display_name":name}}})
        };
        let limits = vec![
            mk("A", 1), mk("B", 2), mk("C", 3),                       // cap 2
            serde_json::json!({"kind":"weekly_scoped","percent":9}),  // 無 display_name 濾掉
            serde_json::json!("garbage"),                             // 非物件不炸
        ];
        let m = model_limits_from(&limits);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].label, "A");
        // epoch 已轉 RFC3339（不再是原始 epoch 數字字串）
        assert!(!m[0].resets_at.as_deref().unwrap().starts_with("1754899200"));
        assert!(m[0].resets_at.is_some());
    }

    #[test]
    fn model_limits_shown_regardless_of_is_active() {
        // is_active 會在一天內自行翻轉、percent 卻持續有意義，故不據以過濾
        // （使用者裁定 2026-08-09：有資料就顯示）
        let limits = vec![serde_json::json!({"kind":"weekly_scoped","percent":22,
            "is_active":false,
            "scope":{"model":{"display_name":"Fable"}}})];
        let m = model_limits_from(&limits);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].label, "Fable");
        assert_eq!(m[0].percent, 22.0);
    }

    #[test]
    fn cache_roundtrip_and_freshness() {
        let dir = std::env::temp_dir().join(format!("cum-qcache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("quota-cache.json");

        let u = QuotaUsage {
            five_hour: Some(QuotaWindow { utilization: 4.0, resets_at: None }),
            limits: vec![serde_json::json!({"kind":"weekly_scoped","percent":17,
                "scope":{"model":{"display_name":"Fable"}}})],
            ..Default::default()
        };
        write_cache_at(&p, &u);
        let back = read_cache_fresh_at(&p, 600).expect("fresh cache should read back");
        assert_eq!(model_limits_from(&back.limits).len(), 1);

        // 0 秒容忍 = 一定過期
        assert!(read_cache_fresh_at(&p, 0).is_none());
        // 不存在 → None
        assert!(read_cache_fresh_at(&dir.join("nope.json"), 600).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn quota_usage_parses_limits_field() {
        let json = r#"{"five_hour":{"utilization":4.0,"resets_at":null},
            "limits":[{"kind":"weekly_scoped","percent":17,"is_active":true,
                "scope":{"model":{"display_name":"Fable"}}}]}"#;
        let u: QuotaUsage = serde_json::from_str(json).unwrap();
        assert_eq!(model_limits_from(&u.limits).len(), 1);
        // 缺 limits 欄位 → 空 vec，不炸
        let u2: QuotaUsage = serde_json::from_str("{}").unwrap();
        assert!(u2.limits.is_empty());
        // roundtrip：serialize 後 limits 原樣保留（cache 檔要用）
        let s = serde_json::to_string(&u).unwrap();
        let back: QuotaUsage = serde_json::from_str(&s).unwrap();
        assert_eq!(model_limits_from(&back.limits).len(), 1);
    }
}
