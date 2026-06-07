mod activity;
mod config;
mod icon;
mod quota;
mod statusline;
mod usage;

use config::Config;
use quota::{OAuthProvider, QuotaProvider, QuotaUsage};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow, WindowEvent};

const COMPACT: (f64, f64) = (190.0, 56.0);
const DETAILED: (f64, f64) = (270.0, 214.0);
const SETTINGS: (f64, f64) = (300.0, 480.0);
const ACTIVITY: (f64, f64) = (210.0, 208.0);
const MARGIN: f64 = 12.0;
const BOTTOM_PANEL_ALLOWANCE: f64 = 44.0; // leave room for a bottom taskbar

/// Unified snapshot pushed to the frontend on every poll.
#[derive(Debug, Clone, Serialize, Default)]
struct UsageSnapshot {
    quota: QuotaUsage,
    today: usage::TokenUsage,
    status_level: String, // "ok" | "warn" | "crit"
    error: Option<String>,
    fetched_at: String,
}

struct AppState {
    config: Mutex<Config>,
    latest: Mutex<UsageSnapshot>,
    anim_gen: AtomicU64,
    activity: Mutex<activity::ActivityTracker>,
    quota_samples: Mutex<std::collections::VecDeque<(chrono::DateTime<chrono::Local>, f64)>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Statusline hook mode: when Claude Code invokes `<exe> --statusline`, just
    // capture stdin and exit — do not start the GUI.
    if std::env::args().any(|a| a == "--statusline") {
        statusline::run_hook();
        return;
    }

