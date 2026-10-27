"use strict";
// Settings window: tabs over the backend commands in `src/app.rs`.
// * General + AI cleanup edit prefs.json (`get_prefs` / `set_prefs`);
//   field names mirror `src/prefs.rs` (snake_case).
// * History lists the local SQLite transcripts (copy, re-paste, delete,
//   undo AI edit = use the raw ASR text instead of the cleaned one).
// * Dictionary edits the [{word, replacement, isSnippet}] list, shows
//   learn-from-edits suggestions (added only on an explicit click) and
//   imports/exports the shared JSON.
// * OpenRouter (D28) is the only cloud: key, privacy, speech-to-text,
//   text cleanup, model picker and usage (`dev_or_*` commands).
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
    "restore_clipboard",
    "live_preview",
    "llm_enabled",
    "learn_from_edits",
    "spoken_cleanup",
];
const STYLES = [
    ["formal", "Formal — capitals and full punctuation"],
    ["casual", "Casual — capitals, no final period"],
    ["very_casual", "Very casual — no final period; lowercase with AI cleanup"],
    ["excited", "Excited — more exclamation marks (AI cleanup)"],
];
/** Example line per style (shared with Android's Writing style page). */
const STYLE_PREVIEW = {
    formal: "“Sounds good. See you at 7:30.”",
    casual: "“Sounds good. See you at 7:30”",
    very_casual: "“sounds good. see you at 7:30”",
    excited: "“Sounds good! See you at 7:30!”",
};
const BUCKETS = ["personal", "work", "email", "other"];
const BUCKET_NAMES = ["personal", "work", "email", "other"];
function showStylePreview(b) {
    const v = $(`style_${b}`).value;
    $(`style_${b}_preview`).textContent = STYLE_PREVIEW[v] ?? "";
}
const BUCKET_LABELS = {
    personal: "Personal",
    work: "Work",
    email: "Email",
    other: "Other",
};
let appRows = [];
let styleChoices = { recent: [], defaults: [] };
/** Mirrors `context::bucket_key`: `Slack.exe` -> `slack`, `www.X.com` -> `x.com`. */
function appKey(s) {
    let k = s.trim().toLowerCase();
    if (k.endsWith(".exe"))
        k = k.slice(0, -4);
    if (k.startsWith("www."))
        k = k.slice(4);
    return k;
}
/** Manual entry -> key to store, or an error. Accepts an exe name, a full
 *  exe path, a web host or a URL (its host is used). */
