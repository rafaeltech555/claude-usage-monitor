# Claude Usage Monitor

可釘在桌面任何角落的 Claude 用量監控小工具（Tauri v2，Rust + vanilla-TS）。隨時顯示 Claude 方案額度與何時刷新，以及今日 token 用量與等值花費。

## 功能

- **方案額度 + 重置時間**：5 小時滾動視窗、每週額度的使用百分比與重置倒數（資料來自 `/api/oauth/usage`，與 Claude Code `/usage` 同源）。
- **今日 token / 等值花費**：解析本機 `~/.claude/projects/*/*.jsonl`，顯示輸入/輸出 token 與等值 USD（Max 為月費定額，金額僅供參考）。
- **即時活動狀態（可開關，預設開）**：偵測正在跑的 Claude Code session，顯示目前**燒速（tok/min）**、近 ~10 分鐘 sparkline、本次 session 累計（多 session 加總），以及依此速度 5h 額度「≈ N 分見底 / ✓ 重置前不會見底」（由 5h 百分比斜率估算）。資料來源 **statusline hook 優先、`.jsonl` tail 後備**；無 session 時收斂為「💤 無活動 · 最後活動 N 分鐘前」。詳細卡內嵌一塊、精簡膠囊加脈動綠點+燒速、另有獨立的**即時燒速**大字模式。
- **訂閱續訂日**：在設定填入帳單日（每月幾號，見你 Claude 帳單頁的 auto-renew 日期），詳細模式即顯示下次續訂日與倒數。（OAuth token 無法存取帳單端點，故採手動設定。）
- **三種畫面**：
  - 精簡：膠囊 `⚡ % · ⏱ 重置倒數`
  - 詳細：5h / 每週進度條 + 重置 + 今日 token/花費 + 即時活動 + 訂閱續訂日
  - 即時燒速：大字 tok/min + sparkline 的獨立模式
  - 設定：角落、預設模式、更新間隔、警示/危險門檻、帳單日、透明度、開機自動啟動、火焰特效、警示特效、顯示即時活動。詳細卡右上的 **⚙** 按鈕可直接開啟設定（不必經系統匣選單，避免某些平台匣圖示不顯示時無從進入）。
- **系統匣**：兩個並排環形儀表（左=5 小時、右=每週），各自顯示百分比與顏色；左鍵切換顯示/隱藏、右鍵選單。四種狀態：
  - 正常、**用量上升火焰**、**達門檻脈動警示**、**token 過期結冰**
- **門檻警示（可開關）**：5h 與每週**各自獨立**判定顏色（ok → 琥珀(warn) → 紅(crit)）；達門檻時 widget 與系統匣對應的環會以該顏色**脈動**，相當明顯。
- **過期結冰 + 立即恢復**：OAuth token 過期（太久沒開 Claude Code）時，精簡/詳細/系統匣都會「結冰」並**停止顯示舊數據**，明確提示「請開啟 Claude Code 重新登入」。重新登入後，結凍卡上的「**↻ 已重新登入，立即恢復**」按鈕可**馬上**觸發一次刷新，不必乾等下一個輪詢週期（最長 180 秒）；按鈕下方小字說明「不點也會自動恢復」，避免誤認成卡死。恢復成功時播放一次性**解凍融化動畫**（下滴 + 溶解 + 最後一亮，四主題通用）給出明確回饋。
- **可選渲染風格（4 種）**：設定可切換主題,即時換皮、四畫面 + 系統匣雙環同步變色。內嵌字體(OFL)。
  - **經典**:原始 coral/blue 深色(預設)。
  - **奧術 HUD**:黑曜玻璃 + 金色 filigree + 青色 HUD 角標(Cinzel + Orbitron)。
  - **魔法羊皮紙**:哈利波特風老羊皮紙 + 墨水 + 火漆角飾 + 燭火餘燼脈動(IM Fell English + Cinzel Decorative)。
  - **魔導霓虹**:電路網格 + 青/洋紅霓虹 + 掃描線(Orbitron + Share Tech Mono)。
- **釘選任何角落（支援多螢幕）**：無邊框、永遠置頂,拖到四角自動吸附並記住位置;**多螢幕**下會記住你拖去的那台螢幕(該螢幕拔除時自動退回主螢幕)。設定可改「**自由位置**」,放在任何螢幕的任意位置都記住、不吸附角落。
- **單一實例**：重複啟動(或開機自啟與手動啟動相撞)只會把既有視窗叫回前景,不會開出第二個托盤圖示。
- **statusline 即時更新（預設開啟）**：首次啟動時自動在 `~/.claude/settings.json` 安全註冊一次 statusLine（先備份、絕不覆蓋既有的其他 statusLine 設定；僅嘗試一次，記在 `statusline_auto_enable_done` 旗標，之後可自行在設定頁關閉/重開）。有 Claude Code session 在跑時即時更新且免打 API。狀態列格式為 `⚡ N% · 7d N% · Fable N% · ctx N%`：`⚡` 為 5 小時額度、`7d` 為每週額度、`Fable` 為 Fable 模型專屬每週額度（資料來自 OAuth usage endpoint 的 `limits[]`，經 `~/.config/claude-usage-monitor/quota-cache.json` 快取，widget 輪詢時回寫；快取逾時則 hook 自行以 2 秒 timeout 補抓一次），`ctx` 為目前 session 的 context 使用率（直接採用 Claude Code payload 回報的 `context_window` 物件計算，正確辨識 200k／1M 原生窗模型，不再用固定 200k 誤算 1M 窗模型的使用率）——三者皆讀不到時自動省略對應段落。所有百分比依危險度上色（<50 綠、50–79 黃、≥80 紅）。
- **顯示桌面小窗（可開關，預設開）**：設定頁「顯示桌面小窗」勾選框可獨立關閉桌面 widget（與 statusline 互不影響，各自可單獨開關）；widget 的隱藏路徑（系統匣左鍵切換、詳細卡 ✕、設定頁勾選框）統一寫回同一個顯示習慣，下次啟動沿用你最後一次的選擇。