    let config = Config::load();
    let want_autostart = config.autostart;

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(AppState {
            config: Mutex::new(config),
            latest: Mutex::new(UsageSnapshot::default()),
            anim_gen: AtomicU64::new(0),
            activity: Mutex::new(activity::ActivityTracker::new()),
            quota_samples: Mutex::new(std::collections::VecDeque::new()),
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            set_mode,
            set_corner,
            toggle_window,
            hide_window,
            refresh_now,
            get_snapshot,
            get_activity,
            fit_detailed,
            fit_compact,
            set_autostart,
            set_statusline_optin,
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::Moved(_) = event {
                snap_to_nearest_corner(window);
            }
        })
        .setup(move |app| {
            build_tray(app.handle())?;

            #[cfg(target_os = "linux")]
            if let Some(win) = app.get_webview_window("main") {
                linux_undecorate(&win);
            }

            // Apply persisted mode/corner to the window.
            let (mode, corner) = {
                let state = app.state::<AppState>();
                let c = state.config.lock().unwrap();
                (c.mode.clone(), c.corner.clone())
            };

            // Sync OS autostart to the saved preference.
            {
                use tauri_plugin_autostart::ManagerExt;
                let al = app.autolaunch();
                let _ = if want_autostart { al.enable() } else { al.disable() };
            }

            // Size, position, and show the window (it starts hidden so the
            // pre-map set_decorations above takes effect on strict WMs).
            apply_mode(app.handle(), &mode, &corner);

            spawn_poller(app.handle().clone());
            spawn_activity_ticker(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_config(state: State<AppState>) -> Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn save_config(state: State<AppState>, cfg: Config) -> Result<(), String> {
    let mut c = state.config.lock().unwrap();
    *c = cfg;
    c.save()
}

/// Resize the detailed window to fit its measured content height (the live
/// activity block makes the content variable: off / idle / active differ).
/// Width stays fixed; the window is re-pinned to its corner so growth happens
/// in the right direction.
#[tauri::command]
fn fit_detailed(state: State<AppState>, app: AppHandle, height: f64) {
    let (mode, corner) = {
        let c = state.config.lock().unwrap();
        (c.mode.clone(), c.corner.clone())
    };
    if mode != "detailed" {
        return;
    }
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let h = height.clamp(120.0, 600.0);
    let _ = win.set_size(tauri::LogicalSize::new(DETAILED.0, h));
    position_at_corner(&win, &corner, DETAILED.0, h);
}

/// Resize the compact pill to fit its measured content width (the live-activity
/// dot + burn rate, plus a variable-length reset countdown, make it variable).
/// Height stays fixed; the window is re-pinned to its corner.
#[tauri::command]
fn fit_compact(state: State<AppState>, app: AppHandle, width: f64) {
    let (mode, corner) = {
        let c = state.config.lock().unwrap();
        (c.mode.clone(), c.corner.clone())
    };
    if mode != "compact" {
        return;
    }
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let w = width.clamp(150.0, 420.0);
    let _ = win.set_size(tauri::LogicalSize::new(w, COMPACT.1));
    position_at_corner(&win, &corner, w, COMPACT.1);
}

#[tauri::command]
fn set_mode(state: State<AppState>, app: AppHandle, mode: String) -> Result<(), String> {
    let corner = {
        let mut c = state.config.lock().unwrap();
        // "settings" is a transient view — don't persist it as the default mode.
        if mode == "compact" || mode == "detailed" || mode == "activity" {
            c.mode = mode.clone();
            let _ = c.save();
        }
        c.corner.clone()
    };
    apply_mode(&app, &mode, &corner);
    Ok(())
}

#[tauri::command]
fn set_corner(state: State<AppState>, app: AppHandle, corner: String) -> Result<(), String> {
    let mode = {
        let mut c = state.config.lock().unwrap();
        c.corner = corner.clone();
        c.save()?;
        c.mode.clone()
    };
    apply_mode(&app, &mode, &corner);
    Ok(())
}

#[tauri::command]
fn toggle_window(app: AppHandle) {
    toggle_visibility(&app);
}

#[tauri::command]
fn hide_window(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

#[tauri::command]
fn get_snapshot(state: State<AppState>) -> UsageSnapshot {
    state.latest.lock().unwrap().clone()
}

#[tauri::command]
fn get_activity(app: AppHandle) -> activity::LiveActivity {
    build_activity(&app)
}

#[tauri::command]
fn set_autostart(app: AppHandle, state: State<AppState>, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let al = app.autolaunch();
    if enabled {
        al.enable().map_err(|e| e.to_string())?;
    } else {
        al.disable().map_err(|e| e.to_string())?;
    }
    let mut c = state.config.lock().unwrap();
    c.autostart = enabled;
    c.save()
}

#[tauri::command]
fn set_statusline_optin(state: State<AppState>, enabled: bool) -> Result<(), String> {
    if enabled {
        statusline::enable()?;
    } else {
        statusline::disable()?;
    }
    let mut c = state.config.lock().unwrap();
    c.statusline_optin = enabled;
    c.save()
}

#[tauri::command]
fn refresh_now(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let provider = OAuthProvider;
        poll_once(&app, &provider).await;
    });
}

// ---------------------------------------------------------------------------
// Polling
// ---------------------------------------------------------------------------

fn spawn_poller(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let provider = OAuthProvider;
        loop {
            poll_once(&app, &provider).await;
            let poll = app.state::<AppState>().config.lock().unwrap().poll_secs;
            tokio::time::sleep(std::time::Duration::from_secs(poll)).await;
        }
    });
}

fn parse_reset_secs(rfc3339: &str) -> Option<i64> {
    let dt = chrono::DateTime::parse_from_rfc3339(rfc3339).ok()?;
    let secs = (dt.with_timezone(&chrono::Local) - chrono::Local::now()).num_seconds();
    if secs > 0 {
        Some(secs)
    } else {
        None
    }
}

/// Tail transcripts and assemble the current LiveActivity snapshot.
fn build_activity(app: &AppHandle) -> activity::LiveActivity {
    let now = chrono::Local::now();
    let state = app.state::<AppState>();

    let optin = state.config.lock().unwrap().statusline_optin;
    let hint = if optin {
        statusline::read_hint_fresh(15)
    } else {
        None
    };
    let hint_fresh = hint.is_some();
    let force = hint
        .as_ref()
        .and_then(|h| h.transcript_path.clone())
        .map(std::path::PathBuf::from);
    let source = if hint_fresh { "statusline" } else { "jsonl" };

    state.activity.lock().unwrap().tick(now, force);

    let samples: Vec<_> = state.quota_samples.lock().unwrap().iter().cloned().collect();
    let (five_pct, reset_secs) = {
        let snap = state.latest.lock().unwrap();
        let five = snap.quota.five_hour.clone();
        (
            five.as_ref().map(|w| w.utilization),
            five.as_ref()
                .and_then(|w| w.resets_at.as_deref())
                .and_then(parse_reset_secs),
        )
    };

    let result = state
        .activity
        .lock()
        .unwrap()
        .snapshot(now, hint_fresh, source, &samples, five_pct, reset_secs);
    result
}

fn spawn_activity_ticker(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let show = app.state::<AppState>().config.lock().unwrap().show_activity;
            if show {
                let app2 = app.clone();
                if let Ok(act) =
                    tauri::async_runtime::spawn_blocking(move || build_activity(&app2)).await
                {
                    let _ = app.emit("activity-update", &act);
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

async fn poll_once(app: &AppHandle, provider: &OAuthProvider) {
    let (warn, crit, optin, effects, alert_effects) = {
        let state = app.state::<AppState>();
        let c = state.config.lock().unwrap();
        (
            c.warn_threshold,
            c.crit_threshold,
            c.statusline_optin,
            c.effects,
            c.alert_effects,
        )
    };

    let today = tauri::async_runtime::spawn_blocking(usage::today_usage)
        .await
        .unwrap_or_default();
    let fetched_at = chrono::Local::now().format("%H:%M:%S").to_string();

    // Previous values for increase detection.
    let prev = app.state::<AppState>().latest.lock().unwrap().quota.clone();

    // When the statusline source is opted in and fresh (a Claude Code session
    // recently rendered), use it and skip the network call; else hit OAuth.
    let quota_result = match optin.then(|| statusline::read_fresh(150)).flatten() {
        Some(q) => Ok(q),
        None => provider.fetch().await,
    };

    let snap = match quota_result {
        Ok(quota) => {
            let level = level_for(max_util(&quota), warn, crit);
            UsageSnapshot { quota, today, status_level: level, error: None, fetched_at }
        }
        Err(e) => {
            // Keep the last known quota so the UI doesn't flash empty on a blip.
            let level = level_for(max_util(&prev), warn, crit);
            UsageSnapshot { quota: prev.clone(), today, status_level: level, error: Some(e), fetched_at }
        }
    };

    // Detect a rise in either window since the last poll.
    let util = |w: &Option<quota::QuotaWindow>| w.as_ref().map(|x| x.utilization);
    let rose = |old: Option<f64>, new: Option<f64>| matches!((old, new), (Some(o), Some(n)) if n > o + 0.5);
    let flame_left = rose(util(&prev.five_hour), util(&snap.quota.five_hour));
    let flame_right = rose(util(&prev.seven_day), util(&snap.quota.seven_day));

    // Stale = the token is expired (auth error) → Claude Code hasn't run in a
    // while. Show the frozen tray look instead of flames.
    let stale = snap
        .error
        .as_deref()
        .map_or(false, |e| e.contains("401") || e.contains("unauthorized"));

    *app.state::<AppState>().latest.lock().unwrap() = snap.clone();

    // Sample the 5h utilization for the time-to-empty slope (keep ~last 10,
    // drop anything older than 20 min so a slope never spans a window reset).
    if let Some(w) = &snap.quota.five_hour {
        let now = chrono::Local::now();
        let app_state = app.state::<AppState>();
        let mut s = app_state.quota_samples.lock().unwrap();
        s.push_back((now, w.utilization));
        let cutoff = now - chrono::Duration::minutes(20);
        while s.front().map_or(false, |(t, _)| *t < cutoff) {
            s.pop_front();
        }
        while s.len() > 10 {
            s.pop_front();
        }
    }

    let _ = app.emit("usage-update", &snap);
    update_tray(app, &snap, warn, crit, stale);

    // Bump the animation generation to cancel any running tray animation, then
    // start the appropriate one. Priority: frozen (static) > alert > flames.
    let five_u = snap.quota.five_hour.as_ref().map(|w| w.utilization);
    let seven_u = snap.quota.seven_day.as_ref().map(|w| w.utilization);
    let alert_active = !stale
        && alert_effects
        && (five_u.unwrap_or(0.0) >= warn || seven_u.unwrap_or(0.0) >= warn);
    let gen = app.state::<AppState>().anim_gen.fetch_add(1, Ordering::SeqCst) + 1;
    if alert_active {
        spawn_alert(app.clone(), gen, five_u, seven_u, warn, crit);
    } else if !stale && effects && (flame_left || flame_right) {
        spawn_flame(app.clone(), gen, &snap.quota, warn, crit, flame_left, flame_right);
    }
}

/// Briefly animate flames over the ring(s) whose usage just rose.
fn spawn_flame(app: AppHandle, gen: u64, quota: &QuotaUsage, warn: f64, crit: f64, left: bool, right: bool) {
    let five = quota.five_hour.as_ref().map(|w| w.utilization);
    let seven = quota.seven_day.as_ref().map(|w| w.utilization);
    tauri::async_runtime::spawn(async move {
        for frame in 0..26u32 {
            if app.state::<AppState>().anim_gen.load(Ordering::SeqCst) != gen {
                return;
            }
            if let Some(tray) = app.tray_by_id("main") {
                let _ = tray.set_icon(Some(icon::gauge_dual_flame(
                    five, seven, warn, crit, left, right, frame,
                )));
            }
            tokio::time::sleep(Duration::from_millis(85)).await;
        }
        if app.state::<AppState>().anim_gen.load(Ordering::SeqCst) == gen {
            if let Some(tray) = app.tray_by_id("main") {
                let _ = tray.set_icon(Some(icon::gauge_dual(five, seven, warn, crit)));
            }
        }
    });
}

/// Continuously pulse the warn/crit ring(s) in the tray until the generation
/// changes (next poll) — the tray counterpart of the widget alert glow.
fn spawn_alert(app: AppHandle, gen: u64, five: Option<f64>, seven: Option<f64>, warn: f64, crit: f64) {
    tauri::async_runtime::spawn(async move {
        let mut frame: u32 = 0;
        loop {
            if app.state::<AppState>().anim_gen.load(Ordering::SeqCst) != gen {
                return;
            }
            if let Some(tray) = app.tray_by_id("main") {
                let _ = tray.set_icon(Some(icon::gauge_dual_alert(five, seven, warn, crit, frame)));
            }
            frame = frame.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
    });
}

fn max_util(q: &QuotaUsage) -> f64 {
    [&q.five_hour, &q.seven_day]
        .iter()
        .filter_map(|w| w.as_ref().map(|x| x.utilization))
        .fold(0.0_f64, f64::max)
}

fn level_for(util: f64, warn: f64, crit: f64) -> String {
    if util >= crit {
        "crit"
    } else if util >= warn {
        "warn"
    } else {
        "ok"
    }
    .to_string()
}

// ---------------------------------------------------------------------------
// Tray
// ---------------------------------------------------------------------------

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, "toggle", "顯示 / 隱藏", true, None::<&str>)?;
    let compact = MenuItem::with_id(app, "compact", "精簡模式", true, None::<&str>)?;
    let detailed = MenuItem::with_id(app, "detailed", "詳細模式", true, None::<&str>)?;
    let activity = MenuItem::with_id(app, "activity", "即時燒速", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "立即更新", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "設定…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "離開", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &toggle, &sep1, &compact, &detailed, &activity, &refresh, &settings, &sep2, &quit,
        ],
    )?;

    let builder = TrayIconBuilder::with_id("main")
        .icon(icon::gauge_dual(Some(0.0), Some(0.0), 75.0, 90.0))
        .icon_as_template(false)
        .tooltip("Claude Usage Monitor")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "toggle" => toggle_visibility(app),
            "compact" => apply_mode_persist(app, "compact"),
            "detailed" => apply_mode_persist(app, "detailed"),
            "activity" => apply_mode_persist(app, "activity"),
            "refresh" => refresh_now(app.clone()),
            "settings" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                }
                let _ = app.emit("go-settings", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_visibility(tray.app_handle());
            }
        });
    builder.build(app)?;
    Ok(())
}

