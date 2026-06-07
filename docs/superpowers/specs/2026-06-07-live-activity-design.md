# 即時活動狀態（Live Activity）設計

日期：2026-06-07
狀態：已核可，待寫實作計畫

## 目標

在 widget 顯示「目前 Claude Code session 正在燒多少」的即時狀態：燒速（tok/min）、近 10 分鐘走勢、本次 session 累計、以及依目前速度 5h 額度何時見底。需要四種渲染面向全部實作：

- **A** 詳細卡片內的即時活動區塊（有 session 在跑）
- **B** 同區塊的閒置態（沒 session 時收斂成一行）
- **C** 精簡膠囊上的即時指示（脈動點 + 燒速）
- **D** 大字燒速獨立模式（第 4 種顯示模式）

## 已確認的設計決策

1. **燒速數字**：主顯示 `tok/min`（大字），次要顯示「依此速度 5h 約 N 分見底 / ✓ 重置前不會見底」。
2. **資料來源**：statusline hook 優先、`.jsonl` tail 後備。
3. **多 session**：全部加總（燒速與累計都是所有活動中 session 之和）。
4. **D 做成第 4 種顯示模式**（`activity`），會多一點 header / 設定切換 UI。
5. **`show_activity` 預設開**。
6. **見底時間用 5h 百分比斜率估計**，標示為「≈」。

## 資料模型

新結構 `LiveActivity`，透過獨立 Tauri 事件 `activity-update` 推給前端（與既有 `usage-update` 分開）。

```rust
struct LiveActivity {
    active: bool,            // 近 ACTIVE_WINDOW 有 token 進帳，或 statusline hint 新鮮
    burn_tpm: f64,           // tok/min，近 RATE_WINDOW，input+output，多 session 加總
    session_tokens: u64,     // 活動中 session 累計總和（加總）
    last_active_secs: u64,   // 距上次活動秒數（閒置時顯示「N 分鐘前」）
    mins_to_empty: Option<f64>, // 5h 視窗預估見底分鐘數（無法估時為 None）
    beats_reset: bool,       // 見底時間 > 重置倒數 → 顯示「重置前不會見底」
    spark: Vec<f64>,         // 近 ~10 分鐘每分鐘 token 桶（畫 sparkline）
    source: String,          // "statusline" | "jsonl"
}
```

常數（暫定，實作時可微調）：
- `ACTIVE_WINDOW = 120s`（判定 active 的靜默上限）
- `RATE_WINDOW = 5 min`（算 burn_tpm 的視窗）
- `SPARK_MINUTES = 10`（sparkline 桶數）
- `HINT_FRESH = 15s`（statusline hint 視為新鮮的上限）
- `TICK = 5s`（activity ticker 節奏）

## 計算引擎：新模組 `src-tauri/src/activity.rs`

**核心原則：增量 tail，不每 tick 重讀整檔。**

`ActivityTracker` 持有狀態：
- `files: HashMap<PathBuf, FileState>`，`FileState { offset: u64, session_total: u64, last_ts }`
  - 每個 jsonl 記住已讀位移；每 tick 只讀「新增位元組」並解析 assistant entries 的 `message.usage`。
  - 首次見到某檔做一次完整讀取，得出 session 累計與起點，之後只 tail 增量。
- `events: VecDeque<(DateTime<Local>, u64)>`，近 `SPARK_MINUTES` 分鐘的 token 事件，用來算 `burn_tpm` 與 `spark`。
- 用 file mtime 預過濾，只碰近期動過的檔（沿用 `usage.rs` 的 glob pattern `~/.claude/projects/*/*.jsonl`）。

**token 計法**：燒速與 spark 用 `input + output`（與「今日 tok」一致；cache_read/write 會灌爆數字，不計入燒速）。`session_total` 同樣以 input+output 計，保持一致。

**純函式（可單測，不碰檔案）：**
- `burn_rate(events, now, window) -> f64`
- `spark_buckets(events, now, minutes) -> Vec<f64>`
- `is_active(last_ts, now, hint_fresh) -> bool`
- `mins_to_empty(samples, current_pct) -> Option<f64>`（5h % 線性外推）
- `beats_reset(mins_to_empty, reset_secs) -> bool`

IO（glob / open / tail / mtime）留薄，包在 tracker 的 `tick()` 方法，內部呼叫上述純函式。

## statusline 優先 / jsonl 後備

擴充 `statusline.rs::run_hook`：除既有 quota 外，再寫 `activity-hint.json`（權限 0600），內容為 Claude Code stdin 提供的 `transcript_path`、`session_id`、寫入時間。

- **hint 新鮮（mtime < `HINT_FRESH`）**：`source = "statusline"`，立即判定 `active = true`，並確保 `transcript_path` 指的檔被納入 tail（即使 mtime 預過濾邊界擦邊）。
- **沒有新鮮 hint**：`source = "jsonl"`，純靠 mtime + tail 偵測（開箱即用，有幾秒延遲）。

jsonl tail 永遠是底層引擎；statusline 只是讓「正在跑」更即時、更確定，並標示來源。`activity-hint.json` 與既有 `statusline.json` 分開，避免動到 `read_fresh` 既有格式。

