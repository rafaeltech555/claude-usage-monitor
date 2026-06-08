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

// tokens/min, abbreviated like fmtTokens but rounded for small values.
export function fmtRate(tpm: number): string {
  if (tpm >= 1000) return (tpm / 1000).toFixed(1) + "k";
  return String(Math.round(tpm));
}

// "5h empties in N" line. beatsReset wins; null/≤0 -> "" (caller hides it).
export function fmtMinsToEmpty(mins: number | null, beatsReset: boolean): string {
  if (beatsReset) return "✓ 重置前不會見底";
  if (mins == null || !isFinite(mins) || mins <= 0) return "";
  if (mins >= 60) {
    // round to whole minutes first, then split, so a 59.5 remainder that rounds
    // to 60 carries into the hour instead of showing "1時60分".
    const total = Math.round(mins);
    return `≈ ${Math.floor(total / 60)}時${total % 60}分見底`;
  }
  return `≈ ${Math.round(mins)} 分見底`;
}

// Stale = the quota poll failed with an auth error (token expired). Same rule
// used by the frozen UI; centralized here so it is unit-tested.
export function isStale(error: string | null | undefined): boolean {
  return !!error && /401|unauthorized/i.test(error);
}

// Hint under the frozen card. The "still expired" message shows only right
// after a manual refresh that did not recover; otherwise the static note that
// recovery is automatic (so a waiting user does not think it is a bug).
export function frozenHintText(stillStale: boolean, manualRefresh: boolean): string {
  if (stillStale && manualRefresh) return "仍未偵測到登入，請確認 Claude Code 已重新登入";
  return "自動每 ≤180 秒會重試一次";
}
