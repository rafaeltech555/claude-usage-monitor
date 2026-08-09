# Statusline 顯示 Fable 獨立額度＋預設開啟 — 設計 spec

- 日期：2026-08-09
- 狀態：草稿（待使用者核准）
- 來源討論：使用者想要「終端機下方的顯示模式、含 Fable 獨立 usage、成為預設」。收斂結論：即擴充現有 Claude Code statusline 整合（輸入 `claude` 後終端機底部那一行），不做獨立終端機常駐列。

## 目標

1. statusline 那一行加入 **Fable 獨立每週額度**，並為各百分比加上 **ANSI 顏色渲染**。
2. statusline 從「選擇性、預設關」改為 **預設開啟**。
3. widget（桌面小窗）與 statusline 兩種顯示面可**個別開關**，選擇會被記住，下次啟動依使用者習慣。

## 非目標

- 不做 `.bashrc` / shell rc 整合、不做獨立 renderer 行程、不做 tmux 整合。
- 不改動 statusline 既有欄位（⚡ 5h、7d、ctx）的計算邏輯。
- 不處理 macOS 端驗證（依既有慣例，macOS 待實機驗證）。

## 現況背景（實測 2026-08-09）

- statusline 管線：同一顆 binary 以 `--statusline` 旗標當 Claude Code statusLine hook 執行（`lib.rs:45-50`），設定寫入 `~/.claude/settings.json`（`statusline.rs` 的 `enable_at`/`our_command`，有備份、拒絕覆蓋他人設定），輸出格式在 `statusline.rs:207-212`：`⚡ N% · 7d N% · ctx N%`。
- Claude Code 給 hook 的 stdin payload 只有 `five_hour`/`seven_day` 兩個 rate_limit，**沒有 Fable 專屬 bucket**。
- Fable 獨立額度只存在於 OAuth usage endpoint（`GET https://api.anthropic.com/api/oauth/usage`）回傳的 `limits[]` 陣列：`{kind:"weekly_scoped", percent:N, resets_at:…, scope:{model:{id:null, display_name:"Fable"}}, is_active:true}`。現有 `quota.rs:20-33` 未解析此欄位（只解析 `five_hour`/`seven_day`/`seven_day_opus`/`seven_day_sonnet`，且 `seven_day_opus` 實測為 null）。
- 顯示模式設定已有持久化機制：`config.json`（`config.rs`），現記 `mode`（compact/detailed/activity）。

## 設計

### M4：Fable 額度解析＋statusline 顏色渲染

**quota.rs**
- 新增 `limits[]` 的容錯解析：每個元素取 `kind`、`percent`、`resets_at`、`is_active`、`scope.model.display_name`，全部 `Option`／`serde(default)`（非官方 endpoint，欄位可能變動，缺欄不得炸掉整包解析）。
- 「模型專屬額度」的認定：`kind == "weekly_scoped"` 且 `scope.model.display_name` 存在。**不看 `is_active`**——實測該欄位一天內會自行翻轉（true→false）而 percent 持續有意義，據以過濾會讓 Fable 段時有時無（使用者裁定 2026-08-09：有資料就顯示）。**不 hardcode "Fable"**——直接顯示 API 給的 display_name，未來換模型名照樣可用。最多取 2 個（保持一行長度）。

**quota 快取檔（新增）**
- widget 既有輪詢迴圈每次成功抓到 quota 後，把「百分比、resets_at、模型專屬額度清單」寫入 `~/.config/claude-usage-monitor/quota-cache.json`（0600；**不含 token**）。
- statusline hook 讀此快取取得 Fable 段。快取逾時（> 10 分鐘）或不存在時：hook 自行打 OAuth endpoint（timeout 2s、non-blocking file lock 防多發、失敗記 backoff 時戳避免每次 refresh 都重試），成功則回寫快取——涵蓋「widget 被關掉」的情境。
- hook 拿不到 Fable 資料時：**省略該段**，其餘欄位照常輸出（不顯示錯誤字樣）。

**statusline 輸出格式**
- 新格式：`⚡ 4% · 7d 10% · Fable 17% · ctx 32%`（Fable 段插在 7d 與 ctx 之間；其餘段落與現有邏輯完全一致，含既有 reset 時間顯示行為）。
- 顏色：四個百分比（5h、7d、模型專屬、ctx）依同一門檻上色——<50 綠、50–79 黃、≥80 紅（ANSI 256 色；沿用 mock 驗證過的 114/221/203）。標籤與分隔符不上色（維持 Claude Code statusline 預設的暗色調）。
- 既有測試檔 `format.test.ts` 與 Rust 端測試補：`limits[]` 解析（含 `model.id` 為 null、`limits` 缺欄、整包缺 `limits` 三種 fixture）、顏色門檻函式、含 Fable 段與省略 Fable 段的完整輸出字串。

### M5：預設開啟＋顯示面開關記憶

**config.json 擴充**
- 新增 `displays: { widget: bool, statusline: bool }`，預設 `{ widget: true, statusline: true }`。舊設定檔缺此欄位時以預設補齊（向後相容）。
- 新增 `statusline_auto_enable_done: bool`（預設 false），記錄「首次自動啟用」是否已嘗試過，避免使用者手動關掉後每次啟動又被打開。

**啟動行為**
- app 啟動時：`displays.statusline == true` 且 `statusline_auto_enable_done == false` 且 statusline 尚未安裝 → 走既有 `enable_at` 安全路徑自動啟用（備份 `~/.claude/settings.json`；偵測到他人的 statusLine 設定則**不動它**，只在 settings UI 顯示狀態），完成後把 `statusline_auto_enable_done` 設 true。
- `displays.widget == false` → 啟動時不顯示主視窗，只留 tray（tray 為重新開啟的入口）；`true` 則照現況。

**開關 UI**
- settings view 新增兩個 toggle：「桌面小窗」「Claude Code statusline」；tray 選單同步加入。切換即寫回 config，statusline toggle 同時呼叫既有 enable/disable 路徑。

## 錯誤處理

- OAuth endpoint 失敗／token 不存在：statusline 照現有 frozen 行為，Fable 段省略。
- `limits[]` 結構變動：容錯解析，缺什麼省略什麼，絕不讓 statusline 整行掛掉（hook 掛掉會讓 Claude Code 底列消失）。
- `~/.claude/settings.json` 被他人 statusline 佔用：不覆蓋、不報錯干擾，settings UI 呈現「已被其他工具佔用」。

## 驗證

- `cargo test`（新增之解析／格式測試）＋ `npm test`（format.test.ts）。
- 實跑：rebuild ＋ **重新安裝 .deb**（Claude Code 執行的是 `/usr/bin` 的舊 binary，只 build 不裝看不到效果——既有踩坑紀錄），開 `claude` 目視 statusline 出現 Fable 段與顏色。
- GUI 開關行為（settings toggle、tray、重啟記憶）：`verify-tauri-gui` 可自動化部分＋人工確認清單。

## 里程碑編號註記

依 repo 現行推定序列（M0–M3）接續編為 M4/M5。M0–M3 切分尚待本人確認（見 CLAUDE.md backlog）；若確認後重編，本 spec 的 M4/M5 隨之順移，內容不變。完成後同步更新 `~/sideproject/projects.yaml` 的 `milestone` 欄。
