// (d) Double-tap the push-to-talk key to lock hands-free, over every
// combination of:
//   profile  quiet = the seed (no sounds, no ducking, VB-Cable mic);
//            owner = the owner's own sounds / ducking / mic settings, with
//            the default speakers and mic MUTED for the duration (the app
//            does the same work, nobody hears or is recorded)
//   key      RAlt / LAlt (only the one bound to push-to-talk is graded)
//   variant  plain, or (--altgr) Right Alt as an AltGr layout sends it:
//            fake Left Ctrl (scan 0x21D) + Right Alt
//   tap      how long each tap is held (--tap-ms)
//   gap      release to second press (--gaps)
// A try is only pressed while our pad is in the foreground (the agent checks
// before each press and before the last release); one that loses it is
// cancelled with Esc and recorded as aborted, not graded.
// Every tap and gap gets +-jitter. Tries go round-robin over the
// combinations, so drift on the laptop hits them alike. A try locks if the
// overlay pill reaches "active_locked" within 1.5 s; Esc then cancels.
// Every try is graded, slow ones too: the measured press-to-press time says
// whether it was inside the app's 500 ms window.
// Pass = each bound combination whose nominal press-to-press (tap + gap)
// is at most 450 ms locks at least 90% of the time.
import { OVERLAY_RECORDER } from "../lib/app.mjs";
import { Remote } from "../lib/remote.mjs";
import { focusPad, openPad, readAndClosePad, sleep, windowShot } from "../lib/steps.mjs";

const WINDOW_MS = 500; // app: second press within 500 ms of the first
const NOMINAL_LIMIT_MS = 450; // graded combinations: tap + gap at most this
const PTT_NAMES = { RAlt: "Right Alt", LAlt: "Left Alt" };

const jitter = (ms, j) => Math.max(20, Math.round(ms + (Math.random() * 2 - 1) * j));

/** Change prefs inside the page (the full prefs object never leaves it). */
async function setPrefs(app, patch) {
  const s = await app.settings();
  return s.eval(`(async () => {
    const inv = window.__TAURI__.core.invoke;
    const p = await inv("get_prefs");
    Object.assign(p, ${JSON.stringify(patch)});
    await inv("set_prefs", { prefs: p });
    return true;
  })()`);
}

function ownerPrefs(ctx) {
  const out = ctx.remote.ps("appdata.ps1", ["-Action", "ownerprefs", "-Keys", "enable_sounds,mute_audio_while_dictating,microphone,shortcuts"]).out;
  return JSON.parse(Remote.kv(out).ownerprefs || "{}");
}

