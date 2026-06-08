# Frozen-Card Instant Refresh + Thaw Effect Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When the OAuth token expires (frozen state), give the user a button to recover immediately instead of waiting up to 180s for the next poll, an always-visible note that recovery is automatic, and a one-shot "melt" animation when the widget thaws.

**Architecture:** Pure helpers (`isStale`, `frozenHintText`) move into `format.ts` and get unit-tested. The frozen card (`index.html`) gains a button + hint line. `main.ts` `render()` detects the stale→live transition to fire a CSS-only thaw animation, and wires the button to the existing `refresh_now` Tauri command. No Rust changes — `refresh_now` already triggers an immediate poll.

**Tech Stack:** Tauri v2, vanilla TypeScript, Vite, Vitest (front-end unit tests), plain CSS.

Spec: `docs/superpowers/specs/2026-06-08-frozen-refresh-design.md`

---

## File Structure

- `src/format.ts` — add two pure helpers (`isStale`, `frozenHintText`). Keeps testable logic out of the DOM-heavy `main.ts`, matching the existing pattern.
- `src/format.test.ts` — unit tests for the two new helpers.
- `index.html` — add button + hint inside the existing `.card-frozen` block.
- `src/styles.css` — styles for the button/hint, plus the thaw overlay + keyframes.
- `src/main.ts` — import the helpers, use them in `render()`, detect the thaw transition, add the button click handler and a `frozenRefreshing` flag.

---

## Task 1: Pure helpers `isStale` + `frozenHintText` (format.ts)

**Files:**
- Modify: `src/format.ts`
- Test: `src/format.test.ts`

- [ ] **Step 1: Write the failing tests**

Append to `src/format.test.ts`:

```ts
import { isStale, frozenHintText } from "./format";

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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npm test`
Expected: FAIL — `isStale`/`frozenHintText` are not exported from `./format`.

- [ ] **Step 3: Implement the helpers**

Append to `src/format.ts`:

```ts
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `npm test`
Expected: PASS (all suites, including the new ones).

- [ ] **Step 5: Commit**

```bash
git add src/format.ts src/format.test.ts
git commit -m "feat: add isStale + frozenHintText pure helpers"
```

---

## Task 2: Frozen-card button + hint markup (index.html)

**Files:**
- Modify: `index.html` (the `.card-frozen` block, currently lines 76-80)

- [ ] **Step 1: Add the button + hint**

Replace this block:

```html
      <div class="card-frozen">
        <div class="frozen-icon">❄</div>
        <div class="frozen-title">Token 已過期</div>
        <div class="frozen-sub">請開啟 Claude Code 重新登入<br>以恢復用量監控</div>
      </div>
```

with:

```html
      <div class="card-frozen">
        <div class="frozen-icon">❄</div>
        <div class="frozen-title">Token 已過期</div>
        <div class="frozen-sub">請開啟 Claude Code 重新登入<br>以恢復用量監控</div>
        <button id="btn-frozen-refresh" class="frozen-btn">↻ 已重新登入，立即恢復</button>
        <div id="frozen-hint" class="frozen-hint">自動每 ≤180 秒會重試一次</div>
      </div>
