// Building blocks the scenarios share.
import { join } from "node:path";
import { similarity } from "./similarity.mjs";

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** CDP screenshot of an app page into <run>/shots/<name>.png; returns the relative path. */
export async function pageShot(ctx, page, name) {
  const rel = `shots/${name}.png`;
  try {
    await page.shot(join(ctx.dir, rel));
    return rel;
  } catch (e) {
    ctx.log(`screenshot ${name} failed: ${e.message}`);
    return null;
  }
}

/** Screen grab of one window's rectangle (agent), copied back; returns the relative path. */
export async function windowShot(ctx, hwnd, name) {
  const remote = ctx.remote.winPath(`shots/${name}.png`);
  try {
    await ctx.agent.call("shot", { hwnd, path: remote });
    ctx.remote.scpFrom(`shots/${name}.png`, join(ctx.dir, "shots", `${name}.png`));
    return `shots/${name}.png`;
  } catch (e) {
    ctx.log(`window shot ${name} failed: ${e.message}`);
    return null;
  }
}

/** First-run setup as the settings page shows it: open or not, and its step. */
export function onboardingDom(page) {
  return page.eval(`(() => {
    const ob = document.getElementById("onboarding");
    const step = ob && [...ob.querySelectorAll("[data-ob-step]")].find((p) => !p.hidden);
    return {
      onboarding_present: !!ob,
      onboarding_open: !!ob && !ob.hidden,
      step: step ? step.dataset.obStep : null,
      visibility: document.visibilityState,
      title: document.title,
    };
  })()`);
}

/** Visible top-level windows of the app. */
export async function appWindows(ctx) {
  const r = await ctx.agent.call("windows", { process: "murmur-windows" });
  return r.windows.filter((w) => w.w > 50 && w.h > 50 && w.class !== "Tao Thread Event Target");
}

/**
 * A fresh text window of ours (ps/pad.ps1), focused. Not Notepad: Windows 11
 * Notepad opens files as tabs in the owner's own Notepad window.
 */
export async function openPad(ctx, name) {
  const { win } = await ctx.agent.call("pad_open", { name }, 30_000);
  const pad = { win, focused: false };
  await focusPad(ctx, pad);
  return pad;
}

/**
 * Bring the pad to the foreground; false (and logged) if it would not come.
 * Callers must not press anything then: the keys would go to another window.
 */
export async function focusPad(ctx, pad) {
  const f = await ctx.agent.call("focus", { hwnd: pad.win.hwnd });
  pad.focused = f.focused;
  if (!f.focused) ctx.log(`pad not in the foreground (fg=${f.fg?.process}): not pressing anything`);
  return f.focused;
}

/** Read the pad's text (UIA), then close it (nothing is saved, no keys sent). */
export async function readAndClosePad(ctx, pad) {
  let text = null;
  let via = null;
  try {
    const r = await ctx.agent.call("uia_text", { hwnd: pad.win.hwnd });
    text = r.text;
    via = `UIA ${r.via} on ${r.control}`;
  } catch (e) {
    ctx.log(`UIA read failed: ${e.message}`);
  }
  try {
    await ctx.agent.call("pad_close", { hwnd: pad.win.hwnd });
  } catch (e) {
    ctx.log(`closing pad: ${e.message}`);
  }
  return { text, via };
}

/** Loose comparison verdict for a dictated clip. */
export function judge(expect, got, pass = 0.6) {
  const s = similarity(expect, got ?? "");
  return { ...s, pass: Math.max(s.score, s.recall) >= pass && s.recall >= pass * 0.8 };
}

/** New debug-log lines since `since` (ISO ts), parsed. */
export function debugLines(ctx, since) {
  const r = ctx.remote.ssh(`type %APPDATA%\\Murmur\\debug\\dictations.jsonl`, { check: false });
  if (r.code !== 0) return [];
  return r.out
    .split(/\r?\n/)
    .filter((l) => l.startsWith("{"))
    .map((l) => {
      try {
        return JSON.parse(l);
      } catch {
        return null;
      }
    })
    .filter((x) => x && (!since || x.ts >= since));
}

/** Keys from the debug-log line worth keeping as evidence (no secrets there: the app scrubs them). */
export function pickLine(l) {
  if (!l) return null;
  const keys = ["ts", "kind", "event", "mode", "route", "fallback", "language", "language_source", "start_language", "stop_language", "stt_model", "audio_ms", "perceived_ms", "target", "insert", "raw", "text", "error"];
  return Object.fromEntries(keys.filter((k) => l[k] !== undefined).map((k) => [k, l[k]]));
}

/**
 * Push-to-talk one clip: hold `keys`, play the clip into the cable after
 * `leadMs`, keep holding `tailMs`, release. `guard` is the hwnd that must be
 * in the foreground: the agent presses nothing if it is not, and if it lost
 * the foreground by the release, Esc cancels the dictation. Resolves with
 * {aborted, aborted_at?, fg_process?, held_ms}.
 */
export function talk(ctx, clipId, { guard, keys = ["RAlt"], leadMs = 400, tailMs = 700 }) {
  if (!guard) throw new Error("talk: a guard window is required");
  const clip = ctx.clips[clipId];
  return ctx.agent.call("talk", { keys, wav: clip.remote, device: ctx.cableIn, leadMs, tailMs, guard }, 60_000);
}

/**
 * Dictate one clip into a fresh pad with push-to-talk: open the pad,
 * hold `keys` over the clip, wait for the text to settle, shoot the window,
 * read it back (UIA), close it. Returns the verdict plus the new debug-log
 * dictation line.
 */
export async function dictateIntoPad(ctx, clipId, name, { keys = ["RAlt"], settleMs = 25_000 } = {}) {
  const clip = ctx.clips[clipId];
  const logBefore = debugLines(ctx).length;
  const np = await openPad(ctx, name);
  const said = np.focused ? await talk(ctx, clipId, { keys, guard: np.win.hwnd }) : { aborted: true, aborted_at: "before the press (pad never in the foreground)" };
  if (said.aborted) {
    ctx.log(`${name}: try aborted ${said.aborted_at} (foreground: ${said.fg_process ?? "?"})`);
    await readAndClosePad(ctx, np);
    return { clip: clipId, expect: clip.expect, got: null, aborted: true, aborted_at: said.aborted_at, verdict: { ...judge(clip.expect, ""), pass: false }, debug: null };
  }
  // Wait until the pad has text and it stopped changing.
  let last = null;
  let stableSince = 0;
  const end = Date.now() + settleMs;
  while (Date.now() < end) {
    await sleep(700);
    const t = (await ctx.agent.call("uia_text", { hwnd: np.win.hwnd }).catch(() => ({ text: "" }))).text?.trim() ?? "";
    if (t && t === last) {
      if (Date.now() - stableSince > 1500) break;
    } else {
      last = t;
      stableSince = Date.now();
    }
  }
  const shot = await windowShot(ctx, np.win.hwnd, name);
  const { text, via } = await readAndClosePad(ctx, np);
  await sleep(500);
  const line = debugLines(ctx)
    .slice(logBefore)
    .filter((l) => l.kind === "dictation")
    .at(-1);
  return { clip: clipId, expect: clip.expect, got: text, via, pad_foreground: np.focused, held_ms: said.held_ms, verdict: judge(clip.expect, text), shot, debug: pickLine(line) };
}
