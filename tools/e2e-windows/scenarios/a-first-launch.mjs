// (a) The first launch after a fresh install shows first-run setup.
// Pass = a visible Murmur window AND the onboarding panel open in it.
import { appWindows, onboardingDom, pageShot, sleep, windowShot } from "../lib/steps.mjs";

export default {
  id: "a",
  title: "First launch opens onboarding",
  needs: "fresh",
  async run(ctx) {
    const { app, remote } = ctx;
    // Give the app the time a person would: it should show setup by itself.
    await sleep(8000);
    const s = await app.settings();
    const dom = await onboardingDom(s);
    const prefs = await app.invoke("get_prefs").catch((e) => ({ error: e.message }));
    const onboarded = prefs?.onboarded ?? null;
    const prefsFile = remote.ssh("if exist %APPDATA%\\Murmur\\prefs.json (echo yes) else (echo no)", { check: false }).out.trim();
    const wins = await appWindows(ctx);
    const visible = wins.filter((w) => w.title === "Murmur");
    const evidence = [];
    const shot = await pageShot(ctx, s, "a-settings-webview");
    if (shot) evidence.push(`settings webview (CDP, rendered even if the window is hidden): ${shot}`);
    // Info only (not graded): a launch from a scheduled task may not be
    // allowed to take the foreground, so a covered window is not by itself
    // the app's fault.
    const fg = (await ctx.agent.call("foreground")).fg;
    const placement = [];
    for (const [i, w] of visible.entries()) {
      const p = await windowShot(ctx, w.hwnd, `a-window-${i}`);
      const cov = await ctx.agent.call("covered", { hwnd: w.hwnd });
      placement.push({ foreground: fg?.hwnd === w.hwnd, covered: cov.covered, covered_by: cov.by_process ?? null });
      if (p) evidence.push(`visible window "${w.title}" ${w.w}x${w.h} at ${w.x},${w.y}; foreground=${fg?.hwnd === w.hwnd}; covered=${cov.covered}${cov.covered ? ` by ${cov.by_process}` : ""}: ${p}`);
    }
    const details = { dom, onboarded_pref: onboarded, prefs_file_exists: prefsFile, visible_windows: wins.map(({ title, w, h }) => ({ title, w, h })), placement, foreground_process: fg?.process ?? null, installer_autostarted: ctx.results.installer_autostarted };
    evidence.push(`DOM: onboarding_open=${dom.onboarding_open} step=${dom.step} visibilityState=${dom.visibility}; get_prefs.onboarded=${onboarded}; prefs.json on disk=${prefsFile}; visible Murmur windows=${visible.length}`);
    const pass = visible.length > 0 && dom.onboarding_open;
    let summary;
    if (pass) summary = `onboarding shown (step ${dom.step}) in a visible window`;
    else if (!visible.length && dom.onboarding_open) summary = "onboarding is open in the DOM but no Murmur window is visible: the user sees nothing";
    else if (!visible.length) summary = "no visible window and no onboarding after first launch";
    else summary = `window visible but onboarding not open (onboarded=${onboarded})`;
    return { status: pass ? "pass" : "fail", summary, details, evidence };
  },
};
