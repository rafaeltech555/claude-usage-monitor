# M4: Statusline Fable 額度＋顏色渲染 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Claude Code statusline 一行加入模型專屬（Fable）每週額度段，並為所有百分比加上依危險度變色的 ANSI 顏色。

**Architecture:** OAuth usage endpoint 的 `limits[]` 以 raw JSON 進 `QuotaUsage`，純函式抽取為 `ModelLimit`；widget 輪詢成功時寫 `quota-cache.json`（0600），statusline hook 讀快取、逾時才帶防抖自抓（tokio current-thread、2s timeout）。顏色與段落組裝全是純函式，TDD。

**Tech Stack:** Rust（serde、reqwest、tokio rt、chrono）、既有 cargo test。spec：`docs/superpowers/specs/2026-08-09-statusline-fable-default-design.md`。

**注意：** 全程只動 `src-tauri/`；前端 TS 不需改（`QuotaUsage` 多出的 `limits` 欄位對前端無害）。每個 task 結尾 commit（依 /produce 模式一授權）。commit message 繁中、比照 git log 既有風格，結尾加 `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`。

---

### Task 1: `ModelLimit` 解析（quota.rs）

**Files:**
- Modify: `src-tauri/src/quota.rs`（struct 區 ~line 18-33、tests mod）
- Modify: `src-tauri/src/statusline.rs:121-131`（`normalize_reset` 搬家後改引用）

- [ ] **Step 1: 寫失敗測試**（加入 `quota.rs` 的 `mod tests`）

```rust
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
            serde_json::json!({"kind":"weekly_scoped","percent":9,
                "is_active":false,
                "scope":{"model":{"display_name":"Off"}}}),           // inactive 濾掉
            serde_json::json!({"kind":"weekly_scoped","percent":9}),  // 無 display_name 濾掉
            serde_json::json!("garbage"),                             // 非物件不炸
        ];
        let m = model_limits_from(&limits);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].label, "A");
        assert!(m[0].resets_at.as_deref().unwrap().starts_with("2025") == false); // epoch 已轉 RFC3339
        assert!(m[0].resets_at.is_some());
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
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cargo test --manifest-path src-tauri/Cargo.toml model_limits`
Expected: FAIL（`model_limits_from` 未定義、`limits` 欄位不存在）

- [ ] **Step 3: 實作**

`quota.rs`：`QuotaUsage` 加欄位、新 struct 與純函式；`normalize_reset` 從 statusline.rs **搬**過來（pub(crate)）：

```rust
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

/// Pure: pick the active model-scoped weekly limits out of raw `limits[]`.
/// Tolerant by construction: anything malformed is skipped, never an error.
/// Capped at 2 entries to keep the statusline a single short row.
pub fn model_limits_from(limits: &[serde_json::Value]) -> Vec<ModelLimit> {
    limits
        .iter()
        .filter(|v| v.get("kind").and_then(|k| k.as_str()) == Some("weekly_scoped"))
        .filter(|v| v.get("is_active").and_then(|a| a.as_bool()).unwrap_or(true))
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
```

注意 `normalize_reset` 簽名差異：原 statusline.rs 版吃 `&serde_json::Value`，`filter_map` 裡用 `v.get("resets_at").and_then(normalize_reset)` 時 closure 型別是 `&Value` — 直接可用。statusline.rs：刪除原 `normalize_reset`（`:121-131`），`win_from` 內改呼叫 `crate::quota::normalize_reset`（檔頭 use 不必加，全路徑即可）。

- [ ] **Step 4: 跑測試確認通過（含既有測試沒被搬家弄壞）**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: 全部 PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/quota.rs src-tauri/src/statusline.rs
git commit -m "feat(quota): 解析 limits[] 模型專屬額度（Fable 每週配額）"
```

---

### Task 2: quota 快取檔讀寫＋可設 timeout 的 fetch（quota.rs）

**Files:**
- Modify: `src-tauri/src/quota.rs`

- [ ] **Step 1: 寫失敗測試**

```rust
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
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cargo test --manifest-path src-tauri/Cargo.toml cache_roundtrip`
Expected: FAIL（函式未定義）

- [ ] **Step 3: 實作**

```rust
use std::path::{Path, PathBuf};
use std::time::Duration;

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
pub fn read_cache_fresh(max_age_secs: u64) -> Option<QuotaUsage> {
    read_cache_fresh_at(&cache_path(), max_age_secs)
}

