// OpenRouter (D28, docs/proposals/openrouter-only-cloud.md §2.2-§2.6): the
// only cloud. One key (Credential Manager flow-clone/cloud/openrouter,
// never sent back to this page), two separately consented uses (speech =
// audio, cleanup = text), shared privacy + budget. Every change is applied
// at once by a `dev_or_set_*` command that returns the whole OrView.
import { $, askDialog, el, setStatus, setValue, when } from "../lib/dom.js";
import { cloudHint, failText, money, usd } from "../lib/format.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { views } from "../lib/views.js";
const PARAPHRASE_BADGE = "chat model, may paraphrase";
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
        views.setOr(null);
    }
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
    $("or_used_stt").textContent = s.allow
        ? `Sends audio · ${s.model}`
        : "Off: speech is turned into text on your own engines.";
    $("or_used_cleanup").textContent = c.allow
        ? `Sends text · ${c.model} · ${c.order[0] === "local" ? "after this PC" : "first, then this PC"}`
        : "Off: text is cleaned up on your own engines.";
    setValue("or_budget", v.budget == null ? "" : String(v.budget));
    showOrUsage(v.usage);
    showCatalogStatus(v.catalog);
    views.setOr(v);
}
function showOrUsage(u) {
    if (views.or)
        views.or.usage = u;
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
    if (views.or)
        views.or.cleanup.status = st;
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
    if (views.or)
        views.or.catalog = c;
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
// ---- confirm dialog
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
    const v = views.or;
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
    else if (views.or)
        showOr(views.or);
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
    else if (views.or)
        showOr(views.or);
    return r !== null;
}
async function orSttToggled() {
    const box = $("or_stt_allow");
    if (!box.checked) {
        await orApplyStt(false);
        return;
    }
    let consent = false;
    if (!views.or?.stt.consent) {
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
    else if (views.or)
        showOr(views.or);
    return r !== null;
}
async function orCleanupToggled() {
    const box = $("or_cleanup_allow");
    if (!box.checked) {
        await orApplyCleanup(false);
        return;
    }
    let consent = false;
    if (!views.or?.cleanup.consent) {
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
    return pickerPurpose === "stt" ? (views.or?.stt.model ?? "") : (views.or?.cleanup.model ?? "");
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
    else if (views.or)
        showCatalogStatus(views.or.catalog);
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
function wire() {
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
    void onEvent("dev:route", () => {
        if (views.or?.supported)
            void refreshOrAfterDictation();
    });
}
export const openrouter = {
    init() {
        wire();
    },
    load: loadOr,
    shown(page, opened) {
        // D28 fetch policy: the catalogue is refreshed (network) once per opening of the OpenRouter page.
        if (page === "settings-openrouter" && opened && views.or?.supported)
            void refreshCatalog(false);
    },
};
