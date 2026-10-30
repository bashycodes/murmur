// AI cleanup: the local LLM, the cleanup level and (D28) which engine
// cleans the text and in what order, on the AI cleanup settings page
// (design: SettingsCleanup.dc.html). The OpenRouter switch and the order
// select belong to features/openrouter.ts; the ranked list here only
// moves rows and changes that select.
import { $ } from "../lib/dom.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { prefsStore } from "../lib/prefs-store.js";
import { views } from "../lib/views.js";
/** What each cleanup level does (D27), with the design's example of what is pasted. */
const LEVELS = {
    verbatim: [
        "Verbatim",
        "Exactly what you said. No punctuation or fixes.",
        "um so the meeting is on tuesday no wait thursday at 3 and uh bring the the slides",
    ],
    none: [
        "Rules only",
        'Punctuation, fillers, numbers and "scratch that" are handled by fixed rules on this PC. No AI model runs.',
        "So the meeting is on Thursday at 3 and bring the slides.",
    ],
    light: [
        "Light",
        "Removes fillers and fixes grammar and obvious slips. Keeps your wording.",
        "So the meeting is on Thursday at 3, and bring the slides.",
    ],
    medium: [
        "Medium",
        "Also smooths run-on sentences and corrections for clarity.",
        "The meeting is on Thursday at 3. Bring the slides.",
    ],
    high: [
        "High",
        "Also tightens wording and structure. Best for email and docs.",
        "Meeting: Thursday at 3 p.m. Please bring the slides.",
    ],
};
let llm = { state: "off" };
function showLevel() {
    const level = $("cleanup_level").value;
    for (const b of document.querySelectorAll("#level_seg [data-level]")) {
        const on = b.dataset.level === level;
        b.classList.toggle("is-on", on);
        b.setAttribute("aria-checked", String(on));
    }
    const [title, desc, sample] = LEVELS[level] ?? ["", "", ""];
    $("level_title").textContent = title;
    $("level_desc").textContent = desc;
    $("level_sample").textContent = sample;
}
function llmText(st) {
    switch (st.state) {
        case "off":
            return "Off: the formatting rules only.";
        case "loading":
            return `Loading ${st.path}… (can take a minute; dictation works meanwhile, without cleanup)`;
        case "ready":
            return `${fileStem(st.path)}. Private; loaded in ${(st.load_ms / 1000).toFixed(1)} s.`;
        case "missing":
            return `Model file not found: ${st.path}. Dictation keeps working without cleanup.`;
        case "plugin_missing":
            return `The cleanup engine is missing next to the app: ${st.message}`;
        case "failed":
            return `Model failed to load (${st.path}): ${st.message}`;
    }
}
function fileStem(path) {
    return (path.split(/[\\/]/).pop() ?? path).replace(/\.gguf$/i, "");
}
function showLlm(st) {
    llm = st;
    const e = $("aieng_desc_local");
    e.textContent = llmText(st);
    e.classList.toggle("err", st.state === "missing" || st.state === "plugin_missing" || st.state === "failed");
    renderEngines();
}
function setBadge(r, text, kind) {
    const b = $(`aieng_badge_${r}`);
    b.textContent = text;
    b.className = `fl-badge fl-badge-${kind}`;
}
/** The ranked cleanup engines: OpenRouter and this PC, in the saved order. */
function renderEngines() {
    const v = views.or;
    const cloudShown = !!v?.supported;
    const localShown = !!views.dev?.caps.local;
    const order = v?.cleanup.order[0] === "local" ? ["local", "cloud"] : ["cloud", "local"];
    const shown = order.filter((r) => (r === "cloud" ? cloudShown : localShown));
    const list = $("ai_routes");
    const foot = list.querySelector(".lw-foot-row");
    for (const r of order)
        list.insertBefore($(`aieng_row_${r}`), foot);
    shown.forEach((r, i) => {
        const row = $(`aieng_row_${r}`);
        row.querySelector(".lw-rank").textContent = String(i + 1);
        row.querySelector("[data-move=up]").disabled = i === 0;
        row.querySelector("[data-move=down]").disabled = i === shown.length - 1;
        row.querySelector(".lw-move").hidden = shown.length < 2;
    });
    const cloudOn = !!v?.cleanup.allow;
    $("aieng_row_cloud").classList.toggle("is-off", !cloudOn);
    $("aieng_desc_cloud").textContent = cloudOn
        ? `Sends the transcript as text to openrouter.ai · ${(v?.cleanup.model ?? "").split("/").pop()}.`
        : "Sends the transcript as text to openrouter.ai. Asks first.";
    const st = v?.cleanup.status;
    if (!cloudOn)
        setBadge("cloud", "Off", "light");
    else if (st?.blocked)
        setBadge("cloud", "Not set up", "warning");
    else if (st?.circuit?.open)
        setBadge("cloud", "Paused", "warning");
    else
        setBadge("cloud", "On", "success");
    const localOn = $("llm_enabled").checked;
    $("aieng_row_local").classList.toggle("is-off", !localOn);
    if (!localOn)
        setBadge("local", "Off", "light");
    else if (llm.state === "ready")
        setBadge("local", "Ready", "success");
    else if (llm.state === "loading" || llm.state === "off")
        setBadge("local", "Loading", "light");
    else if (llm.state === "plugin_missing")
        setBadge("local", "Not installed", "destructive");
    else if (llm.state === "missing")
        setBadge("local", "Model missing", "destructive");
    else
        setBadge("local", "Failed", "destructive");
}
/** Move a cleanup engine: the order select is features/openrouter.ts's, which saves it. */
function moveEngine(r, dir) {
    const sel = $("or_cleanup_order");
    const cloudFirst = sel.value !== "local,cloud";
    const cloudUp = (r === "cloud" && dir === -1) || (r === "local" && dir === 1);
    if (cloudUp === cloudFirst)
        return;
    sel.value = cloudUp ? "cloud,local" : "local,cloud";
    sel.dispatchEvent(new Event("change"));
}
/** D27: cleanup "None" = No AI (formatting still runs); Verbatim = Auto-format off too. */
function cleanupChoice(p) {
    return p.cleanup_level === "none" && !p.spoken_cleanup ? "verbatim" : p.cleanup_level;
}
/** Which engine cleans the text (local LLM toggle + OrView). */
function showAiEngine() {
    const v = views.or;
    const level = $("cleanup_level").value;
    const local = $("llm_enabled").checked && !!views.dev?.caps.local;
    const cloud = !!v?.supported && v.cleanup.allow && v.cleanup.order.includes("cloud");
    const or = `OpenRouter (${v?.cleanup.model ?? ""})`;
    let text;
    if (level === "none" || level === "verbatim")
        text = `no AI (the cleanup level is ${level === "none" ? "Rules only" : "Verbatim"})`;
    else if (cloud && local)
        text =
            v?.cleanup.order[0] === "local"
                ? `This PC · ${or} only when no local model is loaded`
                : `${or}, then this PC`;
    else if (cloud)
        text = or;
    else if (local)
        text = "This PC";
    else
        text = "the formatting rules only";
    $("ai_engine_line").textContent = text;
}
function fill(p) {
    $("llm_enabled").checked = Boolean(p.llm_enabled);
    $("cleanup_level").value = cleanupChoice(p);
    showLevel();
    setField("llm_model_path", p.llm_model_path ?? "");
    setField("llm_lib_path", p.llm_lib_path ?? "");
    ensureOption("llm_timeout_ms", String(p.llm_timeout_ms), `${p.llm_timeout_ms / 1000} seconds`);
    renderEngines();
}
/** Select `value` in a preset list, adding it as a choice when it is not a preset. */
function ensureOption(id, value, label) {
    const sel = $(id);
    if (![...sel.options].some((o) => o.value === value)) {
        const after = [...sel.options].find((o) => Number(o.value) > Number(value));
        sel.add(new Option(label, value), after ?? null);
    }
    sel.value = value;
}
/** Set a field unless the user is typing in it. */
function setField(id, v) {
    const i = $(id);
    if (document.activeElement !== i)
        i.value = v;
}
/** The prefs fields this module owns. Runs after general's collect,
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
export const ai = {
    init() {
        $("llm_enabled").addEventListener("change", showAiEngine);
        $("llm_enabled").addEventListener("change", renderEngines);
        for (const b of document.querySelectorAll("#ai_routes [data-move]")) {
            const r = b.closest("[data-cleanup]").dataset.cleanup;
            b.addEventListener("click", () => moveEngine(r, b.dataset.move === "up" ? -1 : 1));
        }
        const level = $("cleanup_level");
        for (const b of document.querySelectorAll("#level_seg [data-level]")) {
            b.addEventListener("click", () => {
                if (level.value === b.dataset.level)
                    return;
                level.value = b.dataset.level ?? "none";
                // Bubbles to the shell, which saves the prefs.
                level.dispatchEvent(new Event("change", { bubbles: true }));
            });
        }
        level.addEventListener("change", showLevel);
        level.addEventListener("change", showAiEngine);
        // Verbatim and the Auto-format switch are the same switch (D27): keep them in step.
        level.addEventListener("change", () => {
            const fmt = $("spoken_cleanup");
            if (level.value === "verbatim")
                fmt.checked = false;
            else if (!fmt.checked && level.value === "none")
                fmt.checked = true;
        });
        $("spoken_cleanup").addEventListener("change", () => {
            const fmt = $("spoken_cleanup");
            if (!fmt.checked && level.value === "none")
                level.value = "verbatim";
            else if (fmt.checked && level.value === "verbatim")
                level.value = "none";
            showLevel();
        });
        views.onOr(() => {
            showAiEngine();
            renderEngines();
        });
        views.onDev(() => {
            showAiEngine();
            renderEngines();
        });
        prefsStore.subscribe((p) => {
            fill(p);
            showAiEngine();
        });
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
};