fn update_tray(app: &AppHandle, snap: &UsageSnapshot, warn: f64, crit: f64, stale: bool) {
    let Some(tray) = app.tray_by_id("main") else { return };

    // Redraw the dual gauge: left = 5h (current), right = weekly.
    let five_u = snap.quota.five_hour.as_ref().map(|w| w.utilization);
    let seven_u = snap.quota.seven_day.as_ref().map(|w| w.utilization);
    let icon = if stale {
        icon::gauge_dual_frozen(five_u, seven_u)
    } else {
        icon::gauge_dual(five_u, seven_u, warn, crit)
    };
    let _ = tray.set_icon(Some(icon));
    let _ = tray.set_title(Some(format!(
        "{:.0}/{:.0}%",
        five_u.unwrap_or(0.0),
        seven_u.unwrap_or(0.0)
    )));

    if stale {
        let _ = tray.set_tooltip(Some(
            "❄ Claude token 已過期，請開啟 Claude Code 以更新用量".to_string(),
        ));
        return;
    }

    let five = snap
        .quota
        .five_hour
        .as_ref()
        .map(|w| format!("{:.0}%", w.utilization))
        .unwrap_or_else(|| "—".into());
    let seven = snap
        .quota
        .seven_day
        .as_ref()
        .map(|w| format!("{:.0}%", w.utilization))
        .unwrap_or_else(|| "—".into());
    let tip = match &snap.error {
        Some(e) => format!("Claude  5h {five} · 7d {seven}\n⚠ {e}"),
        None => format!("Claude  5h {five} · 7d {seven}  (更新 {})", snap.fetched_at),
    };
    let _ = tray.set_tooltip(Some(tip));
}

