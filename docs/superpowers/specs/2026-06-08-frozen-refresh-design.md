# 結凍卡：立即恢復按鈕 + 自動重試說明

日期：2026-06-08

## 問題

OAuth token 過期時，widget 進入「結凍」狀態，顯示「Token 已過期 / 請開啟 Claude
Code 重新登入以恢復用量監控」。但後端 poller 每 `poll_secs`（最低 **180 秒**，
`src-tauri/src/config.rs:6`）才打一次 API。使用者重新登入 Claude Code 後，**畫面要
等到下一個 poll 週期（最長 180 秒）才會自動解凍**。這段空窗沒有任何提示，很容易被
誤認成程式壞掉。

## 目標

消除空窗期的困惑，靠兩件事：

1. **立即恢復按鈕** — 讓使用者重新登入後可手動觸發一次刷新，不必乾等。
2. **靜態說明小字** — 告知「不點也會自動恢復」，讓乾等的人知道這不是 bug。
3. **解凍融化特效** — 恢復成功時播放一次性「融化」動畫，給出明確的「已恢復」回饋。

## 現況關鍵點

- 結凍 class 由前端判定：`src/main.ts:267-269`（`s.error` 含 `401|unauthorized`
  → `body.classList.toggle("stale", stale)`）。
- 結凍卡 markup：`index.html:76-80`（`.card-frozen`，直向 flex）。
- 結凍卡顯示由 CSS 控制：`src/styles.css:173`（`body.stale .card-frozen { display: flex }`）。
- 立即刷新能力已存在：`invoke("refresh_now")`（`src/main.ts:503`），後端
  `refresh_now` 會立刻跑一次 `poll_once`（`src-tauri/src/lib.rs:320`）。
- 重新登入成功後，`poll_once` 的 `provider.fetch()` 會成功 → `stale=false` →
  下一次 `usage-update` 進來時 `render()` 自動移除 `body.stale`、結凍卡自動隱藏。

→ 後端**不需要改**；按鈕本質就是手動觸發一次既有的 `refresh_now`。

## 設計

### 1. markup（`index.html` 的 `.card-frozen`）

在 `frozen-sub` 之後新增：

```html
<button id="btn-frozen-refresh" class="frozen-btn">↻ 已重新登入，立即恢復</button>
<div id="frozen-hint" class="frozen-hint">自動每 ≤180 秒會重試一次</div>
```

### 2. 行為（`src/main.ts`）

- 綁定 `#btn-frozen-refresh` click：
  - 設按鈕文字「重新整理中…」、`disabled = true`。
  - 記一個 `frozenRefreshing = true` 旗標。
  - `invoke("refresh_now")`。
- 在 `render()`（或 `usage-update` listener）判定 stale 後：
  - 若**已不再 stale**：`body.stale` 自動移除、結凍卡隱藏 → 恢復成功，無需額外處理
    （旗標重置即可）。
  - 若**仍 stale 且 `frozenRefreshing` 為 true**（代表這次是按鈕觸發的回應）：
    - 按鈕復原（文字還原、`disabled = false`）。
    - `#frozen-hint` 改成「仍未偵測到登入，請確認 Claude Code 已重新登入」。
    - 重置 `frozenRefreshing = false`。
  - 平常（非按鈕觸發）仍 stale：`#frozen-hint` 維持靜態預設文字。

### 3. 樣式（`src/styles.css`，接在 167「frozen / stale」區塊）

- `.frozen-btn`：冰藍系（`--ice`）小按鈕，與結凍主題一致；hover 微亮；`disabled`
  時降透明度、`cursor: default`。
- `.frozen-hint`：沿用 `--muted` 小字。

### 4. 解凍融化特效（thaw）

解凍那一刻（stale `true → false`）播放一次性「融化」動畫：往下滴落 + 溶解淡出 +
最後一亮，約 1.1 秒，純 CSS，四種主題通用。

**時機偵測（`src/main.ts` `render()`）**
- `render()` 內已有舊狀態 `staleNow` 與新值 `stale`。在 `staleNow = stale` 覆寫**之前**
  比較：`if (staleNow && !stale) { /* 剛解凍 */ }`。
- 剛解凍時：對 widget 容器加暫時 class `thawing`，並在動畫結束（`animationend`，
  fallback `setTimeout ~1200ms`）移除。

**為何要 overlay**：`.card-frozen` 在 `stale=false` 瞬間就被隱藏，無法在它身上演融化。
因此用一層蓋在 widget 上的冰霜 overlay——解凍時短暫蓋住已露出的即時資料再融化掉。

**樣式（`src/styles.css`）**
- overlay：優先用 widget 容器的 pseudo-element（`#detailed::after` / `#compact::after`，
  規劃時先確認未被佔用，否則改用一個專屬 `.thaw-overlay` div）。內容為冰藍半透明
  漸層（`--ice` 低 alpha），含一個 `❄` 字符供下滴。
- keyframe `thaw`（one-shot，`body.thawing` 時啟用）：
  - **下滴**：`translateY(0) → translateY(100%)`。
  - **溶解**：`opacity 1 → 0`。
  - **最後一亮**：起手對容器邊框/inset glow 做一次 `--ice` 高亮再退（沿用
    `frostPulse` 的 inset glow 手法，但為單次）。
- 與既有 `frostPulse` 一致，thaw 不受 `cfg.effects` 開關控制（屬結凍狀態本身的視覺、
  且僅一次性 ~1.1s）。

## 範圍界線（YAGNI）

- 精簡膠囊（`index.html:22` 的 `❄ 請重新登入`）**不**加按鈕——點膠囊本來就會展開成
  詳細卡（`src/main.ts:451`），按鈕在詳細卡即可。
- 系統匣結凍環不動。
- 不顯示倒數計時（採靜態文字），不需從後端多傳下次 poll 時間。
- 後端 Rust 不改。

## 測試

- 文案/旗標邏輯為純前端 DOM 行為，主要以手動驗證為主：
  - 模擬 stale → 結凍卡出現按鈕與靜態說明。
  - 點按鈕 → 進入「重新整理中…」、disabled。
  - 模擬刷新後仍 stale → 按鈕復原、說明改為「仍未偵測到登入…」。
  - 模擬刷新後不再 stale → 結凍卡消失、恢復正常顯示，並播放一次 thaw 融化動畫
    （下滴 + 溶解 + 最後一亮），動畫結束後 `thawing` class 移除、容器無殘留。
  - 四種主題下 thaw 動畫皆正常、不破版。
