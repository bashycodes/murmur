// AI cleanup tab: the local LLM, the cleanup level and (D28) which engine
// cleans the text. Part of the shared Save form.
import { $, html } from "../lib/dom.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { prefsStore } from "../lib/prefs-store.js";
import { router } from "../lib/router.js";
import { views } from "../lib/views.js";
const TEMPLATE = `
<section class="panel" data-panel="ai">
  <fieldset data-needs="local">
    <legend>AI cleanup (local LLM)</legend>
    <label class="row"><input type="checkbox" id="llm_enabled" /> Clean up dictations with a local language model</label>
    <p id="llm_status" class="hint">Off.</p>
    <p class="hint">Polishes what the formatting rules leave: run-on sentences, leftover fillers, corrections, tone per app. Runs llama.cpp on this PC (<code id="llm_plugin"></code> next to the app). Off: the formatting rules still run. Skipped when the text is already clean; if the model is slow or changes too much, the formatted transcript is pasted instead. Loading takes a little longer than before (the instructions are prepared at load).</p>
    <label>Model file (GGUF, empty = default) <input id="llm_model_path" spellcheck="false" /></label>
    <p class="hint">Qwen2.5-0.5B-Instruct Q4_K_M is the fast default; Qwen2.5-1.5B-Instruct is more accurate and about twice as slow. Never downloaded automatically.</p>
    <label>Engine library (empty = next to the app) <input id="llm_lib_path" spellcheck="false" /></label>
    <label>Time budget per dictation (ms, default 3000) <input id="llm_timeout_ms" type="number" min="200" max="30000" step="100" /></label>
    <label>Cleanup level
      <select id="cleanup_level">
        <option value="none">No AI — formatting rules only</option>
        <option value="verbatim">Verbatim — exactly what you said (no formatting)</option>
        <option value="light">Light — fillers and grammar</option>
        <option value="medium">Medium — clarity</option>
        <option value="high">High — rewrite for polish</option>
      </select>
    </label>
  </fieldset>

  <!-- D28: which engine cleans the text; hidden in builds without OpenRouter. -->
  <fieldset id="ai_engine_box" hidden>
    <legend>Cleanup engine</legend>
    <div class="toolbar">
      <span id="ai_engine_line" class="grow">Engine: On-device</span>
      <button type="button" class="small secondary" id="ai_or_link">OpenRouter settings…</button>
    </div>
    <p class="hint">On-device = the local model above. OpenRouter = text cleanup in the cloud, off until you turn it on and agree in the OpenRouter tab.</p>
  </fieldset>
</section>`;
function showLlm(st) {
    const e = $("llm_status");
    switch (st.state) {
        case "off":
            e.textContent = "Off: deterministic formatting only.";
            e.className = "hint";
            break;
        case "loading":
            e.textContent = `Loading ${st.path}… (can take a minute; dictation works meanwhile, without cleanup)`;
            e.className = "hint";
            break;
        case "ready":
            e.textContent = `Ready: ${st.path} (loaded in ${(st.load_ms / 1000).toFixed(1)} s)`;
            e.className = "hint ok";
            break;
        case "missing":
            e.textContent = `Model file not found: ${st.path}. Dictation keeps working without cleanup.`;
            e.className = "hint err";
            break;
        case "plugin_missing":
            e.textContent = `The cleanup engine is missing next to the app: ${st.message}`;
            e.className = "hint err";
            break;
        case "failed":
            e.textContent = `Model failed to load (${st.path}): ${st.message}`;
            e.className = "hint err";
            break;
    }
}
/** D27: cleanup "None" = No AI (formatting still runs); Verbatim = Auto-format off too. */
function cleanupChoice(p) {
    return p.cleanup_level === "none" && !p.spoken_cleanup ? "verbatim" : p.cleanup_level;
}
/** Which engine cleans the text (local LLM toggle + OrView). */
function showAiEngine() {
    const v = views.or;
    $("ai_engine_box").hidden = !v?.supported;
    if (!v?.supported)
        return;
    const level = $("cleanup_level").value;
    const local = $("llm_enabled").checked && !!views.dev?.caps.local;
    const cloud = v.cleanup.allow && v.cleanup.order.includes("cloud");
    const or = `OpenRouter (${v.cleanup.model})`;
    let text;
    if (level === "none" || level === "verbatim")
        text = "Engine: Off (cleanup level: no AI)";
    else if (cloud && local)
        text =
            v.cleanup.order[0] === "local"
                ? `Engine: On-device · ${or} only when no local model is loaded`
                : `Engine: ${or}, then On-device`;
    else if (cloud)
        text = `Engine: ${or}`;
    else if (local)
        text = "Engine: On-device";
    else
        text = "Engine: Off (formatting rules only)";
    $("ai_engine_line").textContent = text;
}
function fill(p) {
    $("llm_enabled").checked = Boolean(p.llm_enabled);
    $("cleanup_level").value = cleanupChoice(p);
    $("llm_model_path").value = p.llm_model_path ?? "";
    $("llm_lib_path").value = p.llm_lib_path ?? "";
    $("llm_timeout_ms").value = String(p.llm_timeout_ms);
}
/** The prefs fields this tab owns. Runs after the General tab's collect,
 *  so Verbatim can switch Auto-format off. */
export function collectAi() {
    const next = {};
    next.llm_enabled = $("llm_enabled").checked;
    const level = $("cleanup_level").value;
    next.cleanup_level = level === "verbatim" ? "none" : level;
    if (level === "verbatim")
        next.spoken_cleanup = false;
    // Raw mode only ever skipped the LLM; "No AI" says that now (D27).
    next.raw_mode = false;
    next.llm_model_path = $("llm_model_path").value.trim() || null;
    next.llm_lib_path = $("llm_lib_path").value.trim() || null;
    next.llm_timeout_ms = Math.min(30000, Math.max(200, parseInt($("llm_timeout_ms").value, 10) || 3000));
    return next;
}
export const aiPage = {
    id: "ai",
    mount(root) {
        root.append(html(TEMPLATE));
        $("ai_or_link").addEventListener("click", () => router.go("openrouter"));
        $("llm_enabled").addEventListener("change", showAiEngine);
        $("cleanup_level").addEventListener("change", showAiEngine);
        // Verbatim and the Auto-format box (General tab) are the same switch (D27): keep them in step.
        $("cleanup_level").addEventListener("change", () => {
            const sel = $("cleanup_level");
            const fmt = $("spoken_cleanup");
            if (sel.value === "verbatim")
                fmt.checked = false;
            else if (!fmt.checked && sel.value === "none")
                fmt.checked = true;
        });
        $("spoken_cleanup").addEventListener("change", () => {
            const sel = $("cleanup_level");
            const fmt = $("spoken_cleanup");
            if (!fmt.checked && sel.value === "none")
                sel.value = "verbatim";
            else if (fmt.checked && sel.value === "verbatim")
                sel.value = "none";
        });
        views.onOr(() => showAiEngine());
        void onEvent("llm:status", (e) => showLlm(e.payload));
    },
    async load() {
        const model = await invoke("get_model_status");
        showLlm(model.llm);
        $("llm_plugin").textContent = model.llm_plugin;
        $("llm_model_path").placeholder = model.default_llm_path;
        const p = prefsStore.get();
        if (p)
            fill(p);
    },
    show() {
        showAiEngine();
    },
};