## 下載 / Releases

每次推送 `v*` 版本標籤,GitHub Actions 會自動打包並發佈各平台安裝檔到 [Releases](https://github.com/rafaeltech555/claude-usage-monitor/releases):Linux `.deb` / `.AppImage`、macOS `.dmg`(universal,Intel + Apple Silicon)。

- **macOS 為未簽章**:首次開啟請對 `.app` 按右鍵 → 開啟(或 `xattr -dr com.apple.quarantine /Applications/"Claude Usage Monitor.app"`)。

## 安裝

```bash
# Debian/Ubuntu/Mint（檔名含空格,記得加引號）
sudo dpkg -i "Claude Usage Monitor_0.2.0_amd64.deb"

# 或免安裝
chmod +x "Claude Usage Monitor_0.2.0_amd64.AppImage"
"./Claude Usage Monitor_0.2.0_amd64.AppImage"
```

安裝後在應用程式選單搜尋「**Claude Usage Monitor**」即可開啟(可釘到 Dock/我的最愛);設定裡勾「開機自動啟動」後重開機會自動出現。

需要已安裝並登入 Claude Code 取得 OAuth token(僅在記憶體使用、只透過 TLS 送往官方 `api.anthropic.com`,不寫入磁碟或 log)。token 來源:**Linux** 讀 `~/.claude/.credentials.json`;**macOS** 讀登入 Keychain(`Claude Code-credentials`);任一平台都可用環境變數 `CLAUDE_CODE_OAUTH_TOKEN` 覆寫。

## 從原始碼建置

```bash
npm install
npm run tauri dev      # 開發
npm run tauri build    # 打包 .deb + AppImage
```

**Linux 系統依賴**：`libgtk-3-dev`、`libwebkit2gtk-4.1-dev`、`libayatana-appindicator3-dev`、`librsvg2-dev`、`libxdo-dev`。

## 測試

```bash
cargo test --manifest-path src-tauri/Cargo.toml   # Rust：config / usage 成本 / quota 解析 / statusline / icon
npm test                                           # 前端 vitest：格式化 + 續訂日計算
```

## 設定檔

`~/.config/claude-usage-monitor/config.json`：模式、角落、更新間隔(≥180s)、警示/危險門檻、帳單日、透明度、開機啟動、火焰特效(`effects`)、警示特效(`alert_effects`)、顯示即時活動(`show_activity`)、渲染風格(`theme`:classic/arcane/wizard/neon)、多螢幕位置記憶(`monitor`/`free_position`)、statusline 開關（`statusline`，預設開，`statusline_auto_enable_done` 記錄是否已自動註冊過）、顯示桌面小窗（`show_widget`，預設開）。

## 桌面環境備註

- 在 LXQt 等桌面，系統匣需啟用面板的 **StatusNotifier / AppIndicator** 外掛，否則匣圖示不顯示。
- 無邊框在 Cinnamon/Muffin 上需在視窗首次顯示前設定 `decorations(false)`（本專案已處理）。

## 已知限制 / 待辦

- `/api/oauth/usage` 為非官方端點，未來可能變動（已抽象成可抽換的 `QuotaProvider`）。
- 訂閱續訂日需手動填帳單日：OAuth token 無法存取帳單端點（`/api/oauth/profile` 的訂閱建立日 ≠ 實際帳單日）。
- macOS 已由 CI 打包(unsigned)且支援 Keychain token,但尚未在實機完整驗證(Keychain service 名稱 `Claude Code-credentials` 待真機確認);簽章/公證與 Windows 支援尚未做。
- 即時活動的「見底時間」依 180s 取樣的 5h 百分比斜率估算，較粗、會跳，故標「≈」；session 累計採「首次完整讀 + 之後增量 tail」近似。

## 版本紀錄

### v0.2.0（2026-06-28）

**新功能**

- **statusline context 使用率** `· ctx N%`：啟用 statusLine hook 後，狀態列尾端顯示目前 session 的 context window 使用率，自動辨識 200k / 1M window，讀不到時省略。

**修正**

- **重置時間顯示「—」**：Claude Code 在某些版本改以 epoch int（而非 RFC 3339）回傳 `resets_at`，導致重置倒數顯示破損；`win_from` 改為同時接受兩種格式，問題修正。
- **5h 見底估算溢位**：當速率極低時浮點相消導致估算結果爆成兆級小時；改以安全的 saturating subtraction 修正。
- **statusline context 讀取容忍 mid-codepoint tail 邊界**：尾端截斷在多位元組字元中間時，改為向前找完整 UTF-8 邊界，避免 panic 或亂碼。
- **in-widget ⚙ 設定按鈕**：詳細卡右上角的設定按鈕修正，不必依賴系統匣即可開啟設定，方便系統匣圖示不顯示時也能設定。
- **429 指數退避**：API 回 429 時改採 exponential backoff，避免過度重試。
- **statusLine 路徑含空格引號**：`statusLine` 設定的可執行檔路徑若含空格，改為自動加引號，避免路徑解析失敗。
- **macOS dock-reopen handler**：修正 macOS 點選 Dock 圖示時恢復隱藏視窗的行為（macOS 相關功能仍待實機驗證）。

### v0.1.0

初始發佈：基本 quota 監控、即時活動、四主題、系統匣、門檻警示、過期結冰、statusline hook opt-in。
