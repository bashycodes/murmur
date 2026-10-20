"use strict";
// Settings window: edits prefs.json through the `get_prefs` / `set_prefs`
// commands. Field names mirror `src/prefs.rs` (snake_case).
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const BOOL_FIELDS = [
    "enable_sounds",
    "mute_audio_while_dictating",
    "launch_at_login",
    "show_flow_bar_always",
    "use_ax_context",
    "raw_mode",
    "restore_clipboard",
    "live_preview",
];
let current = null;
function showModel(st) {
    const el = $("model_status");
    switch (st.state) {
        case "loading":
            el.textContent = `Loading ${st.path}…`;
            el.className = "hint";
            break;
        case "ready":
            el.textContent = `Ready: ${st.path} (loaded in ${st.load_ms} ms)`;
            el.className = "hint ok";
            break;
        case "missing":
            el.textContent = `Model file not found: ${st.path}. Dictation is unavailable until it exists.`;
            el.className = "hint err";
            break;
        case "failed":
            el.textContent = `Model failed to load (${st.path}): ${st.message}`;
            el.className = "hint err";
            break;
    }
}
/** One entry per line: `word`, `spoken => replacement`, `trigger ==> snippet`. */
function parseDictionary(text) {
    const out = [];
    for (const raw of text.split("\n")) {
        const line = raw.trim();
        if (!line)
            continue;
        const snip = line.split("==>");
        if (snip.length === 2) {
            out.push({ word: snip[0].trim(), replacement: snip[1].trim(), isSnippet: true });
            continue;
        }
        const rep = line.split("=>");
        if (rep.length === 2) {
            out.push({ word: rep[0].trim(), replacement: rep[1].trim(), isSnippet: false });
            continue;
        }
        out.push({ word: line, replacement: null, isSnippet: false });
    }
    return out;
}
function formatDictionary(d) {
    return d
        .map((e) => e.replacement ? `${e.word} ${e.isSnippet ? "==>" : "=>"} ${e.replacement}` : e.word)
        .join("\n");
}
async function load() {
    const [prefs, mics, model] = await Promise.all([
        invoke("get_prefs"),
        invoke("list_microphones"),
        invoke("get_model_status"),
    ]);
    current = prefs;
    showModel(model.status);
    $("models_dir").textContent = model.models_dir;
    const mp = $("model_path");
    mp.placeholder = model.default_path;
    mp.value = prefs.model_path ?? "";
    $("paste_last").value = prefs.shortcuts.paste_last;
    $("retention").value = prefs.retention;
    $("ptt").value = prefs.shortcuts.ptt;
    $("hands_free").value = prefs.shortcuts.hands_free;
    $("cancel").value = prefs.shortcuts.cancel;
    const mic = $("microphone");
    mic.innerHTML = "";
    const def = new Option("System default", "");
    mic.add(def);
    for (const m of mics)
        mic.add(new Option(m, m));
    mic.value = prefs.microphone ?? "";
    for (const f of BOOL_FIELDS)
        $(f).checked = Boolean(prefs[f]);
    $("cleanup_level").value = prefs.cleanup_level;
    $("paste_method").value = prefs.paste_method;
    $("languages").value = prefs.languages.join(", ");
    $("max_dictation_minutes").value = String(prefs.max_dictation_minutes);
    $("dictionary").value = formatDictionary(prefs.dictionary);
}
async function save(ev) {
    ev.preventDefault();
    if (!current)
        return;
    const next = { ...current };
    next.shortcuts = {
        ptt: $("ptt").value.trim(),
        hands_free: $("hands_free").value.trim(),
        cancel: $("cancel").value.trim(),
        paste_last: $("paste_last").value.trim(),
    };
    next.model_path = $("model_path").value.trim() || null;
    next.retention = $("retention").value;
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
    next.dictionary = parseDictionary($("dictionary").value);
    const status = $("status");
    try {
        await invoke("set_prefs", { prefs: next });
        current = next;
        status.textContent = "Saved";
        status.className = "ok";
    }
    catch (e) {
        status.textContent = String(e);
        status.className = "err";
    }
}
$("form").addEventListener("submit", (e) => void save(e));
void window.__TAURI__.event.listen("model:status", (e) => showModel(e.payload));
void load();
