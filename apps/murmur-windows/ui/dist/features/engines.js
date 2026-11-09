// Engines: hybrid local/server routing (docs/hybrid-architecture.md §5),
// speech decoding, the server connection and engine status, on the Speech
// to text, System, Your server and Developer settings pages (designs:
// SettingsSpeech / SettingsServer .dc.html). The backend
// owns validation (`dev_validate`), so the inline errors and Save use the
// same rules; the token is typed here but stored DPAPI-encrypted. Saves go
// through the dev store: switches save at once, the server form on Save.
import { $, el, setStatus } from "../lib/dom.js";
import { devStore } from "../lib/dev-store.js";
import { cloudHint } from "../lib/format.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { prefsStore } from "../lib/prefs-store.js";
import { router } from "../lib/router.js";
import { moveShown, onMove, rankRows } from "../lib/ranked-list.js";
import { views } from "../lib/views.js";
import { NOTETAKER_ENABLED } from "../pages/notetaker.js";
/** Engine switches that save at once (checkbox id = `dev_<field>`). */
const DEV_SWITCHES = ["allow_server", "allow_client", "keep_local_warm", "show_engine_badge", "decode_while_speaking"];
/** The server form (Your server page), saved with its Save button. */
const SRV_NUM = ["port", "connect_timeout_ms", "commit_timeout_base_ms", "health_ttl_s"];
const SRV_TEXT = ["host", "tls_pin_sha256", "token"];
let devOrder = ["server", "client", "cloud"];
/** The speech model on this PC (get_model_status / `model:status`), for its engine badge. */
let pcModel = { state: "loading" };
/** A Test button's result is on screen: the last known health must not replace it. */
let testShown = false;
let devValid = true;
/** The server form has edits that are not saved yet (a switch's save must not overwrite them). */
let serverDirty = false;
/** Clear was pressed: the saved token is removed on Save. */
let tokenCleared = false;
const HINTS = {
    auth_failed: "token rejected — copy it again from the server's token file",
    connect_failed: "is flow-server running / LAN or VPN up?",
    proto_mismatch: "client and server protocol versions differ — update one of them",
    server_not_configured: "enter the server host first",
    timeout: "the server is too slow or stuck",
    busy: "the server is busy with another dictation",
};
function hint(code) {
    return code ? (HINTS[code] ?? "") : "";
}
/** The Your server page's fields as a form patch. */
function readServer() {
    const f = { tls: $("dev_tls").checked };
    for (const k of SRV_TEXT)
        f[k] = $(`dev_${k}`).value;
    for (const k of SRV_NUM) {
        const v = $(`dev_${k}`).value.trim();
        f[k] = v === "" ? -1 : Number(v);
    }
    if (tokenCleared)
        f.token_saved = false;
    return f;
}
/** "1.5 seconds" / "2 minutes" for a saved timeout that is not one of the presets. */
function durationLabel(field, v) {
    const s = field.endsWith("_ms") ? v / 1000 : v;
    if (s >= 60 && s % 60 === 0)
        return `${s / 60} minute${s === 60 ? "" : "s"}`;
    return `${s} second${s === 1 ? "" : "s"}`;
}
/** Set a number field; a preset list gets the saved value as an extra choice when it is not a preset. */
function setNum(field, v) {
    const e = $(`dev_${field}`);
    if (e instanceof HTMLSelectElement && ![...e.options].some((o) => o.value === String(v))) {
        const opt = new Option(durationLabel(field, v), String(v));
        const after = [...e.options].find((o) => Number(o.value) > v);
        e.add(opt, after ?? null);
    }
    e.value = String(v);
}
function fillServer(f) {
    $("dev_tls").checked = f.tls;
    for (const k of SRV_TEXT)
        $(`dev_${k}`).value = f[k];
    for (const k of SRV_NUM) {
        if (k === "port")
            $("dev_port").value = String(f.port);
        else
            setNum(k, f[k]);
    }
    tokenCleared = false;
    showTokenSaved(!!f.token_saved);
}
function fillDev(f) {
    for (const k of DEV_SWITCHES)
        $(`dev_${k}`).checked = f[k];
    // Beam search with hotwords = dictionary words; greedy = without.
    $("dev_parakeet_decoding").checked = (f.parakeet_decoding ?? "auto") !== "greedy";
    devOrder = [...f.route_order];
    if (!serverDirty)
        fillServer(f);
    renderRoutes();
}
// The saved token never comes back to the page (S18): an empty field
// keeps it; typing replaces it; Clear removes it on Save.
function showTokenSaved(saved) {
    $("dev_token").placeholder = saved
        ? "saved (hidden) — leave empty to keep it"
        : "from flow-server gen-token";
}
/** Which engines this build (and the OpenRouter page) offers. */
function available(r) {
    const caps = views.dev?.caps;
    if (r === "server")
        return !!caps?.remote;
    if (r === "client")
        return !!caps?.local;
    return !!caps?.cloud && !!views.or?.supported;
}
function engineOn(r) {
    if (r === "server")
        return $("dev_allow_server").checked;
    if (r === "client")
        return $("dev_allow_client").checked;
    return !!views.or?.stt.allow;
}
function setBadge(r, text, kind) {
    const b = $(`eng_badge_${r}`);
    b.textContent = text;
    b.className = `fl-badge fl-badge-${kind}`;
}
/** A link to another settings page inside a row's description. */
function goLink(page, text) {
    const b = el("button", "lw-link", text);
    b.type = "button";
    b.dataset.go = page;
    return b;
}
function fileStem(path) {
    return ((path ?? "").split(/[\\/]/).pop() ?? "").replace(/\.(bin|onnx)$/i, "");
}
/** One line per engine: what it is and whether it answers. */
function describe(r) {
    const on = engineOn(r);
    const d = $(`eng_desc_${r}`);
    if (r === "server") {
        const f = views.dev?.form;
        const h = views.dev?.route.health;
        const rtt = h?.ok && h.rtt_ms ? ` · ${h.rtt_ms.median} ms` : "";
        if (f?.host)
            d.replaceChildren(`${f.host}${rtt}. `, goLink("settings-server", "Connection →"));
        else
            d.replaceChildren("Not set up yet. ", goLink("settings-server", "Set up →"));
        if (!on)
            setBadge(r, "Off", "light");
        else if (!h)
            setBadge(r, "On", "light");
        else
            setBadge(r, h.ok ? "Connected" : "Unreachable", h.ok ? "success" : "destructive");
    }
    else if (r === "client") {
        const parakeet = prefsStore.get()?.asr_engine === "parakeet";
        const stem = fileStem(pcModel.path);
        const name = parakeet ? "Parakeet TDT 0.6B" : `Whisper${stem ? ` (${stem})` : ""}`;
        d.textContent = `${name}. Private and works offline.`;
        const st = pcModel.state;
        if (!on)
            setBadge(r, "Off", "light");
        else if (st === "ready")
            setBadge(r, "Ready", "success");
        else if (st === "loading")
            setBadge(r, "Loading", "light");
        else if (st === "missing")
            setBadge(r, "Model missing", "destructive");
        else
            setBadge(r, "Failed", "destructive");
    }
    else {
        const or = views.or;
        d.textContent =
            on && or ? `${or.stt.model.split("/").pop()}. Sends your audio to openrouter.ai.` : "Sends your audio to openrouter.ai. Asks first.";
        setBadge(r, on ? "On" : "Off", on ? "success" : "light");
    }
}
function renderRoutes() {
    const shown = rankRows({
        list: $("dev_routes"),
        row: (r) => $(`eng_row_${r}`),
        shown: available,
        on: engineOn,
    }, devOrder);
    shown.forEach(describe);
    // Builds with one engine and no server or cloud: nothing to choose.
    $("eng_group").hidden = shown.length === 0 || (shown.length === 1 && !views.dev?.visible);
    $("dev_pin_row").hidden = !$("dev_tls").checked;
    // Dictionary words while listening = Parakeet's beam search with hotwords.
    $("dict_words_row").classList.toggle("lw-gone", $("asr_engine").value !== "parakeet");
    renderMeetings();
}
function move(r, dir) {
    const order = moveShown(devOrder, (x) => x, available, r, dir);
    if (order)
        void saveSwitch({ route_order: order });
}
// ------------------------------------------- meeting order (D47)
/** The saved meeting order; an older backend without it = dictation's. */
function meetingRoutes() {
    return (views.dev?.meetings?.routes ?? devOrder.map((route) => ({ route, enabled: route === "cloud" ? !!views.or?.stt.allow : engineOn(route) })));
}
function meetingOn(r) {
    return meetingRoutes().some((e) => e.route === r && e.enabled);
}
function setMeetingBadge(r, text, kind) {
    const b = $(`meet_badge_${r}`);
    b.textContent = text;
    b.className = `fl-badge fl-badge-${kind}`;
}
/** One line per meeting engine: what it is and whether it would answer. */
function describeMeeting(r) {
    const on = meetingOn(r);
    const d = $(`meet_desc_${r}`);
    if (r === "server") {
        const f = views.dev?.form;
        const h = views.dev?.route.health;
        if (f?.host)
            d.textContent = `${f.host}. Uses the server's model.`;
        else
            d.replaceChildren("Not set up yet. ", goLink("settings-server", "Set up →"));
        if (!on)
            setMeetingBadge(r, "Off", "light");
        else if (!f?.host)
            setMeetingBadge(r, "Not set up", "warning");
        else if (!h)
            setMeetingBadge(r, "On", "light");
        else
            setMeetingBadge(r, h.ok ? "Connected" : "Unreachable", h.ok ? "success" : "destructive");
    }
    else if (r === "client") {
        d.textContent = "Uses its own copy of the model while a meeting is transcribed. Private and works offline.";
        setMeetingBadge(r, on ? "On" : "Off", on ? "success" : "light");
    }
    else {
        d.textContent = "Sends the meeting's audio to openrouter.ai.";
        setMeetingBadge(r, on ? "On" : "Off", on ? "success" : "light");
    }
}
function renderMeetings() {
    const routes = meetingRoutes();
    for (const r of ["server", "client", "cloud"])
        $(`meet_on_${r}`).checked = meetingOn(r);
    const shown = rankRows({
        list: $("meet_routes"),
        row: (r) => $(`meet_row_${r}`),
        shown: available,
        on: meetingOn,
        before: $("meet_routes").querySelector(".lw-foot-row"),
    }, routes.map((e) => e.route));
    shown.forEach(describeMeeting);
    $("meet_group").hidden = !NOTETAKER_ENABLED || !views.dev?.meetings || shown.length === 0 || (shown.length === 1 && !views.dev?.visible);
    $("meet_follow").textContent = views.dev?.meetings?.follows_dictation
        ? "Same as the engines above until you change it here."
        : "";
}
/** Save the meeting order and switches; the answer is the new DevView. */
async function saveMeetings(routes) {
    try {
        views.setDev(await invoke("dev_meeting_routes_set", { routes }));
        $("meet_order_msg").textContent = "";
        setStatus("status", "Saved — applies to the next meeting", true);
    }
    catch (e) {
        setStatus("meet_order_msg", String(e), false);
        renderMeetings(); // back to what is saved
    }
}
function moveMeeting(r, dir) {
    const routes = moveShown(meetingRoutes(), (e) => e.route, available, r, dir);
    if (routes)
        void saveMeetings(routes);
}
function meetingToggled(r) {
    const on = $(`meet_on_${r}`).checked;
    void saveMeetings(meetingRoutes().map((e) => (e.route === r ? { ...e, enabled: on } : { ...e })));
}
function showCheck(c) {
    for (const s of document.querySelectorAll("[data-err]"))
        s.textContent = "";
    for (const i of document.querySelectorAll("[data-dev-field]"))
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
        if (!views.dev)
            return;
        void invoke("dev_validate", { form: devStore.form(readServer()) }).then(showCheck);
    }, 150);
}
/** A backend error as text; field errors also go next to their fields. */
function errorText(e) {
    const errs = e;
    if (Array.isArray(errs)) {
        showCheck({ errors: errs, warnings: [], tls_forced: false });
        const fields = errs.map((x) => x.message).join("; ");
        return `Your server needs fixing first: ${fields}`;
    }
    return String(e);
}
/** A switch or the order changed: save it alone, over the saved form. */
async function saveSwitch(patch) {
    try {
        await devStore.save(patch);
        setStatus("status", "Saved — applies to the next dictation", true);
    }
    catch (e) {
        setStatus("status", errorText(e), false);
        if (views.dev)
            fillDev(views.dev.form); // back to what is saved
    }
}
function healthText(h) {
    if (!h.ok) {
        const code = h.error ?? "error";
        // flow-remote refuses ws:// to an address that is not private/VPN (S6).
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
/** The Your server page's result box: `ok` null = neutral (running). */
function showResult(ok, lead, rest) {
    const box = $("dev_test");
    box.hidden = false;
    box.classList.toggle("is-ok", ok === true);
    box.classList.toggle("is-err", ok === false);
    const t = $("dev_test_text");
    t.replaceChildren();
    if (lead)
        t.append(el("b", undefined, lead), " ");
    t.append(rest);
}
/** "38 ms · flow-server 0.6.0 · parakeet · same pipeline version as this app". */
function connectedLine(h) {
    const pp = h.postprocess_match === false
        ? "different pipeline version: results may differ from this PC"
        : h.postprocess_match
            ? "same pipeline version as this app"
            : "";
    return [h.rtt_ms ? `${h.rtt_ms.median} ms` : "", h.server_version ?? "", h.asr_backend ?? "", pp]
        .filter(Boolean)
        .join(" · ");
}
function showHealth(h) {
    if (h.ok)
        showResult(h.postprocess_match === false ? null : true, "Connected.", connectedLine(h));
    else
        showResult(false, "Not connected.", healthText(h));
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
    if (r.health && !testShown)
        showHealth(r.health);
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
    fillDev(v.form);
    $("dev_lock").hidden = v.caps.remote;
    const c = v.capabilities;
    $("dev_build").textContent =
        `Murmur ${v.full} · ` +
            `${[c.local ? "client" : "", c.remote ? "server" : ""].filter(Boolean).join(" + ") || "no engine"}` +
            ` · ASR ${c.asr_engines.join(", ") || "none"} · LLM ${c.llm ? "yes" : "no"} · proto ${c.proto} · core ${c.core_version}`;
    showCheck(v.check);
    showRoute(v.route);
}
async function loadDev() {
    try {
        pcModel = (await invoke("get_model_status")).status;
    }
    catch {
        /* the badge stays "Loading" */
    }
    try {
        views.setDev(await invoke("dev_get"));
    }
    catch (e) {
        setStatus("status", String(e), false);
    }
}
async function devCall(cmd, args, okText, statusId) {
    try {
        const r = await invoke(cmd, args);
        setStatus(statusId, okText, true);
        return r;
    }
    catch (e) {
        setStatus(statusId, errorText(e), false);
        return null;
    }
}
async function serverSave() {
    if (!devValid)
        return;
    try {
        serverDirty = false;
        await devStore.save(readServer());
        setStatus("dev_status", "Saved — applies to the next dictation", true);
    }
    catch (e) {
        serverDirty = true;
        setStatus("dev_status", errorText(e), false);
    }
}
async function devTestConnection() {
    if (!devValid)
        return;
    testShown = true;
    showResult(null, "", "Testing…");
    try {
        showHealth(await invoke("dev_test_connection", { form: devStore.form(readServer()) }));
    }
    catch (e) {
        showResult(false, "Not connected.", String(e));
    }
}
/** One silent 1 s session through the saved settings (Developer: Test session; Your server: Test a dictation). */
async function devTestSession(show) {
    show(null, "Running a 1 s silent session through the saved settings…");
    try {
        const r = await invoke("dev_test_session");
        const engine = r.engine === "remote" ? "server" : r.engine === "local" ? "client" : "none";
        const ok = r.status === "EMPTY" && r.empty_reason === "no_speech";
        show(ok, `Test session: ${r.status}${r.empty_reason ? ` (${r.empty_reason})` : ""} via ${engine}` +
            `${r.fallback ? ` (fallback: ${r.fallback} — ${hint(r.fallback)})` : ""} in ${r.total_ms} ms` +
            `${r.error ? ` — ${r.error}` : ""}`);
    }
    catch (e) {
        show(false, String(e));
    }
    void refreshRoute();
}
function showDiag(ok, text) {
    const out = $("dev_diag");
    out.textContent = text;
    out.className = ok === null ? "hint" : ok ? "hint ok" : "hint err";
}
async function refreshRoute() {
    try {
        showRoute(await invoke("dev_route_status"));
    }
    catch {
        /* not loaded yet */
    }
}
let versionTaps = 0;
let versionTimer = 0;
/** Seven taps on the version line unlock the Developer page. */
export function versionTapped() {
    const dv = views.dev;
    if (!dv || dv.visible)
        return;
    versionTaps++;
    clearTimeout(versionTimer);
    versionTimer = setTimeout(() => (versionTaps = 0), 3000);
    if (versionTaps >= 7) {
        versionTaps = 0;
        void devCall("dev_unlock", { unlocked: true }, "Developer settings unlocked", "status").then((v) => {
            if (v) {
                views.setDev(v);
                router.go("settings-developer");
            }
        });
    }
}
export const engines = {
    init() {
        for (const k of DEV_SWITCHES) {
            const box = $(`dev_${k}`);
            box.addEventListener("change", () => void saveSwitch({ [k]: box.checked }));
        }
        const dec = $("dev_parakeet_decoding");
        dec.addEventListener("change", () => void saveSwitch({ parakeet_decoding: dec.checked ? "auto" : "greedy" }));
        $("asr_engine").addEventListener("change", () => views.dev && renderRoutes());
        void onEvent("model:status", (e) => {
            pcModel = e.payload;
            if (views.dev)
                renderRoutes();
        });
        onMove($("dev_routes"), "route", move);
        onMove($("meet_routes"), "meeting", moveMeeting);
        for (const r of ["server", "client", "cloud"])
            $(`meet_on_${r}`).addEventListener("change", () => meetingToggled(r));
        for (const i of document.querySelectorAll("[data-dev-field]")) {
            const edited = () => {
                serverDirty = true;
                scheduleCheck();
            };
            i.addEventListener("input", edited);
            i.addEventListener("change", edited);
        }
        $("dev_save").addEventListener("click", () => void serverSave());
        $("dev_test_conn").addEventListener("click", () => void devTestConnection());
        $("dev_test_session").addEventListener("click", () => void devTestSession(showDiag));
        $("srv_test_dictation").addEventListener("click", () => {
            testShown = true;
            void devTestSession((ok, text) => showResult(ok, ok ? "Dictation works." : ok === false ? "Dictation failed." : "", text));
        });
        $("dev_reset_circuit").addEventListener("click", () => void devCall("dev_reset_circuit", {}, "Circuit reset", "dev_diag_status").then((r) => r && showRoute(r)));
        $("dev_reset").addEventListener("click", () => void devCall("dev_reset_defaults", {}, "Defaults restored", "dev_diag_status").then((v) => {
            if (!v)
                return;
            serverDirty = false;
            views.setDev(v);
        }));
        $("dev_lock").addEventListener("click", () => void devCall("dev_unlock", { unlocked: false }, "Developer settings locked", "status").then((v) => {
            if (v) {
                views.setDev(v);
                router.go("settings-general");
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
                serverDirty = true;
                scheduleCheck();
            })
                .catch(() => setStatus("dev_status", "Clipboard not readable: paste with Ctrl+V", false));
        });
        $("dev_token_clear").addEventListener("click", () => {
            $("dev_token").value = "";
            tokenCleared = true;
            serverDirty = true;
            showTokenSaved(false);
            scheduleCheck();
        });
        views.onDev(showDev);
        // Turning STT on put `cloud` first in the saved route order (backend):
        // show that order here too.
        views.onOr((v, prev) => {
            if (v?.stt.allow && prev && !prev.stt.allow && v.route_order)
                devOrder = [...v.route_order];
            renderRoutes();
        });
        prefsStore.subscribe(() => {
            if (views.dev)
                renderRoutes();
        });
        void onEvent("dev:route", (e) => {
            showRoute(e.payload);
            if (views.dev) {
                views.dev.route = e.payload;
                renderRoutes();
            }
        });
    },
    load: loadDev,
    shown(page, opened) {
        if (opened && (page === "settings-speech" || page === "settings-developer"))
            void refreshRoute();
    },
};
