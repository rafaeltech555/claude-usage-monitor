# Claude Usage Monitor

可釘在桌面任何角落的 Claude 用量監控小工具（Tauri v2，Rust + vanilla-TS）。隨時顯示 Claude 方案額度與何時刷新，以及今日 token 用量與等值花費。

## 功能

- **方案額度 + 重置時間**：5 小時滾動視窗、每週額度的使用百分比與重置倒數（資料來自 `/api/oauth/usage`，與 Claude Code `/usage` 同源）。
- **今日 token / 等值花費**：解析本機 `~/.claude/projects/*/*.jsonl`，顯示輸入/輸出 token 與等值 USD（Max 為月費定額，金額僅供參考）。
- **三種畫面**：
  - 精簡：膠囊 `⚡ % · ⏱ 重置倒數`
  - 詳細：5h / 每週進度條 + 重置 + 今日 token/花費
  - 設定：角落、預設模式、更新間隔、警示門檻、透明度、開機自動啟動
- **系統匣**：動態環形儀表圖示，中央直接顯示使用百分比；左鍵切換顯示/隱藏、右鍵選單。
- **釘選任何角落**：無邊框、永遠置頂，可拖到四角並自動記住位置。
- **額度警示**：超過門檻時整個 widget 與匣圖示變橘 / 變紅閃爍。
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

## 設定檔

`~/.config/claude-usage-monitor/config.json`（模式、角落、更新間隔≥180s、警示門檻、透明度、開機啟動）。

## 桌面環境備註

- 在 LXQt 等桌面，系統匣需啟用面板的 **StatusNotifier / AppIndicator** 外掛，否則匣圖示不顯示。
- 無邊框在 Cinnamon/Muffin 上需在視窗首次顯示前設定 `decorations(false)`（本專案已處理）。

## 已知限制 / 待辦

- `/api/oauth/usage` 為非官方端點，未來可能變動（已抽象成可抽換的 `QuotaProvider`）。
- 即時活動狀態（目前 session 正在燒多少）為未來擴充。