export default {
  id: "d",
  title: "Double-Alt hands-free",
  needs: "seeded",
  async run(ctx) {
    const { app, agent, args, log, remote } = ctx;
    const owner = ownerPrefs(ctx);
    const ptt = (await app.invoke("get_prefs")).shortcuts?.ptt ?? owner.shortcuts?.ptt ?? "Right Alt";
    const profiles = {
      quiet: { enable_sounds: false, mute_audio_while_dictating: false, microphone: ctx.cableMic },
      owner: { enable_sounds: owner.enable_sounds ?? true, mute_audio_while_dictating: owner.mute_audio_while_dictating ?? true, microphone: owner.microphone ?? null },
    };
    const variants = [];
    for (const key of args.keys) {
      variants.push({ key, altgr: false, chord: [key] });
      if (args.altgr && key === "RAlt") variants.push({ key, altgr: true, chord: ["AltGrCtrl", "RAlt"] });
    }
    const combos = [];
    for (const v of variants) for (const tap of args.taps) for (const gap of args.gaps) combos.push({ ...v, tap, gap });

    const ov = await app.overlay();
    await ov.eval(OVERLAY_RECORDER);
    const np = await openPad(ctx, "d-double");
    const evidence = [];
    const details = { ptt, jitter_ms: args.jitter, layout: null, profiles: {}, tries: [] };
    let layoutBefore = null;
    if (args.layout === "he") {
      layoutBefore = (await agent.call("layout", { hwnd: np.win.hwnd })).layout;
      details.layout = (await agent.call("layout", { hwnd: np.win.hwnd, set: "0000040D" })).layout;
      log(`pad layout ${layoutBefore} -> ${details.layout}`);
    }
    let shotTaken = false;
    try {
      for (const profile of args.profiles) {
        details.profiles[profile] = { sounds: profiles[profile].enable_sounds, ducking: profiles[profile].mute_audio_while_dictating, mic: profiles[profile].microphone ? (profile === "quiet" ? "VB-Cable" : "owner's choice") : "system default" };
        if (profile === "owner") log(remote.ps("audio.ps1", ["-Action", "mute"]).out.trim());
        await setPrefs(app, profiles[profile]);
        await sleep(1000);
        try {
          for (let i = 0; i < args.tries; i++) {
            for (const c of combos) {
              const tap1 = jitter(c.tap, args.jitter);
              const tap2 = jitter(c.tap, args.jitter);
              const gap = jitter(c.gap, args.jitter);
              const base = { profile, key: c.key, altgr: c.altgr, tap: c.tap, gap: c.gap };
              if (!(await focusPad(ctx, np))) {
                details.tries.push({ ...base, aborted: true, aborted_at: "before the first press (pad not in the foreground)" });
                await sleep(400);
                continue;
              }
              await ov.eval("window.__e2e.states.length = 0");
              const t = await agent.call("double", { keys: c.chord, tapMs: tap1, gapMs: gap, tap2Ms: tap2, guard: np.win.hwnd });
              if (t.aborted) {
                log(`d: try aborted ${t.aborted_at} (foreground: ${t.fg_process ?? "?"})`);
                await ov.waitFor(`document.getElementById("pill").dataset.state === "resting"`, 6000).catch(() => log("pill did not return to resting"));
                details.tries.push({ ...base, aborted: true, aborted_at: t.aborted_at });
                await sleep(400);
                continue;
              }
              let states = [];
              const end = Date.now() + 1500;
              while (Date.now() < end) {
                await sleep(100);
                states = await ov.eval("window.__e2e.states.map((x) => x.s)");
                if (states.includes("active_locked")) break;
              }
              const locked = states.includes("active_locked");
              if (locked && !shotTaken) {
                const pill = (await agent.call("windows", { process: "murmur-windows" })).windows.find((w) => w.title !== "Murmur" && w.w > 50 && w.h > 20 && w.class !== "Tao Thread Event Target");
                if (pill) evidence.push(`locked pill: ${await windowShot(ctx, pill.hwnd, "d-locked-pill")}`);
                shotTaken = true;
              }
              // Cancel whatever started (only into our pad); wait for the pill to rest.
              if (await focusPad(ctx, np)) await agent.call("hold", { keys: ["Esc"], ms: 40 });
              await ov.waitFor(`document.getElementById("pill").dataset.state === "resting"`, 6000).catch(() => log("pill did not return to resting"));
              await sleep(400);
              const p2p = Math.round(t.tap1_ms + t.gap_ms);
              details.tries.push({ ...base, aborted: false, locked, tap1_ms: Math.round(t.tap1_ms), gap_ms: Math.round(t.gap_ms), tap2_ms: Math.round(t.tap2_ms), press_to_press_ms: p2p, in_window: p2p < WINDOW_MS, states: states.join(">") });
            }
          }
        } finally {
          if (profile === "owner") log(remote.ps("audio.ps1", ["-Action", "restore", "-Keep", "-File", ctx.audioDefaults]).out.trim());
        }
      }
    } finally {
      if (layoutBefore) await agent.call("layout", { hwnd: np.win.hwnd, set: layoutBefore }).catch(() => {});
      await setPrefs(app, profiles.quiet).catch(() => {});
      await readAndClosePad(ctx, np);
    }

    // One row per combination.
    const rows = [];
    for (const profile of args.profiles) {
      for (const c of combos) {
        const all = details.tries.filter((x) => x.profile === profile && x.key === c.key && x.altgr === c.altgr && x.tap === c.tap && x.gap === c.gap);
        const xs = all.filter((x) => !x.aborted);
        const p2p = xs.map((x) => x.press_to_press_ms).sort((a, b) => a - b);
        const bound = PTT_NAMES[c.key] === ptt;
        const locked = xs.filter((x) => x.locked).length;
        rows.push({
          profile,
          key: c.altgr ? "RAlt (AltGr)" : c.key,
          bound,
          tap_ms: c.tap,
          gap_ms: c.gap,
          locked,
          tries: xs.length,
          aborted: all.length - xs.length,
          rate: xs.length ? locked / xs.length : 0,
          out_of_window: xs.filter((x) => !x.in_window).length,
          locked_in_window: xs.filter((x) => x.in_window && x.locked).length,
          in_window: xs.filter((x) => x.in_window).length,
          press_to_press_ms: [p2p[0], p2p[Math.floor(p2p.length / 2)], p2p.at(-1)],
          graded: bound && c.tap + c.gap <= NOMINAL_LIMIT_MS,
        });
      }
    }
    details.rows = rows;
    for (const r of rows) {
      evidence.push(`${r.profile} ${r.key}${r.bound ? "" : " (not the PTT key)"} tap ${r.tap_ms} gap ${r.gap_ms}: ${r.locked}/${r.tries} locked; ${r.out_of_window} out of the 500 ms window; press-to-press ${r.press_to_press_ms.join("/")} ms${r.graded ? "" : " (not graded)"}`);
    }
    const abortedTries = details.tries.filter((x) => x.aborted).length;
    const graded = rows.filter((r) => r.graded);
    const bad = graded.filter((r) => r.tries && r.rate < 0.9);
    const empty = graded.filter((r) => !r.tries); // every try aborted
    const sum = (xs, f) => xs.reduce((a, x) => a + f(x), 0);
    const perProfile = args.profiles
      .map((p) => {
        const g = graded.filter((r) => r.profile === p);
        return `${p} ${sum(g, (r) => r.locked)}/${sum(g, (r) => r.tries)}`;
      })
      .join(", ");
    return {
      status: bad.length || !graded.length ? "fail" : empty.length ? "aborted" : "pass",
      summary: `graded combinations locked: ${perProfile}; below 90%: ${bad.length ? bad.map((r) => `${r.profile} ${r.key} ${r.tap_ms}+${r.gap_ms} ${r.locked}/${r.tries}`).join(", ") : "none"}${abortedTries ? `; ${abortedTries} tries aborted (pad lost the foreground)` : ""}`,
      details,
      evidence,
    };
  },
};