pub(crate) fn read_cache_fresh_at(p: &Path, max_age_secs: u64) -> Option<QuotaUsage> {
    let modified = std::fs::metadata(p).ok()?.modified().ok()?;
    let age = std::time::SystemTime::now().duration_since(modified).ok()?;
    if age.as_secs() > max_age_secs {
        return None;
    }
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}
```

同檔把 `OAuthProvider::fetch` 改為呼叫新的 inherent method（行為不變，多 timeout 參數；trait 介面不動）：

```rust
impl OAuthProvider {
    pub async fn fetch_timeout(&self, timeout: Duration) -> Result<QuotaUsage, String> {
        let token = read_token()?;
        let ua = format!("claude-code/{}", claude_version());
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| e.to_string())?;
        // …（原 fetch 內容自 `let resp = client` 起全部原樣搬入）…
    }
}

impl QuotaProvider for OAuthProvider {
    async fn fetch(&self) -> Result<QuotaUsage, String> {
        self.fetch_timeout(Duration::from_secs(10)).await
    }
}
```

注意 `read_cache_fresh_at(&p, 0)`：mtime 剛寫入、`age.as_secs()` 為 0，`0 > 0` 為 false 會誤判新鮮——條件要寫成 `if age.as_secs() >= max_age_secs.max(1) && max_age_secs == 0 { … }` 太繞，**直接改成**：`max_age_secs == 0` 時一律回 `None`（在函式開頭 early return），其餘照 `>` 比較。測試即靠這個語意。

- [ ] **Step 4: 跑測試確認通過**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: 全部 PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/quota.rs
git commit -m "feat(quota): quota-cache.json 讀寫與可設 timeout 的 OAuth fetch"
```

---

### Task 3: 顏色與段落純函式（statusline.rs）

**Files:**
- Modify: `src-tauri/src/statusline.rs`（`ctx_segment` ~line 111、`run_hook` 內 `fmt` closure ~line 200、tests）

- [ ] **Step 1: 寫失敗測試**

```rust
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
```

並**改寫**既有 `ctx_segment_formats_and_clamps`（`:401-410`）為著色版：

```rust
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
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cargo test --manifest-path src-tauri/Cargo.toml -- paint_pct model_segment ctx_segment`
Expected: FAIL（`paint_pct`/`model_segment` 未定義）

- [ ] **Step 3: 實作**

```rust
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
```

`ctx_segment` 改為：

```rust
pub fn ctx_segment(fill: Option<u64>, window: u64) -> String {
    match fill {
        Some(f) if window > 0 => {
            let pct = ((f as f64 / window as f64) * 100.0).round().min(100.0);
            format!(" · ctx {}", paint_pct(pct))
        }
        _ => String::new(),
    }
}
```

`run_hook` 內 `fmt` closure（`:200-204`）改為著色（`—` 不上色）：

```rust
    let fmt = |w: &Option<QuotaWindow>| {
        w.as_ref()
            .map(|x| paint_pct(x.utilization))
            .unwrap_or_else(|| "—".into())
    };
```

- [ ] **Step 4: 跑測試確認通過**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: 全部 PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/statusline.rs
git commit -m "feat(statusline): 百分比依危險度上色，新增模型額度段純函式"
```

---

### Task 4: hook 接線＋widget 寫快取（statusline.rs、lib.rs、Cargo.toml）

**Files:**
- Modify: `src-tauri/src/statusline.rs`（`run_hook` ~line 151-219）
- Modify: `src-tauri/src/lib.rs:485-488`（poll_once 的 quota_result）
- Modify: `src-tauri/Cargo.toml`（tokio 依賴）

- [ ] **Step 1: Cargo.toml 加 tokio（若 `[dependencies]` 尚無 tokio 條目）**

```toml
tokio = { version = "1", features = ["rt"] }
```

- [ ] **Step 2: hook 端資料取得（statusline.rs 新增）**

```rust
const CACHE_FRESH_SECS: u64 = 600;
const FETCH_RETRY_SECS: u64 = 120;

