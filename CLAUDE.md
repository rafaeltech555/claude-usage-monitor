# Claude Usage Monitor

> 通用規則（語言、委派、secret scan、紅線）以上層 `../CLAUDE.md` 與 `../playbook/` 為權威源；本檔只放本專案特有內容。

可釘在桌面任何角落的 Claude 用量監控小工具（Tauri v2，Rust + vanilla-TS）。顯示 Claude 方案額度、重置時間、今日 token 用量與等值花費，詳細功能見 `README.md`。

## 里程碑（暫定，依 git 歷史重建，待本人確認）

repo 內完全查無任何里程碑/roadmap 文件，以下純依 git commit 與僅有的 2 個 tag（`v0.1.0`、`v0.2.0`）逆向重建，**尚未經本人確認，切分可能不準**：

- **M0**（done）：初版核心——tray 雙環量表、live activity 燃燒速率估算、frozen（斷線）卡片狀態＋警示效果（依 commit `b491400`..`3af4d28`，06-06~06-07 前段推測）
- **M1**（done）：打磨與跨平台收斂為 v0.1.0——主題切換、multi-monitor、single-instance、macOS Keychain token + CI 打包（依 `37cbae8`..`f93f7cf` = tag `v0.1.0`）
- **M2**（done）：frozen-card 即時刷新＋融化動畫、statusline 顯示目前 session context 使用率 %，收斂為 v0.2.0（依 `af0dfcd`..`94b602f` = tag `v0.2.0`）
- **M3**（進行中，近期已停滯）：發布後小型硬化——epoch resets_at 相容修正、gitleaks secret-scan CI（依 `5d74812`、`358405a`；最後一筆 07-03 起無新 commit）
- **M4**（done 2026-08-09）：statusline 顯示 Fable 模型專屬額度＋百分比顏色渲染（spec/plan 見 docs/superpowers/）

## Backlog

- **（待確認）以上 M0~M3 全部里程碑純屬 git 歷史推測**：repo 無任何里程碑文件佐證，僅有 2 個正式 tag（v0.1.0/v0.2.0）可信；階段切分與描述需要本人逐條核對是否符合實際開發脈絡 — 依 git log 重建 — 來源：本次里程碑盤點 2026-07-20
- **（待確認）`projects.yaml` 現值 `milestone: M5/M6` 與本 repo 實況不符**：repo 內從未使用過 Mx 這種里程碑代號，git 歷史只支撐得起約 4 個階段（M0~M3）。疑似誤植——很可能是照抄 Almanaut 自己的里程碑編號（Almanaut 目前正好在 M6），與這個 repo 自身的開發階段無關。待本人確認後更正或清空該欄位 — 依 `projects.yaml` 現值比對 git 歷史 — 來源：本次里程碑盤點 2026-07-20
