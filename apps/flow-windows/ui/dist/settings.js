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
    if (name === "developer")
        void loadDev();
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
const ROUTE_LABEL = { server: "Server", client: "This PC (client)" };
const DEV_NUM = ["port", "connect_timeout_ms", "commit_timeout_base_ms", "health_ttl_s"];
const DEV_TEXT = ["host", "tls_pin_sha256", "token"];
const DEV_BOOL = ["allow_server", "allow_client", "tls", "keep_local_warm", "show_engine_badge"];
let devView = null;
let devOrder = ["server", "client"];
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
    return f;
}
function fillDev(f) {
    for (const k of DEV_BOOL)
        $(`dev_${k}`).checked = f[k];
    for (const k of DEV_TEXT)
        $(`dev_${k}`).value = f[k];
    for (const k of DEV_NUM)
        $(`dev_${k}`).value = String(f[k]);
    devOrder = [...f.route_order];
    renderRoutes();
}
function renderRoutes() {
    const srv = $("dev_allow_server").checked;
    const cli = $("dev_allow_client").checked;
    const on = (r) => (r === "server" ? srv : cli);
    $("dev_order_box").hidden = devOrder.filter(on).length < 2;
    $("dev_routes").replaceChildren(...devOrder.map((r, i) => {
        const li = el("li", on(r) ? "" : "off", ROUTE_LABEL[r] + " ");
        if (i > 0) {
            li.append(button("Move up", () => {
                [devOrder[i - 1], devOrder[i]] = [devOrder[i], devOrder[i - 1]];
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
function showRoute(r) {
    const last = r.last;
    const e = $("dev_last");
    if (!last) {
        e.textContent = "No dictation yet";
    }
    else {
        const engine = last.engine === "remote" ? "server" : last.engine === "local" ? "client" : "none";
        const bits = [
            engine,
            last.fallback ? `fallback: ${last.fallback}` : "",
            last.server ? `server ${last.server}` : "",
            last.rtt_ms != null ? `rtt ${last.rtt_ms} ms` : "",
            last.queue_ms != null ? `queue ${last.queue_ms} ms` : "",
            `asr ${last.asr_ms} ms / total ${last.total_ms} ms`,
            last.status,
            last.empty_reason ?? "",
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
}
function showDev(v) {
    devView = v;
    $("tab_developer").hidden = !v.visible;
    $("version_row").textContent = `Flow Clone ${v.version}`;
    $("privacy_line").textContent = v.caps.remote
        ? "Speech is processed on this PC or on your own server. Nothing goes to a third party."
        : "Everything runs on this PC. Nothing is sent over the network.";
    fillDev(v.form);
    // Server-only builds have no local model: no model paths or LLM files.
    for (const f of document.querySelectorAll("[data-needs=local]"))
        f.hidden = !v.caps.local;
    $("dev_allow_server").disabled = !v.caps.remote;
    $("dev_allow_client").disabled = !v.caps.local;
    $("dev_server_na").hidden = v.caps.remote;
    $("dev_client_na").hidden = v.caps.local;
    $("dev_lock").hidden = v.caps.remote;
    const c = v.capabilities;
    $("dev_build").textContent =
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
    await loadDev();
}
for (const b of document.querySelectorAll("nav.tabs button")) {
    b.addEventListener("click", () => showTab(b.dataset.tab ?? "general"));
}
$("form").addEventListener("submit", (e) => void saveForm(e));
$("asr_engine").addEventListener("change", showEngineFields);
for (const i of document.querySelectorAll("[data-panel=developer] input")) {
    i.addEventListener("input", scheduleCheck);
    i.addEventListener("change", scheduleCheck);
}
$("dev_save").addEventListener("click", () => void devSave());
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
    scheduleCheck();
});
$("version_row").addEventListener("click", versionTapped);
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
void onEvent("dev:route", (e) => showRoute(e.payload));
void onEvent("history:changed", () => {
    if (!$("hist_list").closest("[hidden]"))
        void loadHistory();
});
showTab("general");
void load();
