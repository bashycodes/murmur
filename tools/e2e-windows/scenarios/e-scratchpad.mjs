// (e) Scratchpad: Settings > Scratchpad > New note should open the notes
// window on its editor (scratchpad.html?note=<id>); dictating with the
// window focused should land in the note and be saved under notes\.
// Pass = editor loaded, the window opened in front with the caret in the
// editor (checked before the harness forces any focus) AND the dictated
// clip is in the note.
import { Page, listPages } from "../lib/cdp.mjs";
import { CDP_PORT } from "../lib/app.mjs";
import { appWindows, judge, pageShot, sleep, talk, windowShot } from "../lib/steps.mjs";

const PAGE_STATE = `({ href: location.href, ready: document.readyState, body_len: document.body ? document.body.innerHTML.length : 0, tauri: !!window.__TAURI__, editor: !!document.getElementById("editor"), editor_visible: !!document.getElementById("editor") && !document.getElementById("editor-view").hidden })`;

export default {
  id: "e",
  title: "Scratchpad",
  needs: "seeded",
  async run(ctx) {
    const { app, agent, clips, remote } = ctx;
    const evidence = [];
    const details = {};
    const s = await app.settings();
    await s.eval(`document.querySelector('button[data-page="scratchpad"]').click()`);
    await sleep(1000);
    evidence.push(`Scratchpad page: ${await pageShot(ctx, s, "e-1-scratchpad-page")}`);
    const before = new Set((await listPages(CDP_PORT)).map((p) => p.id));
    details.new_button = await s.eval(`(() => { const b = document.getElementById("sp_new"); if (!b) return false; b.click(); return true; })()`);
    if (!details.new_button) return { status: "fail", summary: "no New note button on the Scratchpad page", details, evidence };

    // The notes window: as a native window and as a webview target.
    let win = null;
    let target = null;
    const end = Date.now() + 10_000;
    while (Date.now() < end && !(win && target)) {
      await sleep(500);
      win = (await appWindows(ctx)).find((w) => w.title.startsWith("Notes")) ?? null;
      target = (await listPages(CDP_PORT)).find((p) => !before.has(p.id) || p.url.includes("scratchpad.html")) ?? null;
    }
    details.window = win && { title: win.title, w: win.w, h: win.h };
    details.target_url = target?.url ?? null;
    if (win) evidence.push(`notes window "${win.title}" ${win.w}x${win.h}: ${await windowShot(ctx, win.hwnd, "e-2-notes-window")}`);
    if (!target) {
      if (win) await agent.call("close", { hwnd: win.hwnd });
      return { status: "fail", summary: `New note: ${win ? "a window opened but" : "no window and"} no webview target appeared`, details, evidence };
    }
    const np = await Page.open(target);
    try {
      await sleep(1500);
      details.page = await np.eval(PAGE_STATE);
      evidence.push(`notes webview: url=${details.page.href} readyState=${details.page.ready} body=${details.page.body_len} chars __TAURI__=${details.page.tauri} editor visible=${details.page.editor_visible}`);
      evidence.push(`notes webview render (CDP): ${await pageShot(ctx, np, "e-3-notes-webview")}`);
      if (!details.page.editor_visible) {
        return { status: "fail", summary: `the notes window opens but its page never loads (url ${details.page.href}, body ${details.page.body_len} chars): nothing to dictate into`, details, evidence };
      }
      // The app's own job, checked before the harness touches the focus:
      // the window it opened is in front with the caret in the editor.
      const fg0 = (await agent.call("foreground")).fg;
      const active = await np.eval(`document.activeElement ? document.activeElement.id : null`);
      details.opened_focused = { foreground: fg0?.process ?? null, foreground_is_notes: !!win && fg0?.hwnd === win.hwnd, active_element: active };
      const openedFocused = details.opened_focused.foreground_is_notes && active === "editor";
      evidence.push(`on open: foreground=${fg0?.process} "${fg0?.title ?? ""}" (notes window: ${details.opened_focused.foreground_is_notes}), activeElement=${active}`);
      // Fallback only, so the dictation part still runs: the harness's own focus.
      if (!openedFocused) {
        const f = win ? await agent.call("focus", { hwnd: win.hwnd }) : { focused: false };
        details.foreground = f.fg?.process ?? null;
        if (!f.focused) return { status: "aborted", summary: `the notes window would not come to the foreground (fg=${details.foreground}): nothing pressed`, details, evidence };
        await np.eval(`document.getElementById("editor").focus()`);
      }
      const clip = clips["en-short"];
      const said = await talk(ctx, "en-short", { guard: win.hwnd });
      if (said.aborted) return { status: "aborted", summary: `dictation aborted ${said.aborted_at}: the notes window lost the foreground`, details, evidence };
      await np.waitFor(`document.getElementById("editor").value.trim().length > 0`, 25_000).catch(() => {});
      await sleep(3000); // the shell writes the note after a short pause
      const text = await np.eval(`document.getElementById("editor").value`);
      const j = judge(clip.expect, text);
      details.dictation = { got: text, score: j.score };
      evidence.push(`dictated into the note: "${text}" (score ${j.score.toFixed(2)}); ${await pageShot(ctx, np, "e-4-after-dictation")}`);
      const files = remote.ssh("dir /b %APPDATA%\\Murmur\\notes", { check: false }).out.split(/\r?\n/).map((x) => x.trim()).filter(Boolean);
      details.note_files = files;
      evidence.push(`notes folder: ${files.join(", ") || "(empty)"}`);
      const problems = [];
      if (!openedFocused) problems.push(`the notes window did not open focused with the caret in the editor (foreground ${details.opened_focused.foreground}, activeElement ${active})`);
      if (!j.pass) problems.push(`the dictation did not land (got "${text}")`);
      return { status: problems.length ? "fail" : "pass", summary: problems.length ? problems.join("; ") : "note editor opened focused and took the dictation", details, evidence };
    } finally {
      np.close();
      if (win) await agent.call("close", { hwnd: win.hwnd }).catch(() => {});
    }
  },
};
