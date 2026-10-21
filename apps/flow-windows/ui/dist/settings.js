"use strict";
// Settings window: four tabs over the backend commands in `src/app.rs`.
// * General + AI cleanup edit prefs.json (`get_prefs` / `set_prefs`);
//   field names mirror `src/prefs.rs` (snake_case).
// * History lists the local SQLite transcripts (copy, re-paste, delete,
//   undo AI edit = use the raw ASR text instead of the cleaned one).
// * Dictionary edits the [{word, replacement, isSnippet}] list, shows
//   learn-from-edits suggestions (added only on an explicit click) and
//   imports/exports the shared JSON.
// All user text is inserted with textContent, never innerHTML.
const { invoke } = window.__TAURI__.core;
const onEvent = (name, cb) => window.__TAURI__.event.listen(name, cb);
const $ = (id) => document.getElementById(id);
function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls)
        e.className = cls;
    if (text !== undefined)
        e.textContent = text;
    return e;
}
function button(label, onClick, cls = "small") {
    const b = el("button", cls, label);
    b.type = "button";
    b.addEventListener("click", onClick);
    return b;
}
function setStatus(id, text, ok) {
    const s = $(id);
    s.textContent = text;
    s.className = ok ? "ok" : "err";
}
const BOOL_FIELDS = [
    "enable_sounds",
    "mute_audio_while_dictating",
    "launch_at_login",
    "show_flow_bar_always",
    "use_ax_context",
    "raw_mode",
    "restore_clipboard",
    "live_preview",
    "llm_enabled",
    "learn_from_edits",
    "spoken_cleanup",
];
const STYLES = [
    ["formal", "Formal — capitals, full punctuation"],
    ["casual", "Casual — capitals, lighter punctuation"],
    ["very_casual", "Very casual — no caps, less punctuation"],
    ["excited", "Excited — more exclamations!"],
];
const BUCKETS = ["personal", "work", "email", "other"];
let current = null;
// ------------------------------------------------------------------ tabs
function showTab(name) {
    for (const b of document.querySelectorAll("nav.tabs button")) {
        b.setAttribute("aria-selected", String(b.dataset.tab === name));
    }
    for (const p of document.querySelectorAll("[data-panel]")) {
        p.hidden = p.dataset.panel !== name;
    }
    $("form").hidden = name !== "general" && name !== "ai";
    if (name === "history")
        void loadHistory();
    if (name === "dictionary")
        void loadSuggestions();
}
// ------------------------------------------------------------------ status lines
function showModel(st) {
    const e = $("model_status");
    switch (st.state) {
        case "loading":
            e.textContent = `Loading ${st.path}…`;
            e.className = "hint";
            break;
        case "ready":
            e.textContent = `Ready: ${st.path} (loaded in ${st.load_ms} ms)`;
            e.className = "hint ok";
            break;
        case "missing":
            e.textContent = `Model file not found: ${st.path}. Dictation is unavailable until it exists.`;
            e.className = "hint err";
            break;
        case "failed":
            e.textContent = `Model failed to load (${st.path}): ${st.message}`;
            e.className = "hint err";
            break;
    }
}
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
// ------------------------------------------------------------------ general + AI form
function showEngineFields() {
    const engine = $("asr_engine").value;
    for (const l of document.querySelectorAll("[data-engine]")) {
        l.hidden = l.dataset.engine !== engine;
    }
}
function fillForm(prefs) {
    $("model_path").value = prefs.model_path ?? "";
    $("asr_engine").value = prefs.asr_engine;
    $("parakeet_model_dir").value = prefs.parakeet_model_dir ?? "";
    showEngineFields();
    $("ptt").value = prefs.shortcuts.ptt;
    $("hands_free").value = prefs.shortcuts.hands_free;
    $("cancel").value = prefs.shortcuts.cancel;
    $("paste_last").value = prefs.shortcuts.paste_last;
    $("copy_last").value = prefs.shortcuts.copy_last;
    $("microphone").value = prefs.microphone ?? "";
    for (const f of BOOL_FIELDS)
        $(f).checked = Boolean(prefs[f]);
    $("cleanup_level").value = prefs.cleanup_level;
    $("paste_method").value = prefs.paste_method;
    $("languages").value = prefs.languages.join(", ");
    $("max_dictation_minutes").value = String(prefs.max_dictation_minutes);
    $("llm_model_path").value = prefs.llm_model_path ?? "";
    $("llm_lib_path").value = prefs.llm_lib_path ?? "";
    $("llm_timeout_ms").value = String(prefs.llm_timeout_ms);
    for (const b of BUCKETS) {
        const v = prefs.styles[b].toLowerCase().replace(/[ -]/g, "_");
        $(`style_${b}`).value = STYLES.some(([k]) => k === v) ? v : "formal";
    }
    $("retention").value = prefs.retention;
}
function readForm(base) {
    const next = { ...base };
    next.shortcuts = {
        ptt: $("ptt").value.trim(),
        hands_free: $("hands_free").value.trim(),
        cancel: $("cancel").value.trim(),
        paste_last: $("paste_last").value.trim(),
        copy_last: $("copy_last").value.trim(),
    };
    next.model_path = $("model_path").value.trim() || null;
    next.asr_engine = $("asr_engine").value;
    next.parakeet_model_dir = $("parakeet_model_dir").value.trim() || null;
    next.microphone = $("microphone").value || null;
    for (const f of BOOL_FIELDS)
        next[f] = $(f).checked;
    next.cleanup_level = $("cleanup_level").value;
    next.paste_method = $("paste_method").value;
    next.languages = $("languages")
        .value.split(",")
        .map((s) => s.trim())
        .filter(Boolean);
    next.max_dictation_minutes = Math.max(1, parseInt($("max_dictation_minutes").value, 10) || 20);
    next.llm_model_path = $("llm_model_path").value.trim() || null;
    next.llm_lib_path = $("llm_lib_path").value.trim() || null;
    next.llm_timeout_ms = Math.min(30000, Math.max(200, parseInt($("llm_timeout_ms").value, 10) || 1500));
    next.styles = {
        personal: $("style_personal").value,
        work: $("style_work").value,
        email: $("style_email").value,
        other: $("style_other").value,
    };
    return next;
}
/** Save `next` and make it the base for every tab. */
async function savePrefs(next) {
    await invoke("set_prefs", { prefs: next });
    current = next;
}
async function saveForm(ev) {
    ev.preventDefault();
    if (!current)
        return;
    try {
        await savePrefs(readForm(current));
        setStatus("status", "Saved", true);
    }
    catch (e) {
        setStatus("status", String(e), false);
    }
}
// ------------------------------------------------------------------ history
/** Word-level LCS diff: [kind, word] with kind "=", "-" (raw only), "+" (final only). */
function wordDiff(a, b) {
    const x = a.split(/\s+/).filter(Boolean);
    const y = b.split(/\s+/).filter(Boolean);
    const n = x.length;
    const m = y.length;
    const dp = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
    for (let i = n - 1; i >= 0; i--)
        for (let j = m - 1; j >= 0; j--)
            dp[i][j] = x[i] === y[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    const out = [];
    let i = 0;
    let j = 0;
    while (i < n && j < m) {
        if (x[i] === y[j]) {
            out.push(["=", x[i]]);
            i++;
            j++;
        }
        else if (dp[i + 1][j] >= dp[i][j + 1])
            out.push(["-", x[i++]]);
        else
            out.push(["+", y[j++]]);
    }
    while (i < n)
        out.push(["-", x[i++]]);
    while (j < m)
        out.push(["+", y[j++]]);
    return out;
}
function diffView(raw, final) {
    const p = el("p", "diff");
    for (const [kind, w] of wordDiff(raw, final)) {
        const tag = kind === "-" ? "del" : kind === "+" ? "ins" : "span";
        p.append(el(tag, undefined, w), " ");
    }
    return p;
}
function when(ts) {
    const d = new Date(ts);
    const today = new Date();
    return d.toDateString() === today.toDateString()
        ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
        : d.toLocaleString([], { dateStyle: "medium", timeStyle: "short" });
}
async function act(what, okText) {
    try {
        await what;
        setStatus("hist_status", okText, true);
    }
    catch (e) {
        setStatus("hist_status", String(e), false);
    }
}
function historyRow(h) {
    const li = el("li");
    const meta = el("div", "meta");
    meta.append(el("span", undefined, when(h.ts)));
    if (h.app)
        meta.append(el("span", undefined, h.app.replace(/\.exe$/i, "")));
    meta.append(el("span", undefined, `${h.words} words`));
    if (h.ai_edited)
        meta.append(el("span", "chip", "AI edited"));
    if (h.edited)
        meta.append(el("span", "chip", "you corrected it"));
    if (h.pasted === null)
        meta.append(el("span", "chip err", "not pasted"));
    li.append(meta, el("p", "text", h.final_text));
    const actions = el("div", "actions");
    actions.append(button("Copy", () => void act(invoke("history_copy", { id: h.id, raw: false }), "Copied")), button("Re-paste", () => void act(invoke("history_repaste", { id: h.id, raw: false }), "Pasted")));
    if (h.ai_edited) {
        const undo = el("div", "undo");
        undo.hidden = true;
        undo.append(el("p", "hint", "What you said (struck = removed by AI, underlined = added):"), diffView(h.raw, h.final_text), button("Re-paste original", () => void act(invoke("history_repaste", { id: h.id, raw: true }), "Pasted the original")), button("Copy original", () => void act(invoke("history_copy", { id: h.id, raw: true }), "Copied the original")));
        actions.append(button("Undo AI edit…", () => {
            undo.hidden = !undo.hidden;
        }));
        li.append(actions, undo);
    }
    else {
        li.append(actions);
    }
    if (h.edited) {
        const c = el("p", "hint");
        c.append("Your correction: ", el("q", undefined, h.edited));
        li.append(c);
    }
    actions.append(button("Delete", () => void (async () => {
        try {
            await invoke("history_delete", { id: h.id });
            li.remove();
        }
        catch (e) {
            setStatus("hist_status", String(e), false);
        }
    })(), "small danger"));
    return li;
}
let histSeq = 0;
async function loadHistory() {
    const seq = ++histSeq;
    try {
        const rows = await invoke("history_list", {
            query: $("hist_query").value,
            limit: 200,
        });
        if (seq !== histSeq)
            return; // a newer search is in flight
        const list = $("hist_list");
        list.replaceChildren(...rows.map(historyRow));
        $("hist_empty").hidden = rows.length > 0;
    }
    catch (e) {
        setStatus("hist_status", String(e), false);
    }
}
let clearArmed = 0;
async function clearHistory() {
    const b = $("hist_clear");
    if (Date.now() - clearArmed > 3000) {
        clearArmed = Date.now();
        b.textContent = "Click again to delete all";
        setTimeout(() => (b.textContent = "Delete all"), 3000);
        return;
    }
    clearArmed = 0;
    b.textContent = "Delete all";
    await act(invoke("history_clear"), "History deleted");
    await loadHistory();
}
async function saveRetention() {
    if (!current)
        return;
    try {
        await savePrefs({ ...current, retention: $("retention").value });
        setStatus("hist_status", "Saved", true);
        await loadHistory();
    }
    catch (e) {
        setStatus("hist_status", String(e), false);
    }
}
// ------------------------------------------------------------------ dictionary
let dict = [];
function dictRow(e, idx) {
    const tr = el("tr");
    const word = el("input");
    word.value = e.word;
    word.spellcheck = false;
    word.addEventListener("input", () => (dict[idx].word = word.value));
    const rep = el("textarea");
    rep.rows = 1;
    rep.value = e.replacement ?? "";
    rep.placeholder = "(vocabulary word)";
    rep.addEventListener("input", () => (dict[idx].replacement = rep.value || null));
    const snip = el("input");
    snip.type = "checkbox";
    snip.checked = Boolean(e.isSnippet);
    snip.title = "Snippet: the trigger inserts this text";
    snip.addEventListener("change", () => (dict[idx].isSnippet = snip.checked));
    const del = button("✕", () => {
        dict.splice(idx, 1);
        renderDict();
    }, "small danger");
    del.title = "Remove";
    for (const c of [word, rep, snip, del]) {
        const td = el("td");
        td.append(c);
        tr.append(td);
    }
    return tr;
}
function renderDict() {
    const f = $("dict_filter").value.trim().toLowerCase();
    const rows = [];
    dict.forEach((e, i) => {
        if (!f || e.word.toLowerCase().includes(f) || (e.replacement ?? "").toLowerCase().includes(f))
            rows.push(dictRow(e, i));
    });
    $("dict_rows").replaceChildren(...rows);
}
async function saveDict() {
    if (!current)
        return;
    try {
        await savePrefs({ ...current, dictionary: dict.map((e) => ({ ...e })) });
        // The backend normalises (trims, drops blanks, collapses duplicates).
        current = await invoke("get_prefs");
        dict = current.dictionary.map((e) => ({ ...e }));
        renderDict();
        setStatus("dict_status", "Saved", true);
        await loadSuggestions();
    }
    catch (e) {
        setStatus("dict_status", String(e), false);
    }
}
async function importFile() {
    const input = $("dict_file");
    const file = input.files?.[0];
    if (!file)
        return;
    try {
        const json = await file.text();
        const r = await invoke("dictionary_import", {
            entries: dict,
            json,
            replace: $("dict_mode").value === "replace",
        });
        dict = r.entries;
        renderDict();
        const skipped = r.skipped ? `, ${r.skipped} invalid skipped` : "";
        setStatus("io_status", `${r.added} added, ${r.updated} updated${skipped}. Press "Save dictionary" to keep them.`, true);
    }
    catch (e) {
        setStatus("io_status", String(e), false);
    }
    finally {
        input.value = "";
    }
}
async function exportFile() {
    try {
        const path = await invoke("dictionary_export", { entries: dict });
        setStatus("io_status", `Saved ${dict.length} entries to ${path}`, true);
    }
    catch (e) {
        setStatus("io_status", String(e), false);
    }
}
async function copyJson() {
    try {
        const text = await invoke("dictionary_export_text", { entries: dict });
        await invoke("clipboard_copy", { text });
        setStatus("io_status", "JSON copied to the clipboard", true);
    }
    catch (e) {
        setStatus("io_status", String(e), false);
    }
}
// ------------------------------------------------------------------ suggestions
function showSuggestionCount(n) {
    const b = $("sugg_badge");
    b.hidden = n === 0;
    b.textContent = String(n);
}
async function loadSuggestions() {
    const list = await invoke("suggestions_list");
    showSuggestionCount(list.length);
    $("sugg_box").hidden = list.length === 0;
    $("sugg_list").replaceChildren(...list.map((s) => {
        const li = el("li");
        const what = el("span", "what");
        what.append(el("q", undefined, s.heard), " → ", el("strong", undefined, s.word));
        const meta = el("span", "hint", ` ${s.app.replace(/\.exe$/i, "")}${s.count > 1 ? ` ×${s.count}` : ""}`);
        const accept = async (asReplacement) => {
            try {
                current = await invoke("suggestion_accept", { word: s.word, asReplacement });
                dict = current.dictionary.map((e) => ({ ...e }));
                renderDict();
                setStatus("dict_status", `Added "${s.word}"`, true);
            }
            catch (e) {
                setStatus("dict_status", String(e), false);
            }
            await loadSuggestions();
        };
        li.append(what, meta, button("Add word", () => void accept(false)), button("Add as replacement", () => void accept(true), "small secondary"), button("Dismiss", () => void invoke("suggestion_dismiss", { word: s.word }).then(loadSuggestions), "small secondary"));
        return li;
    }));
}
// ------------------------------------------------------------------ boot
async function load() {
    const [prefs, mics, model] = await Promise.all([
        invoke("get_prefs"),
        invoke("list_microphones"),
        invoke("get_model_status"),
    ]);
    current = prefs;
    showModel(model.status);
    showLlm(model.llm);
    $("models_dir").textContent = model.models_dir;
    $("llm_plugin").textContent = model.llm_plugin;
    $("model_path").placeholder = model.default_path;
    $("parakeet_model_dir").placeholder = model.default_parakeet_dir;
    $("llm_model_path").placeholder = model.default_llm_path;
    const mic = $("microphone");
    mic.replaceChildren(new Option("System default", ""), ...mics.map((m) => new Option(m, m)));
    for (const b of BUCKETS) {
        $(`style_${b}`).replaceChildren(...STYLES.map(([v, label]) => new Option(label, v)));
    }
    fillForm(prefs);
    dict = prefs.dictionary.map((e) => ({ ...e }));
    renderDict();
    await loadSuggestions();
}
for (const b of document.querySelectorAll("nav.tabs button")) {
    b.addEventListener("click", () => showTab(b.dataset.tab ?? "general"));
}
$("form").addEventListener("submit", (e) => void saveForm(e));
$("asr_engine").addEventListener("change", showEngineFields);
let searchTimer = 0;
$("hist_query").addEventListener("input", () => {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => void loadHistory(), 200);
});
$("hist_clear").addEventListener("click", () => void clearHistory());
$("retention").addEventListener("change", () => void saveRetention());
$("dict_filter").addEventListener("input", renderDict);
$("dict_add").addEventListener("click", () => {
    $("dict_filter").value = "";
    dict.unshift({ word: "", replacement: null, isSnippet: false });
    renderDict();
    $("dict_rows").querySelector("input")?.focus();
});
$("dict_save").addEventListener("click", () => void saveDict());
$("dict_file").addEventListener("change", () => void importFile());
$("dict_export").addEventListener("click", () => void exportFile());
$("dict_copy").addEventListener("click", () => void copyJson());
void onEvent("model:status", (e) => showModel(e.payload));
void onEvent("llm:status", (e) => showLlm(e.payload));
void onEvent("suggestions:changed", () => void loadSuggestions());
void onEvent("settings:tab", (e) => showTab(e.payload));
void onEvent("history:changed", () => {
    if (!$("hist_list").closest("[hidden]"))
        void loadHistory();
});
showTab("general");
void load();
