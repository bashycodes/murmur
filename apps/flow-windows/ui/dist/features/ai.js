// AI cleanup: the local LLM, the cleanup level and (D36) the cleanup
// order — this PC, your server, OpenRouter, each with its switch — on the
// AI cleanup settings page (design: SettingsCleanup.dc.html). Same widget
// as the speech-to-text order (features/engines.ts). The order and the
// server switch save with `dev_cleanup_set_engines`; this PC's switch is
// the `llm_enabled` pref; OpenRouter's switch (with its consent) belongs to
// features/openrouter.ts.
import { $, el, setStatus } from "../lib/dom.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { prefsStore } from "../lib/prefs-store.js";
import { views } from "../lib/views.js";
import { showOr } from "./openrouter.js";
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
            return `${fileStem(st.path)}. Private; ${where(st.backend)}loaded in ${(st.load_ms / 1000).toFixed(1)} s.`;
        case "missing":
            return `Model file not found: ${st.path}. Dictation keeps working without cleanup.`;
        case "plugin_missing":
            return `The cleanup engine is missing next to the app: ${st.message}`;
        case "failed":
            return `Model failed to load (${st.path}): ${st.message}`;
    }
}
/** "on the GPU (Intel Arc 140V); " / "on the CPU; " / "" (older engine). */
function where(b) {
    if (!b)
        return "";
    if (b.backend.toLowerCase() !== "cpu")
        return `on the graphics chip (${b.device}); `;
    return b.gpu_error ? "on the CPU (the graphics chip failed to load it); " : "on the CPU; ";
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
/** The D36 default, until the backend's view arrives. */
const DEFAULT_ENGINES = [
    { route: "local", enabled: true },
    { route: "server", enabled: true },
    { route: "cloud", enabled: false },
];
function engines() {
    return views.or?.cleanup.engines ?? DEFAULT_ENGINES;
}
/** The engine is in this build (its row is shown). */
function available(r) {
    if (r === "cloud")
        return !!views.or?.supported;
    if (r === "local")
        return !!views.dev?.caps.local;
    return !!views.dev?.caps.remote;
}
/** The engine's switch is on. */
function engineOn(r) {
    if (r === "local")
        return $("llm_enabled").checked;
    if (r === "cloud")
        return !!views.or?.cleanup.allow;
    return engines().some((e) => e.route === r && e.enabled);
}
/** The server could clean now: on, set up, and not known to lack an AI model. */
function serverUsable() {
    return engineOn("server") && !!views.dev?.form.host && views.or?.cleanup.status.server_llm !== false;
}
function setBadge(r, text, kind) {
    const b = $(`aieng_badge_${r}`);
    b.textContent = text;
    b.className = `fl-badge fl-badge-${kind}`;
}
function goLink(page, text) {
    const b = el("button", "lw-link", text);
    b.type = "button";
    b.dataset.go = page;
    return b;
}
/** One line + badge per engine: what it is and whether it can clean now. */
function describe(r) {
    const on = engineOn(r);
    if (r === "local") {
        if (!on)
            setBadge(r, "Off", "light");
        else if (llm.state === "ready")
            setBadge(r, "Ready", "success");
        else if (llm.state === "loading" || llm.state === "off")
            setBadge(r, "Loading", "light");
        else if (llm.state === "plugin_missing")
            setBadge(r, "Not installed", "destructive");
        else if (llm.state === "missing")
            setBadge(r, "Model missing", "destructive");
        else
            setBadge(r, "Failed", "destructive");
    }
    else if (r === "server") {
        const host = views.dev?.form.host;
        const st = views.or?.cleanup.status;
        const d = $("aieng_desc_server");
        if (!host)
            d.replaceChildren("Not set up yet. ", goLink("settings-server", "Set up →"));
        else if (st?.server_llm === false)
            d.textContent = `${host} has no AI cleanup model: skipped.`;
        else
            d.textContent = `${host}. Cleans the text with its own AI model.`;
        if (!on)
            setBadge(r, "Off", "light");
        else if (!host)
            setBadge(r, "Not set up", "warning");
        else if (st?.server_llm === false)
            setBadge(r, "No AI model", "warning");
        else if (st?.server_circuit?.open)
            setBadge(r, "Paused", "warning");
        else if (st?.server_llm === true)
            setBadge(r, "Ready", "success");
        else
            setBadge(r, "On", "light");
    }
    else {
        const v = views.or;
        $("aieng_desc_cloud").textContent = on
            ? `Sends the transcript as text to openrouter.ai · ${(v?.cleanup.model ?? "").split("/").pop()}.`
            : "Sends the transcript as text to openrouter.ai. Asks first.";
        const st = v?.cleanup.status;
        if (!on)
            setBadge(r, "Off", "light");
        else if (st?.blocked)
            setBadge(r, "Not set up", "warning");
        else if (st?.circuit?.open)
            setBadge(r, "Paused", "warning");
        else
            setBadge(r, "On", "success");
    }
}
/** The cleanup order: rows in the saved order, ranked among the shown ones. */
function renderEngines() {
    const order = engines().map((e) => e.route);
    const shown = order.filter(available);
    const list = $("ai_routes");
    const foot = list.querySelector(".lw-foot-row");
    for (const r of order)
        list.insertBefore($(`aieng_row_${r}`), foot);
    $("aieng_server_on").checked = engineOn("server");
    shown.forEach((r, i) => {
        const row = $(`aieng_row_${r}`);
        row.classList.toggle("is-off", !engineOn(r));
        row.querySelector(".lw-rank").textContent = String(i + 1);
        row.querySelector("[data-move=up]").disabled = i === 0;
        row.querySelector("[data-move=down]").disabled = i === shown.length - 1;
        row.querySelector(".lw-move").hidden = shown.length < 2;
        describe(r);
    });
}
/** Save the order and the server switch; the answer is shown everywhere. */
async function saveEngines(list) {
    try {
        showOr(await invoke("dev_cleanup_set_engines", { engines: list }));
        $("cleanup_order_msg").textContent = "";
    }
    catch (e) {
        setStatus("cleanup_order_msg", String(e), false);
        renderEngines();
    }
}
/** Swap with the next shown engine up or down. */
function moveEngine(r, dir) {
    const list = engines().map((e) => ({ ...e }));
    const shown = list.filter((e) => available(e.route));
    const i = shown.findIndex((e) => e.route === r);
    const other = shown[i + dir];
    if (i < 0 || !other)
        return;
    const a = list.indexOf(shown[i]);
    const b = list.indexOf(other);
    [list[a], list[b]] = [list[b], list[a]];
    void saveEngines(list);
}
function serverToggled() {
    const on = $("aieng_server_on").checked;
    void saveEngines(engines().map((e) => (e.route === "server" ? { ...e, enabled: on } : { ...e })));
}
/** D27: cleanup "None" = No AI (formatting still runs); Verbatim = Auto-format off too. */
function cleanupChoice(p) {
    return p.cleanup_level === "none" && !p.spoken_cleanup ? "verbatim" : p.cleanup_level;
}
/** Which engines clean the text, in order (switches + OrView). */
function showAiEngine() {
    const level = $("cleanup_level").value;
    const name = {
        local: "this PC",
        server: "your server",
        cloud: `OpenRouter (${views.or?.cleanup.model ?? ""})`,
    };
    const used = engines()
        .map((e) => e.route)
        .filter((r) => available(r) && (r === "server" ? serverUsable() : engineOn(r)));
    let text;
    if (level === "none" || level === "verbatim")
        text = `no AI (the cleanup level is ${level === "none" ? "Rules only" : "Verbatim"})`;
    else if (used.length === 0)
        text = "the formatting rules only";
    else {
        const t = used.map((r) => name[r]).join(", then ");
        text = t.charAt(0).toUpperCase() + t.slice(1);
    }
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
/** While the page is on screen the server's answers can change (it was
 *  upgraded, restarted, its circuit closed): re-read the cleanup status.
 *  Cheap: the backend only reconnects when a cached "no AI model" expired. */
const STATUS_POLL_MS = 10000;
let statusPoll;
async function pollCleanupStatus() {
    const v = views.or;
    if (!v || document.hidden)
        return;
    let st;
    try {
        st = await invoke("dev_or_cleanup_status");
    }
    catch {
        return; // older backend
    }
    const prev = v.cleanup.status;
    if (prev.server_llm === st.server_llm && !!prev.server_circuit?.open === !!st.server_circuit?.open)
        return;
    if (views.or !== v)
        return; // a newer view arrived meanwhile
    views.setOr({ ...v, cleanup: { ...v.cleanup, status: st } });
}
export const ai = {
    init() {
        $("llm_enabled").addEventListener("change", showAiEngine);
        $("llm_enabled").addEventListener("change", renderEngines);
        $("aieng_server_on").addEventListener("change", serverToggled);
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
    shown(page, opened) {
        if (page !== "settings-cleanup") {
            clearInterval(statusPoll);
            statusPoll = undefined;
            return;
        }
        if (opened)
            void pollCleanupStatus();
        statusPoll ?? (statusPoll = setInterval(() => void pollCleanupStatus(), STATUS_POLL_MS));
    },
};
