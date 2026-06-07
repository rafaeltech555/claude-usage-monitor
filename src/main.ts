import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { fmtTokens, fmtCountdown, nextRenewal, fmtRate, fmtMinsToEmpty } from "./format";

type QuotaWindow = { utilization: number; resets_at: string | null };
type Quota = {
  five_hour: QuotaWindow | null;
  seven_day: QuotaWindow | null;
  seven_day_opus: QuotaWindow | null;
  seven_day_sonnet: QuotaWindow | null;
};
type TokenUsage = {
  input: number;
  output: number;
  cache_write: number;
  cache_read: number;
  total: number;
  cost_usd: number;
};
type Snapshot = {
  quota: Quota;
  today: TokenUsage;
  status_level: "ok" | "warn" | "crit";
  error: string | null;
  fetched_at: string;
};
type LiveActivity = {
  active: boolean;
  burn_tpm: number;
  session_tokens: number;
  last_active_secs: number;
  mins_to_empty: number | null;
  beats_reset: boolean;
  spark: number[];
  source: string;
};
type Config = {
  mode: string;
  corner: string;
  poll_secs: number;
  warn_threshold: number;
  crit_threshold: number;
  opacity: number;
  autostart: boolean;
  statusline_optin: boolean;
  effects: boolean;
  alert_effects: boolean;
  show_activity: boolean;
  renewal_day: number;
};

let latest: Snapshot | null = null;
let cfg: Config;
let latestActivity: LiveActivity | null = null;
let staleNow = false;
let lastFitH = 0;
let lastFitW = 0;

function $(id: string): HTMLElement {
  return document.getElementById(id)!;
}

function setMode(mode: string) {
  document.body.classList.remove("mode-compact", "mode-detailed", "mode-settings", "mode-activity");
  const m = ["detailed", "settings", "activity"].includes(mode) ? mode : "compact";
  document.body.classList.add("mode-" + m);
  lastFitH = 0; // re-fit after any mode switch
  lastFitW = 0;
}

// The detailed card has variable height (the live-activity block differs across
// off / idle / active), so the window must grow/shrink to fit its content.
// Measure the card at natural height and ask the backend to resize + re-pin.
function fitDetailed() {
  if (!document.body.classList.contains("mode-detailed")) return;
  const card = $("detailed");
  const prev = card.style.height;
  card.style.height = "auto";
  const h = Math.ceil(card.getBoundingClientRect().height);
  card.style.height = prev;
  if (h > 0 && h !== lastFitH) {
    lastFitH = h;
    invoke("fit_detailed", { height: h });
  }
}

// The compact pill has variable width (the live dot + burn rate, plus a
// variable-length reset countdown). Measure natural width and re-pin.
function fitCompact() {
  if (!document.body.classList.contains("mode-compact")) return;
  const pill = $("compact");
  const prev = pill.style.width;
  pill.style.width = "max-content";
  const w = Math.ceil(pill.getBoundingClientRect().width) + 2;
  pill.style.width = prev;
  if (w > 0 && w !== lastFitW) {
    lastFitW = w;
    invoke("fit_compact", { width: w });
  }
}

// Fit whichever mode is active (each helper no-ops unless its mode is showing).
function fitWindow() {
  fitDetailed();
  fitCompact();
}

function applyOpacity(v: number) {
  document.documentElement.style.setProperty("--bg", `rgba(30,30,40,${v})`);
}

function saveCfg() {
  invoke("save_config", { cfg });
}

function populateSettings() {
  (document.getElementById("s-corner") as HTMLSelectElement).value = cfg.corner;
  (document.getElementById("s-mode") as HTMLSelectElement).value = cfg.mode;
  (document.getElementById("s-poll") as HTMLInputElement).value = String(cfg.poll_secs);
  (document.getElementById("s-warn") as HTMLInputElement).value = String(cfg.warn_threshold);
  (document.getElementById("s-crit") as HTMLInputElement).value = String(cfg.crit_threshold);
  (document.getElementById("s-renewal") as HTMLInputElement).value = String(cfg.renewal_day);
  (document.getElementById("s-opacity") as HTMLInputElement).value = String(cfg.opacity);
  (document.getElementById("s-autostart") as HTMLInputElement).checked = cfg.autostart;
  (document.getElementById("s-effects") as HTMLInputElement).checked = cfg.effects;
  (document.getElementById("s-alerts") as HTMLInputElement).checked = cfg.alert_effects;
  (document.getElementById("s-activity") as HTMLInputElement).checked = cfg.show_activity;
  (document.getElementById("s-statusline") as HTMLInputElement).checked = cfg.statusline_optin;
  $("s-statusline-msg").hidden = true;
}