```

- [ ] **Step 2: Verify the build still compiles**

Run: `npm run build`
Expected: PASS (tsc + vite build succeed; no TS errors).

- [ ] **Step 3: Commit**

```bash
git add index.html
git commit -m "feat: add instant-refresh button + hint to frozen card"
```

---

## Task 3: Button/hint styles + thaw animation (styles.css)

**Files:**
- Modify: `src/styles.css` (frozen section starts at line 167; `.card-frozen` rules at 115-126)

- [ ] **Step 1: Style the button and hint**

After the existing `.card-frozen .frozen-sub { ... }` rule (line 126), add:

```css
.frozen-btn {
  margin-top: 6px;
  padding: 5px 12px;
  background: rgba(143, 196, 232, 0.14); /* --ice tint */
  border: 1px solid var(--ice);
  border-radius: 8px;
  color: var(--ice);
  font-size: 11px;
  font-weight: 700;
  cursor: pointer;
}
.frozen-btn:hover { background: rgba(143, 196, 232, 0.26); }
.frozen-btn:disabled { opacity: 0.55; cursor: default; }
.frozen-hint { font-size: 10px; color: var(--muted); line-height: 1.3; }
```

- [ ] **Step 2: Add the thaw overlay + keyframes**

At the end of the frozen section (after the `@keyframes frostPulse { ... }` block, which ends at line 179), add:

```css
/* thaw: one-shot melt when the token recovers (frozen -> live). Driven by the
   transient `.thawing` class added in main.ts. The overlay is absolutely
   positioned (out of flow, so it never affects fitWindow's measurement) and is
   clipped by the host's scoped overflow so it appears to drip off the bottom. */
#compact, #detailed { position: relative; }
#compact.thawing, #detailed.thawing { overflow: hidden; }
#compact.thawing::after,
#detailed.thawing::after {
  content: "❄";
  position: absolute;
  inset: 0;
  display: flex;
  align-items: flex-start;
  justify-content: center;
  padding-top: 6px;
  font-size: 26px;
  color: var(--ice);
  background: linear-gradient(180deg, rgba(143, 196, 232, 0.45), rgba(143, 196, 232, 0.12));
  border-radius: inherit;
  pointer-events: none;
  animation: thaw 1.1s ease-in forwards;
}
@keyframes thaw {
  0%   { transform: translateY(0);    opacity: 1; filter: brightness(1.4); } /* flash */
  15%  { transform: translateY(0);    opacity: 1; filter: brightness(1.7); } /* peak flash */
  100% { transform: translateY(100%); opacity: 0; filter: brightness(1);   } /* drip + dissolve */
}
```

- [ ] **Step 3: Verify the build still compiles**

Run: `npm run build`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/styles.css
git commit -m "feat: style frozen-refresh button/hint + thaw melt animation"
```

---

## Task 4: Wire helpers, thaw trigger, and button handler (main.ts)

**Files:**
- Modify: `src/main.ts` — import line (3), module state (~60), `render()` (266-269), `DOMContentLoaded` handler block (~464-477)

- [ ] **Step 1: Import the new helpers**

Change line 3 from:

```ts
import { fmtTokens, fmtCountdown, nextRenewal, fmtRate, fmtMinsToEmpty } from "./format";
```

to:

```ts
import { fmtTokens, fmtCountdown, nextRenewal, fmtRate, fmtMinsToEmpty, isStale, frozenHintText } from "./format";
```

- [ ] **Step 2: Add the `frozenRefreshing` flag and `playThaw` helper**

After the existing module state (the `let staleNow = false;` line, currently line 60), add:

```ts
let frozenRefreshing = false; // true between a manual frozen-card refresh click and the next render

// One-shot melt on the widget container when the token recovers. The .thawing
// class drives the CSS animation; remove it after the run so it can replay.
function playThaw() {
  for (const el of [$("compact"), $("detailed")]) {
    el.classList.remove("thawing");
    void el.offsetWidth; // force reflow so re-adding restarts the animation
    el.classList.add("thawing");
    setTimeout(() => el.classList.remove("thawing"), 1200);
  }
}
```

(`$` is defined just below at line 64; hoisted `function`/`let` are fine to reference it.)

- [ ] **Step 3: Use the helpers + detect the thaw transition in `render()`**

Replace these three lines (currently 267-269):

```ts
  const stale = !!s.error && /401|unauthorized/i.test(s.error);
  document.body.classList.toggle("stale", stale);
  staleNow = stale;
```

with:

