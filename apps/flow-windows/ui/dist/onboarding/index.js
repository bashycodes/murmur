// First-run setup (docs/design/app-design/Onboard*.dc.html): a full-window
// overlay over the app shell, shown while prefs.onboarded is false or when
// the user asks for it again (Settings → General). Five steps: welcome,
// where speech runs, the speech model, AI cleanup, try it. Each step saves
// its choice with the existing commands when the user presses Continue
// (dev_set through the dev store, prefs through the prefs store,
// dev_cloud_set_key / dev_or_set_* for OpenRouter); Finish sets
// onboarded = true. Nothing downloads by itself (D9/D31): the model step
// points the app at files the user already has, or downloads a catalogue
// model after the user pressed Download and confirmed it (3a/3b, through
// features/models.ts). A server test or an OpenRouter key/consent happens
// only on the user's own click.
import { mb, models, pickPath, progressLine } from "../features/models.js";
import { devStore } from "../lib/dev-store.js";
import { $, el, html } from "../lib/dom.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { captureChord, stopCapture } from "../lib/keycapture.js";
import { prefsStore } from "../lib/prefs-store.js";
import { initialEngine, recommendedEngine } from "../lib/setup-engine.js";
import { views } from "../lib/views.js";
import { TEMPLATE } from "./templates.js";
const ENGINE_ROUTE = { pc: "client", server: "server", cloud: "cloud" };
const ENGINE_OF = { client: "pc", server: "server", cloud: "cloud" };
const SAMPLE = {
    rules: "So lunch is Thursday.",
    pc: "Lunch is on Thursday.",
    cloud: "Lunch is on Thursday.",
};
/** The catalogue model behind each row (model_catalog ids). */
const CATALOG_ID = {
    parakeet: "parakeet-tdt-0.6b-v2-int8",
    whisper: "whisper-base",
};
const st = {
    step: "welcome",
    engine: "pc",
    cleanup: "rules",
    /** D78: which Cloud provider cleans up (speech is always OpenRouter). */
    prov: "openrouter",
    model: null,
    info: null,
    openedAt: 0,
    busy: false,
};
let onDone = () => { };
// ------------------------------------------------------------ capabilities
function has(e) {
    const caps = views.dev?.caps;
    if (e === "pc")
        return caps ? caps.local : true;
    if (e === "server")
        return !!caps?.remote;
    return !!views.or?.supported;
}
function hasCleanup(c) {
    if (c === "rules")
        return true;
    if (c === "pc")
        return views.dev ? views.dev.caps.local || views.dev.capabilities.llm : true;
    return !!views.or?.supported;
}
/** Steps shown in this build: no model step in a build without engines on this PC. */
function steps() {
    return has("pc") ? ["welcome", "engine", "model", "cleanup", "try"] : ["welcome", "engine", "cleanup", "try"];
}
// ------------------------------------------------------------ helpers
const panel = (s) => document.querySelector(`#onboarding [data-ob-step="${s}"]`);
function status(id, text, ok) {
    const e = $(id);
    e.textContent = text;
    e.classList.toggle("ok", ok === true);
    e.classList.toggle("err", ok === false);
}
function nextBtn(s) {
    return panel(s).querySelector('[data-ob="next"]');
}
function keySet() {
    return !!views.or?.key.set;
}
/** Fill both "OpenRouter API key" rows: a saved key is not asked for again. */
function showKeyRows() {
    for (const r of document.querySelectorAll("#onboarding [data-ob-key-row]")) {
        const input = r.querySelector("[data-ob-key]");
        const desc = r.querySelector("[data-ob-key-desc]");
        if (keySet()) {
            input.placeholder = `saved ${views.or?.key.masked ?? ""}`;
            desc.textContent = "A key is saved. Paste a new one only to replace it.";
        }
    }
}
/** Save a typed OpenRouter key; true when a key is (now) saved. */
async function saveKey(inputId, statusId) {
    const typed = $(inputId).value.trim();
    if (typed) {
        try {
            await invoke("dev_cloud_set_key", { key: typed });
            $(inputId).value = "";
            views.setOr(await invoke("dev_or_get"));
            showKeyRows();
        }
        catch (e) {
            status(statusId, String(e), false);
            return false;
        }
    }
    if (!keySet()) {
        status(statusId, "Paste your OpenRouter API key first.", false);
        return false;
    }
    return true;
}
async function refreshDev() {
    views.setDev(await invoke("dev_get"));
}
function errText(e) {
    if (Array.isArray(e))
        return e.map((x) => x.message).join("; ");
    return String(e);
}
// ------------------------------------------------------------ step: engine
function pickEngine(e) {
    if (!has(e))
        return;
    st.engine = e;
    renderEngine();
}
function renderEngine() {
    for (const b of document.querySelectorAll("#onboarding [data-ob-engine]")) {
        const id = b.dataset.obEngine;
        const on = id === st.engine;
        b.setAttribute("aria-checked", String(on));
        b.classList.toggle("is-on", on);
        b.disabled = !has(id);
        b.querySelector("[data-ob-na]").hidden = has(id);
        b.querySelector("[data-ob-rec]").hidden = id !== recommendedEngine(has);
    }
    $("ob_server_detail").hidden = st.engine !== "server";
    $("ob_cloud_detail").hidden = st.engine !== "cloud";
    $("ob_engine_hint").textContent = "";
}
async function testServer() {
    const form = devStore.form({ host: $("ob_host").value.trim(), token: $("ob_token").value.trim() });
    status("ob_server_status", "Connecting…");
    try {
        const h = await invoke("dev_test_connection", { form });
        if (h.ok)
            status("ob_server_status", `Connected${h.server ? ` to ${h.server}` : ""}${h.rtt_ms ? ` · ${h.rtt_ms.median} ms` : ""}.`, true);
        else
            status("ob_server_status", h.message ?? h.error ?? "No answer.", false);
    }
    catch (e) {
        status("ob_server_status", errText(e), false);
    }
}
/** Continue on the engine step: turn the picked engine on and put it first. */
async function applyEngine() {
    const dev = views.dev;
    if (!dev)
        return true;
    const order = [ENGINE_ROUTE[st.engine], ...dev.form.route_order.filter((r) => r !== ENGINE_ROUTE[st.engine])];
    if (st.engine === "pc") {
        // An unset server would only add a "not set up" warning.
        await devStore.save({ allow_client: true, allow_server: dev.form.allow_server && !!dev.form.host, route_order: order });
        return true;
    }
    if (st.engine === "server") {
        $("ob_host_err").textContent = "";
        $("ob_token_err").textContent = "";
        const host = $("ob_host").value.trim();
        if (!host) {
            $("ob_host_err").textContent = "Type your server's address.";
            return false;
        }
        try {
            await devStore.save({ allow_server: true, host, token: $("ob_token").value.trim(), route_order: order });
            $("ob_token").value = "";
            return true;
        }
        catch (e) {
            if (Array.isArray(e)) {
                for (const f of e) {
                    const slot = document.getElementById(`ob_${f.field}_err`);
                    if (slot)
                        slot.textContent = f.message;
                    else
                        status("ob_server_status", f.message, false);
                }
            }
            else
                status("ob_server_status", String(e), false);
            return false;
        }
    }
    // OpenRouter speech: key + this use's consent (D18/D28), then cloud first.
    if (!(await saveKey("ob_stt_key", "ob_cloud_status")))
        return false;
    if (!$("ob_stt_consent").checked) {
        status("ob_cloud_status", "Tick “Send my audio to OpenRouter” to use it.", false);
        return false;
    }
    const s = views.or?.stt;
    try {
        const v = await invoke("dev_or_set_stt", {
            form: { allow: true, consent: true, language: s?.language ?? "", send_dictionary: s?.send_dictionary ?? true, stt_path: s?.stt_path ?? "" },
        });
        views.setOr(v);
        await refreshDev();
        if (views.dev && views.dev.form.route_order[0] !== "cloud")
            await devStore.save({ route_order: order });
        status("ob_cloud_status", "");
        return true;
    }
    catch (e) {
        status("ob_cloud_status", errText(e), false);
        return false;
    }
}
// ------------------------------------------------------------ step: model
function activeModel() {
    return prefsStore.get()?.asr_engine === "parakeet" ? "parakeet" : "whisper";
}
function renderModel() {
    const needed = st.engine === "pc";
    $("ob_model_title").textContent = needed ? "Add a speech model" : "A backup model (optional)";
    const dl = models.canDownload();
    $("ob_model_lead").textContent = !needed
        ? "Your choice does the listening. A model on this PC keeps dictation working when it can't be reached. You can skip this."
        : dl
            ? "The model is the part that listens. We can download it for you once, from where its makers publish it, or you can add the file yourself."
            : "The model is the part that listens. Get it once from where its makers publish it, then tell Murmur where it is.";
    $("ob_model_note").textContent = dl
        ? "A download only fetches the file. Nothing about you or your voice is sent, and the file is checked against a fingerprint built into the app before it's used."
        : "Murmur never downloads anything by itself. Your voice is turned into text by the model on this PC.";
    const p = prefsStore.get();
    if (p) {
        const pk = $("ob_parakeet_dir");
        const wh = $("ob_whisper_path");
        if (document.activeElement !== pk)
            pk.value = p.parakeet_model_dir ?? "";
        if (document.activeElement !== wh)
            wh.value = p.model_path ?? "";
    }
    if (st.info) {
        $("ob_models_dir").textContent = st.info.models_dir;
        $("ob_parakeet_dir").placeholder = st.info.default_parakeet_dir;
        $("ob_whisper_path").placeholder = st.info.default_path;
    }
    const active = activeModel();
    const m = st.model;
    for (const r of document.querySelectorAll("#onboarding [data-ob-model]")) {
        const id = r.dataset.obModel;
        const on = id === active;
        r.classList.toggle("is-on", on);
        r.querySelector("[data-ob-rec]").hidden = id !== "parakeet";
        const badge = r.querySelector("[data-ob-model-badge]");
        const line = r.querySelector("[data-ob-model-status]");
        const use = r.querySelector("[data-ob-use]");
        use.textContent = on ? "Check again" : "Use";
        renderDownload(r, id);
        badge.hidden = !on || !m;
        line.textContent = "";
        line.className = "lw-ob-model-status";
        if (!on || !m)
            continue;
        const [text, kind, desc] = m.state === "ready"
            ? ["Ready", "success", `Found and loaded: ${m.path}`]
            : m.state === "loading"
                ? ["Loading", "light", `Loading ${m.path}…`]
                : m.state === "missing"
                    ? ["Not found", "warning", `Nothing at ${m.path}.`]
                    : ["Failed", "destructive", `Could not load ${m.path}: ${m.message ?? ""}`];
        badge.textContent = text;
        badge.className = `fl-badge fl-badge-${kind}`;
        line.textContent = desc;
        line.classList.add(m.state === "ready" ? "ok" : m.state === "loading" ? "hint" : "err");
    }
    const downloading = models.busy();
    const ok = !needed || downloading || (!!m && (m.state === "ready" || m.state === "loading"));
    const next = nextBtn("model");
    next.disabled = !ok;
    $("ob_model_hint").textContent = !ok
        ? "Add a speech model to continue"
        : downloading
            ? "You can carry on. It finishes in the background."
            : m?.state === "loading"
                ? "You can carry on. It finishes loading in the background."
                : "";
}
/** A row's Download / Cancel buttons and progress (3b). */
function renderDownload(r, id) {
    const e = models.entry(CATALOG_ID[id]);
    const p = e ? models.progress(e.id) : undefined;
    const busy = !!e && models.busy(e.id);
    const dl = r.querySelector("[data-ob-dl]");
    const cancel = r.querySelector("[data-ob-dl-cancel]");
    const use = r.querySelector("[data-ob-use]");
    const box = r.querySelector("[data-ob-dl-box]");
    const bar = r.querySelector("[data-ob-dl-bar]");
    const line = r.querySelector("[data-ob-dl-line]");
    const desc = r.querySelector("[data-ob-model-desc]");
    if (e) {
        const what = e.files.length > 1 ? `a folder of ${mb(e.size)}` : `a ${mb(e.size)} file`;
        desc.textContent = `${e.desc} · ${what}${e.installed ? " · downloaded and checked" : ""}`;
    }
    dl.hidden = !e || !e.available || e.installed || busy;
    dl.disabled = models.busy();
    dl.textContent = p?.state === "error" ? "Try again" : "Download";
    cancel.hidden = !busy;
    use.hidden = busy;
    const shown = !!p && p.state !== "done";
    box.hidden = !shown;
    box.classList.toggle("is-err", p?.state === "error");
    bar.hidden = !busy;
    if (!p || !shown)
        return;
    const pct = p.total ? Math.round((p.received / p.total) * 100) : 0;
    bar.setAttribute("aria-valuenow", String(pct));
    bar.firstElementChild.style.width = `${pct}%`;
    line.textContent = progressLine(p, e);
}
/** "Choose folder" / "Choose file": the native picker, then Use it. */
async function pickModel(id) {
    try {
        const p = id === "parakeet"
            ? await pickPath("folder", "Choose the Parakeet model folder")
            : id === "whisper"
                ? await pickPath("file", "Choose a Whisper model", [{ name: "Whisper model (ggml)", pattern: "*.bin" }, { name: "All files", pattern: "*.*" }])
                : await pickPath("file", "Choose an AI cleanup model", [{ name: "GGUF model", pattern: "*.gguf" }, { name: "All files", pattern: "*.*" }]);
        if (!p)
            return;
        if (id === "llm") {
            $("ob_llm_path").value = p;
            return;
        }
        $(id === "parakeet" ? "ob_parakeet_dir" : "ob_whisper_path").value = p;
        await useModel(id);
    }
    catch (e) {
        console.error("pick_path", e);
    }
}
/** "Use": save the engine + its location; the app reloads the model and reports model:status. */
async function useModel(id) {
    const patch = id === "parakeet"
        ? { asr_engine: "parakeet", parakeet_model_dir: $("ob_parakeet_dir").value.trim() || null }
        : { asr_engine: "whisper", model_path: $("ob_whisper_path").value.trim() || null };
    try {
        await prefsStore.save(patch);
        st.model = (await invoke("get_model_status")).status;
    }
    catch (e) {
        st.model = { state: "failed", path: "", message: String(e) };
    }
    renderModel();
}
// ------------------------------------------------------------ step: cleanup
function pickCleanup(c) {
    if (!hasCleanup(c))
        return;
    st.cleanup = c;
    renderCleanup();
}
function llmLine(s) {
    switch (s?.state) {
        case "ready":
            return `Ready: ${s.path ?? ""}`;
        case "loading":
            return `Loading ${s.path ?? ""}…`;
        case "missing":
            return `Not found: ${s.path ?? ""}. Put the file there or type where it is.`;
        case "plugin_missing":
            return `The cleanup engine is missing next to the app: ${s.message ?? ""}`;
        case "failed":
            return `Could not load it: ${s.message ?? ""}`;
        default:
            return "Empty = the default file in your models folder.";
    }
}
function renderCleanup() {
    for (const b of document.querySelectorAll("#onboarding [data-ob-cleanup]")) {
        const id = b.dataset.obCleanup;
        const on = id === st.cleanup;
        b.setAttribute("aria-checked", String(on));
        b.classList.toggle("is-on", on);
        b.disabled = !hasCleanup(id);
        b.querySelector("[data-ob-na]").hidden = hasCleanup(id);
        b.querySelector("[data-ob-rec]").hidden = id !== "pc" || !hasCleanup("pc");
    }
    $("ob_sample").textContent = SAMPLE[st.cleanup];
    $("ob_llm_detail").hidden = st.cleanup !== "pc";
    $("ob_or_detail").hidden = st.cleanup !== "cloud";
    renderCloudProvider();
    const p = prefsStore.get();
    const path = $("ob_llm_path");
    if (p && document.activeElement !== path)
        path.value = p.llm_model_path ?? "";
    if (st.info)
        path.placeholder = st.info.default_llm_path;
    $("ob_llm_status").textContent = llmLine(st.info?.llm);
    status("ob_cleanup_status", "");
}
/** D78: the OpenRouter | LiteLLM switch of the Cloud cleanup option. */
function renderCloudProvider() {
    const custom = st.prov === "custom";
    for (const b of document.querySelectorAll("#ob_cloud_prov_seg [data-ob-cloud-provider]")) {
        const on = b.dataset.obCloudProvider === st.prov;
        b.setAttribute("aria-checked", String(on));
        b.classList.toggle("is-on", on);
    }
    $("ob_cloud_prov_hint").textContent = custom ? "Your own OpenAI-compatible proxy" : "Pay per use with your key";
    $("ob_or_key_row").hidden = custom;
    $("ob_ll_rows").hidden = !custom;
    $("ob_clean_consent_title").textContent = custom
        ? "Send my dictated text to my proxy"
        : "Send my dictated text to OpenRouter";
    $("ob_clean_consent_desc").textContent = custom
        ? `Your words go to ${llHost() || "your proxy"} as text. You confirm once for this address; a new address asks again.`
        : "Your words go to openrouter.ai as text, to zero-retention providers only. Your voice is not sent unless Cloud speech to text is also on.";
}
/** The host typed in the LiteLLM address field ("" when none). */
function llHost() {
    try {
        return new URL($("ob_ll_url").value.trim()).host;
    }
    catch {
        return "";
    }
}
function pickCloudProvider(p) {
    if (p === st.prov)
        return;
    st.prov = p;
    // Consent names a host: switching provider asks again.
    $("ob_clean_consent").checked = consentFor(p);
    renderCloudProvider();
    status("ob_cleanup_status", "");
}
function consentFor(p) {
    const c = views.or?.cleanup;
    if (!c)
        return false;
    if (p === "custom")
        return c.provider === "custom" && c.custom.consent;
    return c.provider === "openrouter" && c.consent;
}
/** "Test connection": the proxy's model list (typed key, else the saved one). */
async function testProxy() {
    const out = $("ob_ll_test_out");
    const baseUrl = $("ob_ll_url").value.trim();
    if (!baseUrl) {
        out.textContent = "Enter your proxy's address first.";
        return;
    }
    out.textContent = `Asking ${llHost() || "your proxy"} for its models…`;
    const typed = $("ob_ll_key").value.trim();
    const args = { baseUrl, key: typed === "" ? null : typed };
    try {
        let t0 = performance.now();
        let r = await invoke("dev_or_custom_models", { ...args, consent: false });
        // A saved key goes only to a host the user agreed to (D52): the tick is that agreement.
        if (r.error === "consent_needed") {
            if (!$("ob_clean_consent").checked) {
                out.textContent = "Not sent: tick “Send my dictated text to my proxy” below first, or paste the key here.";
                return;
            }
            t0 = performance.now();
            r = await invoke("dev_or_custom_models", { ...args, consent: true });
            views.setOr(await invoke("dev_or_get"));
        }
        const ms = Math.round(performance.now() - t0);
        if (r.models) {
            $("ob_ll_models").replaceChildren(...r.models.map((id) => Object.assign(document.createElement("option"), { value: id })));
            const model = $("ob_ll_model");
            if (!model.value && r.models.length === 1)
                model.value = r.models[0] ?? "";
            const n = r.models.length;
            out.textContent = `Connected · ${n} model${n === 1 ? "" : "s"} · ${ms} ms${n ? ": pick one in the Model field" : ""}`;
        }
        else {
            out.textContent = r.message ?? r.error ?? "No answer";
        }
    }
    catch (e) {
        out.textContent = errText(e);
    }
}
/** Save the LiteLLM address, model and (typed) key as the Cloud provider. */
async function applyProxy() {
    const baseUrl = $("ob_ll_url").value.trim();
    const model = $("ob_ll_model").value.trim();
    if (!baseUrl) {
        status("ob_cleanup_status", "Enter your proxy's address.", false);
        return false;
    }
    if (!model) {
        status("ob_cleanup_status", "Pick a model: Test connection lists them.", false);
        return false;
    }
    // D78: one Cloud provider at a time on Windows, and LiteLLM has no speech.
    if (st.engine === "cloud") {
        status("ob_cleanup_status", "A LiteLLM proxy can't do speech to text, and Cloud is your speech engine: pick OpenRouter here, or go back and pick another engine.", false);
        return false;
    }
    const typed = $("ob_ll_key").value.trim();
    if (typed) {
        await invoke("dev_cloud_set_custom_key", { key: typed });
        $("ob_ll_key").value = "";
    }
    const c = views.or?.cleanup.custom;
    views.setOr(await invoke("dev_or_set_custom_cleanup", {
        form: { provider: "custom", base_url: baseUrl, model, timeout_ms: c?.timeout_ms ?? 4000, consent: true },
    }));
    return true;
}
/** Back to OpenRouter when the saved provider is LiteLLM (its fields are kept). */
async function useOpenRouter() {
    const c = views.or?.cleanup;
    if (!c || c.provider === "openrouter")
        return;
    views.setOr(await invoke("dev_or_set_custom_cleanup", {
        form: {
            provider: "openrouter",
            base_url: c.custom.base_url,
            model: c.custom.model,
            timeout_ms: c.custom.timeout_ms,
            consent: false,
        },
    }));
}
function cleanupForm(allow, consent) {
    const c = views.or?.cleanup;
    return {
        allow,
        consent,
        // Choosing Cloud here makes it the first cleaner (D36); otherwise
        // the cleanup order is kept (undefined = keep).
        order: allow ? ["cloud", "local", "server"] : undefined,
        sort: c?.sort ?? "latency",
        timeout_ms: c?.timeout_ms ?? 2500,
        send_surrounding_text: c?.send_surrounding_text ?? false,
        send_screen_words: c?.send_screen_words ?? false,
        send_app_name: c?.send_app_name ?? false,
    };
}
async function applyCleanup() {
    const p = prefsStore.get();
    if (!p)
        return true;
    const level = p.cleanup_level === "none" ? "light" : p.cleanup_level;
    try {
        if (st.cleanup === "cloud") {
            if (!$("ob_clean_consent").checked) {
                status("ob_cleanup_status", `Tick “${$("ob_clean_consent_title").textContent ?? ""}” to use it.`, false);
                return false;
            }
            if (st.prov === "custom") {
                if (!(await applyProxy()))
                    return false;
            }
            else {
                if (!(await saveKey("ob_clean_key", "ob_cleanup_status")))
                    return false;
                await useOpenRouter();
            }
            views.setOr(await invoke("dev_or_set_cleanup", { form: cleanupForm(true, true) }));
            await prefsStore.save({ cleanup_level: level, spoken_cleanup: true, raw_mode: false });
            return true;
        }
        // Rules or this PC: Cloud cleanup off if it was on.
        if (views.or?.supported && views.or.cleanup.allow) {
            views.setOr(await invoke("dev_or_set_cleanup", { form: cleanupForm(false, false) }));
        }
        if (st.cleanup === "rules") {
            await prefsStore.save({ llm_enabled: false, cleanup_level: "none", spoken_cleanup: true, raw_mode: false });
        }
        else {
            await prefsStore.save({
                llm_enabled: true,
                cleanup_level: level,
                spoken_cleanup: true,
                raw_mode: false,
                llm_model_path: $("ob_llm_path").value.trim() || null,
            });
        }
        return true;
    }
    catch (e) {
        status("ob_cleanup_status", errText(e), false);
        return false;
    }
}
// ------------------------------------------------------------ step: try
let recordingPtt = false;
function showRecording(on) {
    recordingPtt = on;
    $("ob_ptt_rec").hidden = !on;
    $("ob_ptt_type").hidden = !on;
    $("ob_ptt_change").textContent = on ? "Cancel" : "Change";
}
/** Change: hold the new push-to-talk keys and let go (keyboard hook,
 *  lib/keycapture.ts); it saves through the field's `change`. */