function openSettings() {
  populateSettings();
  setMode("settings");
  invoke("set_mode", { mode: "settings" });
}

function closeSettings() {
  setMode(cfg.mode);
  invoke("set_mode", { mode: cfg.mode });
}

function wireSettings() {
  const on = (id: string, ev: string, fn: (el: HTMLInputElement | HTMLSelectElement) => void) =>
    document.getElementById(id)!.addEventListener(ev, (e) => fn(e.target as any));

  $("s-done").addEventListener("click", closeSettings);
  on("s-corner", "change", (el) => {
    cfg.corner = el.value;
    invoke("set_corner", { corner: cfg.corner });
  });
  on("s-mode", "change", (el) => {
    cfg.mode = el.value;
    saveCfg();
  });
  on("s-poll", "change", (el) => {
    cfg.poll_secs = Math.max(180, parseInt(el.value || "180", 10));
    saveCfg();
  });
  on("s-warn", "change", (el) => {
    cfg.warn_threshold = parseFloat(el.value);
    saveCfg();
  });
  on("s-crit", "change", (el) => {
    cfg.crit_threshold = parseFloat(el.value);
    saveCfg();
  });
  on("s-renewal", "change", (el) => {
    cfg.renewal_day = Math.min(31, Math.max(0, parseInt(el.value || "0", 10)));
    saveCfg();
    if (latest) render(latest);
  });
  on("s-opacity", "input", (el) => {
    cfg.opacity = parseFloat(el.value);
    applyOpacity(cfg.opacity);
    saveCfg();
  });
  on("s-autostart", "change", (el) => {
    const enabled = (el as HTMLInputElement).checked;
    cfg.autostart = enabled;
    invoke("set_autostart", { enabled });
  });
  on("s-effects", "change", (el) => {
    cfg.effects = (el as HTMLInputElement).checked;
    saveCfg();
  });
  on("s-alerts", "change", (el) => {
    cfg.alert_effects = (el as HTMLInputElement).checked;
    saveCfg();
    if (latest) render(latest);
  });
  on("s-activity", "change", (el) => {
    cfg.show_activity = (el as HTMLInputElement).checked;
    saveCfg();
    applyActivityVisibility();
  });
  on("s-statusline", "change", async (el) => {
    const box = el as HTMLInputElement;
    const enabled = box.checked;
    const msg = $("s-statusline-msg");
    try {
      await invoke("set_statusline_optin", { enabled });
      cfg.statusline_optin = enabled;
      msg.hidden = true;
    } catch (e) {
      // Revert the checkbox and surface the reason (e.g. existing statusLine).
      box.checked = !enabled;
      msg.textContent = "⚠ " + String(e);
      msg.hidden = false;
    }
  });
}

type Level = "ok" | "warn" | "crit";

function levelOf(u: number | null | undefined): Level {
  if (u == null) return "ok";
  if (u >= cfg.crit_threshold) return "crit";
  if (u >= cfg.warn_threshold) return "warn";
  return "ok";
}

function setLvl(el: HTMLElement, lvl: Level) {
  el.classList.remove("warn", "crit");
  if (lvl !== "ok") el.classList.add(lvl);
}

function worse(a: Level, b: Level): Level {
  const rank = { ok: 0, warn: 1, crit: 2 } as const;
  return rank[a] >= rank[b] ? a : b;
}

