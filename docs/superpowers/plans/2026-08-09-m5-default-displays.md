# M5: statusline 預設開啟＋顯示面開關記憶 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** statusline 出廠預設開啟（首啟自動走安全註冊）；widget 與 statusline 可個別關閉且選擇被記住，下次啟動依習慣。

**Architecture:** 沿用既有 `statusline_optin` 欄位＋`set_statusline_optin` command（已存在），只翻預設值並加一次性 auto-enable 旗標；widget 顯示習慣以新欄位 `show_widget` 記錄，所有 hide/show 路徑（tray toggle、✕ 按鈕、設定頁 checkbox）統一寫回。

**Tech Stack:** Rust（tauri v2）、vanilla TS。spec：`docs/superpowers/specs/2026-08-09-statusline-fable-default-design.md`（M5 節）。

**與 spec 的既定偏差（已決策，實作照本 plan）：**
1. 不新增巢狀 `displays: {widget, statusline}`——config 既有風格是平面欄位：statusline 開關沿用 `statusline_optin`（預設改 true）、widget 開關新增 `show_widget`（預設 true）。
2. tray 選單不加 statusline 勾選項（設定頁既有 toggle 已覆蓋；tray「設定…」一鍵可達）。
3. Task 5 會把偏差回寫 spec 文件。

**每個 task 結尾 commit**（僅 commit 不 push），message 繁中照 git log 風格，結尾加 `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`。

---

### Task 1: config 欄位（config.rs）

**Files:**
- Modify: `src-tauri/src/config.rs`

- [ ] **Step 1: 寫失敗測試**（改既有 + 新增，`mod tests`）

改 `default_is_valid`（`:120-128`）最後一行斷言為：

```rust
        assert!(c.statusline_optin); // M5 起出廠預設開
        assert!(c.show_widget);
        assert!(!c.statusline_auto_enable_done);
```

新增：

```rust
    #[test]
    fn old_config_json_gets_new_field_defaults() {
        // 舊版設定檔沒有 M5 新欄位：補預設值，且既有明確值不被蓋掉
        let old = r#"{"mode":"detailed","statusline_optin":false}"#;
        let c: Config = serde_json::from_str(old).unwrap();
        assert!(!c.statusline_optin); // 使用者存過的明確 false 要保留
        assert!(c.show_widget);
        assert!(!c.statusline_auto_enable_done);
    }
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cargo test --manifest-path src-tauri/Cargo.toml config`
Expected: FAIL（欄位不存在／預設值不符）

- [ ] **Step 3: 實作**

struct 內 `statusline_optin` 註解與欄位區塊改為（並緊接其後加兩個新欄位）：

```rust
    /// Register a statusline command in ~/.claude/settings.json.
    /// Default ON since M5 (auto-enabled once on first launch via the safe
    /// path: backup + refuse to clobber a foreign statusLine). Turning it off
    /// is persisted and never auto-reverted.
    pub statusline_optin: bool,
    /// Show the desktop widget window. Hiding it (tray toggle / ✕ button /
    /// settings) is remembered; the tray stays as the re-entry point.
    pub show_widget: bool,
    /// One-shot flag: the first-launch statusline auto-enable has been
    /// attempted (success or not), so we never force it again.
    pub statusline_auto_enable_done: bool,
```

`Default` impl 對應改／加：

```rust
            statusline_optin: true,
            show_widget: true,
            statusline_auto_enable_done: false,
```

