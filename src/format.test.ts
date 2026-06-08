import { describe, it, expect } from "vitest";
import { fmtTokens, fmtCountdown, nextRenewal } from "./format";
import { fmtRate, fmtMinsToEmpty } from "./format";
import { isStale, frozenHintText } from "./format";

describe("fmtTokens", () => {
  it("formats millions/thousands/units", () => {
    expect(fmtTokens(2_400_000)).toBe("2.4M");
    expect(fmtTokens(12_300)).toBe("12.3k");
    expect(fmtTokens(950)).toBe("950");
  });
});

describe("fmtCountdown", () => {
  const now = Date.parse("2026-06-06T00:00:00Z");
  it("handles null / past / future", () => {
    expect(fmtCountdown(null, now)).toBe("—");
    expect(fmtCountdown("2026-06-05T00:00:00Z", now)).toBe("已重置");
    expect(fmtCountdown("2026-06-06T02:30:00Z", now)).toBe("2時30分");
    expect(fmtCountdown("2026-06-08T01:00:00Z", now)).toBe("2天1時");
    expect(fmtCountdown("2026-06-06T00:45:00Z", now)).toBe("45分");
  });
});

describe("nextRenewal", () => {
  it("returns null for unset / invalid days", () => {
    expect(nextRenewal(0)).toBeNull();
    expect(nextRenewal(32)).toBeNull();
  });

  it("picks this month if the day is still ahead", () => {
    const now = new Date(2026, 5, 6); // Jun 6, 2026
    const r = nextRenewal(11, now)!;
    expect(r.getMonth()).toBe(5); // June
    expect(r.getDate()).toBe(11);
  });

  it("rolls to next month once the day has passed", () => {
    const now = new Date(2026, 5, 20); // Jun 20
    const r = nextRenewal(11, now)!;
    expect(r.getMonth()).toBe(6); // July
    expect(r.getDate()).toBe(11);
  });

  it("includes today when the day matches", () => {
    const now = new Date(2026, 5, 11);
    const r = nextRenewal(11, now)!;
    expect(r.getMonth()).toBe(5);
    expect(r.getDate()).toBe(11);
  });

  it("clamps to month length (day 31 in a 30-day month)", () => {
    const now = new Date(2026, 8, 15); // Sep 15 (Sep has 30 days)
    const r = nextRenewal(31, now)!;
    expect(r.getMonth()).toBe(8); // September
    expect(r.getDate()).toBe(30); // clamped
  });
});

describe("fmtRate", () => {
  it("abbreviates tok/min", () => {
    expect(fmtRate(12400)).toBe("12.4k");
    expect(fmtRate(940)).toBe("940");
    expect(fmtRate(0)).toBe("0");
  });
});

describe("fmtMinsToEmpty", () => {
  it("prefers the beats-reset message", () => {
    expect(fmtMinsToEmpty(120, true)).toBe("✓ 重置前不會見底");
  });
  it("formats minutes and hours with the ≈ marker", () => {
    expect(fmtMinsToEmpty(25, false)).toBe("≈ 25 分見底");
    expect(fmtMinsToEmpty(95, false)).toBe("≈ 1時35分見底");
    expect(fmtMinsToEmpty(119.6, false)).toBe("≈ 2時0分見底"); // carries, no "1時60分"
  });
  it("returns empty for unknown / non-positive", () => {
    expect(fmtMinsToEmpty(null, false)).toBe("");
    expect(fmtMinsToEmpty(0, false)).toBe("");
  });
});

describe("isStale", () => {
  it("is true for 401 / unauthorized errors, false otherwise", () => {
    expect(isStale("unauthorized (401) — token expired? open Claude Code to refresh")).toBe(true);
    expect(isStale("HTTP 401")).toBe(true);
    expect(isStale("Unauthorized")).toBe(true);
    expect(isStale("network timeout")).toBe(false);
    expect(isStale(null)).toBe(false);
    expect(isStale(undefined)).toBe(false);
  });
});

describe("frozenHintText", () => {
  it("shows the retry-failed hint only after a manual refresh that is still stale", () => {
    expect(frozenHintText(true, true)).toBe("仍未偵測到登入，請確認 Claude Code 已重新登入");
  });
  it("otherwise shows the static auto-retry hint", () => {
    expect(frozenHintText(true, false)).toBe("自動每 ≤180 秒會重試一次");
    expect(frozenHintText(false, true)).toBe("自動每 ≤180 秒會重試一次");
    expect(frozenHintText(false, false)).toBe("自動每 ≤180 秒會重試一次");
  });
});