/// Model-scoped limits for the hook: cache first; on a stale/missing cache do a
/// rate-limited (attempt-marker) self-fetch with a hard 2s timeout so the
/// statusline never hangs. Any failure degrades to "no segment".
fn model_limits_for_hook() -> Vec<crate::quota::ModelLimit> {
    use crate::quota;
    if let Some(u) = quota::read_cache_fresh(CACHE_FRESH_SECS) {
        return quota::model_limits_from(&u.limits);
    }
    let marker = Config::dir().join("quota-fetch-attempt");
    let recently_tried = std::fs::metadata(&marker)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|m| std::time::SystemTime::now().duration_since(m).ok())
        .is_some_and(|age| age.as_secs() < FETCH_RETRY_SECS);
    if recently_tried {
        return Vec::new();
    }
    if let Some(dir) = marker.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&marker, b"");

    let fetched = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()
        .and_then(|rt| {
            rt.block_on(async {
                quota::OAuthProvider
                    .fetch_timeout(std::time::Duration::from_secs(2))
                    .await
                    .ok()
            })
        });
    match fetched {
        Some(u) => {
            quota::write_cache(&u);
            quota::model_limits_from(&u.limits)
        }
        None => Vec::new(),
    }
}
```

- [ ] **Step 3: `run_hook` 輸出接上模型段**（`:213-218` 的 `print!` 改為）

```rust
    let model_seg = model_segment(&model_limits_for_hook());
    print!(
        "⚡ {} · 7d {}{}{}",
        fmt(&usage.five_hour),
        fmt(&usage.seven_day),
        model_seg,
        ctx_segment(ctx, window)
    );
```

（`model_seg` 位置在 7d 之後、ctx 之前——與 spec 格式 `⚡ 4% · 7d 10% · Fable 17% · ctx 32%` 一致。）

- [ ] **Step 4: widget 輪詢成功時寫快取**（lib.rs `poll_once` 的 `quota_result`，`:485-488` 改為）

```rust
    let quota_result = match optin.then(|| statusline::read_fresh(150)).flatten() {
        Some(q) => Ok(q),
        None => {
            let r = provider.fetch().await;
            if let Ok(q) = &r {
                quota::write_cache(q);
            }
            r
        }
    };
```

（僅 OAuth 來源寫快取；statusline.json 來源沒有 `limits[]`，寫入反而會把有 Fable 的快取蓋成沒有。）

- [ ] **Step 5: 編譯＋全測試**

Run: `cargo test --manifest-path src-tauri/Cargo.toml && cargo build --manifest-path src-tauri/Cargo.toml`
Expected: 測試全 PASS、編譯無 error（warning 不新增）

- [ ] **Step 6: hook 冒煙測試（不需 Claude Code）**

```bash
echo '{"rate_limits":{"five_hour":{"used_percentage":4},"seven_day":{"used_percentage":10}}}' \
  | ./src-tauri/target/debug/claude-usage-monitor --statusline | cat -v
```

Expected: 一行含 `^[[38;5;114m4%^[[0m`、`7d`、（快取有資料時）`Fable` 段；不得 panic、不得掛住超過 ~3 秒。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/statusline.rs src-tauri/src/lib.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat(statusline): 顯示 Fable 模型額度段；widget 輪詢回寫 quota 快取"
```

---

### Task 5: 收尾驗證

- [ ] **Step 1: 全量測試最後一跑**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: 全 PASS（回報測試數）

- [ ] **Step 2: 留給人工的驗收清單（不執行，寫進回報）**

- rebuild ＋ **重新安裝 .deb**（Claude Code 跑的是 `/usr/bin` 的舊 binary，不重裝看不到——既有踩坑紀錄）
- 開 `claude`，目視 statusline：`⚡ N% · 7d N% · Fable N% · ctx N%` 且百分比有顏色

## Self-Review 紀錄

- spec 覆蓋：M4 兩要件（limits 解析＋顏色）與快取／自抓 fallback、frozen 時省略段落均有對應 task；M5（預設開啟）不在本 plan。
- 型別一致：`model_limits_from(&[serde_json::Value]) -> Vec<ModelLimit>`、`model_segment(&[ModelLimit])`、`paint_pct(f64)` 各 task 引用一致。
- 無 placeholder；Task 2 fetch 搬移段落標明「原樣搬入」屬既有碼位移，非留白。
