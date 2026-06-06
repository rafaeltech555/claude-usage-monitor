# Claude Usage Monitor

可釘在桌面任何角落的 Claude 用量監控小工具（Tauri v2，Rust + vanilla-TS）。隨時顯示 Claude 方案額度與何時刷新，以及今日 token 用量與等值花費。

## 功能

- **方案額度 + 重置時間**：5 小時滾動視窗、每週額度的使用百分比與重置倒數（資料來自 `/api/oauth/usage`，與 Claude Code `/usage` 同源）。
- **今日 token / 等值花費**：解析本機 `~/.claude/projects/*/*.jsonl`，顯示輸入/輸出 token 與等值 USD（Max 為月費定額，金額僅供參考）。
- **訂閱續訂日**：在設定填入帳單日（每月幾號，見你 Claude 帳單頁的 auto-renew 日期），詳細模式即顯示下次續訂日與倒數。（OAuth token 無法存取帳單端點，故採手動設定。）
- **三種畫面**：
  - 精簡：膠囊 `⚡ % · ⏱ 重置倒數`
  - 詳細：5h / 每週進度條 + 重置 + 今日 token/花費 + 訂閱續訂日
  - 設定：角落、預設模式、更新間隔、警示/危險門檻、帳單日、透明度、開機自動啟動、火焰特效、警示特效
- **系統匣**：兩個並排環形儀表（左=5 小時、右=每週），各自顯示百分比與顏色；左鍵切換顯示/隱藏、右鍵選單。四種狀態：
  - 正常、**用量上升火焰**、**達門檻脈動警示**、**token 過期結冰**
- **門檻警示（可開關）**：5h 與每週**各自獨立**判定顏色（ok → 琥珀(warn) → 紅(crit)）；達門檻時 widget 與系統匣對應的環會以該顏色**脈動**，相當明顯。
- **過期結冰**：OAuth token 過期（太久沒開 Claude Code）時，精簡/詳細/系統匣都會「結冰」並**停止顯示舊數據**，明確提示「請開啟 Claude Code 重新登入」。
- **釘選任何角落**：無邊框、永遠置頂，可拖到四角並自動記住位置。
- **statusline 即時更新（opt-in，預設關閉）**：啟用後在 `~/.claude/settings.json` 註冊 statusLine（先備份、不覆蓋既有設定），有 Claude Code session 在跑時即時更新且免打 API。

## 安裝

```bash
# Debian/Ubuntu/Mint
sudo dpkg -i claude-usage-monitor_0.1.0_amd64.deb

# 或免安裝
chmod +x claude-usage-monitor_0.1.0_amd64.AppImage
./claude-usage-monitor_0.1.0_amd64.AppImage
```

需要已安裝並登入 Claude Code（讀取 `~/.claude/.credentials.json` 的 OAuth token；token 僅在記憶體使用、只透過 TLS 送往官方 `api.anthropic.com`，不寫入磁碟或 log）。

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

`~/.config/claude-usage-monitor/config.json`：模式、角落、更新間隔(≥180s)、警示/危險門檻、帳單日、透明度、開機啟動、火焰特效(`effects`)、警示特效(`alert_effects`)、statusline opt-in。

## 桌面環境備註

- 在 LXQt 等桌面，系統匣需啟用面板的 **StatusNotifier / AppIndicator** 外掛，否則匣圖示不顯示。
- 無邊框在 Cinnamon/Muffin 上需在視窗首次顯示前設定 `decorations(false)`（本專案已處理）。

## 已知限制 / 待辦

- `/api/oauth/usage` 為非官方端點，未來可能變動（已抽象成可抽換的 `QuotaProvider`）。
- 訂閱續訂日需手動填帳單日：OAuth token 無法存取帳單端點（`/api/oauth/profile` 的訂閱建立日 ≠ 實際帳單日）。
- 目前僅在 Linux 建置/驗證；跨平台（Windows/macOS）程式碼大致可移植，待補 macOS Keychain token、視窗設定與 CI（GitHub Actions）。
- 即時活動狀態（目前 session 正在燒多少）為未來擴充。