function render(s: Snapshot) {
  const stale = !!s.error && /401|unauthorized/i.test(s.error);
  document.body.classList.toggle("stale", stale);
  staleNow = stale;

  const five = s.quota.five_hour;
  const seven = s.quota.seven_day;

  // Independent per-window threshold colors (current vs weekly).
  const fiveLvl = levelOf(five?.utilization);
  const sevenLvl = levelOf(seven?.utilization);
  setLvl($("compact"), fiveLvl); // pill shows the 5h window
  setLvl($("d-five-bar"), fiveLvl);
  setLvl($("d-five-pct"), fiveLvl);
  setLvl($("d-seven-bar"), sevenLvl);
  setLvl($("d-seven-pct"), sevenLvl);

  // Prominent alert pulse on the worst window — gated by the toggle, off when stale.
  document.body.classList.remove("alert-warn", "alert-crit");
  const maxLvl = worse(fiveLvl, sevenLvl);
  if (!stale && cfg.alert_effects && maxLvl !== "ok") {
    document.body.classList.add("alert-" + maxLvl);
  }

  // compact
  $("c-five").textContent = five ? `${five.utilization.toFixed(0)}%` : "—";
  $("c-reset").textContent = "⏱ " + fmtCountdown(five?.resets_at ?? null);

  // detailed
  $("d-five-pct").textContent = five ? `${five.utilization.toFixed(0)}%` : "—";
  ($("d-five-bar") as HTMLElement).style.width = `${five ? Math.min(100, five.utilization) : 0}%`;
  $("d-five-reset").textContent = "重置 " + fmtCountdown(five?.resets_at ?? null);

  $("d-seven-pct").textContent = seven ? `${seven.utilization.toFixed(0)}%` : "—";
  ($("d-seven-bar") as HTMLElement).style.width = `${seven ? Math.min(100, seven.utilization) : 0}%`;
  $("d-seven-reset").textContent = "重置 " + fmtCountdown(seven?.resets_at ?? null);

  // Headline = actual conversation tokens (input+output); cache reads/writes
  // inflate the raw total, so keep them in the hover breakdown instead.
  const io = s.today.input + s.today.output;
  const todayEl = $("d-today");
  todayEl.textContent = `今日 ${fmtTokens(io)} tok`;
  todayEl.title =
    `輸入 ${fmtTokens(s.today.input)} · 輸出 ${fmtTokens(s.today.output)}\n` +
    `快取寫 ${fmtTokens(s.today.cache_write)} · 快取讀 ${fmtTokens(s.today.cache_read)}\n` +
    `總計 ${fmtTokens(s.today.total)} tok`;
  $("d-cost").textContent = `~$${s.today.cost_usd.toFixed(2)}`;

  // subscription renewal countdown (computed from the user-set billing day)
  const renewEl = $("d-renew");
  const r = nextRenewal(cfg?.renewal_day ?? 0);
  if (r) {
    const today0 = new Date();
    today0.setHours(0, 0, 0, 0);
    const days = Math.round((r.getTime() - today0.getTime()) / 86400000);
    renewEl.innerHTML = `訂閱續訂 <b>${r.getMonth() + 1}/${r.getDate()}</b> · ${days}天後`;
    renewEl.hidden = false;
  } else {
    renewEl.hidden = true;
  }

  const err = $("d-error");
  if (s.error) {
    err.textContent = "⚠ " + s.error;
    err.hidden = false;
  } else {
    err.hidden = true;
  }

  fitWindow();
}

function drawSpark(svg: SVGElement, w: number, data: number[]) {
  const max = Math.max(...data, 1);
  const n = data.length;
  if (n < 2) {
    svg.innerHTML = "";
    return;
  }
  const step = w / (n - 1);
  const line = data
    .map((v, i) => `${i ? "L" : "M"}${(i * step).toFixed(1)},${(26 - (v / max) * 24).toFixed(1)}`)
    .join(" ");
  const area = `${line} L${w},26 L0,26 Z`;
  svg.innerHTML =
    `<defs><linearGradient id="lg" x1="0" x2="0" y1="0" y2="1">` +
    `<stop offset="0" stop-color="#3ddc84" stop-opacity=".35"/>` +
    `<stop offset="1" stop-color="#3ddc84" stop-opacity="0"/></linearGradient></defs>` +
    `<path d="${area}" fill="url(#lg)"/>` +
    `<path d="${line}" fill="none" stroke="#3ddc84" stroke-width="1.6" stroke-linejoin="round"/>`;
}

function idleAgo(secs: number): string {
  if (secs <= 0) return "—";
  const m = Math.round(secs / 60);
  return m < 1 ? "剛剛" : `${m} 分鐘前`;
}

