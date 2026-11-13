// (b) Walk first-run setup to the end through the settings webview (CDP),
// one screenshot per step. Engine: this PC with a Whisper model from the
// owner's backed-up models, copied into the test profile. At the
// "try" step: pick the VB-Cable mic, dictate one clip into the practice box
// with the push-to-talk key, then Finish.
// Pass = every step reached and get_prefs.onboarded true at the end. The
// practice-box dictation is reported as a sub-check.
import { OVERLAY_RECORDER } from "../lib/app.mjs";
import { appWindows, judge, pageShot, sleep, talk, windowShot } from "../lib/steps.mjs";

const STATE = `(() => {
  const ob = document.getElementById("onboarding");
  const p = ob && [...ob.querySelectorAll("[data-ob-step]")].find((x) => !x.hidden);
  const next = p && p.querySelector("[data-ob=next]");
  return { open: !!ob && !ob.hidden, step: p ? p.dataset.obStep : null, next_disabled: next ? next.disabled : null, next_label: next ? next.textContent.trim() : null };
})()`;

const CLICK_NEXT = `(() => {
  const p = [...document.querySelectorAll("#onboarding [data-ob-step]")].find((x) => !x.hidden);
  const b = p && p.querySelector("[data-ob=next]");
  if (!b || b.disabled) return false;
  b.click();
  return true;
})()`;

/**
 * The smallest Whisper file among the owner's backed-up models, COPIED into
 * the test profile (the app is never pointed at the owner's file, so it
 * cannot change it; the copy goes away with the test profile on restore).
 */
function copyModel(ctx) {
  const dir = ctx.remote.winPath(`backups/${ctx.results.run}/Roaming-Murmur/models`);
  const r = ctx.remote.ssh(`dir /s /b /o:s "${dir}\\ggml-*.bin"`, { check: false });
  const src = r.out.split(/\r?\n/).map((l) => l.trim()).find((l) => /\.bin$/i.test(l));
  if (!src) return null;
  const appdata = ctx.remote.ssh("echo %APPDATA%").out.trim().split(/\r?\n/).at(-1).trim();
  const to = `${appdata}\\Murmur\\e2e-models`;
  ctx.remote.ssh(`if not exist "${to}" mkdir "${to}"`);
  ctx.remote.ssh(`copy /y "${src}" "${to}\\"`);
  return `${to}\\${src.split("\\").pop()}`;
}

export default {
  id: "b",
  title: "Onboarding walk-through",
  needs: "fresh",
  async run(ctx) {
    const { app, log } = ctx;
    const s = await app.settings();
    const evidence = [];
    const steps = [];
    const details = { steps };
    let st = await s.eval(STATE);
    if (!st.open) {
      // Onboarding did not open by itself (scenario a): open it as the
      // owner would have to (Settings > General > Run setup), so the rest
      // of the walk still gets tested.
      const opened = await s.eval(`(() => { const b = document.querySelector("[data-run-setup]"); if (b) { b.click(); return true; } return false; })()`);
      details.opened_by_hand = opened;
      await sleep(800);
      st = await s.eval(STATE);
      if (!st.open) return { status: "fail", summary: "onboarding is not open and no way to open it was found", details, evidence };
    }
    for (let i = 0; i < 10 && st.open; i++) {
      const name = `b-${String(i + 1).padStart(2, "0")}-${st.step}`;
      const shot = await pageShot(ctx, s, name);
      steps.push({ step: st.step, next_label: st.next_label, shot });
      evidence.push(`step ${st.step}: ${shot}`);
      if (st.step === "engine") {
        await s.eval(`document.querySelector('#onboarding [data-ob-engine="pc"]')?.click()`);
        await sleep(500);
      } else if (st.step === "model") {
        const model = copyModel(ctx);
        details.model = model ? model.split("\\").pop() : null;
        if (!model) return { status: "fail", summary: "model step: no Whisper .bin in the owner's models to point at", details, evidence };
        await s.eval(`(() => {
          const i = document.getElementById("ob_whisper_path");
          i.value = ${JSON.stringify(model)};
          i.dispatchEvent(new Event("input", { bubbles: true }));
          i.dispatchEvent(new Event("change", { bubbles: true }));
          document.querySelector('#onboarding [data-ob-use="whisper"]').click();
        })()`);
        await s.waitFor(`(${STATE}).next_disabled === false`, 30_000).catch(() => {});
        await pageShot(ctx, s, `${name}-model-set`);
      } else if (st.step === "try") {
        details.practice = await practice(ctx, s, name);
        evidence.push(`practice box: ${details.practice.summary}`);
      }
      const before = st.step;
      if (!(await s.eval(CLICK_NEXT))) {
        return { status: "fail", summary: `stuck at step "${before}": Next/Continue is missing or disabled`, details, evidence };
      }
      await s.waitFor(`(${STATE}).step !== ${JSON.stringify(before)}`, 15_000).catch(() => {});
      st = await s.eval(STATE);
    }
    await sleep(800);
    const end = await pageShot(ctx, s, "b-99-after-finish");
    evidence.push(`after Finish: ${end}`);
    const prefs = await app.invoke("get_prefs");
    details.onboarded = prefs.onboarded;
    details.microphone = prefs.microphone;
    const pass = !st.open && prefs.onboarded === true;
    const walked = steps.map((x) => x.step).join(" > ");
    evidence.push(`get_prefs after Finish: onboarded=${prefs.onboarded}, microphone=${prefs.microphone}`);
    return {
      status: pass ? "pass" : "fail",
      summary: `${pass ? "walked" : "did not finish"}: ${walked}; onboarded=${prefs.onboarded}; practice dictation ${details.practice?.aborted ? "ABORTED (focus)" : details.practice?.ok ? "worked" : "did NOT work"}`,
      details,
      evidence,
    };
  },
};

