# 在 Claude Code statusline 顯示當前 session 的 context 使用比例

**日期：** 2026-06-15
**狀態：** 設計定稿，待實作

---

## 動機 / 目標

在本專案（claude-usage-monitor）註冊給 Claude Code 的 statusline 字串尾端，加上「當前 session 的 context 佔用比例」。只放在 Claude Code statusline，**不**放 app 內的 widget —— 因為 app widget 是跨 session 的總計，而 context 是每個 claude CLI session 各自獨立的數字，混在一起會誤導。

---

## 資料來源與關鍵發現

- Claude Code 餵給 statusline 的 stdin payload 本身**沒有** context 比例這個欄位。它有 `rate_limits`、`model`、`transcript_path`、`session_id`、`cost`，以及一個 `exceeds_200k_tokens` 布林值。
- context 佔用量必須**自行從 transcript 算出**。payload 的 `transcript_path` 指向當前 session 的 .jsonl。
- 公式：取 transcript 中**最後一筆帶有 usage 的 assistant 訊息**，`ctx = input_tokens + cache_creation_input_tokens + cache_read_input_tokens`。這代表那次請求送進模型的總 context 大小（會隨 compact 上下波動）。已用真實 transcript 驗證：input=2 + cache_creation=1693 + cache_read=48565 ≈ 50.3k。
- **關鍵發現（影響分母偵測）**：真實 transcript 裡的 model id 是 `"claude-opus-4-8"`，**不帶** `[1m]` 後綴，即使該 session 是 1M context 變體。所以光憑 model id 無法分辨 200k 還是 1M。
- 注意：這跟 activity.rs 既有的 `session_tokens` **不是同一個東西**。`session_tokens` 把每輪 input+output 累加（累積燃燒量，只增不減）；context % 要的是「最後一次請求的單次 input 總量」。語意不同，需新解析邏輯，不複用。

---

## 設計

### 1. context window 偵測（純函式，可單元測試）

依序：

1. `model.id`（小寫）含子字串 `"1m"` → 1,000,000（若 payload 的 model.id 真帶後綴，此條最準）
2. 否則 payload `exceeds_200k_tokens == true` → 1,000,000（只有 1M session 能超過 200k，作為 fallback 修正）
3. 否則 → 200,000

殘留盲點：1M session 但目前 <200k 且 model.id 不帶後綴 → 暫時用 200k 當分母而高估；跨過 200k 後會由 `exceeds_200k_tokens` 自動修正。等下方 Open Item 確認後可消除。

### 2. 計算

在 `run_hook()` 內：讀 payload 的 `transcript_path` → **tail 讀最後約 128 KiB**（避免每次 render 重讀整個多 MB 檔）→ 丟掉開頭可能不完整的那一行 → 在完整行中找**最後一筆**帶 usage 的 assistant 訊息 → 算 `ctx` → 百分比 = `ctx / window * 100`，顯示時 **clamp 到 100%**。

### 3. 顯示

在現有 statusline 字串尾端接一段，格式 `· ctx {n}%`。

範例：`⚡ 12% · 7d 8% · ctx 43%`

讀不到 transcript、或找不到帶 usage 的 assistant 訊息 → **整段 `· ctx …` 省略**（不顯示 `—`，保持乾淨）。

---

## 動到的範圍

全部集中在 `src-tauri/src/statusline.rs`。新增 3 個純函式：`context_fill`（從 usage value 算 ctx）、`context_window`（依上方規則回傳分母）、tail 解析最後一筆 usage 的函式；各自加單元測試，沿用該檔現有 test 風格（`#[cfg(test)] mod tests`）。`activity.rs` 完全不碰。

---

## 邊界情況

| 情況 | 處置 |
|---|---|
| transcript 檔不存在 / 讀取失敗 | 省略 ctx 段 |
| 找不到任何帶 usage 的 assistant 行（session 剛開始）| 省略 ctx 段 |
| 算出 >100%（分母偵測錯誤造成）| clamp 顯示為 100% |
| tail 讀到的第一行可能被切斷 | 丟棄該行，只解析其後完整行 |

---

## 測試

- `context_fill`：給定 usage JSON value，正確加總 `input_tokens + cache_creation_input_tokens + cache_read_input_tokens`（缺欄位視為 0）。
- `context_window`：model.id 含 `"1m"` → 1M；`exceeds_200k_tokens` 旗標 true → 1M；兩者皆無 → 200k。
- tail 解析：多行 transcript 內容中正確取到最後一筆 assistant usage；開頭不完整行被略過；無 usage 行時回 `None`。

---

## 不在範圍（Out of scope / YAGNI）

- app widget 顯示（刻意排除，理由見動機）
- config.json 的分母 override 欄位（暫不做，先靠自動偵測）

---

## Open Item（待確認，不阻擋實作）

確認 Claude Code statusline payload 的 `model.id` 到底帶不帶 `[1m]` 後綴 —— 需等新 build 把 `statusline-raw.json` 寫出來後檢視。若帶後綴，偵測規則第 1 條即足夠精準，盲點消失；若不帶，維持 `exceeds_200k_tokens` fallback。此事與既有 PAUSED 的「dev build 被 single-instance/autostart 蓋掉、raw dump 寫不出來」問題相關。