// ---------------------------------------------------------------------------
// Window helpers
// ---------------------------------------------------------------------------

fn apply_mode_persist(app: &AppHandle, mode: &str) {
    let corner = {
        let state = app.state::<AppState>();
        let mut c = state.config.lock().unwrap();
        c.mode = mode.to_string();
        let _ = c.save();
        c.corner.clone()
    };
    apply_mode(app, mode, &corner);
    // Tray switches the window size in Rust, but the webview decides which view
    // to render from its body class — tell it to switch (and re-fit).
    let _ = app.emit("set-mode", mode);
}

fn apply_mode(app: &AppHandle, mode: &str, corner: &str) {
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
    position_at_corner(&win, corner, w, h);
    let _ = win.show();
}

fn position_at_corner(win: &WebviewWindow, corner: &str, w: f64, h: f64) {
    // Default to the primary monitor (the user's main screen) rather than
    // current_monitor(), which on multi-monitor setups can be a secondary
    // HiDPI panel and push the widget off-screen. Work entirely in physical
    // pixels to avoid logical<->physical double-scaling.
    let mon = win
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| win.current_monitor().ok().flatten());
    let Some(mon) = mon else { return };

    let scale = mon.scale_factor();
    let ms = mon.size(); // physical
    let mp = mon.position(); // physical
    let wp = (w * scale).round() as i32;
    let hp = (h * scale).round() as i32;
    let m = (MARGIN * scale).round() as i32;
    let bp = (BOTTOM_PANEL_ALLOWANCE * scale).round() as i32;
    let mw = ms.width as i32;
    let mh = ms.height as i32;

    let (x, y) = match corner {
        "tl" => (mp.x + m, mp.y + m),
        "bl" => (mp.x + m, mp.y + mh - hp - m - bp),
        "br" => (mp.x + mw - wp - m, mp.y + mh - hp - m - bp),
        _ => (mp.x + mw - wp - m, mp.y + m), // "tr"
    };
    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
}