async function recordPtt() {
    if (recordingPtt) {
        showRecording(false);
        stopCapture();
        return;
    }
    showRecording(true);
    const r = await captureChord();
    if (!recordingPtt)
        return; // Cancel / Type instead
    showRecording(false);
    if ("chord" in r) {
        const i = $("ob_ptt");
        i.value = r.chord;
        i.dispatchEvent(new Event("change"));
    }
    else if (r.message) {
        $("ob_shortcut_desc").textContent = r.message;
    }
}
/** Type instead: the text field, for keys that can't be pressed. */
function typePtt() {
    showRecording(false);
    stopCapture();
    const i = $("ob_ptt");
    i.hidden = false;
    $("ob_ptt_change").hidden = true;
    i.focus();
    i.select();
}
function renderTry() {
    const p = prefsStore.get();
    const ptt = p?.shortcuts.ptt || "Right Alt";
    $("ob_keys").replaceChildren(...ptt.split("+").map((k) => el("kbd", "fl-kbd", k.trim())));
    const hf = p?.shortcuts.hands_free;
    $("ob_shortcut_desc").textContent = `Hold ${ptt.replace(/\+/g, " + ")} to talk.${hf ? ` ${hf.replace(/\+/g, " + ")} = hands-free.` : ""}`;
    const pttInput = $("ob_ptt");
    if (document.activeElement !== pttInput)
        pttInput.value = ptt;
    const mic = $("ob_mic");
    mic.value = p?.microphone ?? "";
}
const WHERE = {
    local: "on this PC",
    client: "on this PC",
    remote: "by your server",
    server: "by your server",
    cloud: "in the cloud",
};
function showResult(v) {
    const last = v.last;
    if (!last || last.at_ms < st.openedAt || st.step !== "try")
        return;
    const r = $("ob_result");
    r.hidden = false;
    const good = last.status === "FORMATTED" || last.status === "RAW_TRANSCRIPT";
    r.classList.toggle("is-bad", !good);
    const where = WHERE[last.engine ?? ""] ?? (last.engine ? `by ${last.engine}` : "");
    const secs = (ms) => `${(ms / 1000).toFixed(1)} s`;
    $("ob_result_text").textContent = good
        ? `It works. Heard ${where} in ${secs(last.asr_ms)}${last.total_ms > last.asr_ms ? `, ready in ${secs(last.total_ms)}` : ""}.`
        : `That didn't work: ${last.notice ?? last.empty_reason ?? last.status}. Try again, or check Settings → Speech to text after setup.`;
}
// ------------------------------------------------------------ navigation
function show(s) {
    st.step = s;
    for (const p of document.querySelectorAll("#onboarding [data-ob-step]"))
        p.hidden = p.dataset.obStep !== s;
    const list = steps();
    const n = list.indexOf(s) + 1;
    const over = panel(s).querySelector(".fl-overline");
    if (over)
        over.textContent = `Step ${n} of ${list.length}`;
    if (s === "engine")
        renderEngine();
    if (s === "model")
        renderModel();
    if (s === "cleanup")
        renderCleanup();
    if (s === "try") {
        renderTry();
        st.openedAt = Date.now();
        $("ob_result").hidden = true;
        $("ob_try_box").focus();
    }
    panel(s).querySelector('[data-ob="next"]')?.focus({ preventScroll: true });
    if (s === "try")
        $("ob_try_box").focus();
}
async function next() {
    if (st.busy)
        return;
    st.busy = true;
    const btn = nextBtn(st.step);
    btn.disabled = true;
    try {
        let ok = true;
        if (st.step === "engine")
            ok = await applyEngine().catch((e) => {
                $("ob_engine_hint").textContent = errText(e);
                return false;
            });
        if (st.step === "cleanup")
            ok = await applyCleanup();
        if (!ok)
            return;
        if (st.step === "try") {
            await finish();
            return;
        }
        const list = steps();
        show(list[list.indexOf(st.step) + 1]);
    }
    finally {
        st.busy = false;
        btn.disabled = false;
        if (st.step === "model")
            renderModel();
    }
}
function back() {
    const list = steps();
    const i = list.indexOf(st.step);
    if (i > 0)
        show(list[i - 1]);
}
async function finish() {
    try {
        await prefsStore.save({ onboarded: true });
    }
    catch (e) {
        console.error("onboarded save failed", e);
    }
    $("onboarding").hidden = true;
    document.body.classList.remove("lw-ob-open");
    onDone();
}
// ------------------------------------------------------------ choices from the saved settings
function initialChoices() {
    const dev = views.dev?.form;
    const or = views.or;
    const first = dev?.route_order.find((r) => r === "client" ? dev.allow_client : r === "server" ? dev.allow_server && !!dev.host : !!or?.stt.allow);
    const p = prefsStore.get();
    st.engine = initialEngine(first ? ENGINE_OF[first] : null, !!p?.onboarded, has);
    st.cleanup = or?.cleanup.allow ? "cloud" : p?.llm_enabled && p.cleanup_level !== "none" ? "pc" : "rules";
    $("ob_host").value = dev?.host ?? "";
    $("ob_token").placeholder = dev?.token_saved ? "saved (hidden), leave empty to keep" : "from flow-server gen-token";
    $("ob_stt_consent").checked = !!or?.stt.consent;
    st.prov = or?.cleanup.provider ?? "openrouter";
    $("ob_clean_consent").checked = consentFor(st.prov);
    $("ob_ll_url").value = or?.cleanup.custom.base_url ?? "";
    $("ob_ll_model").value = or?.cleanup.custom.model ?? "";
    if (or?.cleanup.custom.key.set) {
        $("ob_ll_key").placeholder = `saved ${or.cleanup.custom.key.masked ?? ""}`;
        $("ob_ll_key_desc").textContent = "A key is saved. Paste a new one only to replace it.";
    }
    showKeyRows();
}
// ------------------------------------------------------------ public
export const onboarding = {
    /** Mount the overlay (hidden) once at boot. `done` runs after Finish / Set up later. */
    init(done) {
        onDone = done;
        document.body.append(html(TEMPLATE));
        const root = $("onboarding");
        root.addEventListener("click", (e) => {
            const t = e.target;
            const act = t.closest("[data-ob]")?.dataset.ob;
            if (act === "next")
                void next();
            else if (act === "back")
                back();
            else if (act === "skip")
                void finish();
            const eng = t.closest("[data-ob-engine]")?.dataset.obEngine;
            if (eng)
                pickEngine(eng);
            const cl = t.closest("[data-ob-cleanup]")?.dataset.obCleanup;
            if (cl)
                pickCleanup(cl);
            const use = t.closest("[data-ob-use]")?.dataset.obUse;
            if (use)
                void useModel(use);
            const pick = t.closest("[data-ob-pick]")?.dataset.obPick;
            if (pick)
                void pickModel(pick);
            const dl = t.closest("[data-ob-dl]")?.dataset.obDl;
            if (dl)
                void models.confirmAndStart(CATALOG_ID[dl]);
            const stop = t.closest("[data-ob-dl-cancel]")?.dataset.obDlCancel;
            if (stop)
                void models.cancel(CATALOG_ID[stop]);
        });
        // Enter in a model path field = Use.
        for (const [id, m] of [["ob_parakeet_dir", "parakeet"], ["ob_whisper_path", "whisper"]]) {
            $(id).addEventListener("keydown", (e) => {
                if (e.key === "Enter")
                    void useModel(m);
            });
        }
        $("ob_stt_consent").addEventListener("change", () => status("ob_cloud_status", ""));
        $("ob_clean_consent").addEventListener("change", () => status("ob_cleanup_status", ""));
        for (const b of document.querySelectorAll("#ob_cloud_prov_seg [data-ob-cloud-provider]")) {
            b.addEventListener("click", () => pickCloudProvider((b.dataset.obCloudProvider ?? "openrouter")));
        }
        $("ob_ll_test").addEventListener("click", () => void testProxy());
        $("ob_ll_url").addEventListener("input", renderCloudProvider);
        $("ob_test_conn").addEventListener("click", () => void testServer());
        $("ob_mic").addEventListener("change", () => {
            void prefsStore.save({ microphone: $("ob_mic").value || null });
        });
        $("ob_ptt_change").addEventListener("click", () => void recordPtt());
        $("ob_ptt_type").addEventListener("click", typePtt);
        $("ob_ptt").addEventListener("change", async () => {
            const p = prefsStore.get();
            const v = $("ob_ptt").value.trim();
            if (!p || !v)
                return;
            try {
                await prefsStore.save({ shortcuts: { ...p.shortcuts, ptt: v } });
                $("ob_ptt").hidden = true;
                $("ob_ptt_change").hidden = false;
            }
            catch (e) {
                $("ob_shortcut_desc").textContent = String(e);
                return;
            }
            renderTry();
        });
        // Download progress, and the prefs the backend saved when one finished.
        models.onChange(() => {
            if (st.step === "model")
                renderModel();
        });
        prefsStore.subscribe(() => {
            if (st.step === "model")
                renderModel();
        });
        void onEvent("model:download", (e) => {
            if (e.payload.state !== "done")
                return;
            void invoke("get_model_status").then((i) => {
                st.info = i;
                st.model = i.status;
                if (st.step === "model")
                    renderModel();
            });
        });
        void onEvent("model:status", (e) => {
            st.model = e.payload;
            if (st.step === "model")
                renderModel();
        });
        void onEvent("llm:status", (e) => {
            if (st.info)
                st.info.llm = e.payload;
            if (st.step === "cleanup")
                $("ob_llm_status").textContent = llmLine(e.payload);
        });
        void onEvent("dev:route", (e) => showResult(e.payload));
        void onEvent("history:changed", () => {
            if (st.step !== "try" || !$("ob_result").hidden)
                return;
            // Builds whose dictations bypass the router still say it arrived.
            setTimeout(() => {
                if (!$("ob_result").hidden)
                    return;
                $("ob_result").hidden = false;
                $("ob_result_text").textContent = "It works. Your dictation arrived.";
            }, 300);
        });
    },
    isOpen() {
        return !$("onboarding").hidden;
    },
    /** Show setup from the first step, choices filled from the saved settings. */
    async open() {
        try {
            const [info, mics] = await Promise.all([
                invoke("get_model_status"),
                invoke("list_microphones"),
                models.refresh().catch(() => []),
            ]);
            st.info = info;
            st.model = info.status;
            $("ob_mic").replaceChildren(new Option("System default", ""), ...mics.map((m) => new Option(m, m)));
        }
        catch (e) {
            console.error("setup: model status failed", e);
        }
        initialChoices();
        $("onboarding").hidden = false;
        document.body.classList.add("lw-ob-open");
        show("welcome");
    },
};