- [ ] **Step 4: 跑測試確認通過**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: 全 PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/config.rs
git commit -m "feat(config): statusline 預設開啟；新增 show_widget 與 auto-enable 一次性旗標"
```

---

### Task 2: statusline 註冊狀態查詢（statusline.rs）

**Files:**
- Modify: `src-tauri/src/statusline.rs`

- [ ] **Step 1: 寫失敗測試**（`mod tests`，沿用既有 `tmp(...)` helper 的寫檔模式；若無 helper 就照 `enable_refuses_existing_user_statusline` 測試（`:393-398`）的做法自建暫存路徑）

```rust
    #[test]
    fn status_at_reports_ours_foreign_none() {
        let dir = std::env::temp_dir().join(format!("cum-status-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");

        // 檔案不存在 → none
        assert_eq!(status_at(&path), "none");
        // 沒有 statusLine 鍵 → none
        std::fs::write(&path, r#"{"theme":"dark"}"#).unwrap();
        assert_eq!(status_at(&path), "none");
        // 他人的 statusLine → foreign
        std::fs::write(&path, r#"{"statusLine":{"command":"other --bar"}}"#).unwrap();
        assert_eq!(status_at(&path), "foreign");
        // 我們的 → ours
        std::fs::write(
            &path,
            format!(r#"{{"statusLine":{{"command":{}}}}}"#, serde_json::json!(our_command())),
        )
        .unwrap();
        assert_eq!(status_at(&path), "ours");
        // 壞 JSON → foreign（保守：不明狀態不宣稱是我們的，也不覆蓋）
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(status_at(&path), "foreign");

        std::fs::remove_dir_all(&dir).ok();
    }
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cargo test --manifest-path src-tauri/Cargo.toml status_at`
Expected: FAIL（函式未定義）

- [ ] **Step 3: 實作**（放在 `disable_at` 之後）

```rust
/// Registration state of Claude Code's statusLine: "ours" | "foreign" | "none".
/// Unreadable/corrupt settings count as "foreign" (be conservative: never claim
/// or touch what we can't positively identify as ours).
pub fn status() -> String {
    settings_path().map(|p| status_at(&p)).unwrap_or_else(|| "none".into())
}

fn status_at(path: &std::path::Path) -> String {
    if !path.exists() {
        return "none".into();
    }
    let Ok(s) = std::fs::read_to_string(path) else {
        return "foreign".into();
    };
    let Ok(obj) = serde_json::from_str::<serde_json::Value>(&s) else {
        return "foreign".into();
    };
    match obj.get("statusLine") {
        None => "none".into(),
        Some(sl) if is_ours(sl) => "ours".into(),
        Some(_) => "foreign".into(),
    }
}
```

- [ ] **Step 4: 跑測試確認通過**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: 全 PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/statusline.rs
git commit -m "feat(statusline): 新增註冊狀態查詢（ours/foreign/none）"
```

---

### Task 3: 啟動行為與 hide/show 記憶（lib.rs）

**Files:**
- Modify: `src-tauri/src/lib.rs`（setup `:123-135`、`save_config` `:168-180`、`hide_window` `:283-287`、`toggle_visibility` `:916-925`、`apply_mode` `:767-784`、`invoke_handler` 清單 `:77-` 區）

- [ ] **Step 1: setup 啟動邏輯**（把 `:123-135` 的 self-heal 區塊與 `apply_mode` 呼叫改為）

```rust
            // Statusline registration: opted-in users get self-healed every
            // start (exe path may change between installs). The first launch
            // ever also auto-attempts once (default-on since M5); the one-shot
            // flag makes sure a user who later turns it off stays off.
            {
                let (optin, done) = {
                    let state = app.state::<AppState>();
                    let c = state.config.lock().unwrap();
                    (c.statusline_optin, c.statusline_auto_enable_done)
                };
                if optin {
                    if let Err(e) = statusline::enable() {
                        eprintln!("[statusline] register on startup failed: {e}");
                    }
                }
                if !done {
                    let state = app.state::<AppState>();
                    let mut c = state.config.lock().unwrap();
                    c.statusline_auto_enable_done = true;
                    let _ = c.save();
                }
            }

            // Size, position, and (when the widget display is on) show the
            // window (it starts hidden so the pre-map set_decorations above
            // takes effect on strict WMs). With show_widget off the window
            // stays hidden; the tray is the re-entry point.
            let show_widget = {
                let state = app.state::<AppState>();
                let s = state.config.lock().unwrap().show_widget;
                s
            };
            apply_mode_visibility(app.handle(), &mode, show_widget);
```

- [ ] **Step 2: `apply_mode` 拆出可控顯示版**（`:767-784` 改為）

```rust
fn apply_mode(app: &AppHandle, mode: &str) {
    apply_mode_visibility(app, mode, true);
}

fn apply_mode_visibility(app: &AppHandle, mode: &str, show: bool) {
    let Some(win) = app.get_webview_window("main") else { return };
    let (w, h) = match mode {
        "detailed" => DETAILED,
        "settings" => SETTINGS,
        "activity" => ACTIVITY,
        _ => COMPACT,
    };
    // Re-assert frameless at runtime: some WMs (e.g. Muffin/Mutter on Cinnamon)
    // draw a server-side title bar if the decorations:false config request
    // races window creation.
    let _ = win.set_decorations(false);
    let _ = win.set_shadow(false);
    let _ = win.set_always_on_top(true);
    let _ = win.set_size(tauri::LogicalSize::new(w, h));
    place_window(app, &win, w, h);
    if show {
        let _ = win.show();
    }
}
```

- [ ] **Step 3: hide/show 全路徑寫回 `show_widget`**

`toggle_visibility`（`:916-925`）改為：

```rust
fn toggle_visibility(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let was_visible = w.is_visible().unwrap_or(false);
        if was_visible {
            let _ = w.hide();
        } else {
            let _ = w.show();
            let _ = w.set_focus();
        }
        set_show_widget_pref(app, !was_visible);
    }
}

/// Persist the widget-display habit (the "which displays do I keep open"
/// memory) — every hide/show path funnels through here.
fn set_show_widget_pref(app: &AppHandle, show: bool) {
    let state = app.state::<AppState>();
    let mut c = state.config.lock().unwrap();
    if c.show_widget != show {
        c.show_widget = show;
        let _ = c.save();
    }
}
```

`hide_window`（`:283-287`）改為：

```rust
#[tauri::command]
fn hide_window(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    set_show_widget_pref(&app, false);
}
```

- [ ] **Step 4: 新 commands＋註冊＋`save_config` 防倒灌**

新增（放在 `hide_window` 附近）：

```rust
/// Settings-UI toggle for the desktop widget display. Enabling re-applies the
/// current mode (size + place + show); disabling hides to tray.
#[tauri::command]
fn set_show_widget(state: State<AppState>, app: AppHandle, enabled: bool) {
    let mode = {
        let mut c = state.config.lock().unwrap();
        c.show_widget = enabled;
        let _ = c.save();
        c.mode.clone()
    };
    if enabled {
        apply_mode(&app, &mode);
    } else if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

/// Claude Code statusLine registration state for the settings UI:
/// "ours" | "foreign" | "none".
#[tauri::command]
fn statusline_status() -> String {
    statusline::status()
}
```

`invoke_handler` 清單（`:77-` 區，`set_statusline_optin,` 那行之後）加：

```rust
            set_show_widget,
            statusline_status,
```

`save_config`（`:168-180`）在既有 placement 保留區之後、`*c = next;` 之前加：

```rust
    // Backend-owned toggles: written by tray/✕/dedicated commands — a stale
    // frontend config object must not clobber them.
    next.show_widget = c.show_widget;
    next.statusline_auto_enable_done = c.statusline_auto_enable_done;
```

- [ ] **Step 5: 編譯＋全測試**

Run: `cargo test --manifest-path src-tauri/Cargo.toml && cargo build --manifest-path src-tauri/Cargo.toml`
Expected: 全 PASS、無新 warning

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat: widget 顯示習慣記憶（tray/✕/設定統一寫回），statusline 首啟自動註冊一次"
```

---

### Task 4: 設定頁 UI（index.html、main.ts）

**Files:**
- Modify: `index.html`（settings view，`s-autostart` 那行 `:163-165` 附近）
- Modify: `src/main.ts`（Config 型別 `:45` 區、settings 載入 `:151-165` 區、綁定 `:250-264` 區）

- [ ] **Step 1: index.html 加 widget checkbox**（放在 `s-autostart` 的 label 之前，同樣的 label 結構）

```html
      <label class="row">
        <input id="s-widget" type="checkbox" /> 顯示桌面小窗（關閉後可由系統匣重開）
      </label>
```

- [ ] **Step 2: main.ts 型別與載入**

Config 型別（`statusline_optin: boolean;` 之後）加：

```ts
  show_widget: boolean;
  statusline_auto_enable_done: boolean;
```

settings 載入區（`:164` `s-statusline` 那行附近）加：

```ts
  (document.getElementById("s-widget") as HTMLInputElement).checked = cfg.show_widget;
```

並在載入 `s-statusline` checkbox 之後補 foreign 警示（沿用既有 `s-statusline-msg`；若載入函式非 async，改用 `.then()` 寫法，行為等價即可）：

```ts
  if (cfg.statusline_optin) {
    invoke<string>("statusline_status").then((st) => {
      if (st === "foreign") {
        const msg = $("s-statusline-msg");
        msg.textContent = "⚠ statusline 未生效：偵測到其他工具的 statusLine 設定";
        msg.hidden = false;
      }
    });
  }
```

- [ ] **Step 3: main.ts 綁定**（`s-statusline` 綁定 `:250-264` 之後，照同一 async 模式）

```ts
  on("s-widget", "change", async (el) => {
    const enabled = (el as HTMLInputElement).checked;
    await invoke("set_show_widget", { enabled });
    cfg.show_widget = enabled;
  });
```

- [ ] **Step 4: 前端建置＋測試**

Run: `npm run build && npm test`
Expected: tsc/vite 無錯、vitest 全 PASS

- [ ] **Step 5: Commit**

```bash
git add index.html src/main.ts
git commit -m "feat(ui): 設定頁新增桌面小窗開關；statusline 被他人佔用時顯示警示"
```

---

### Task 5: spec 回寫與收尾驗證

- [ ] **Step 1: spec 偏差回寫**：編輯 `docs/superpowers/specs/2026-08-09-statusline-fable-default-design.md` 的「M5」節——`displays: {…}` 段改述為平面欄位（`statusline_optin` 沿用＋`show_widget` 新增）、刪去 tray 加 toggle 的句子並註明（設定頁既有 toggle 覆蓋，實作決策 2026-08-09）、補「所有 hide 路徑（tray/✕/設定）都寫回 `show_widget`」一句。

- [ ] **Step 2: 全量測試最後一跑**

Run: `cargo test --manifest-path src-tauri/Cargo.toml && npm test`
Expected: 全 PASS（回報數字）

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-08-09-statusline-fable-default-design.md
git commit -m "docs(spec): M5 依實作定案回寫（平面欄位、tray 不加 toggle、hide 路徑統一記憶）"
```

- [ ] **Step 4: 留給人工／GUI 驗收的清單（寫進回報，不執行）**

- 設定頁關「顯示桌面小窗」→ 窗即隱藏；重啟 app → 窗不出現、tray 在；tray 左鍵或「顯示 / 隱藏」→ 窗回來且下次重啟記得
- ✕ 按鈕收回後重啟 → 窗不出現（習慣被記住）
- 首次啟動（刪 `~/.config/claude-usage-monitor/config.json` 模擬）→ statusline 自動註冊成功（`~/.claude/settings.json` 出現我們的 statusLine，且原檔有 `.json.cum-backup` 備份）

## Self-Review 紀錄

- spec 覆蓋：M5 三要件（預設開啟＋auto-enable 一次性、widget/statusline 個別開關、習慣記憶）全對應；偏差 3 點已列並含 spec 回寫 task。
- 型別一致：`set_show_widget_pref(&AppHandle, bool)`／`apply_mode_visibility(&AppHandle, &str, bool)`／`statusline::status() -> String` 各處引用一致；TS `show_widget` 與 Rust 欄位同名。
- 無 placeholder。