/// On manual drag, remember which corner the widget ended up nearest to.
fn snap_to_nearest_corner(win: &tauri::Window) {
    let Ok(pos) = win.outer_position() else { return };
    let Ok(size) = win.outer_size() else { return };
    let Ok(Some(mon)) = win.current_monitor() else { return };
    let ms = mon.size();
    let mp = mon.position();

    let cx = pos.x + size.width as i32 / 2;
    let cy = pos.y + size.height as i32 / 2;
    let midx = mp.x + ms.width as i32 / 2;
    let midy = mp.y + ms.height as i32 / 2;

    let corner = match (cx < midx, cy < midy) {
        (true, true) => "tl",
        (false, true) => "tr",
        (true, false) => "bl",
        (false, false) => "br",
    };

    let app = win.app_handle();
    let state = app.state::<AppState>();
    let mut c = state.config.lock().unwrap();
    if c.corner != corner {
        c.corner = corner.to_string();
        let _ = c.save();
    }
}

/// Force a true frameless window on GTK-based WMs (Muffin/Mutter, etc.) that
/// otherwise apply a server-side title bar. A "Utility" type hint also keeps it
/// out of the taskbar and Alt-Tab while remaining clickable.
#[cfg(target_os = "linux")]
fn linux_undecorate(win: &WebviewWindow) {
    use gtk::prelude::*;
    if let Ok(gtk_win) = win.gtk_window() {
        gtk_win.set_decorated(false);
        gtk_win.set_skip_taskbar_hint(true);
        gtk_win.set_skip_pager_hint(true);
    }
}

fn toggle_visibility(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
        } else {
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
}
