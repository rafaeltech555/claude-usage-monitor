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
use std::sync::OnceLock;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
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
}

/// Abstraction so the data source can be swapped (oauth / statusline / approx).
#[allow(async_fn_in_trait)]
pub trait QuotaProvider {
    async fn fetch(&self) -> Result<QuotaUsage, String>;
}

pub struct OAuthProvider;

impl QuotaProvider for OAuthProvider {
    async fn fetch(&self) -> Result<QuotaUsage, String> {
        let token = read_token()?;
        let ua = format!("claude-code/{}", claude_version());
        let client = reqwest::Client::builder()
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

/// Subscription info derived from the OAuth profile endpoint.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Subscription {
    /// Day-of-month the subscription was created (used as the renewal day).
    pub renewal_day: u32,
    pub active: bool,
}

/// Fetch the account profile once to learn the billing day (the renewal date is
/// computed locally from it). Best-effort; returns None on any failure.
pub async fn fetch_subscription() -> Option<Subscription> {
    let token = read_token().ok()?;
    let ua = format!("claude-code/{}", claude_version());
    let client = reqwest::Client::builder().build().ok()?;
    let resp = client
        .get(PROFILE_URL)
        .header("Authorization", format!("Bearer {token}"))
        .header("anthropic-beta", OAUTH_BETA)
        .header("User-Agent", ua)
        .header("Content-Type", "application/json")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: serde_json::Value = resp.json().await.ok()?;
    let org = &v["organization"];
    let created = org["subscription_created_at"].as_str()?;
    let dt = chrono::DateTime::parse_from_rfc3339(created).ok()?;
    let active = org["subscription_status"].as_str() == Some("active");
    Some(Subscription {
        renewal_day: chrono::Datelike::day(&dt),
        active,
    })
}

/// Read the OAuth access token, preferring the env override, then the
/// credentials file. The token never leaves this process except as a TLS
/// Authorization header to the official host.
fn read_token() -> Result<String, String> {
    if let Ok(t) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        if !t.is_empty() {
            return Ok(t);
        }
    }
    let path = dirs::home_dir()
        .ok_or("no home dir")?
        .join(".claude/.credentials.json");
    let data = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read credentials: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&data).map_err(|e| format!("bad credentials json: {e}"))?;
    v["claudeAiOauth"]["accessToken"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| "no accessToken in credentials".into())
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
