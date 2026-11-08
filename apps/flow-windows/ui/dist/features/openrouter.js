// OpenRouter (D28, docs/proposals/openrouter-only-cloud.md §2.2-§2.6): the
// only cloud. One key (Credential Manager flow-clone/cloud/openrouter,
// never sent back to this page), two separately consented uses (speech =
// audio, cleanup = text), shared privacy + budget. Every change is applied
// at once by a `dev_or_set_*` command that returns the whole OrView.
// Designs: SettingsOpenRouter / ModelPicker .dc.html, and the OpenRouter
// groups of SettingsSpeech / SettingsCleanup.
// D52: text cleanup can go to the user's own OpenAI-compatible server
// instead (provider selector on the cleanup card; its own optional key
// `flow-clone/cloud/custom` and consent bound to the typed host).
import { $, askDialog, el, setStatus, setValue, when } from "../lib/dom.js";
import { iconEl } from "../lib/icons.js";
import { cleanupBlockedLine, cleanupEngineName, cleanupTestFailText, plainHint } from "../lib/cleanup-state.js";
import { cleanupMonthCost, failText, money, usd } from "../lib/format.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { Serial } from "../lib/serial.js";
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
function consentLine(at, given, what, host = "openrouter.ai") {
    return given && at > 0
        ? `You agreed to send ${what} to ${host} on ${new Date(at).toLocaleDateString()}.`
        : "Not yet agreed: turning this on asks first.";
}
/** The address as the router normalises it (the page never parses URLs). */
async function addressOf(url, model = "") {
    try {
        return await invoke("dev_custom_cleanup_address", { baseUrl: url, model });
    }
    catch {
        return { error: "bad_base_url", model_ok: false };
    }
}
/** Canonical host of a typed server address (`""` when it is unusable). */
async function hostOf(url) {
    return (await addressOf(url)).host ?? "";
}
/** Show where requests go, as normalised. */
async function showNormalisedUrl() {
    const raw = $("cu_base_url").value;
    const a = raw.trim() === "" ? null : await addressOf(raw);
    $("cu_url_norm").textContent = a?.url ? `Requests go to ${a.url}/chat/completions` : a ? a.message ?? "" : "";
}
/** Where cloud cleanup text goes, for status lines. */
export function cleanupTarget(v) {
    if (v?.cleanup.provider !== "custom")
        return "openrouter.ai";
    return v.cleanup.custom.host || "your LiteLLM proxy";
}
/** "Transcription model · $0.04/h audio" (+ this month's cleanup spend). */
function modelDesc(info, extra) {
    if (!info)
        return "Not in the model list.";
    const kind = info.stt_path === "transcriptions" ? "Transcription model" : info.stt_path === "chat_audio" ? "Chat model with audio" : "Text model";
    return [kind, info.price_label, extra].filter(Boolean).join(" · ");
}
function showModelRow(prefix, id, info, extra = "") {
    $(`${prefix}_model_name`).textContent = info?.name || id;
    $(`${prefix}_model_desc`).textContent = modelDesc(info, extra);
    $(`${prefix}_model_id`).textContent = id;
    $(`${prefix}_model_badges`).replaceChildren(...badges(info, true));
}
/** A model's badges as the design shows them (fl-badge). `zdr`: show "Zero retention". */
function badges(r, zdr) {
    if (!r)
        return [];
    const out = [];
    if (r.recommended)
        out.push(el("span", "fl-badge fl-badge-dark", "Recommended"));
    if (zdr && r.zdr)
        out.push(el("span", "fl-badge fl-badge-neutral", "Zero retention"));
    for (const b of r.badges) {
        if (b === "ZDR")
            continue;
        if (b === "fast")
            out.push(el("span", "fl-badge fl-badge-light", "Fast"));
        else
            out.push(el("span", b === PARAPHRASE_BADGE ? "fl-badge fl-badge-warning" : "fl-badge fl-badge-light", b));
    }
    if (r.allowed_by_account === false)
        out.push(el("span", "fl-badge fl-badge-destructive", "blocked by your OpenRouter privacy settings"));
    return out;
}
/** Select `value` in a preset list, adding it as a choice when it is not a preset. */
function ensureOption(id, value, label) {
    const sel = $(id);
    if (![...sel.options].some((o) => o.value === value)) {
        const after = [...sel.options].find((o) => o.value !== "" && Number(o.value) > Number(value));
        sel.add(new Option(label, value), after ?? null);
    }
    sel.value = value;
}
function setBadge(id, on) {
    const b = $(id);
    b.textContent = on ? "On" : "Off";
    b.className = `fl-badge ${on ? "fl-badge-success" : "fl-badge-light"}`;
}
/** "$0.0234" or 0.0234 as a number. */
function amount(v) {
    const n = typeof v === "number" ? v : Number(String(v ?? "").replace(/[^0-9.]/g, ""));
    return Number.isFinite(n) ? n : 0;
}
/** Show an OrView everywhere (also used by the AI cleanup page's cleanup order). */
export function showOr(v) {
    $("or_warnings").replaceChildren(...v.warnings.map((w) => el("li", "", w)));
    $("or_notice_box").hidden = !v.notice;
    $("or_notice").textContent = v.notice ?? "";
    const ks = $("or_key_state");
    ks.textContent = v.key.set ? `Key ${v.key.masked}` : "No key saved";
    ks.classList.toggle("lw-mono", v.key.set);
    $("or_key_saved_ctl").hidden = !v.key.set;
    $("or_key_entry").hidden = v.key.set;
    $("or_credit_row").hidden = !v.key.set;
    $("or_zdr").checked = v.privacy.zdr;
    $("or_dc_deny").checked = v.privacy.data_collection === "deny";
    const s = v.stt;
    $("or_stt_allow").checked = s.allow;
    $("or_stt_consent").textContent = consentLine(s.consent_at_ms, s.consent, "audio");
    showModelRow("or_stt", s.model, v.stt_model_info);
    $("or_stt_box").classList.toggle("lw-dim", !s.allow);
    $("or_stt_used_badge").hidden = s.allow;
    setValue("or_stt_language", s.language);
    $("or_stt_send_dictionary").checked = s.send_dictionary;
    $("or_stt_path").value = s.stt_path;
    const c = v.cleanup;
    const custom = c.provider === "custom";
    $("or_cleanup_allow").checked = c.allow;
    $("or_cleanup_consent").textContent = custom
        ? consentLine(c.custom.consent_at_ms, c.custom.consent, "dictated text", c.custom.consent_host)
        : consentLine(c.consent_at_ms, c.consent, "dictated text");
    showCustom(v);
    const orSpent = cleanupMonthCost(c.status).openrouter;
    const spent = orSpent > 0 ? `${money(orSpent)} so far this month` : "";
    showModelRow("or_cleanup", c.model, v.cleanup_model_info, spent);
    $("or_cleanup_box").classList.toggle("lw-dim", !c.allow);
    $("or_cleanup_sort").value = c.sort;
    ensureOption("or_cleanup_timeout_ms", String(c.timeout_ms), `${c.timeout_ms / 1000} seconds`);
    $("or_cleanup_send_surrounding_text").checked = c.send_surrounding_text;
    $("or_cleanup_send_screen_words").checked = c.send_screen_words;
    $("or_cleanup_send_app_name").checked = c.send_app_name;
    showCleanupStatus(c.status);
    const short = (id) => id.split("/").pop() ?? id;
    // D78: LiteLLM does cleanup only; speech stays on this PC and your server.
    const sttAvailable = s.available !== false && !custom;
    $("or_used_stt").textContent = !sttAvailable
        ? "Your proxy has no speech model here, so speech stays on this PC and your server. Pick OpenRouter to send speech to the cloud."
        : s.allow
            ? `Sends audio · ${short(s.model)}`
            : `Would send audio · ${short(s.model)}`;
    // D36: the cloud's place among the engines that are on.
    const firstOn = c.engines.find((e) => e.enabled)?.route;
    const cleanupModel = custom ? c.custom.model || "no model picked yet" : short(c.model);
    $("or_used_cleanup").textContent = `${c.allow ? "Sends" : "Would send"} text · ${cleanupModel}${c.allow ? ` · ${firstOn === "cloud" ? "first" : "as a fallback"}` : ""}`;
    setBadge("or_used_stt_badge", s.allow && sttAvailable);
    if (!sttAvailable) {
        const b = $("or_used_stt_badge");
        b.textContent = "Not available";
        b.className = "fl-badge fl-badge-light";
    }
    setBadge("or_used_cleanup_badge", c.allow);
    const sttLink = $("or_used_stt_link");
    sttLink.hidden = !sttAvailable;
    sttLink.textContent = s.allow ? "Manage →" : "Set up →";
    $("or_used_cleanup_link").textContent = c.allow ? "Manage →" : "Set up →";
    $("cloud_ll_host").textContent = c.custom.host || "your proxy";
    $("cloud_ll_consent").textContent = c.custom.consent
        ? `You agreed on ${new Date(c.custom.consent_at_ms).toLocaleDateString()} to send dictated text here.`
        : "Asks before the first dictation is sent.";
    if (document.activeElement !== $("or_budget")) {
        ensureOption("or_budget", v.budget == null ? "" : String(v.budget), `$${v.budget} a month`);
    }
    showOrUsage(v.usage);
    showCatalogStatus(v.catalog);
    views.setOr(v);
}
function showOrUsage(u) {
    if (views.or)
        views.or.usage = u;
    $("or_usage_amount").textContent = money(u.total);
    const of = $("or_usage_of");
    of.textContent =
        u.budget != null
            ? `of your $${u.budget.toFixed(2)} budget${u.budget_reached ? ": reached, OpenRouter is paused until next month" : ""}`
            : `spent in ${u.month} · no budget set`;
    of.classList.toggle("err", u.budget_reached);
    const meter = $("or_usage_meter");
    meter.hidden = u.budget == null || u.budget <= 0;
    if (u.budget)
        $("or_usage_bar").style.width = `${Math.min(100, (amount(u.total) / u.budget) * 100)}%`;
    meter.classList.toggle("is-full", u.budget_reached);
    // OpenRouter's own counts; the custom server (D52) has its own row below.
    const dictations = u.rows.filter((r) => r.purpose === "stt" || r.purpose === "removed").reduce((n, r) => n + r.requests, 0);
    const cleanups = u.rows.filter((r) => r.purpose === "cleanup").reduce((n, r) => n + r.requests, 0);
    $("or_usage_total").textContent =
        `Speech ${money(u.speech)} · ${dictations} dictations    Cleanup ${money(u.cleanup)} · ${cleanups} cleanups`;
    $("or_usage_rows").replaceChildren(...u.rows.map((r) => el("li", "", usageRowText(r))));
}
/** One usage line; the custom server's counts its cleanups even when it reports no cost (D52). */
function usageRowText(r) {
    // Its cost is the server's own figure or "no cost reported", already worded.
    if (r.purpose === "custom")
        return `${r.label}: ${r.requests} cleanups · ${r.cost} (not in budget)`;
    const what = r.purpose === "cleanup" ? "cleanups" : "dictations";
    const mins = r.purpose === "cleanup" ? "" : ` · ${r.minutes} min`;
    return `${r.label}: ${r.requests} ${what}${mins} · ${money(r.cost)}`;
}
function showCleanupStatus(st) {
    if (views.or)
        views.or.cleanup.status = st;
    const e = $("or_cleanup_state");
    const blocked = cleanupBlockedLine(st, views.or);
    let text;
    let cls = "hint";
    if (blocked) {
        text = blocked.text;
        cls = blocked.cls;
    }
    else if (st.circuit?.open) {
        const until = st.circuit.until_config_change
            ? "until the key or cleanup settings change"
            : `until ${new Date(Date.now() + (st.circuit.retry_in_ms ?? 0)).toLocaleTimeString()}`;
        text = `Paused ${until} (${plainHint(st.circuit.reason)})`;
        cls = "hint err";
    }
    else {
        text = `On · ${st.model} · up to ${st.timeout_ms} ms`;
        cls = "hint ok";
    }
    const l = st.last;
    if (l && l.engine) {
        const t = l.latency_ms != null ? ` ${l.latency_ms} ms` : "";
        const ft = l.ttft_ms != null ? ` (first token ${l.ttft_ms} ms)` : "";
        const who = cleanupEngineName(l.engine, st.provider, st.host);
        text += ` · last: ${who}${l.model ? ` ${l.model}` : ""}${t}${ft}${usd(l.cost_usd)}`;
    }
    const month = cleanupMonthCost(st);
    if (month.openrouter > 0)
        text += ` · ${st.provider === "custom" ? "OpenRouter " : ""}${money(month.openrouter)} this month`;
    if (month.custom > 0)
        text += ` · ${money(month.custom)} on your server this month (not in budget)`;
    e.textContent = text;
    e.className = cls;
    if (blocked?.turnOn)
        e.append(" ", turnOnButton());
}
/** "Turn on": the Cleanup order switch, so its consent ask runs as when clicked there. */
function turnOnButton() {
    const b = el("button", "lw-link", "Turn on in Cleanup order");
    b.type = "button";
    b.addEventListener("click", () => {
        // orCleanupToggled reads the switch: on, it asks for consent when needed.
        $("or_cleanup_allow").checked = true;
        void writes.run(orCleanupToggled);
        $("cleanup_order_group").scrollIntoView({ block: "nearest" });
    });
    return b;
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
    return askDialog("or_confirm", "or_confirm_ok", "or_confirm_cancel", () => {
        $("or_confirm_title").textContent = title;
        $("or_confirm_body").textContent = body;
        $("or_confirm_ok").textContent = ok;
    });
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
function keyBadge(text, kind) {
    const b = $("or_key_badge");
    b.textContent = text;
    b.className = `fl-badge fl-badge-${kind}`;
}
/** "$8.42 left of a $10.00 limit on this key." + what was used and the label. */
function creditText(r) {
    const limit = r.limit == null
        ? "No credit limit on this key."
        : `${r.limit_remaining != null ? `${money(r.limit_remaining)} left of a ` : "A "}${money(r.limit)} limit on this key${r.limit_reset ? ` (resets ${r.limit_reset})` : ""}.`;
    const used = ` ${money(r.usage_monthly ?? 0)} used this month.`;
    const free = r.is_free_tier
        ? ` Free tier${r.free_requests_remaining != null ? `: ${r.free_requests_remaining} free requests left` : ""}.`
        : "";
    const label = r.label ? ` Key "${r.label}".` : "";
    return limit + used + free + label;
}
async function orCheckKey() {
    const info = $("or_key_info");
    const out = $("or_key_status");
    out.textContent = "Checking the key with openrouter.ai…";
    out.className = "hint";
    info.hidden = true;
    $("or_key_warn").hidden = true;
    keyBadge("Checking", "light");
    try {
        const r = await invoke("dev_or_key_info");
        if ("error" in r) {
            setStatus("or_key_status", failText(r.error), false);
            keyBadge("Not valid", "destructive");
            return;
        }
        info.textContent = creditText(r);
        info.hidden = false;
        $("or_key_warn").hidden = !r.warn_no_limit;
        setStatus("or_key_status", ` Checked ${when(r.checked_at_ms || Date.now())}.`, true);
        keyBadge("Valid", "success");
    }
    catch (e) {
        setStatus("or_key_status", String(e), false);
        keyBadge("Not checked", "light");
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
/** Every cleanup-card write (switch, options, provider, custom server, its
 *  key, consent from Load models / Test connection, the cleanup model
 *  picker) runs one at a time, so a reload never reads half-saved settings
 *  and a field's save fired by a click lands before that click's own save. */
const writes = new Serial();
async function orApplyCleanup(consent) {
    const t = parseInt($("or_cleanup_timeout_ms").value, 10);
    const form = {
        allow: $("or_cleanup_allow").checked,
        consent,
        // The order is the AI cleanup page's (dev_cleanup_set_engines): kept.
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
    const c = views.or?.cleanup;
    const custom = c?.provider === "custom";
    let consent = false;
    // A custom server without an address yet has no host to agree to: it is
    // switched on, and asked once its address is typed.
    if (custom ? !c?.custom.consent && c?.custom.host !== "" : !c?.consent) {
        consent = await askCleanupConsent(custom ? "custom" : "openrouter", c?.custom.base_url ?? "");
        if (!consent) {
            box.checked = false;
            return;
        }
    }
    if ((await orApplyCleanup(consent)) && !custom)
        void refreshCatalog(false);
}
// ---- D52: custom cleanup server
/** The consent dialog of `provider`; the custom one names the typed host. */
async function askCleanupConsent(provider, url) {
    if (provider === "openrouter") {
        return askDialog("or_cleanup_consent_dlg", "or_cleanup_consent_ok", "or_cleanup_consent_cancel");
    }
    const host = (await hostOf(url)) || url.trim();
    // Filled when shown: an ask waiting for an open one must not change its text.
    return askDialog("cu_consent_dlg", "cu_consent_ok", "cu_consent_cancel", () => {
        $("cu_consent_host_title").textContent = host;
        $("cu_consent_host").textContent = host;
        $("cu_consent_http").hidden = !/^\s*http:\/\//i.test(url);
    });
}
function showCustom(v) {
    const c = v.cleanup;
    const cu = c.custom;
    showProviderRows(c.provider);
    if (document.activeElement !== $("cu_base_url"))
        setValue("cu_base_url", cu.base_url);
    void showNormalisedUrl();
    if (document.activeElement !== $("cu_model"))
        setValue("cu_model", cu.model);
    ensureOption("cu_timeout_ms", String(cu.timeout_ms), `${cu.timeout_ms / 1000} seconds`);
    $("cu_http_warn").hidden = !cu.plain_http;
    const ks = $("cu_key_state");
    ks.textContent = cu.key.set ? `Key ${cu.key.masked}` : "No key saved.";
    ks.classList.toggle("lw-mono", cu.key.set);
    $("cu_key_saved_ctl").hidden = !cu.key.set;
    $("cu_key_entry").hidden = cu.key.set;
}
/** D78 provider texts on the Cloud page (design: provDesc / offers). */
const PROVIDER_TEXT = {
    openrouter: {
        name: "OpenRouter",
        desc: "openrouter.ai: hundreds of models from many companies. Pay per use with your own key.",
        offers: "Speech to text: yes · AI cleanup: yes",
    },
    custom: {
        name: "LiteLLM",
        desc: "Your own LiteLLM proxy (or any OpenAI-compatible server), on your network or wherever you run it.",
        offers: "Speech to text: not yet · AI cleanup: yes",
    },
};
/** The parts of the Cloud and AI cleanup pages for the saved `provider`
 *  (`data-cloud` on the Cloud page, `data-provider` in the cleanup card). */
function showProviderRows(provider) {
    $("or_cleanup_provider").value = provider;
    for (const b of document.querySelectorAll("#cloud_provider_seg [data-cloud-provider]")) {
        const on = b.dataset.cloudProvider === provider;
        b.setAttribute("aria-checked", String(on));
        b.classList.toggle("is-on", on);
    }
    const t = PROVIDER_TEXT[provider];
    $("cloud_provider_desc").textContent = t.desc;
    $("cloud_provider_offers").textContent = t.offers;
    for (const n of document.querySelectorAll("[data-cloud-name]"))
        n.textContent = t.name;
    for (const r of document.querySelectorAll("[data-cloud]")) {
        r.hidden = r.dataset.cloud !== provider;
    }
    for (const r of $("or_cleanup_box").querySelectorAll("[data-provider]")) {
        r.hidden = r.dataset.provider !== provider;
    }
    // "Test cleanup" sends a fixed sample to the selected provider (D52).
    $("or_cleanup_test_out").textContent = "";
}
/** Save the provider selector and the custom server's fields together.
 *  Custom is saved as soon as it is picked (D52): the selector always shows
 *  what dictation uses, and the server is used once address and model are set. */
async function cuApply(provider, consent) {
    const t = parseInt($("cu_timeout_ms").value, 10);
    const form = {
        provider,
        base_url: $("cu_base_url").value,
        model: $("cu_model").value,
        timeout_ms: Number.isFinite(t) ? t : 4000,
        consent,
    };
    const r = await orCall("dev_or_set_custom_cleanup", { form }, "or_cleanup_msg", "Saved — applies to the next dictation");
    if (r) {
        showOr(r);
        const cu = r.cleanup.custom;
        if (r.cleanup.provider === "custom" && (cu.base_url === "" || cu.model === "")) {
            setStatus("or_cleanup_msg", cu.base_url === "" ? "Saved — enter your proxy's address, then pick a model on the AI cleanup page" : "Saved — pick a model on the AI cleanup page to use your proxy", false);
        }
    }
    else if (views.or)
        showOr(views.or);
    return r !== null;
}
/** A provider switch, or a custom address on a new host, while cloud
 *  cleanup is on asks that destination's consent first (a custom server
 *  once it has an address). */
function cuChanged(provider) {
    return writes.run(async () => {
        const c = views.or?.cleanup;
        let consent = false;
        if (c?.allow) {
            const url = $("cu_base_url").value;
            const needs = provider === "custom"
                ? url.trim() !== "" && (await hostOf(url)) !== c.custom.consent_host
                : !c.consent;
            if (needs) {
                consent = await askCleanupConsent(provider, url);
                if (!consent) {
                    if (views.or)
                        showOr(views.or);
                    return;
                }
            }
        }
        await cuApply(provider, consent);
    });
}
/** A key change, then the card again (the key itself is never sent back). */
function cuKey(command, args, done) {
    return writes.run(async () => {
        const k = await orCall(command, args, "or_cleanup_msg", done);
        if (k)
            await loadOr();
    });
}
function cuSaveKey() {
    const input = $("cu_key");
    if (input.value.trim() === "") {
        setStatus("or_cleanup_msg", "Paste the key first", false);
        return Promise.resolve();
    }
    const key = input.value;
    input.value = "";
    return cuKey("dev_cloud_set_custom_key", { key }, "Key saved in Credential Manager");
}
function cuClearKey() {
    return cuKey("dev_cloud_clear_custom_key", {}, "Key removed");
}
/** `GET /v1/models` on the typed address (typed key, else the saved one).
 *  The backend sends the saved key only to a host the user agreed to (D52);
 *  for another host it answers `consent_needed` and the dialog is shown.
 *  All of it is queued with the card's writes: it reads the saved consent,
 *  may record it, and shares the consent dialog with a provider change. */
function cuModels(outId, fill) {
    return writes.run(() => cuModelsNow(outId, fill));
}
async function cuModelsNow(outId, fill) {
    const out = $(outId);
    const baseUrl = $("cu_base_url").value.trim();
    if (baseUrl === "") {
        out.textContent = "Enter your proxy's address on the Cloud page first.";
        out.className = "hint err";
        return;
    }
    out.textContent = `Asking ${(await hostOf(baseUrl)) || "your proxy"} for its models…`;
    out.className = "hint";
    try {
        const typed = $("cu_key").value.trim();
        const args = { baseUrl, key: typed === "" ? null : typed };
        let t0 = performance.now();
        let r = await invoke("dev_or_custom_models", { ...args, consent: false });
        if (r.error === "consent_needed") {
            if (!(await askCleanupConsent("custom", baseUrl))) {
                out.textContent =
                    "Not sent: your saved key only goes to a server you agreed to send text to. A key pasted in the API key field is sent to this server for this test only.";
                out.className = "hint err";
                return;
            }
            t0 = performance.now();
            r = await invoke("dev_or_custom_models", { ...args, consent: true });
            await loadOr();
        }
        const ms = Math.round(performance.now() - t0);
        if (r.models) {
            // The AI cleanup page's Model field offers the proxy's list (D78).
            $("cu_model_list").replaceChildren(...r.models.map((id) => Object.assign(document.createElement("option"), { value: id })));
            const n = r.models.length;
            out.textContent = `Connected · ${n} model${n === 1 ? "" : "s"} · ${ms} ms${fill && n ? ": pick one in the Model field" : ""}`;
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
}
async function orTestCleanup() {
    const out = $("or_cleanup_test_out");
    out.textContent = `Sending a fixed sample sentence (not your text) to ${cleanupTarget(views.or)}…`;
    out.className = "hint";
    try {
        const r = await invoke("dev_or_test_cleanup");
        if (r.ok) {
            out.textContent = `OK · "${r.text ?? ""}" · ${r.latency_ms ?? "?"} ms${usd(r.cost_usd)}${r.model ? ` · ${r.model}` : ""}`;
            out.className = "hint ok";
        }
        else {
            out.textContent = cleanupTestFailText(r.error, r.message, views.or?.cleanup.provider);
            out.className = "hint err";
        }
    }
    catch (e) {
        console.warn("cleanup test failed", e);
        out.textContent = cleanupTestFailText(null, null, views.or?.cleanup.provider);
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
    for (const [id, on] of CHIPS) {
        const b = $(id);
        b.setAttribute("aria-pressed", String(on()));
        b.classList.toggle("is-on", on());
    }
    const stt = pickerPurpose === "stt";
    $("or_chip_transcriptions").hidden = !stt;
    $("or_chip_chat_audio").hidden = !stt;
}
function currentModel() {
    return pickerPurpose === "stt" ? (views.or?.stt.model ?? "") : (views.or?.cleanup.model ?? "");
}
/** One model in the picker (ModelPicker.dc.html): name + badges, id, price, a check on the current one. */
function pickerRow(r) {
    const current = r.id === currentModel();
    const b = el("button", current ? "fl-list-row lw-model is-current" : "fl-list-row lw-model");
    b.type = "button";
    b.setAttribute("role", "option");
    b.setAttribute("aria-selected", String(current));
    const text = el("span", "lw-model-text");
    const head = el("span", "lw-model-head");
    head.append(el("span", "lw-model-name", r.name || r.id), ...badges(r, false));
    const id = el("code", "lw-mono lw-model-id", r.id);
    text.append(head, id);
    const price = el("span", "lw-model-price", r.price_label);
    if (pickerPurpose === "cleanup" && r.context_length > 0)
        price.title = `${Math.round(r.context_length / 1000)}k context`;
    const check = el("span", "lw-model-check");
    if (current)
        check.append(iconEl(ICON_CHECK, 18));
    b.append(text, price, check);
    b.addEventListener("click", () => void pickModel(r.id, r.badges.includes(PARAPHRASE_BADGE)));
    return b;
}
const ICON_CHECK = "M5 12.5l4.5 4.5L19 7";
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
        $("or_picker_list").hidden = rows.length === 0;
        $("or_picker_empty").hidden = rows.length > 0;
        // How many the zero-retention filter hides (the same search without it).
        const hiddenNote = $("or_picker_hidden");
        hiddenNote.hidden = true;
        if (pickerFilters.zdr_only) {
            const all = await invoke("dev_or_catalog", {
                purpose: pickerPurpose,
                query: $("or_picker_query").value.trim(),
                filters: { ...pickerFilters, zdr_only: false, stt_path: pickerPurpose === "stt" ? pickerFilters.stt_path : null },
            });
            if (seq !== pickerSeq)
                return;
            const n = all.length - rows.length;
            if (n > 0) {
                hiddenNote.textContent = `${n} more model${n === 1 ? " is" : "s are"} hidden because no zero-retention provider serves ${n === 1 ? "it" : "them"}.`;
                hiddenNote.hidden = false;
            }
        }
    }
    catch (e) {
        setStatus("or_picker_status", String(e), false);
    }
}
async function openPicker(purpose) {
    pickerPurpose = purpose;
    pickerFilters = { zdr_only: true, fast_only: false, free_only: false, stt_path: null };
    $("or_picker_title").textContent =
        purpose === "stt" ? "Choose a speech model" : "Choose a cleanup model";
    const cat = views.or?.catalog;
    const n = cat ? (purpose === "stt" ? cat.stt : cat.cleanup) : 0;
    $("or_picker_query").placeholder = n > 0 ? `Search ${n} models` : "Search models";
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
/** A model from the picker (speech or cleanup), queued with the cleanup card's writes. */
function pickModel(id, paraphrase) {
    return writes.run(() => pickModelNow(id, paraphrase));
}
async function pickModelNow(id, paraphrase) {
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
    // The dialogs are opened from the Speech to text and AI cleanup pages too: a
    // modal inside a hidden page root would never show, so they live on <body>.
    for (const id of ["or_stt_consent_dlg", "or_cleanup_consent_dlg", "cu_consent_dlg", "or_confirm", "or_picker"]) {
        document.body.append($(id));
    }
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
    $("or_cleanup_allow").addEventListener("change", () => void writes.run(orCleanupToggled));
    for (const id of [
        "or_cleanup_sort",
        "or_cleanup_timeout_ms",
        "or_cleanup_send_surrounding_text",
        "or_cleanup_send_screen_words",
        "or_cleanup_send_app_name",
    ]) {
        $(id).addEventListener("change", () => void writes.run(() => orApplyCleanup(false)));
    }
    $("or_cleanup_change").addEventListener("click", () => void openPicker("cleanup"));
    $("or_cleanup_test").addEventListener("click", () => void orTestCleanup());
    const provider = () => $("or_cleanup_provider").value;
    $("or_cleanup_provider").addEventListener("change", () => void cuChanged(provider()));
    // D78: the Cloud page's provider switch drives the hidden selector.
    for (const b of document.querySelectorAll("#cloud_provider_seg [data-cloud-provider]")) {
        b.addEventListener("click", () => {
            const sel = $("or_cleanup_provider");
            if (sel.value === b.dataset.cloudProvider)
                return;
            sel.value = b.dataset.cloudProvider ?? "openrouter";
            sel.dispatchEvent(new Event("change"));
        });
    }
    for (const id of ["cu_base_url", "cu_model", "cu_timeout_ms"]) {
        $(id).addEventListener("change", () => void cuChanged(provider()));
    }
    $("cu_base_url").addEventListener("input", () => {
        $("cu_http_warn").hidden = !/^\s*http:\/\//i.test($("cu_base_url").value);
        void showNormalisedUrl();
    });
    $("cu_key_save").addEventListener("click", () => void cuSaveKey());
    $("cu_key_clear").addEventListener("click", () => void cuClearKey());
    $("cu_models_load").addEventListener("click", () => void cuModels("cu_models_out", true));
    $("cu_test").addEventListener("click", () => void cuModels("cu_test_out", false));
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
    // The list applies a model on click; "Use this model" applies a pasted ID, or closes.
    $("or_picker_use").addEventListener("click", () => {
        const custom = $("or_picker_custom").value;
        if (custom.trim() === "")
            $("or_picker").close();
        else
            void pickModel(custom, false);
    });
    $("or_picker_custom").addEventListener("keydown", (e) => {
        if (e.key === "Enter")
            $("or_picker_use").click();
    });
    for (const id of ["or_picker_close", "or_picker_cancel"]) {
        $(id).addEventListener("click", () => $("or_picker").close());
    }
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