## 見底時間估算

API 只給 5h 百分比、不給絕對 token 上限，故無法用 token 燒速直接換算。改用 **5h 百分比斜率**：

- quota poller 每次輪詢時，往 `AppState` 的環形緩衝記一筆 `(DateTime<Local>, 5h_pct)`（保留近幾筆即可）。
- `mins_to_empty(samples, current_pct)`：對近期樣本做線性擬合取斜率（% / 分鐘），外推到 100%。
- 僅在 `active` 且斜率為正、且樣本 ≥ 2 且時間跨度足夠時回傳 `Some`，否則 `None`。
- 跨 5h 重置時斜率變負 → 自動回 `None`（不顯示）。
- `beats_reset`：若 `mins_to_empty > 5h 重置倒數`，前端顯示「✓ 重置前不會見底」，否則顯示「≈ N 分見底」。
- 因 180s 取樣較粗、數字會跳，UI 一律標「≈」。

## 兩個獨立節奏

- **既有 quota poller（≥180s，邏輯不動）**：額外記 `(時間, 5h%)` 樣本供見底推算。
- **新 activity ticker（`TICK` ≈ 5s）**：獨立的 `tokio` 迴圈，跑 `ActivityTracker::tick`、併入最新的見底估計與 5h 重置倒數、`emit("activity-update", LiveActivity)`。本地讀檔便宜，5s 沒問題。

`AppState` 新增：`activity: Mutex<ActivityTracker>`、`quota_samples: Mutex<VecDeque<(DateTime<Local>, f64)>>`。

## 前端渲染（A/B/C/D）

新增 CSS 變數 `--live: #3ddc84`。`main.ts` 新增 `listen("activity-update", ...)` 與 `renderActivity(LiveActivity)`。

- **A／B（詳細卡片區塊）**：在 `index.html` 的 `.card-live` 內、meters 之後加 `live-block`：
  - active（A）：脈動綠點 + 專案名 + 大字 `tok/min` + 「≈ 5h N 分見底」或「✓ 重置前不會見底」+ SVG sparkline + 「本次 session 累計」。
  - idle（B）：收斂成灰點一行「最後活動 N 分鐘前」。
  - 放在 `.card-live` 內 → `body.stale`（結冰）時隨既有規則自動隱藏，與「過期停止顯示舊數據」一致。
- **C（精簡膠囊）**：`#compact` 右側加脈動綠點 + `12k/m`；`active=false` 時兩者隱藏，回原本膠囊外觀。
- **D（大字燒速模式）**：新增第 4 種顯示模式 `activity`。
  - `lib.rs` 新增 `ACTIVITY` 視窗尺寸常數與 `apply_mode` 分支。
  - 詳細卡 header 加「🔥」鈕切到 D；D 卡 header 加返回鈕回詳細。
  - 設定的「預設模式」下拉新增此選項。
- sparkline：`main.ts` 由 `spark[]` 產 SVG path（綠色漸層），同 mockup `/tmp/activity-mockup.html` 的畫法。

## 設定

- `config.rs` 的 `Config` 新增 `show_activity: bool`（預設 `true`）。
- 設定面板新增勾選「顯示即時活動」，wiring 同其他 toggle（存檔 + 重繪）。關閉時：前端完全不顯示 A/B/C 區塊與指示；「預設模式」下拉**移除** `activity` 選項；詳細卡 header 的「🔥」鈕**隱藏**；若當下正處於 `activity` 模式則退回 `detailed`。

## 與既有狀態的互動

- **結冰（stale token）**：live-block 在 `.card-live` 內，自動被 `body.stale` 隱藏。膠囊指示亦在 stale 時隱藏。D 模式下若 stale，顯示與其他模式一致的重新登入提示（沿用 frozen 樣式）。
- **門檻警示 / 火焰**：不受影響；即時活動是獨立的綠色語彙。

## 測試

- **Rust（`activity.rs`）**：對五個純函式各寫單元測試
  - `burn_rate`：合成 events 算 tok/min。
  - `spark_buckets`：events 正確分到每分鐘桶、超出視窗的被排除。
  - `is_active`：靜默逾時 / hint 新鮮兩條件。
  - `mins_to_empty`：正斜率外推、斜率 ≤0 回 None、樣本不足回 None。
  - `beats_reset`：見底時間與重置倒數比較。
- **前端（vitest）**：`format.ts` 新增 `fmtRate`（tok/min，含 k/M 縮寫）與 `fmtMinsToEmpty`，各加測試。

## 已知限制

- 見底時間依 180s 取樣的 5h% 斜率，較粗、會跳，故標「≈」。
- `session_tokens` 累計用「首次完整讀 + 之後 tail」近似；app 啟動前的歷史靠那次完整讀補回。
- 多 session 加總會把不同專案的燒速混在一起（依決策 3 為預期行為）。

## 不在範圍（YAGNI）

- 個別 session 分項列表 / 切換（決策為加總）。
- $/min 燒速（Max 為定額，金額僅供參考）。
- 跨平台特定處理（沿用既有 Linux 主線；不在本 spec）。