function renderActivity(a: LiveActivity) {
  const block = $("live-block");
  const dot = $("c-livedot");
  const rate = $("c-liverate");

  // Master toggle off: hide everything related.
  if (!cfg.show_activity) {
    block.hidden = true;
    dot.hidden = true;
    rate.hidden = true;
    fitWindow(); // card shrank — refit the window
    return;
  }

  // ----- A/B: detailed live-block -----
  block.hidden = false;
  block.classList.toggle("idle", !a.active);
  if (a.active) {
    $("la-state").textContent = "活動中";
    $("la-proj").textContent = a.source === "statusline" ? "· session 進行中" : "";
    $("la-rate").textContent = fmtRate(a.burn_tpm);
    $("la-empty").textContent = fmtMinsToEmpty(a.mins_to_empty, a.beats_reset);
    $("la-empty").hidden = $("la-empty").textContent === "";
    $("la-sess").textContent = `本次 session ${fmtTokens(a.session_tokens)} tok`;
    drawSpark($("la-spark") as unknown as SVGElement, 240, a.spark);
  } else {
    $("la-state").textContent = "💤 無活動 session";
    $("la-proj").textContent = "";
    $("la-sess").textContent = `最後活動 ${idleAgo(a.last_active_secs)}`;
  }

  // ----- C: compact pill indicator (auto-hidden by .live when stale) -----
  dot.hidden = !a.active;
  rate.hidden = !a.active;
  if (a.active) rate.textContent = fmtRate(a.burn_tpm) + "/m";

  // ----- D: standalone burn card -----
  if (staleNow) {
    $("act-state").textContent = "❄ token 已過期";
    $("act-rate").textContent = "—";
    $("act-empty").textContent = "請開啟 Claude Code 重新登入";
    ($("act-spark") as unknown as SVGElement).innerHTML = "";
  } else if (a.active) {
    $("act-state").textContent = "活動中";
    $("act-rate").textContent = fmtRate(a.burn_tpm);
    $("act-empty").textContent = fmtMinsToEmpty(a.mins_to_empty, a.beats_reset);
    drawSpark($("act-spark") as unknown as SVGElement, 180, a.spark);
  } else {
    $("act-state").textContent = "💤 無活動 session";
    $("act-rate").textContent = "0";
    $("act-empty").textContent = `最後活動 ${idleAgo(a.last_active_secs)}`;
    ($("act-spark") as unknown as SVGElement).innerHTML = "";
  }

  // the detailed live-block height / pill width change between idle/active — refit
  fitWindow();
}

// Reflect show_activity changes immediately (hide block, leave burn mode).
function applyActivityVisibility() {
  const opt = document.querySelector('#s-mode option[value="activity"]') as HTMLOptionElement | null;
  if (opt) opt.hidden = !cfg.show_activity;
  $("btn-activity").hidden = !cfg.show_activity;
  if (!cfg.show_activity && cfg.mode === "activity") {
    cfg.mode = "detailed";
    setMode("detailed");
    invoke("set_mode", { mode: "detailed" });
  }
  if (latestActivity) renderActivity(latestActivity);
}

function tick() {
  // refresh only the live countdowns each second; data comes from polls
  if (!latest) return;
  $("c-reset").textContent = "⏱ " + fmtCountdown(latest.quota.five_hour?.resets_at ?? null);
  $("d-five-reset").textContent = "重置 " + fmtCountdown(latest.quota.five_hour?.resets_at ?? null);
  $("d-seven-reset").textContent = "重置 " + fmtCountdown(latest.quota.seven_day?.resets_at ?? null);
}

window.addEventListener("DOMContentLoaded", async () => {
  cfg = await invoke<Config>("get_config");
  setMode(cfg.mode);
  applyOpacity(cfg.opacity);
  wireSettings();

  // compact pill click -> expand
  $("compact").addEventListener("click", () => {
    setMode("detailed");
    invoke("set_mode", { mode: "detailed" }).then(fitWindow);
  });
  $("btn-collapse").addEventListener("click", (e) => {
    e.stopPropagation();
    setMode("compact");
    invoke("set_mode", { mode: "compact" }).then(fitWindow);
  });
  $("btn-hide").addEventListener("click", (e) => {
    e.stopPropagation();
    invoke("hide_window");
  });
  $("btn-activity").addEventListener("click", (e) => {
    e.stopPropagation();
    setMode("activity");
    invoke("set_mode", { mode: "activity" });
  });
  $("btn-act-back").addEventListener("click", (e) => {
    e.stopPropagation();
    setMode("detailed");
    invoke("set_mode", { mode: "detailed" }).then(fitWindow);
  });
  $("btn-act-hide").addEventListener("click", (e) => {
    e.stopPropagation();
    invoke("hide_window");
  });

  await listen<Snapshot>("usage-update", (ev) => {
    latest = ev.payload;
    render(latest);
  });

  await listen<LiveActivity>("activity-update", (ev) => {
    latestActivity = ev.payload;
    renderActivity(latestActivity);
  });

  await listen("go-settings", () => openSettings());

  // show whatever we have, then force a fresh poll
  const snap = await invoke<Snapshot>("get_snapshot");
  latest = snap;
  render(snap);
  invoke("refresh_now");
  applyActivityVisibility();
  invoke<LiveActivity>("get_activity").then((a) => {
    latestActivity = a;
    renderActivity(a);
  });

  // re-fit once layout/fonts have settled (first measure can be slightly short)
  requestAnimationFrame(() => {
    lastFitH = 0;
    lastFitW = 0;
    fitWindow();
  });

  setInterval(tick, 1000);
});
