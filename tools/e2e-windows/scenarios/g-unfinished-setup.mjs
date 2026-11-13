// (g) First-run setup left unfinished opens again by itself on the next
// start. Setup saves each step's choice as it goes, so a person who quits
// half way has a prefs.json with onboarded=false. Before the fix only a
// missing prefs.json showed the window: the next start sat in the tray with
// setup open in a hidden window.
// Pass = prefs.json on disk with onboarded=false, then after a restart a
// visible Murmur window with onboarding open. Runs between a and b (b walks
// the setup that is open afterwards).
import { appWindows, onboardingDom, pageShot, sleep, windowShot } from "../lib/steps.mjs";

const PREFS_FILE = "if exist %APPDATA%\\Murmur\\prefs.json (echo yes) else (echo no)";

export default {
  id: "g",
  title: "Unfinished setup reopens at start",
  needs: "fresh",
  async run(ctx) {
    const { app, remote } = ctx;
    const evidence = [];
    const before = await app.invoke("get_prefs");
    if (before?.onboarded) return { status: "error", summary: "setup is already finished: run g before b", details: {}, evidence };
    // What a setup step does: save the prefs, still not onboarded.
    await app.invoke("set_prefs", { prefs: before });
    const saved = remote.ssh(PREFS_FILE, { check: false }).out.trim();
    evidence.push(`prefs.json after a setup step's save: ${saved}`);
    if (saved !== "yes") return { status: "error", summary: "set_prefs did not write prefs.json", details: { saved }, evidence };
    app.stop();
    await sleep(1500);
    await app.launch();
    await sleep(8000);
    const s = await app.settings();
    const dom = await onboardingDom(s);
    const onboarded = (await app.invoke("get_prefs").catch(() => null))?.onboarded ?? null;
    const wins = await appWindows(ctx);
    const visible = wins.filter((w) => w.title === "Murmur");
    const shot = await pageShot(ctx, s, "g-settings-webview");
    if (shot) evidence.push(`settings webview after the restart (CDP): ${shot}`);
    for (const [i, w] of visible.entries()) {
      const p = await windowShot(ctx, w.hwnd, `g-window-${i}`);
      if (p) evidence.push(`visible window "${w.title}" ${w.w}x${w.h}: ${p}`);
    }
    evidence.push(`after restart: onboarding_open=${dom.onboarding_open} step=${dom.step}; get_prefs.onboarded=${onboarded}; visible Murmur windows=${visible.length}`);
    const details = { dom, onboarded_pref: onboarded, visible_windows: wins.map(({ title, w, h }) => ({ title, w, h })) };
    const pass = visible.length > 0 && dom.onboarding_open && onboarded === false;
    let summary;
    if (pass) summary = `setup reopened (step ${dom.step}) in a visible window`;
    else if (!visible.length && dom.onboarding_open) summary = "setup is open in a hidden window: the user sees nothing";
    else if (!visible.length) summary = "no visible window and no setup after the restart";
    else summary = `window visible but setup not open (onboarded=${onboarded})`;
    return { status: pass ? "pass" : "fail", summary, details, evidence };
  },
};
