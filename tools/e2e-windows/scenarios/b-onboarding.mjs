// (b) Walk first-run setup to the end through the settings webview (CDP),
// one screenshot per step. Engine: this PC. Step 3 (D94) offers one
// recommended row, "English — fast" (Parakeet, 661 MB): before the walk the
// model is copied into the test profile from a copy already on the laptop
// (ps/models.ps1: the harness's cache, the owner's backed-up models), so the
// row reads Installed. With no copy anywhere, the row's Download is pressed
// and waited for, and the downloaded model is stashed in the cache for the
// next run. At the "try" step: pick the VB-Cable mic, dictate one clip into
// the practice box with Right Alt, then Finish.
// Pass = every step reached, get_prefs.onboarded true at the end, and the
// practice box got text while the pill went active (the words are info:
// an STT mishearing is not a setup bug).
import { OVERLAY_RECORDER } from "../lib/app.mjs";
import { filesArg, gradePractice, recommendedEntry } from "../lib/onboarding.mjs";
import { Remote } from "../lib/remote.mjs";
import { appWindows, pageShot, sleep, talk, windowShot } from "../lib/steps.mjs";

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

/** How long a fallback download of the recommended model may take. */
const DOWNLOAD_MS = 30 * 60_000;

/** The recommended row on step 3: its chip state, Download button and pick. */
const ROW = `(() => {
  const r = document.querySelector('#onboarding [data-ob-model="parakeet"]');
  if (!r) return { found: false };
  const dl = r.querySelector("[data-ob-dl]");
  return {
    found: true,
    picked: r.getAttribute("aria-checked") === "true",
    chip: r.querySelector("[data-ob-chip]")?.dataset.state ?? null,
    download: dl && !dl.hidden ? dl.textContent.trim() : null,
    line: r.querySelector("[data-ob-dl-line]")?.textContent.trim() ?? "",
    hint: document.getElementById("ob_model_hint")?.textContent.trim() ?? "",
    status: document.getElementById("ob_model_status")?.textContent.trim() ?? "",
  };
})()`;

/**
 * Copy the recommended model into the test profile (ps/models.ps1) so step 3
 * shows it Installed. The page reads the catalogue when setup opens, so a
 * copy made now needs a restart (setup reopens by itself, scenario g).
 * Returns {entry, seed, source?, reason?}.
 */
async function seedRecommended(ctx) {
  const { app, remote, log } = ctx;
  const entry = recommendedEntry(await app.invoke("model_catalog"));
  if (!entry) return { entry: null, seed: "none", reason: "the recommended model is not in model_catalog" };
  if (entry.installed) return { entry, seed: "already" };
  const res = remote.ps("models.ps1", ["-Action", "seed", "-Dest", entry.dest, "-Files", filesArg(entry)], { timeoutMs: 600_000, check: false });
  const r = Remote.kv(res.out);
  // A refusal (no pending backup, an unexpected dest) falls back to the download.
  if (res.code !== 0) Object.assign(r, { seed: "none", reason: `models.ps1 failed: ${res.out.trim().split(/\r?\n/).at(-1)}` });
  log(`model seed: ${r.seed ?? "?"}${r.source ? ` from ${r.source} in ${r.seconds} s` : ""}${r.reason ? ` (${r.reason})` : ""}`);
  if (r.seed === "copied") {
    app.stop();
    await sleep(1500);
    await app.launch();
    await sleep(8000);
  }
  return { entry, seed: r.seed ?? "none", source: r.source, reason: r.reason };
}

/**
 * Step 3: pick the recommended row. Installed (seeded): nothing to do. Not
 * installed: press Download, confirm, wait for Installed (pinging the agent
 * so it does not idle out), then stash the model for the next run.
 */