function parseAppInput(raw) {
    let s = raw.trim();
    const url = /^[a-z][a-z0-9+.-]*:\/\/([^/?#]*)/i.exec(s);
    if (url)
        s = url[1].replace(/^.*@/, "").replace(/:\d+$/, "");
    else if (/[\\/]/.test(s))
        s = s.split(/[\\/]/).pop() ?? "";
    const key = appKey(s);
    if (!key)
        return { error: "Type an exe name (mychat.exe) or a web host (mail.example.com)" };
    if (key.includes("="))
        return { error: "An app name can't contain “=”" };
    return { key };
}
function choiceFor(key) {
    const k = appKey(key);
    return (styleChoices.recent.find((c) => c.key === k) ?? styleChoices.defaults.find((c) => c.key === k));
}
function hasAppRow(key) {
    const k = appKey(key);
    return appRows.some((r) => appKey(r.key) === k);
}
/** "Slack  slack.exe" (name when known, the stored key otherwise). */
function appCell(key, name, exe) {
    const td = el("td");
    td.append(el("span", "", name || exe || key));
    if (name)
        td.append(el("span", "exe", exe ?? key));
    return td;
}
function bucketSelect(value, onChange) {
    const sel = el("select");
    sel.setAttribute("aria-label", "Group");
    for (const b of BUCKETS)
        sel.append(new Option(BUCKET_LABELS[b], b));
    sel.value = BUCKET_NAMES.includes(value) ? value : "other";
    sel.addEventListener("change", () => onChange(sel.value));
    return sel;
}
function addAppRow(key, bucket) {
    setStatus("app_add_status", "", true);
    if (hasAppRow(key)) {
        setStatus("app_add_status", "Already in the list", false);
        return;
    }
    appRows.push({ key, bucket });
    renderAppRows();
    const sels = $("app_rows").querySelectorAll("select");
    sels[appRows.length - 1]?.focus();
}
function renderAppRows() {
    const body = $("app_rows");
    body.replaceChildren(...appRows.map((row, i) => {
        const c = choiceFor(row.key);
        const label = c?.name ?? row.key;
        const del = button("Remove", () => {
            appRows.splice(i, 1);
            renderAppRows();
        }, "small danger");
        del.setAttribute("aria-label", `Remove ${label}`);
        const group = el("td");
        group.append(bucketSelect(row.bucket, (v) => (row.bucket = v)));
        const act = el("td");
        act.append(del);
        const tr = el("tr");
        tr.append(appCell(row.key, c?.name, c?.exe), group, act);
        return tr;
    }));
    $("app_rows_empty").hidden = appRows.length > 0;
    renderRecentPicker();
    renderDefaultRows();
}
function renderRecentPicker() {
    const sel = $("app_recent");
    const free = styleChoices.recent.filter((c) => !hasAppRow(c.key));
    if (free.length === 0) {
        const o = new Option(styleChoices.recent.length
            ? "All recent apps are in the list"
            : "No recent apps yet — dictate into an app first", "");
        sel.replaceChildren(o);
        sel.disabled = true;
    }
    else {
        sel.replaceChildren(...free.map((c) => new Option(`${c.name ? `${c.name} (${c.exe})` : c.exe} — built-in: ${BUCKET_LABELS[c.default_bucket] ?? "Other"}`, c.key)));
        sel.disabled = false;
    }
    $("app_add_recent").disabled = free.length === 0;
}
function renderDefaultRows() {
    const defs = styleChoices.defaults;
    $("app_defaults_count").textContent = String(defs.length);
    $("app_default_rows").replaceChildren(...defs.map((c) => {
        const tr = el("tr");
        const mine = hasAppRow(c.key);
        if (mine)
            tr.className = "overridden";
        const group = el("td");
        group.append(el("span", "tag", `default: ${BUCKET_LABELS[c.default_bucket] ?? c.default_bucket}`));
        const act = el("td");
        if (mine)
            act.append(el("span", "hint", "yours above"));
        else
            act.append(button("Change", () => addAppRow(c.key, c.default_bucket), "small secondary"));
        tr.append(appCell(c.key, c.name, c.exe), group, act);
        return tr;
    }));
}
function fillAppRows(m) {
    appRows = Object.entries(m ?? {})
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([key, bucket]) => ({ key, bucket }));
    renderAppRows();
}
function readAppRows() {
    const out = {};
    for (const r of appRows)
        out[r.key] = r.bucket;
    return out;
}
async function loadAppChoices() {
    try {
        styleChoices = await invoke("app_style_choices");
    }
    catch {
        styleChoices = { recent: [], defaults: [] };
    }
    renderAppRows();
}
function addRecentApp() {
    const key = $("app_recent").value;
    const c = styleChoices.recent.find((r) => r.key === key);
    if (c)
        addAppRow(c.key, c.default_bucket);
}
function addManualApp() {
    const input = $("app_manual");
    const r = parseAppInput(input.value);
    if ("error" in r) {
        setStatus("app_add_status", r.error, false);
        return;
    }
    const before = appRows.length;
    addAppRow(r.key, choiceFor(r.key)?.default_bucket ?? "other");
    if (appRows.length > before)
        input.value = "";
}
/** D27: cleanup "None" = No AI (formatting still runs); Verbatim = Auto-format off too. */
function cleanupChoice(p) {
    return p.cleanup_level === "none" && !p.spoken_cleanup ? "verbatim" : p.cleanup_level;
}
let current = null;
// ------------------------------------------------------------------ tabs
let activeTab = "";
function showTab(name) {
    const opened = name !== activeTab;
    activeTab = name;
    for (const b of document.querySelectorAll("nav.tabs button")) {
        b.setAttribute("aria-selected", String(b.dataset.tab === name));
    }
    for (const p of document.querySelectorAll("[data-panel]")) {
        p.hidden = p.dataset.panel !== name;
    }
    $("form").hidden = name !== "general" && name !== "ai";
    if (name === "history")
        void loadHistory();
    if (name === "developer")
        void loadDev();
    if (name === "dictionary")
        void loadSuggestions();
    // D28 fetch policy: the catalogue is refreshed (network) once per opening of this tab.
    if (name === "openrouter" && opened)
        void openOrTab();
    if (name === "ai") {
        showAiEngine();
        void loadAppChoices();
    }
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
    $("cleanup_level").value = cleanupChoice(prefs);
    $("paste_method").value = prefs.paste_method;
    $("languages").value = prefs.languages.join(", ");
    $("max_dictation_minutes").value = String(prefs.max_dictation_minutes);
    $("hands_free_silence_s").value =
        prefs.hands_free_silence_ms > 0 ? String(prefs.hands_free_silence_ms / 1000) : "0";
    $("llm_model_path").value = prefs.llm_model_path ?? "";
    $("llm_lib_path").value = prefs.llm_lib_path ?? "";
    $("llm_timeout_ms").value = String(prefs.llm_timeout_ms);
    for (const b of BUCKETS) {
        const v = prefs.styles[b].toLowerCase().replace(/[ -]/g, "_");
        $(`style_${b}`).value = STYLES.some(([k]) => k === v) ? v : "formal";
        showStylePreview(b);
    }
    fillAppRows(prefs.app_buckets);
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
    const level = $("cleanup_level").value;
    next.cleanup_level = level === "verbatim" ? "none" : level;
    if (level === "verbatim")
        next.spoken_cleanup = false;
    // Raw mode only ever skipped the LLM; "No AI" says that now (D27).
    next.raw_mode = false;
    next.paste_method = $("paste_method").value;
    next.languages = $("languages")
        .value.split(",")
        .map((s) => s.trim())
        .filter(Boolean);
    next.max_dictation_minutes = Math.max(1, parseInt($("max_dictation_minutes").value, 10) || 20);
    // Hands-free auto-stop (D19): 0 = off, else 0.3-10 s of silence.
    const hf = parseFloat($("hands_free_silence_s").value);
    next.hands_free_silence_ms =
        !Number.isFinite(hf) || hf <= 0 ? 0 : Math.round(Math.min(10, Math.max(0.3, hf)) * 1000);
    next.llm_model_path = $("llm_model_path").value.trim() || null;
    next.llm_lib_path = $("llm_lib_path").value.trim() || null;
    next.llm_timeout_ms = Math.min(30000, Math.max(200, parseInt($("llm_timeout_ms").value, 10) || 3000));
    next.styles = {
        personal: $("style_personal").value,
        work: $("style_work").value,
        email: $("style_email").value,
        other: $("style_other").value,
    };
    next.app_buckets = readAppRows();
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
const ROUTE_LABEL = {
    server: "Server",
    client: "This PC (client)",
    cloud: "OpenRouter speech-to-text (cloud)",
};
const DEV_NUM = ["port", "connect_timeout_ms", "commit_timeout_base_ms", "health_ttl_s"];
const DEV_TEXT = ["host", "tls_pin_sha256", "token"];
const DEV_BOOL = [
    "allow_server",
    "allow_client",
    "tls",
    "keep_local_warm",
    "show_engine_badge",
    "decode_while_speaking",
];
let devView = null;
let devOrder = ["server", "client", "cloud"];
let devValid = true;
const HINTS = {
    auth_failed: "token rejected — copy it again from the server's token file",
    connect_failed: "is flow-server running / Tailscale up?",
    proto_mismatch: "client and server protocol versions differ — update one of them",
    server_not_configured: "enter the server host first",
    timeout: "the server is too slow or stuck",
    busy: "the server is busy with another dictation",
};
function hint(code) {
    return code ? (HINTS[code] ?? "") : "";
}
function readDev() {
    const base = devView?.form;
    const f = { ...base };
    for (const k of DEV_BOOL)
        f[k] = $(`dev_${k}`).checked;
    for (const k of DEV_TEXT)
        f[k] = $(`dev_${k}`).value;
    for (const k of DEV_NUM) {
        const v = $(`dev_${k}`).value.trim();
        f[k] = v === "" ? -1 : Number(v);
    }
    f.route_order = [...devOrder];
    f.parakeet_decoding = $("dev_parakeet_decoding").value || f.parakeet_decoding || "auto";
    // D28: the cloud settings live on the OpenRouter tab; never sent from here.
    delete f.cloud;
    delete f.allow_cloud;
    return f;
}
function fillDev(f) {
    for (const k of DEV_BOOL)
        $(`dev_${k}`).checked = f[k];
    for (const k of DEV_TEXT)
        $(`dev_${k}`).value = f[k];
    for (const k of DEV_NUM)
        $(`dev_${k}`).value = String(f[k]);
    showTokenSaved(!!f.token_saved);
    devOrder = [...f.route_order];
    $("dev_parakeet_decoding").value = f.parakeet_decoding ?? "auto";
    renderRoutes();
}
// The saved token never comes back to the page (S18): an empty field
// keeps it; typing replaces it; Clear removes it on Save.
function showTokenSaved(saved) {
    $("dev_token").placeholder = saved
        ? "saved (hidden) — leave empty to keep it"
        : "from flow-server gen-token";
}
function renderRoutes() {
    const srv = $("dev_allow_server").checked;
    const cli = $("dev_allow_client").checked;
    // Cloud STT is switched on in the OpenRouter tab; only its place in the order is set here.
    const cloud = !!orView?.stt.allow && !!devView?.caps.cloud;
    const on = (r) => (r === "server" ? srv : r === "client" ? cli : cloud);
    // Cloud is listed only in builds that have it.
    const shown = devOrder.filter((r) => r !== "cloud" || !!devView?.caps.cloud);
    $("dev_order_box").hidden = shown.filter(on).length < 2;
    $("dev_routes").replaceChildren(...shown.map((r, i) => {
        const li = el("li", on(r) ? "" : "off", ROUTE_LABEL[r] + " ");
        if (i > 0) {
            li.append(button("Move up", () => {
                const a = devOrder.indexOf(shown[i - 1]);
                const b = devOrder.indexOf(r);
                [devOrder[a], devOrder[b]] = [devOrder[b], devOrder[a]];
                renderRoutes();
                scheduleCheck();
            }, "small secondary"));
        }
        return li;
    }));
    $("dev_server_box").hidden = !srv;
    $("dev_pin_row").hidden = !$("dev_tls").checked;
}
function showCheck(c) {
    for (const s of document.querySelectorAll("[data-err]"))
        s.textContent = "";
    for (const i of document.querySelectorAll("[data-panel=developer] input"))
        i.classList.remove("invalid");
    for (const e of c.errors) {
        const slot = document.querySelector(`[data-err="${e.field}"]`);
        if (slot)
            slot.textContent = e.message;
        $(`dev_${e.field}`)?.classList.add("invalid");
    }
    devValid = c.errors.length === 0;
    $("dev_save").disabled = !devValid;
    $("dev_test_conn").disabled = !devValid;
    $("dev_tls_forced").hidden = !c.tls_forced;
    const tls = $("dev_tls");
    if (c.tls_forced)
        tls.checked = true;
    tls.disabled = c.tls_forced;
    $("dev_warnings").replaceChildren(...c.warnings.map((w) => el("li", w.startsWith("Dictation disabled") ? "err" : "", w)));
}
let checkTimer = 0;
function scheduleCheck() {
    renderRoutes();
    clearTimeout(checkTimer);
    checkTimer = setTimeout(() => {
        void invoke("dev_validate", { form: readDev() }).then(showCheck);
    }, 150);
}
function healthText(h) {
    if (!h.ok) {
        const code = h.error ?? "error";
        // flow-remote refuses ws:// to an address that is not private/tailnet (S6).
        if (/plaintext (refused|to public address)/.test(h.message ?? "")) {
            return "Server needs TLS — its address is not private (or resolves to a public one); turn on TLS and set a certificate pin";
        }
        const why = hint(code);
        return `${code}${h.message ? `: ${h.message}` : ""}${why ? ` — ${why}` : ""}`;
    }
    const r = h.rtt_ms;
    const parts = [
        r ? `OK · median ${r.median} ms (${r.min}-${r.max})` : "OK",
        h.server_version ?? "",
        h.asr_backend ?? "",
    ].filter(Boolean);
    return parts.join(" · ");
}
function showServerInfo(h) {
    const e = $("dev_server_info");
    if (!h) {
        e.textContent = "Not probed yet";
        e.className = "";
        return;
    }
    if (!h.ok) {
        e.textContent = healthText(h);
        e.className = "err";
        return;
    }
    const pp = h.postprocess_match === false ? " · ⚠ server pipeline ≠ client pipeline; results may differ" : " · pipeline ✓";
    e.textContent =
        `${h.server ?? ""} · ${h.server_version ?? "?"} (core ${h.server_core ?? "?"}, proto ${h.proto ?? "?"})` +
            ` · ASR ${h.asr_backend ?? "?"} ${h.asr_model ?? ""} · LLM ${h.llm ? "on" : "off"}` +
            ` · workers ${h.workers ?? "?"} (busy ${h.busy ?? "?"})${pp}`;
    e.className = h.postprocess_match === false ? "err" : "ok";
}
/** Shared with Android (D22): `total {client_ms ?: total_ms} ms · asr {asr_ms} (+{pre_asr_ms} early) · wait {wait_ms} · {segments} seg`. */
function timingLine(l) {
    const parts = [`total ${l.client_ms ?? l.total_ms} ms`];
    parts.push(l.pre_asr_ms > 0 ? `asr ${l.asr_ms} (+${l.pre_asr_ms} early)` : `asr ${l.asr_ms}`);
    if (l.wait_ms > 0)
        parts.push(`wait ${l.wait_ms}`);
    if (l.segments > 0)
        parts.push(`${l.segments} seg`);
    if (l.client_ms != null && l.client_ms !== l.total_ms)
        parts.push(`engine ${l.total_ms}`);
    return parts.join(" · ");
}
function showRoute(r) {
    const last = r.last;
    const e = $("dev_last");
    if (!last) {
        e.textContent = "No dictation yet";
    }
    else {
        const engine = last.engine === "remote" ? "server" : last.engine === "local" ? "client" : (last.engine ?? "none");
        const bits = [
            last.row ?? engine,
            last.notice ?? "",
            last.fallback ? `fallback: ${last.fallback}` : "",
            last.server ? `server ${last.server}` : "",
            last.rtt_ms != null ? `rtt ${last.rtt_ms} ms` : "",
            last.queue_ms != null ? `queue ${last.queue_ms} ms` : "",
            timingLine(last),
            last.status,
            last.empty_reason ?? "",
            last.llm_row ?? last.llm ?? "",
            new Date(last.at_ms).toLocaleTimeString(),
        ].filter(Boolean);
        e.textContent = bits.join(" · ");
    }
    const c = r.circuit;
    $("dev_circuit").textContent = !c.open
        ? "closed"
        : c.until_config_change
            ? `open until the server settings change (${c.reason ?? "?"})`
            : `open until ${new Date(Date.now() + (c.retry_in_ms ?? 0)).toLocaleTimeString()} (${c.reason ?? "?"})`;
    showServerInfo(r.health);
    const cc = r.cloud_circuit;
    $("dev_cloud_circuit_dt").hidden = !cc;
    $("dev_cloud_circuit").hidden = !cc;
    if (cc) {
        $("dev_cloud_circuit").textContent = !cc.open
            ? "closed"
            : cc.until_config_change
                ? `open until the key, model or privacy settings change (${cloudHint(cc.reason)})`
                : `open until ${new Date(Date.now() + (cc.retry_in_ms ?? 0)).toLocaleTimeString()} (${cloudHint(cc.reason)})`;
    }
}
function showDev(v) {
    devView = v;
    $("tab_developer").hidden = !v.visible;
    $("version_row").textContent = `Flow Clone ${v.version} · ${v.build}`;
    showPrivacyLine();
    fillDev(v.form);
    // Server-only builds have no local model: no model paths or LLM files.
    for (const f of document.querySelectorAll("[data-needs=local]"))
        f.hidden = !v.caps.local;
    $("dev_allow_server").disabled = !v.caps.remote;
    $("dev_allow_client").disabled = !v.caps.local;
    $("dev_server_na").hidden = v.caps.remote;
    $("dev_client_na").hidden = v.caps.local;
    $("dev_lock").hidden = v.caps.remote;
    $("dev_or_hint").hidden = !v.caps.cloud;
    const c = v.capabilities;
    $("dev_build").textContent =
        `Flow Clone ${v.full} · ` +
            `${[c.local ? "client" : "", c.remote ? "server" : ""].filter(Boolean).join(" + ") || "no engine"}` +
            ` · ASR ${c.asr_engines.join(", ") || "none"} · LLM ${c.llm ? "yes" : "no"} · proto ${c.proto} · core ${c.core_version}`;
    showCheck(v.check);
    showRoute(v.route);
}
async function loadDev() {
    try {
        showDev(await invoke("dev_get"));
    }
    catch (e) {
        setStatus("dev_status", String(e), false);
    }
}
async function devCall(cmd, args, okText) {
    try {
        const r = await invoke(cmd, args);
        setStatus("dev_status", okText, true);
        return r;
    }
    catch (e) {
        const errs = e;
        if (Array.isArray(errs)) {
            showCheck({ errors: errs, warnings: [], tls_forced: false });
            setStatus("dev_status", "Fix the highlighted fields", false);
        }
        else {
            setStatus("dev_status", String(e), false);
        }
        return null;
    }
}
async function devSave() {
    if (!devValid)
        return;
    const v = await devCall("dev_set", { form: readDev() }, "Saved — applies to the next dictation");
    if (v)
        showDev(v);
}
async function devTestConnection() {
    if (!devValid)
        return;
    const out = $("dev_test");
    out.textContent = "Testing…";
    out.className = "hint";
    try {
        const h = await invoke("dev_test_connection", { form: readDev() });
        out.textContent = `Test connection: ${healthText(h)}`;
        out.className = h.ok ? "hint ok" : "hint err";
    }
    catch (e) {
        out.textContent = String(e);
        out.className = "hint err";
    }
}
async function devTestSession() {
    const out = $("dev_test");
    out.textContent = "Running a 1 s silent session through the saved settings…";
    out.className = "hint";
    try {
        const r = await invoke("dev_test_session");
        const engine = r.engine === "remote" ? "server" : r.engine === "local" ? "client" : "none";
        const ok = r.status === "EMPTY" && r.empty_reason === "no_speech";
        out.textContent =
            `Test session: ${r.status}${r.empty_reason ? ` (${r.empty_reason})` : ""} via ${engine}` +
                `${r.fallback ? ` (fallback: ${r.fallback} — ${hint(r.fallback)})` : ""} in ${r.total_ms} ms` +
                `${r.error ? ` — ${r.error}` : ""}`;
        out.className = ok ? "hint ok" : "hint err";
    }
    catch (e) {
        out.textContent = String(e);
        out.className = "hint err";
    }
    void refreshRoute();
}
async function refreshRoute() {
    try {
        showRoute(await invoke("dev_route_status"));
    }
    catch {
        /* tab not loaded yet */
    }
}
let orView = null;
const CLOUD_HINTS = {
    auth_failed: "key rejected",
    quota_exceeded: "out of credits",
    connect_failed: "can't reach openrouter.ai",
    budget_exceeded: "monthly budget reached",
    cloud_not_configured: "save a key and accept the consent",
    cloud_cleanup_not_configured: "turn on cleanup, accept its consent and save a key",
    no_private_route: "no zero-retention provider serves this model: pick a model marked ZDR, or allow other providers",
    provider_error: "the provider returned an error",
    timeout: "took too long",
    busy: "rate limited",
    circuit_open: "paused after recent failures",
};
const PARAPHRASE_BADGE = "chat model, may paraphrase";
function cloudHint(code) {
    return code ? (CLOUD_HINTS[code] ?? code) : "?";
}
/** `code — hint` (+ the backend's message) for a failed OpenRouter call. */
function failText(code, message) {
    const c = code ?? "error";
    const h = CLOUD_HINTS[c];
    return `${c}${h ? ` — ${h}` : ""}${message ? ` (${message})` : ""}`;
}
/** Money from the backend: preformatted "$0.0234" strings pass through. */
function money(v) {
    if (v == null || v === "")
        return "$0";
    if (typeof v === "string")
        return v.startsWith("$") ? v : `$${v}`;
    return `$${v < 1 && v > 0 ? v.toFixed(4) : v.toFixed(2)}`;
}
function usd(v) {
    return v == null ? "" : ` · $${v.toFixed(v < 0.01 ? 5 : 4)}`;
}
function showPrivacyLine() {
    const line = $("privacy_line");
    if (orView?.supported) {
        line.textContent =
            "Speech is processed on this PC or on your own server, and by OpenRouter only if you turn it on (OpenRouter tab).";
    }
    else if (devView?.caps.remote) {
        line.textContent = "Speech is processed on this PC or on your own server. Nothing goes to a third party.";
    }
    else {
        line.textContent = "Everything runs on this PC. Nothing is sent over the network.";
    }
}
/** Run an OpenRouter command; errors are plain strings shown in `statusId`. */
async function orCall(cmd, args, statusId, okText) {
    try {
        const r = await invoke(cmd, args);
        if (okText)
            setStatus(statusId, okText, true);
        else
            $(statusId).textContent = "";
        return r;
    }
    catch (e) {
        setStatus(statusId, String(e), false);
        return null;
    }
}
async function loadOr() {
    try {
        showOr(await invoke("dev_or_get"));
    }
    catch {
        // A build without the OpenRouter commands: no tab.
        orView = null;
        $("tab_openrouter").hidden = true;
        $("ai_engine_box").hidden = true;
    }
}
/** Set a field unless the user is typing in it. */
function setValue(id, v) {
    const i = $(id);
    if (document.activeElement !== i)
        i.value = v;
}
function consentLine(at, given, what) {
    return given && at > 0
        ? `You agreed to send ${what} to openrouter.ai on ${new Date(at).toLocaleDateString()}.`
        : "Not yet agreed: turning this on asks first.";
}
function modelLine(id, info) {
    if (!info)
        return `${id} (not in the model list)`;
    return `${info.name} · ${info.price_label}`;
}
function showModelRow(prefix, id, info) {
    $(`${prefix}_model_name`).textContent = modelLine(id, info);
    $(`${prefix}_model_id`).textContent = id;
    $(`${prefix}_model_badges`).replaceChildren(...pills(info));
}
function pills(r) {
    if (!r)
        return [];
    const out = [];
    if (r.recommended)
        out.push(el("span", "pill rec", "Recommended"));
    for (const b of r.badges)
        out.push(el("span", b === PARAPHRASE_BADGE ? "pill warn" : "pill", b));
    if (r.allowed_by_account === false)
        out.push(el("span", "pill err", "blocked by your OpenRouter privacy settings"));
    return out;
}
function showOr(v) {
    // Turning STT on put `cloud` first in the saved route order (backend):
    // show that order under Developer → Engines too.
    if (v.stt.allow && orView && !orView.stt.allow && v.route_order)
        devOrder = [...v.route_order];
    orView = v;
    const tab = $("tab_openrouter");
    tab.hidden = !v.supported && !v.notice;
    $("or_badge").hidden = !v.notice;
    $("ai_engine_box").hidden = !v.supported;
    showPrivacyLine();
    $("or_warnings").replaceChildren(...v.warnings.map((w) => el("li", "", w)));
    $("or_notice_box").hidden = !v.notice;
    $("or_notice").textContent = v.notice ?? "";
    $("or_key_state").textContent = v.key.set ? `Key set: ${v.key.masked}` : "No key saved";
    $("or_zdr").checked = v.privacy.zdr;
    $("or_dc_deny").checked = v.privacy.data_collection === "deny";
    const s = v.stt;
    $("or_stt_allow").checked = s.allow;
    $("or_stt_consent").textContent = consentLine(s.consent_at_ms, s.consent, "audio");
    showModelRow("or_stt", s.model, v.stt_model_info);
    setValue("or_stt_language", s.language);
    $("or_stt_send_dictionary").checked = s.send_dictionary;
    $("or_stt_path").value = s.stt_path;
    const c = v.cleanup;
    $("or_cleanup_allow").checked = c.allow;
    $("or_cleanup_consent").textContent = consentLine(c.consent_at_ms, c.consent, "dictated text");
    $("or_cleanup_order").value = c.order[0] === "local" ? "local,cloud" : "cloud,local";
    showModelRow("or_cleanup", c.model, v.cleanup_model_info);
    $("or_cleanup_sort").value = c.sort;
    setValue("or_cleanup_timeout_ms", String(c.timeout_ms));
    $("or_cleanup_send_surrounding_text").checked = c.send_surrounding_text;
    $("or_cleanup_send_screen_words").checked = c.send_screen_words;
    $("or_cleanup_send_app_name").checked = c.send_app_name;
    showCleanupStatus(c.status);
    setValue("or_budget", v.budget == null ? "" : String(v.budget));
    showOrUsage(v.usage);
    showCatalogStatus(v.catalog);
    showAiEngine();
    renderRoutes();
}
function showOrUsage(u) {
    if (orView)
        orView.usage = u;
    const budget = u.budget != null ? ` of $${u.budget.toFixed(2)}${u.budget_reached ? " — budget reached, OpenRouter paused" : ""}` : "";
    const e = $("or_usage_total");
    e.textContent = `${u.month}: Speech ${money(u.speech)} · Cleanup ${money(u.cleanup)} · total ${money(u.total)}${budget}`;
    e.className = u.budget_reached ? "err" : "";
    $("or_usage_rows").replaceChildren(...u.rows.map((r) => {
        const what = r.purpose === "cleanup" ? "cleanups" : "dictations";
        const mins = r.purpose === "cleanup" ? "" : ` · ${r.minutes} min`;
        return el("li", "", `${r.label}: ${r.requests} ${what}${mins} · ${money(r.cost)}`);
    }));
}
function showCleanupStatus(st) {
    if (orView)
        orView.cleanup.status = st;
    const e = $("or_cleanup_state");
    let text;
    let cls = "hint";
    if (!st.supported) {
        text = "Not in this build.";
    }
    else if (st.blocked) {
        text = `Not used: ${failText(st.blocked)}`;
        cls = "hint err";
    }
    else if (st.circuit?.open) {
        const until = st.circuit.until_config_change
            ? "until the key or cleanup settings change"
            : `until ${new Date(Date.now() + (st.circuit.retry_in_ms ?? 0)).toLocaleTimeString()}`;
        text = `Paused ${until} (${cloudHint(st.circuit.reason)})`;
        cls = "hint err";
    }
    else if (st.enabled) {
        text = `On · ${st.model} · up to ${st.timeout_ms} ms`;
        cls = "hint ok";
    }
    else {
        text = "Off: OpenRouter gets no text.";
    }
    const l = st.last;
    if (l && l.engine) {
        const t = l.latency_ms != null ? ` ${l.latency_ms} ms` : "";
        const ft = l.ttft_ms != null ? ` (first token ${l.ttft_ms} ms)` : "";
        const who = l.engine === "cloud" ? "OpenRouter" : l.engine === "local" ? "this PC" : l.engine;
        text += ` · last: ${who}${l.model ? ` ${l.model}` : ""}${t}${ft}${usd(l.cost_usd)}`;
    }
    if (st.month_cost_usd != null && st.month_cost_usd > 0)
        text += ` · ${money(st.month_cost_usd)} this month`;
    e.textContent = text;
    e.className = cls;
}
/** AI cleanup tab: which engine cleans the text (local LLM toggle + OrView). */
function showAiEngine() {
    const v = orView;
    if (!v?.supported)
        return;
    const level = $("cleanup_level").value;
    const local = $("llm_enabled").checked && !!devView?.caps.local;
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
// ---- catalogue
let catalogInFlight = null;
function showCatalogStatus(c, loading = false) {
    const e = $("or_cat_status");
    if (loading) {
        e.textContent = "Loading…";
        e.className = "hint";
        return;
    }
    if (!c)
        return;
    if (orView)
        orView.catalog = c;
    const t = c.fetched_at_ms > 0 ? when(c.fetched_at_ms) : "?";
    const head = c.source === "Live"
        ? `Updated ${t}`
        : c.source === "Cache"
            ? `Cached ${t} (offline copy)`
            : `Offline list from ${c.snapshot_date ?? "?"}`;
    const err = c.error ? ` · refresh failed: ${failText(c.error)}` : "";
    e.textContent = `${head} · ${c.stt} speech, ${c.cleanup} text models${err}`;
    e.className = c.error ? "hint err" : "hint";
}
/** The only network fetch of the model list (§2.2 "When"). */
function refreshCatalog(force) {
    if (catalogInFlight)
        return catalogInFlight;
    showCatalogStatus(null, true);
    $("or_cat_refresh").disabled = true;
    catalogInFlight = (async () => {
        try {
            showCatalogStatus(await invoke("dev_or_refresh_catalog", { force }));
        }
        catch (e) {
            const s = $("or_cat_status");
            s.textContent = String(e);
            s.className = "hint err";
        }
        finally {
            catalogInFlight = null;
            $("or_cat_refresh").disabled = false;
        }
        if ($("or_picker").open)
            await loadPicker();
        // Model names / prices of the current picks may be known now.
        try {
            showOr(await invoke("dev_or_get"));
        }
        catch {
            /* keep the old view */
        }
    })();
    return catalogInFlight;
}
async function openOrTab() {
    if (!orView?.supported)
        return;
    await refreshCatalog(false);
}
// ---- consent + confirm dialogs
function askDialog(id, okId, cancelId) {
    const d = $(id);
    return new Promise((resolve) => {
        const done = (ok) => {
            $(okId).onclick = null;
            $(cancelId).onclick = null;
            d.oncancel = null;
            d.close();
            resolve(ok);
        };
        $(okId).onclick = () => done(true);
        $(cancelId).onclick = () => done(false);
        d.oncancel = (e) => {
            e.preventDefault();
            done(false);
        };
        d.showModal();
    });
}
function confirmOr(title, body, ok) {
    $("or_confirm_title").textContent = title;
    $("or_confirm_body").textContent = body;
    $("or_confirm_ok").textContent = ok;
    return askDialog("or_confirm", "or_confirm_ok", "or_confirm_cancel");
}
// ---- account
async function orSaveKey() {
    const input = $("or_key");
    if (input.value.trim() === "") {
        setStatus("or_key_status", "Paste the key first", false);
        return;
    }
    const k = await orCall("dev_cloud_set_key", { key: input.value }, "or_key_status", "Key saved in Credential Manager");
    input.value = "";
    if (k)
        await loadOr();
}
async function orClearKey() {
    const k = await orCall("dev_cloud_clear_key", {}, "or_key_status", "Key removed");
    if (k)
        await loadOr();
}
function kvRow(dl, k, v) {
    dl.append(el("dt", undefined, k), el("dd", undefined, v));
}
async function orCheckKey() {
    const dl = $("or_key_info");
    const out = $("or_key_status");
    out.textContent = "Checking the key with openrouter.ai…";
    out.className = "hint";
    dl.hidden = true;
    $("or_key_warn").hidden = true;
    try {
        const r = await invoke("dev_or_key_info");
        if ("error" in r) {
            setStatus("or_key_status", failText(r.error), false);
            return;
        }
        dl.replaceChildren();
        kvRow(dl, "Label", r.label || "—");
        kvRow(dl, "Credit limit", r.limit == null
            ? "none"
            : `${money(r.limit)}${r.limit_remaining != null ? ` · ${money(r.limit_remaining)} left` : ""}${r.limit_reset ? ` (resets ${r.limit_reset})` : ""}`);
        kvRow(dl, "Used this month", money(r.usage_monthly ?? 0));
        kvRow(dl, "Free tier", r.is_free_tier
            ? `yes${r.free_requests_remaining != null ? ` · ${r.free_requests_remaining} free requests left` : ""}`
            : "no");
        dl.hidden = false;
        $("or_key_warn").hidden = !r.warn_no_limit;
        setStatus("or_key_status", `Key OK (checked ${when(r.checked_at_ms || Date.now())})`, true);
    }
    catch (e) {
        setStatus("or_key_status", String(e), false);
    }
}
// ---- privacy
async function orPrivacyChanged() {
    const v = orView;
    if (!v)
        return;
    const zdrBox = $("or_zdr");
    const dcBox = $("or_dc_deny");
    if (!zdrBox.checked && v.privacy.zdr) {
        const ok = await confirmOr("Allow providers that keep data?", "Some providers may keep or train on your audio/text.", "Allow other providers");
        if (!ok) {
            zdrBox.checked = true;
            return;
        }
    }
    if (!dcBox.checked && v.privacy.data_collection === "deny") {
        const ok = await confirmOr("Allow providers that collect data?", "Some providers may keep or train on your audio/text.", "Allow data collection");
        if (!ok) {
            dcBox.checked = true;
            return;
        }
    }
    const r = await orCall("dev_or_set_privacy", { zdr: zdrBox.checked, dataCollection: dcBox.checked ? "deny" : "allow" }, "or_privacy_status", "Saved");
    if (r)
        showOr(r);
    else if (orView)
        showOr(orView);
}
// ---- speech-to-text
async function orApplyStt(consent) {
    const form = {
        allow: $("or_stt_allow").checked,
        consent,
        language: $("or_stt_language").value.trim(),
        send_dictionary: $("or_stt_send_dictionary").checked,
        stt_path: $("or_stt_path").value,
    };
    const r = await orCall("dev_or_set_stt", { form }, "or_stt_status", "Saved — applies to the next dictation");
    if (r)
        showOr(r);
    else if (orView)
        showOr(orView);
    return r !== null;
}
async function orSttToggled() {
    const box = $("or_stt_allow");
    if (!box.checked) {
        await orApplyStt(false);
        return;
    }
    let consent = false;
    if (!orView?.stt.consent) {
        consent = await askDialog("or_stt_consent_dlg", "or_stt_consent_ok", "or_stt_consent_cancel");
        if (!consent) {
            box.checked = false;
            return;
        }
    }
    if (await orApplyStt(consent))
        void refreshCatalog(false);
}
async function orTestStt() {
    const out = $("or_stt_test_out");
    out.textContent = "Sending a 3 s public-domain clip to openrouter.ai…";
    out.className = "hint";
    try {
        const typed = $("or_key").value.trim();
        const h = await invoke("dev_or_test_stt", { key: typed === "" ? null : typed });
        if (h.ok) {
            out.textContent = `OK · "${h.transcript ?? ""}" · ${h.latency_ms ?? "?"} ms${usd(h.cost_usd)}`;
            out.className = "hint ok";
        }
        else {
            out.textContent = failText(h.error, h.message);
            out.className = "hint err";
        }
    }
    catch (e) {
        out.textContent = String(e);
        out.className = "hint err";
    }
}
// ---- text cleanup
async function orApplyCleanup(consent) {
    const t = parseInt($("or_cleanup_timeout_ms").value, 10);
    const form = {
        allow: $("or_cleanup_allow").checked,
        consent,
        order: $("or_cleanup_order").value.split(","),
        sort: $("or_cleanup_sort").value,
        timeout_ms: Number.isFinite(t) ? Math.min(8000, Math.max(800, t)) : 2500,
        send_surrounding_text: $("or_cleanup_send_surrounding_text").checked,
        send_screen_words: $("or_cleanup_send_screen_words").checked,
        send_app_name: $("or_cleanup_send_app_name").checked,
    };
    const r = await orCall("dev_or_set_cleanup", { form }, "or_cleanup_msg", "Saved — applies to the next dictation");
    if (r) {
        $("or_cleanup_timeout_ms").value = String(r.cleanup.timeout_ms);
        showOr(r);
    }
    else if (orView)
        showOr(orView);
    return r !== null;
}
async function orCleanupToggled() {
    const box = $("or_cleanup_allow");
    if (!box.checked) {
        await orApplyCleanup(false);
        return;
    }
    let consent = false;
    if (!orView?.cleanup.consent) {
        consent = await askDialog("or_cleanup_consent_dlg", "or_cleanup_consent_ok", "or_cleanup_consent_cancel");
        if (!consent) {
            box.checked = false;
            return;
        }
    }
    if (await orApplyCleanup(consent))
        void refreshCatalog(false);
}
async function orTestCleanup() {
    const out = $("or_cleanup_test_out");
    out.textContent = "Sending a fixed sample sentence (not your text) to openrouter.ai…";
    out.className = "hint";
    try {
        const r = await invoke("dev_or_test_cleanup");
        if (r.ok) {
            out.textContent = `OK · "${r.text ?? ""}" · ${r.latency_ms ?? "?"} ms${usd(r.cost_usd)}${r.model ? ` · ${r.model}` : ""}`;
            out.className = "hint ok";
        }
        else {
            out.textContent = failText(r.error, r.message);
            out.className = "hint err";
        }
    }
    catch (e) {
        out.textContent = String(e);
        out.className = "hint err";
    }
    void refreshCleanupStatus();
}
async function refreshCleanupStatus() {
    try {
        showCleanupStatus(await invoke("dev_or_cleanup_status"));
    }
    catch {
        /* older backend */
    }
}
// ---- usage + budget
async function orBudgetChanged() {
    const raw = $("or_budget").value.trim();
    const n = Number(raw);
    if (raw !== "" && (!Number.isFinite(n) || n < 0)) {
        setStatus("or_usage_status", "Budget: a number of dollars, or empty for no limit", false);
        return;
    }
    const r = await orCall("dev_or_set_budget", { budget: raw === "" ? null : n }, "or_usage_status", "Budget saved");
    if (r)
        showOr(r);
}
async function refreshOrAfterDictation() {
    try {
        showOrUsage(await invoke("dev_cloud_usage"));
    }
    catch {
        /* tab not loaded yet */
    }
    await refreshCleanupStatus();
}
// ---- model picker
let pickerPurpose = "stt";
let pickerFilters = { zdr_only: true, fast_only: false, free_only: false, stt_path: null };
let pickerSeq = 0;
let pickerTimer = 0;
const paraphraseOk = new Set();
const CHIPS = [
    ["or_chip_transcriptions", () => pickerFilters.stt_path === "transcriptions", () => (pickerFilters.stt_path = pickerFilters.stt_path === "transcriptions" ? null : "transcriptions")],
    ["or_chip_chat_audio", () => pickerFilters.stt_path === "chat_audio", () => (pickerFilters.stt_path = pickerFilters.stt_path === "chat_audio" ? null : "chat_audio")],
    ["or_chip_zdr", () => pickerFilters.zdr_only, () => (pickerFilters.zdr_only = !pickerFilters.zdr_only)],
    ["or_chip_fast", () => pickerFilters.fast_only, () => (pickerFilters.fast_only = !pickerFilters.fast_only)],
    ["or_chip_free", () => pickerFilters.free_only, () => (pickerFilters.free_only = !pickerFilters.free_only)],
];
function showChips() {
    for (const [id, on] of CHIPS)
        $(id).setAttribute("aria-pressed", String(on()));
    const stt = pickerPurpose === "stt";
    $("or_chip_transcriptions").hidden = !stt;
    $("or_chip_chat_audio").hidden = !stt;
    $("or_chip_text").hidden = stt;
}
function currentModel() {
    return pickerPurpose === "stt" ? (orView?.stt.model ?? "") : (orView?.cleanup.model ?? "");
}
function pickerRow(r) {
    const li = el("li");
    const b = el("button", r.id === currentModel() ? "model current" : "model");
    b.type = "button";
    const head = el("span", "model-head");
    head.append(el("strong", undefined, r.name || r.id), ...pills(r));
    const meta = el("span", "model-meta");
    meta.append(el("code", undefined, r.id), el("span", undefined, r.price_label));
    if (pickerPurpose === "cleanup" && r.context_length > 0) {
        meta.append(el("span", undefined, `${Math.round(r.context_length / 1000)}k ctx`));
    }
    if (r.id === currentModel())
        meta.append(el("span", "ok", "current"));
    b.append(head, meta);
    b.addEventListener("click", () => void pickModel(r.id, r.badges.includes(PARAPHRASE_BADGE)));
    li.append(b);
    return li;
}
async function loadPicker() {
    const seq = ++pickerSeq;
    try {
        const rows = await invoke("dev_or_catalog", {
            purpose: pickerPurpose,
            query: $("or_picker_query").value.trim(),
            filters: { ...pickerFilters, stt_path: pickerPurpose === "stt" ? pickerFilters.stt_path : null },
        });
        if (seq !== pickerSeq)
            return; // a newer search is in flight
        // Recommended first; otherwise the backend's order (price).
        const sorted = [...rows.filter((r) => r.recommended), ...rows.filter((r) => !r.recommended)];
        $("or_picker_list").replaceChildren(...sorted.map(pickerRow));
        $("or_picker_empty").hidden = rows.length > 0;
    }
    catch (e) {
        setStatus("or_picker_status", String(e), false);
    }
}
async function openPicker(purpose) {
    pickerPurpose = purpose;
    pickerFilters = { zdr_only: true, fast_only: false, free_only: false, stt_path: null };
    $("or_picker_title").textContent =
        purpose === "stt" ? "Speech-to-text model" : "Text cleanup model";
    $("or_picker_query").value = "";
    $("or_picker_custom").value = "";
    $("or_picker_status").textContent = "";
    showChips();
    if (catalogInFlight)
        showCatalogStatus(null, true);
    else if (orView)
        showCatalogStatus(orView.catalog);
    $("or_picker").showModal();
    if (!catalogInFlight) {
        try {
            showCatalogStatus(await invoke("dev_or_catalog_status"));
        }
        catch {
            /* keep the OrView copy */
        }
    }
    await loadPicker();
}
async function pickModel(id, paraphrase) {
    if (id.trim() === "")
        return;
    if (paraphrase && !paraphraseOk.has(id)) {
        const ok = await confirmOr("Use a chat model for speech?", "This is a chat model with audio input: it may paraphrase or answer what you said instead of writing it down word for word.", "Use it anyway");
        if (!ok)
            return;
        paraphraseOk.add(id);
    }
    const r = await orCall("dev_or_set_model", { purpose: pickerPurpose, model: id.trim() }, "or_picker_status", "");
    if (!r)
        return;
    showOr(r);
    $("or_picker").close();
    setStatus(pickerPurpose === "stt" ? "or_stt_status" : "or_cleanup_msg", `Model set: ${id.trim()}`, true);
}
function wireOpenRouter() {
    $("or_notice_ok").addEventListener("click", () => void orCall("dev_or_ack_notice", {}, "or_key_status", "").then((v) => v && showOr(v)));
    $("or_key_paste").addEventListener("click", () => {
        void navigator.clipboard
            .readText()
            .then((t) => ($("or_key").value = t.trim()))
            .catch(() => setStatus("or_key_status", "Clipboard not readable: paste with Ctrl+V", false));
    });
    $("or_key_save").addEventListener("click", () => void orSaveKey());
    $("or_key_clear").addEventListener("click", () => void orClearKey());
    $("or_key_check").addEventListener("click", () => void orCheckKey());
    $("or_zdr").addEventListener("change", () => void orPrivacyChanged());
    $("or_dc_deny").addEventListener("change", () => void orPrivacyChanged());
    $("or_stt_allow").addEventListener("change", () => void orSttToggled());
    for (const id of ["or_stt_language", "or_stt_send_dictionary", "or_stt_path"]) {
        $(id).addEventListener("change", () => void orApplyStt(false));
    }
    $("or_stt_change").addEventListener("click", () => void openPicker("stt"));
    $("or_stt_test").addEventListener("click", () => void orTestStt());
    $("or_cleanup_allow").addEventListener("change", () => void orCleanupToggled());
    for (const id of [
        "or_cleanup_order",
        "or_cleanup_sort",
        "or_cleanup_timeout_ms",
        "or_cleanup_send_surrounding_text",
        "or_cleanup_send_screen_words",
        "or_cleanup_send_app_name",
    ]) {
        $(id).addEventListener("change", () => void orApplyCleanup(false));
    }
    $("or_cleanup_change").addEventListener("click", () => void openPicker("cleanup"));
    $("or_cleanup_test").addEventListener("click", () => void orTestCleanup());
    $("or_budget").addEventListener("change", () => void orBudgetChanged());
    $("or_reset_usage").addEventListener("click", () => void orCall("dev_cloud_reset_usage", {}, "or_usage_status", "Usage counters reset").then((u) => u && showOrUsage(u)));
    $("or_picker_query").addEventListener("input", () => {
        clearTimeout(pickerTimer);
        pickerTimer = setTimeout(() => void loadPicker(), 200);
    });
    for (const [id, , toggle] of CHIPS) {
        $(id).addEventListener("click", () => {
            toggle();
            showChips();
            void loadPicker();
        });
    }
    $("or_cat_refresh").addEventListener("click", () => void refreshCatalog(true));
    $("or_picker_use").addEventListener("click", () => void pickModel($("or_picker_custom").value, false));
    $("or_picker_close").addEventListener("click", () => $("or_picker").close());
    $("ai_or_link").addEventListener("click", () => showTab("openrouter"));
    $("llm_enabled").addEventListener("change", showAiEngine);
    $("cleanup_level").addEventListener("change", showAiEngine);
}
let versionTaps = 0;
let versionTimer = 0;
function versionTapped() {
    if (!devView || devView.visible)
        return;
    versionTaps++;
    clearTimeout(versionTimer);
    versionTimer = setTimeout(() => (versionTaps = 0), 3000);
    if (versionTaps >= 7) {
        versionTaps = 0;
        void devCall("dev_unlock", { unlocked: true }, "Developer settings unlocked").then((v) => {
            if (v) {
                showDev(v);
                showTab("developer");
            }
        });
    }
}
function showDebugLog(v) {
    $("dev_debug_log").checked = v.enabled;
    $("dev_debug_path").textContent = v.path;
    const n = v.count;
    let line = n === null ? "Log file not readable" : n === 0 ? "No entries" : n === 1 ? "1 entry" : `${n} entries`;
    if (v.stats.write_errors > 0)
        line += ` · ${v.stats.write_errors} write errors (ignored)`;
    if (v.stats.dropped > 0)
        line += ` · ${v.stats.dropped} dropped`;
    $("dev_debug_count").textContent = line;
}
async function loadDebugLog() {
    try {
        showDebugLog(await invoke("debug_log_status"));
    }
    catch (e) {
        setStatus("dev_status", String(e), false);
    }
}
/** Turning the log off offers to delete what it already holds. */
function askClearDebugLog(count) {
    const d = $("dev_debug_off");
    $("dev_debug_off_body").textContent =
        `The log still holds ${count === 1 ? "1 entry" : `${count} entries`} with transcripts of earlier dictations. Delete it now?`;
    return new Promise((resolve) => {
        const done = (ok) => {
            $("dev_debug_delete").onclick = null;
            $("dev_debug_keep").onclick = null;
            d.close();
            resolve(ok);
        };
        $("dev_debug_delete").onclick = () => done(true);
        $("dev_debug_keep").onclick = () => done(false);
        d.oncancel = () => done(false);
        d.showModal();
    });
}
async function debugLogToggled() {
    const on = $("dev_debug_log").checked;
    const v = await devCall("debug_log_set", { enabled: on }, on ? "Debug log on: transcripts are being written" : "Debug log off");
    if (!v) {
        $("dev_debug_log").checked = !on;
        return;
    }
    showDebugLog(v);
    if (!on && (v.count ?? 0) > 0 && (await askClearDebugLog(v.count ?? 0))) {
        const c = await devCall("debug_log_clear", {}, "Debug log off and deleted");
        if (c)
            showDebugLog(c);
    }
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
    await loadAppChoices();
    await loadSuggestions();
    await loadDev();
    await loadOr();
    await loadDebugLog();
}
for (const b of document.querySelectorAll("nav.tabs button")) {
    b.addEventListener("click", () => showTab(b.dataset.tab ?? "general"));
}
$("form").addEventListener("submit", (e) => void saveForm(e));
$("asr_engine").addEventListener("change", showEngineFields);
$("app_add_recent").addEventListener("click", addRecentApp);
$("app_add_manual").addEventListener("click", addManualApp);
$("app_manual").addEventListener("keydown", (e) => {
    // Enter adds the app instead of submitting (saving) the whole form.
    if (e.key === "Enter") {
        e.preventDefault();
        addManualApp();
    }
});
$("app_recent_clear").addEventListener("click", () => {
    void invoke("recent_apps_clear").then(loadAppChoices, (e) => setStatus("app_add_status", String(e), false));
});
for (const b of BUCKETS) {
    $(`style_${b}`).addEventListener("change", () => showStylePreview(b));
}
// Verbatim and the Auto-format box are the same switch (D27): keep them in step.
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
for (const i of document.querySelectorAll("[data-panel=developer] input")) {
    i.addEventListener("input", scheduleCheck);
    i.addEventListener("change", scheduleCheck);
}
$("dev_save").addEventListener("click", () => void devSave());
$("dev_debug_log").addEventListener("change", () => void debugLogToggled());
$("dev_debug_open").addEventListener("click", () => void devCall("debug_log_open_folder", {}, "Opened the log folder"));
$("dev_debug_clear").addEventListener("click", () => void devCall("debug_log_clear", {}, "Debug log deleted").then((v) => v && showDebugLog(v)));
$("dev_test_conn").addEventListener("click", () => void devTestConnection());
$("dev_test_session").addEventListener("click", () => void devTestSession());
$("dev_reset_circuit").addEventListener("click", () => void devCall("dev_reset_circuit", {}, "Circuit reset").then((r) => r && showRoute(r)));
$("dev_reset").addEventListener("click", () => void devCall("dev_reset_defaults", {}, "Defaults restored").then((v) => v && showDev(v)));
$("dev_lock").addEventListener("click", () => void devCall("dev_unlock", { unlocked: false }, "Developer settings locked").then((v) => {
    if (v) {
        showDev(v);
        showTab("general");
    }
}));
$("dev_token_show").addEventListener("click", () => {
    const t = $("dev_token");
    t.type = t.type === "password" ? "text" : "password";
    $("dev_token_show").textContent = t.type === "password" ? "Show" : "Hide";
});
$("dev_token_paste").addEventListener("click", () => {
    void navigator.clipboard
        .readText()
        .then((t) => {
        $("dev_token").value = t.trim();
        scheduleCheck();
    })
        .catch(() => setStatus("dev_status", "Clipboard not readable: paste with Ctrl+V", false));
});
$("dev_token_clear").addEventListener("click", () => {
    $("dev_token").value = "";
    if (devView)
        devView.form.token_saved = false;
    showTokenSaved(false);
    scheduleCheck();
});
$("version_row").addEventListener("click", versionTapped);
wireOpenRouter();
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
void onEvent("dev:route", (e) => {
    showRoute(e.payload);
    if (orView?.supported)
        void refreshOrAfterDictation();
});
void onEvent("history:changed", () => {
    if ($("dev_debug_log").checked)
        void loadDebugLog();
    if (!$("hist_list").closest("[hidden]"))
        void loadHistory();
});
showTab("general");
void load();