```ts
  const stale = isStale(s.error);
  if (staleNow && !stale) playThaw(); // frozen -> live: play the melt before updating staleNow
  document.body.classList.toggle("stale", stale);

  // Reset the frozen-card button + hint each render.
  const frozenBtn = $("btn-frozen-refresh") as HTMLButtonElement;
  frozenBtn.disabled = false;
  frozenBtn.textContent = "↻ 已重新登入，立即恢復";
  $("frozen-hint").textContent = frozenHintText(stale, frozenRefreshing);
  frozenRefreshing = false; // consumed (recovery succeeded, or the "still stale" hint was shown once)

  staleNow = stale;
```

- [ ] **Step 4: Wire the button click handler**

In the `DOMContentLoaded` listener, after the existing `$("btn-hide")` handler block (currently ends line 463, before `$("btn-activity")` at 464), add:

```ts
  $("btn-frozen-refresh").addEventListener("click", (e) => {
    e.stopPropagation();
    frozenRefreshing = true;
    const btn = $("btn-frozen-refresh") as HTMLButtonElement;
    btn.disabled = true;
    btn.textContent = "重新整理中…";
    invoke("refresh_now");
  });
```

- [ ] **Step 5: Verify the build compiles and tests pass**

Run: `npm run build && npm test`
Expected: PASS — no TS errors; all unit tests green.

- [ ] **Step 6: Commit**

```bash
git add src/main.ts
git commit -m "feat: wire frozen-card refresh button + thaw transition"
```

---

## Task 5: Manual verification (run the app)

The button/CSS behavior is DOM/visual and not covered by unit tests — verify by driving the app.

**Files:** none (verification only)

- [ ] **Step 1: Launch the dev app**

Run: `npm run tauri dev`
Expected: widget opens; with a valid token it shows normal usage (not frozen).

- [ ] **Step 2: Force the frozen state**

Easiest path: temporarily invalidate the token so the poll returns 401 — set an obviously-bad token in the environment before launch:

Run: `CLAUDE_CODE_OAUTH_TOKEN=invalid npm run tauri dev`
Expected: after the first poll the detailed card shows the frozen card with: ❄ icon, "Token 已過期", the re-login text, the **↻ 已重新登入，立即恢復** button, and the hint **自動每 ≤180 秒會重試一次**.

- [ ] **Step 3: Verify the button "still stale" path**

Click the button while the token is still invalid.
Expected: button shows "重新整理中…" and is disabled; after the poll returns it re-enables, restores its label, and the hint changes to **仍未偵測到登入，請確認 Claude Code 已重新登入**.

- [ ] **Step 4: Verify recovery + thaw animation**

Relaunch normally (`npm run tauri dev`, valid token) but first reach the frozen state, then trigger a successful refresh (e.g. start from invalid, then with Claude Code logged in click the button, or restart with a valid token). On recovery:
Expected: the frozen card disappears, live data returns, and a one-shot **melt** plays on the container — an icy overlay flashes, then drips downward and dissolves over ~1.1s, leaving no residue.

- [ ] **Step 5: Verify all four themes**

In Settings, switch render style across 經典 / 奧術 HUD / 魔法羊皮紙 / 魔導霓虹 and re-trigger a thaw in each.
Expected: button/hint readable and the thaw animation plays without breaking the layout in every theme.

- [ ] **Step 6: Final full check**

Run: `npm run build && npm test`
Expected: build succeeds, all tests pass. Report results.

---

## Self-Review Notes

- **Spec coverage:** goal 1 (button) → Tasks 2/4; goal 2 (static hint + failure hint) → Tasks 1/2/4; goal 3 (thaw melt: drip + dissolve + flash) → Tasks 3/4. Scope guards (no compact-pill button, no tray change, no Rust change) respected — no tasks touch them.
- **Type consistency:** helper names `isStale`/`frozenHintText`, element ids `btn-frozen-refresh`/`frozen-hint`, classes `frozen-btn`/`frozen-hint`/`thawing`, flag `frozenRefreshing`, and helper `playThaw` are used identically across all tasks.
- **No placeholders:** every code/CSS/command step shows the actual content.
