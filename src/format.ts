// Pure formatting/date helpers (no Tauri imports) so they can be unit-tested.

export function fmtTokens(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "k";
  return String(n);
}

export function fmtCountdown(resetsAt: string | null, nowMs: number = Date.now()): string {
  if (!resetsAt) return "—";
  const ms = new Date(resetsAt).getTime() - nowMs;
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

// Next occurrence of a monthly billing day-of-month (clamped to month length).
export function nextRenewal(day: number, now: Date = new Date()): Date | null {
  if (!day || day < 1 || day > 31) return null;
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
