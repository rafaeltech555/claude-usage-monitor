//! Token & equivalent-cost accounting from local Claude Code transcripts.
//!
//! Reads `~/.claude/projects/*/*.jsonl`, sums today's `message.usage` tokens,
//! and estimates an equivalent USD cost using a small built-in pricing table
//! (Claude is a flat-rate subscription, so the dollar figure is a reference).

use chrono::{DateTime, Local};
use serde::Serialize;
use std::io::{BufRead, BufReader};

#[derive(Debug, Clone, Serialize, Default)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub total: u64,
    pub cost_usd: f64,
}

/// USD per million tokens, by pricing class.
struct Pricing {
    input: f64,
    output: f64,
    cache_write_5m: f64,
    cache_write_1h: f64,
    cache_read: f64,
}

fn pricing_for(model: &str) -> Pricing {
    let m = model.to_ascii_lowercase();
    if m.contains("opus") {
        Pricing { input: 15.0, output: 75.0, cache_write_5m: 18.75, cache_write_1h: 30.0, cache_read: 1.5 }
    } else if m.contains("haiku") {
        Pricing { input: 0.80, output: 4.0, cache_write_5m: 1.0, cache_write_1h: 1.6, cache_read: 0.08 }
    } else {
        // sonnet / default
        Pricing { input: 3.0, output: 15.0, cache_write_5m: 3.75, cache_write_1h: 6.0, cache_read: 0.30 }
    }
}

/// Sum today's (local-day) token usage across all transcripts.
pub fn today_usage() -> TokenUsage {
    let mut acc = TokenUsage::default();
    let today = Local::now().date_naive();
    let day_start = match today.and_hms_opt(0, 0, 0) {
        Some(t) => t.and_local_timezone(Local).single(),
        None => None,
    };

    let Some(home) = dirs::home_dir() else { return acc };
    let pattern = home.join(".claude/projects/*/*.jsonl");
    let pattern = pattern.to_string_lossy();

    let Ok(paths) = glob::glob(&pattern) else { return acc };
    for entry in paths.flatten() {
        // Skip files untouched today (cheap pre-filter).
        if let (Some(ds), Ok(meta)) = (day_start, std::fs::metadata(&entry)) {
            if let Ok(modified) = meta.modified() {
                let modified: DateTime<Local> = modified.into();
                if modified < ds {
                    continue;
                }
            }
        }
        let Ok(file) = std::fs::File::open(&entry) else { continue };
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
                continue;
            }
            if !is_today(v.get("timestamp").and_then(|t| t.as_str()), today) {
                continue;
            }
            let Some(msg) = v.get("message") else { continue };
            let Some(usage) = msg.get("usage") else { continue };
            let model = msg.get("model").and_then(|m| m.as_str()).unwrap_or("");
            add_usage(&mut acc, usage, model);
        }
    }
    acc.total = acc.input + acc.output + acc.cache_write + acc.cache_read;
    acc
}

fn is_today(ts: Option<&str>, today: chrono::NaiveDate) -> bool {
    match ts.and_then(|s| DateTime::parse_from_rfc3339(s).ok()) {
        Some(dt) => dt.with_timezone(&Local).date_naive() == today,
        None => false,
    }
}

fn add_usage(acc: &mut TokenUsage, usage: &serde_json::Value, model: &str) {
    let p = pricing_for(model);
    let g = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);

    let input = g("input_tokens");
    let output = g("output_tokens");
    let cache_read = g("cache_read_input_tokens");

    // Prefer the 5m/1h cache-creation split when present (different pricing);
    // otherwise treat the lumped value as 5m writes.
    let (cw5, cw1h) = match usage.get("cache_creation") {
        Some(cc) => (
            cc.get("ephemeral_5m_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
            cc.get("ephemeral_1h_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
        ),
        None => (g("cache_creation_input_tokens"), 0),
    };
    let cache_write = cw5 + cw1h;

    acc.input += input;
    acc.output += output;
    acc.cache_read += cache_read;
    acc.cache_write += cache_write;

    let per_m = 1_000_000.0;
    acc.cost_usd += input as f64 / per_m * p.input
        + output as f64 / per_m * p.output
        + cw5 as f64 / per_m * p.cache_write_5m
        + cw1h as f64 / per_m * p.cache_write_1h
        + cache_read as f64 / per_m * p.cache_read;
}