async function pickModel(ctx, s, name, seed) {
  const { agent, remote, log } = ctx;
  await s.eval(`document.querySelector('#onboarding [data-ob-model="parakeet"]')?.click()`);
  await sleep(400);
  let row = await s.eval(ROW);
  const out = { seed: seed.seed, source: seed.source ?? null, row_before: row };
  if (!row.found) return { ...out, ok: false, why: "no recommended row on step 3" };
  if (row.chip === "installed") return { ...out, ok: true, how: seed.seed === "copied" ? `seeded from ${seed.source}` : "already installed" };
  out.download_reason = seed.reason ?? `seed=${seed.seed}, row ${row.chip}`;
  log(`step 3: the recommended model is not installed (${out.download_reason}): downloading it`);
  if (!row.download) return { ...out, ok: false, why: `not installed and no Download button (${row.status || row.hint})` };
  await s.eval(`document.querySelector('#onboarding [data-ob-dl="parakeet"]').click()`);
  if (!(await s.waitFor(`document.getElementById("dl_confirm")?.open === true`, 10_000))) {
    return { ...out, ok: false, why: "Download did not open its confirm dialog" };
  }
  await pageShot(ctx, s, `${name}-download-confirm`);
  await s.eval(`document.getElementById("dl_ok").click()`);
  const t0 = Date.now();
  for (;;) {
    row = await s.eval(ROW);
    if (row.chip === "installed") break;
    if (row.download === "Try again") return { ...out, ok: false, why: `download failed: ${row.line}` };
    if (Date.now() - t0 > DOWNLOAD_MS) return { ...out, ok: false, why: `download not done after ${DOWNLOAD_MS / 60_000} min: ${row.line}` };
    await agent.call("ping", {}, 5000).catch(() => {});
    await sleep(10_000);
  }
  out.download_s = Math.round((Date.now() - t0) / 1000);
  if (seed.entry) {
    const r = Remote.kv(remote.ps("models.ps1", ["-Action", "stash", "-Dest", seed.entry.dest, "-Files", filesArg(seed.entry)], { timeoutMs: 600_000, check: false }).out);
    out.stash = r.stash ?? "failed";
  }
  return { ...out, ok: true, how: `downloaded in ${out.download_s} s` };
}

export default {
  id: "b",
  title: "Onboarding walk-through",
  needs: "fresh",
  async run(ctx) {
    const { app } = ctx;
    const evidence = [];
    const steps = [];
    const details = { steps };
    const seed = await seedRecommended(ctx);
    details.model_seed = { seed: seed.seed, source: seed.source ?? null, reason: seed.reason ?? null };
    evidence.push(`model seed: ${seed.seed}${seed.source ? ` from ${seed.source}` : ""}${seed.reason ? ` (${seed.reason})` : ""}`);
    const s = await app.settings();
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
        details.model = await pickModel(ctx, s, name, seed);
        evidence.push(`model step: ${details.model.ok ? details.model.how : `FAILED: ${details.model.why}`}`);
        if (!details.model.ok) return { status: "fail", summary: `model step: ${details.model.why}`, details, evidence };
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
    details.asr_engine = prefs.asr_engine;
    const finished = !st.open && prefs.onboarded === true;
    const pass = finished && details.practice?.ok === true;
    const walked = steps.map((x) => x.step).join(" > ");
    evidence.push(`get_prefs after Finish: onboarded=${prefs.onboarded}, asr_engine=${prefs.asr_engine}, microphone=${prefs.microphone}`);
    const practiced = details.practice?.aborted ? "ABORTED (focus)" : details.practice?.ok ? "worked" : "did NOT work";
    return {
      status: pass ? "pass" : "fail",
      summary: `${finished ? "walked" : "did not finish"}: ${walked}; onboarded=${prefs.onboarded}; model ${details.model?.how ?? "not picked"}; practice dictation ${practiced}`,
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
  const seen = await ov.eval("window.__e2e.states.map((x) => x.s)");
  const states = seen.join(">");
  const shot = await pageShot(ctx, s, `${name}-practice`);
  const wshot = win ? await windowShot(ctx, win.hwnd, `${name}-practice-window`) : null;
  // The box only: the result line also carries "nothing heard"-style notes.
  const g = gradePractice({ text: after.box, states: seen, expect: clip.expect });
  return {
    ok: g.ok,
    text_ok: g.text_ok,
    overlay_active: g.overlay_active,
    mic: mic.value,
    settings_window_foreground: fg.focused,
    overlay_states: states,
    box: after.box,
    result_line: after.result_hidden ? null : after.result,
    word_score: g.word_score,
    word_recall: g.word_recall,
    shots: [shot, wshot].filter(Boolean),
    summary: `${g.ok ? "OK" : "FAILED"} (text ${g.text_ok ? "arrived" : "EMPTY"}, pill ${g.overlay_active ? "went active" : "NEVER active"}, words ${g.word_score.toFixed(2)} as info; box="${after.box}", result line="${after.result_hidden ? "" : after.result}", overlay ${states || "no change"}, settings window foreground=${fg.focused}); ${shot}${wshot ? `, ${wshot}` : ""}`,
  };
}
