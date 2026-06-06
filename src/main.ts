import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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
  renewal_day: number;
};

let latest: Snapshot | null = null;
let cfg: Config;

// ---- formatting helpers ----
function fmtTokens(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "k";
  return String(n);
}

function fmtCountdown(resetsAt: string | null): string {
  if (!resetsAt) return "—";
  const ms = new Date(resetsAt).getTime() - Date.now();
  if (isNaN(ms)) return "—";
  if (ms <= 0) return "已重置";
  const totalMin = Math.floor(ms / 60000);
  const d = Math.floor(totalMin / 1440);
  const h = Math.floor((totalMin % 1440) / 60);
  const m = totalMin % 60;
  if (d > 0) return `${d}天${h}時`;
  if (h > 0) return `${h}時${m}分`;
  return `${m}分`;
}

function $(id: string): HTMLElement {
  return document.getElementById(id)!;
}

// Next occurrence of a monthly billing day-of-month (clamped to month length).
function nextRenewal(day: number): Date | null {
  if (!day || day < 1 || day > 31) return null;
  const now = new Date();
  const today0 = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  let y = now.getFullYear();
  let m = now.getMonth();
  for (let i = 0; i < 14; i++) {
    const dim = new Date(y, m + 1, 0).getDate();
    const cand = new Date(y, m, Math.min(day, dim));
    if (cand >= today0) return cand;
    m++;
    if (m > 11) {
      m = 0;
      y++;
    }
  }
  return null;
}

function setMode(mode: string) {
  document.body.classList.remove("mode-compact", "mode-detailed", "mode-settings");
  document.body.classList.add(
    mode === "detailed" ? "mode-detailed" : mode === "settings" ? "mode-settings" : "mode-compact",
  );
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

function render(s: Snapshot) {
  document.body.classList.remove("level-ok", "level-warn", "level-crit");
  document.body.classList.add("level-" + s.status_level);

  const five = s.quota.five_hour;
  const seven = s.quota.seven_day;

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
    invoke("set_mode", { mode: "detailed" });
  });
  $("btn-collapse").addEventListener("click", (e) => {
    e.stopPropagation();
    setMode("compact");
    invoke("set_mode", { mode: "compact" });
  });
  $("btn-hide").addEventListener("click", (e) => {
    e.stopPropagation();
    invoke("hide_window");
  });

  await listen<Snapshot>("usage-update", (ev) => {
    latest = ev.payload;
    render(latest);
  });

  await listen("go-settings", () => openSettings());

  // show whatever we have, then force a fresh poll
  const snap = await invoke<Snapshot>("get_snapshot");
  latest = snap;
  render(snap);
  invoke("refresh_now");

  setInterval(tick, 1000);
});