/** The "try" step: CABLE mic in #ob_mic, focus the box, hold PTT over a clip. */
async function practice(ctx, s, name) {
  const { app, agent, clips } = ctx;
  const mic = await s.eval(`(() => {
    const m = document.getElementById("ob_mic");
    const o = [...m.options].find((x) => x.value.startsWith("CABLE Output"));
    if (!o) return { found: false, options: m.options.length };
    m.value = o.value;
    m.dispatchEvent(new Event("change", { bubbles: true }));
    document.getElementById("ob_try_box").focus();
    return { found: true, value: o.value };
  })()`);
  if (!mic.found) return { ok: false, summary: `no CABLE Output in the mic list (${mic.options} options)` };
  await sleep(1000);
  const ov = await app.overlay();
  await ov.eval(OVERLAY_RECORDER);
  await ov.eval("window.__e2e.states.length = 0");
  // The keys go to the foreground window: bring the settings window up.
  const win = (await appWindows(ctx)).find((w) => w.title === "Murmur");
  const fg = win ? await agent.call("focus", { hwnd: win.hwnd }) : { focused: false };
  const clip = clips["en-short"];
  if (!fg.focused) return { ok: false, aborted: true, summary: "ABORTED: the settings window would not come to the foreground (no keys pressed)" };
  const said = await talk(ctx, "en-short", { guard: win.hwnd });
  if (said.aborted) return { ok: false, aborted: true, summary: `ABORTED ${said.aborted_at}: the settings window lost the foreground (to ${said.fg_process ?? "?"})` };
  await sleep(6000);
  const after = await s.eval(`({ box: document.getElementById("ob_try_box").value, result_hidden: document.getElementById("ob_result").hidden, result: document.getElementById("ob_result_text").textContent })`);
  const states = (await ov.eval("window.__e2e.states.map((x) => x.s)")).join(">");
  const shot = await pageShot(ctx, s, `${name}-practice`);
  const wshot = win ? await windowShot(ctx, win.hwnd, `${name}-practice-window`) : null;
  const j = judge(clip.expect, after.box || after.result);
  return {
    ok: j.pass,
    mic: mic.value,
    settings_window_foreground: fg.focused,
    overlay_states: states,
    box: after.box,
    result_line: after.result_hidden ? null : after.result,
    score: j.score,
    shots: [shot, wshot].filter(Boolean),
    summary: `${j.pass ? "OK" : "FAILED"} (box="${after.box}", result line="${after.result_hidden ? "" : after.result}", overlay ${states || "no change"}, settings window foreground=${fg.focused}); ${shot}${wshot ? `, ${wshot}` : ""}`,
  };
}
