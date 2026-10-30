// Model files on this PC: native pickers ("Browse" next to model paths),
// help links that open in the browser (open_external, allowlisted hosts
// only), and the speech-model downloader (D31): the catalogue, the confirm
// dialog (OnboardDownloadConfirm.dc.html), progress from `model:download`
// and the minimal "Download a model" row in Settings → Speech to text.
// Onboarding uses the same state and dialog. A download starts only from
// the dialog's Download button, never by itself (D9).
import { $, html } from "../lib/dom.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { prefsStore } from "../lib/prefs-store.js";
let catalog = [];
const progress = new Map();
const listeners = [];
const notify = () => listeners.forEach((l) => l());
/** "148 MB" (decimal, like the sizes download pages show). */
export function mb(bytes) {
    return `${Math.max(1, Math.round(bytes / 1e6))} MB`;
}
/** "312 of 661 MB · about 20 seconds left · from huggingface.co". */
export function progressLine(p, entry) {
    if (p.state === "verifying")
        return "Checking the fingerprint…";
    if (p.state === "error")
        return `Download failed: ${p.error ?? "unknown error"}`;
    if (p.state === "cancelled")
        return "Download cancelled. Nothing was kept.";
    if (p.state === "done")
        return "Downloaded and checked.";
    const got = Math.round(p.received / 1e6);
    let left = "";
    const secs = p.started ? (Date.now() - p.started) / 1000 : 0;
    if (secs > 2 && p.received > 0) {
        const rest = ((p.total - p.received) / p.received) * secs;
        left = rest < 60 ? ` · about ${Math.max(1, Math.round(rest / 5) * 5)} seconds left` : ` · about ${Math.round(rest / 60)} min left`;
    }
    const host = entry ? new URL(entry.files[0]?.url ?? entry.page).host : "";
    return `${got} of ${mb(p.total)}${left}${host ? ` · from ${host}` : ""}`;
}
// ------------------------------------------------------------ pickers + links
/** Where "Set up a server" points (flow-server's instructions). */
export const SERVER_HELP = "https://github.com/bashycodes/murmur/blob/integration/docs/server-setup.md";
export async function pickPath(kind, title, filters) {
    return invoke("pick_path", { kind, title, filters: filters ?? null });
}
export function openExternal(url) {
    void invoke("open_external", { url }).catch((e) => console.error("open_external", e));
}
const PICKS = {
    model_path: { kind: "file", title: "Choose a Whisper model", filters: [{ name: "Whisper model (ggml)", pattern: "*.bin" }, { name: "All files", pattern: "*.*" }] },
    parakeet_model_dir: { kind: "folder", title: "Choose the Parakeet model folder" },
    llm_model_path: { kind: "file", title: "Choose an AI cleanup model", filters: [{ name: "GGUF model", pattern: "*.gguf" }, { name: "All files", pattern: "*.*" }] },
};
/** "Browse": put the picked path in the field and save it like a typed one. */
async function browse(field) {
    const spec = PICKS[field];
    if (!spec)
        return;
    try {
        const p = await pickPath(spec.kind, spec.title, spec.filters);
        if (!p)
            return;
        const i = $(field);
        i.value = p;
        i.dispatchEvent(new Event("change", { bubbles: true }));
    }
    catch (e) {
        console.error("pick_path", e);
    }
}
// ------------------------------------------------------------ catalogue + downloads
export const models = {
    catalog() {
        return catalog;
    },
    entry(id) {
        return catalog.find((c) => c.id === id);
    },
    /** Can this build download anything? */
    canDownload() {
        return catalog.some((c) => c.available);
    },
    progress(id) {
        return progress.get(id);
    },
    busy(id) {
        return [...progress.values()].some((p) => (!id || p.id === id) && (p.state === "downloading" || p.state === "verifying"));
    },
    onChange(l) {
        listeners.push(l);
    },
    async refresh() {
        catalog = await invoke("model_catalog");
        notify();
        return catalog;
    },
    /** The confirm dialog (3a); downloads only when the user presses Download there. */
    async confirmAndStart(id) {
        const e = models.entry(id);
        if (!e || !e.available || models.busy())
            return;
        if (!(await confirm(e)))
            return;
        progress.set(id, { id, received: 0, total: e.size, state: "downloading", started: Date.now() });
        notify();
        try {
            await invoke("model_download_start", { id });
        }
        catch (err) {
            progress.set(id, { id, received: 0, total: e.size, state: "error", error: String(err), started: 0 });
            notify();
        }
    },
    async cancel(id) {
        await invoke("model_download_cancel", { id }).catch(() => false);
    },
};
function confirm(e) {
    $("dl_title").textContent = `Download ${e.title}?`;
    const many = e.files.length > 1;
    $("dl_from").textContent = many ? e.page : e.files[0].url;
    $("dl_size").textContent = many ? `${mb(e.size)} in ${e.files.length} files` : mb(e.size);
    $("dl_dest").textContent = e.dest;
    $("dl_ok").textContent = `Download ${mb(e.size)}`;
    const d = $("dl_confirm");
    return new Promise((resolve) => {
        const done = (ok) => {
            $("dl_ok").onclick = null;
            $("dl_cancel").onclick = null;
            d.oncancel = null;
            d.close();
            resolve(ok);
        };
        $("dl_ok").onclick = () => done(true);
        $("dl_cancel").onclick = () => done(false);
        d.oncancel = (ev) => {
            ev.preventDefault();
            done(false);
        };
        d.showModal();
        $("dl_cancel").focus();
    });
}
const CHECK = `<svg class="lw-ico" viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12.5l4.5 4.5L19 7"></path></svg>`;
const DIALOG = `
<dialog class="fl-dialog lw-dl-dialog" id="dl_confirm" aria-labelledby="dl_title">
  <div class="lw-dl-head">
    <h2 class="fl-serif lw-dl-title" id="dl_title">Download?</h2>
    <p class="lw-dl-lead">This is the only thing that will be downloaded. Here is exactly what happens.</p>
  </div>
  <dl class="lw-dl-facts">
    <dt>From</dt><dd class="lw-mono" id="dl_from"></dd>
    <dt>Size</dt><dd id="dl_size"></dd>
    <dt>Saved to</dt><dd class="lw-mono" id="dl_dest"></dd>
    <dt>Checked</dt><dd>Against a SHA-256 fingerprint built into this version. If it doesn't match, the file is deleted and never loaded.</dd>
  </dl>
  <ul class="lw-dl-bullets">
    <li>${CHECK}Nothing about you, your audio or your settings is sent.</li>
    <li>${CHECK}Only happens when you press Download. No automatic updates.</li>
  </ul>
  <div class="lw-dialog-actions">
    <button type="button" class="fl-btn fl-btn-ghost" id="dl_cancel">Cancel</button>
    <button type="button" class="fl-btn fl-btn-primary" id="dl_ok">Download</button>
  </div>
</dialog>`;
// ------------------------------------------------------------ Settings → Speech to text row
function renderSettingsRow() {
    const sel = document.getElementById("dl_model");
    if (!sel)
        return;
    const row = $("dl_row");
    row.hidden = !models.canDownload();
    const keep = sel.value;
    sel.replaceChildren(...catalog
        .filter((c) => c.available && !c.language)
        .map((c) => new Option(`${c.title} · ${mb(c.size)}${c.installed ? " · downloaded" : ""}`, c.id)));
    if (keep && catalog.some((c) => c.id === keep))
        sel.value = keep;
    const id = sel.value;
    const p = models.progress(id);
    const busy = models.busy();
    $("dl_start").hidden = models.busy(id);
    $("dl_start").disabled = busy;
    $("dl_stop").hidden = !models.busy(id);
    const line = $("dl_status");
    const bar = $("dl_bar");
    bar.hidden = !models.busy(id);
    if (p && models.busy(id))
        bar.firstElementChild.style.width = `${p.total ? Math.round((p.received / p.total) * 100) : 0}%`;
    if (!p) {
        line.textContent = "";
        line.className = "";
        return;
    }
    line.textContent = progressLine(p, models.entry(id));
    line.className = `hint ${p.state === "error" ? "err" : p.state === "done" ? "ok" : ""}`;
}
/** prefs.hebrew_model → catalogue id. */
export const HEBREW_IDS = {
    zipformer: "zipformer-he-int8",
    parakeet_he: "parakeet-tdt-0.6b-v3-he-int8",
};
let heStatus = null;
/** True when `languages` forces Hebrew (exactly one, `he`/`he-IL`/`iw`). */
export function hebrewChosen(languages) {
    if (!languages || languages.length !== 1)
        return false;
    const p = languages[0].trim().toLowerCase().split(/[-_]/)[0];
    return p === "he" || p === "iw" || p === "hebrew";
}
/** The catalogue entry of the chosen Hebrew model (undefined = none). */
export function hebrewEntry(choice) {
    return models.entry(HEBREW_IDS[choice ?? ""] ?? "");
}
/** A Hebrew model is chosen and on disk (catalogue `installed`). */
export function hebrewReady(choice) {
    return Boolean(hebrewEntry(choice)?.installed);
}
function renderHebrewRow() {
    const sel = document.getElementById("hebrew_model");
    if (!sel)
        return;
    const e = hebrewEntry(sel.value);
    const wantHe = hebrewChosen(prefsStore.get()?.languages);
    const line = $("he_status");
    const credit = $("he_credit");
    const start = $("he_download");
    const stop = $("he_stop");
    const bar = $("he_bar");
    const busy = Boolean(e && models.busy(e.id));
    start.hidden = !e || e.installed || busy || !e.available;
    start.disabled = models.busy();
    stop.hidden = !busy;
    bar.hidden = !busy;
    credit.replaceChildren();
    if (e?.license) {
        credit.append(`${e.license} · `, html(extLink(e.page, "model page")));
    }
    credit.hidden = !e;
    if (e)
        start.textContent = `Download ${mb(e.size)}…`;
    const set = (text, cls = "") => {
        line.textContent = text;
        line.className = `hint${cls ? ` ${cls}` : ""}`;
    };
    if (!e) {
        set(wantHe ? "Your language is Hebrew, but with no Hebrew model it uses the model above." : "Hebrew uses the model above.", wantHe ? "err" : "");
        return;
    }
    const p = models.progress(e.id);
    if (p && busy) {
        bar.firstElementChild.style.width = `${p.total ? Math.round((p.received / p.total) * 100) : 0}%`;
        set(progressLine(p, e));
        return;
    }
    if (p?.state === "error") {
        set(progressLine(p, e), "err");
        return;
    }
    if (!e.installed) {
        const how = e.available ? "press Download" : "get it from its model page";
        set(`Not downloaded yet: ${how} (${mb(e.size)}).${wantHe ? " Until then Hebrew uses the model above." : ""}`, wantHe ? "err" : "");
        if (!e.available)
            credit.append(" · ", html(extLink(e.page, "Download page")));
        return;
    }
    switch (heStatus?.state) {
        case "ready":
            set(`Ready for Hebrew (loaded in ${heStatus.load_ms} ms).`, "ok");
            break;
        case "loading":
            set("Loading…");
            break;
        case "failed":
            set(`Failed to load: ${heStatus.message}`, "err");
            break;
        case "missing":
            set(`Not found: ${heStatus.path}`, "err");
            break;
        default:
            set("Downloaded. Loads when this PC's engine is in use.");
    }
}
/** Settings → Speech to text: the Hebrew model (D33 route) row. */
export const HEBREW_ROW = `
<div class="fl-row lw-dl-row" id="he_row">
  <div class="fl-row-text">
    <div class="fl-row-title"><label for="hebrew_model">Hebrew model</label></div>
    <div class="fl-row-desc">Used when the language is Hebrew (Settings → General); every other language keeps the model above. Zipformer is smaller and more accurate.<span class="lw-status-sub"><span class="lw-dl-bar" id="he_bar" hidden><span></span></span><span id="he_status"></span></span><span class="lw-status-sub lw-he-credit" id="he_credit" hidden></span></div>
  </div>
  <div class="fl-row-control">
    <select id="hebrew_model" class="fl-select lw-w220" data-save="prefs" aria-label="Hebrew model">
      <option value="zipformer">Zipformer (recommended)</option>
      <option value="parakeet_he">Parakeet</option>
      <option value="none">None</option>
    </select>
    <button type="button" class="fl-btn fl-btn-secondary" id="he_download" hidden>Download…</button>
    <button type="button" class="fl-btn fl-btn-secondary" id="he_stop" hidden>Cancel</button>
  </div>
</div>`;
export const modelsFeature = {
    init() {
        document.body.append(html(DIALOG));
        // Browse buttons next to path fields (data-browse = the field's id).
        document.addEventListener("click", (e) => {
            const t = e.target;
            const b = t.closest("[data-browse]");
            if (b)
                void browse(b.dataset.browse ?? "");
            const link = t.closest("[data-open-url]");
            if (link) {
                e.preventDefault();
                openExternal(link.dataset.openUrl ?? "");
            }
        });
        const sel = document.getElementById("dl_model");
        sel?.addEventListener("change", renderSettingsRow);
        document.getElementById("dl_start")?.addEventListener("click", () => {
            void models.confirmAndStart($("dl_model").value);
        });
        document.getElementById("dl_stop")?.addEventListener("click", () => void models.cancel($("dl_model").value));
        models.onChange(renderSettingsRow);
        models.onChange(renderHebrewRow);
        prefsStore.subscribe(() => renderHebrewRow());
        document.getElementById("hebrew_model")?.addEventListener("change", renderHebrewRow);
        document.getElementById("he_download")?.addEventListener("click", () => {
            const e = hebrewEntry($("hebrew_model").value);
            if (e)
                void models.confirmAndStart(e.id);
        });
        document.getElementById("he_stop")?.addEventListener("click", () => {
            const e = hebrewEntry($("hebrew_model").value);
            if (e)
                void models.cancel(e.id);
        });
        void onEvent("model:hebrew", (ev) => {
            heStatus = ev.payload;
            renderHebrewRow();
        });
        void onEvent("model:download", (ev) => {
            const p = ev.payload;
            const started = progress.get(p.id)?.started || Date.now();
            progress.set(p.id, { ...p, started });
            if (p.state === "done" || p.state === "error" || p.state === "cancelled") {
                // The backend pointed the engine at the new model: take its prefs.
                if (p.state === "done")
                    void prefsStore.load().catch(() => { });
                void models.refresh().catch(() => { });
            }
            notify();
        });
    },
    async load() {
        const info = await invoke("get_model_status").catch(() => ({ hebrew: null }));
        heStatus = info.hebrew ?? null;
        await models.refresh();
    },
};
/** A "Browse" button for a path field (`field` = the input's id). */
export function browseButton(field, label = "Browse") {
    return `<button type="button" class="fl-btn fl-btn-secondary" data-browse="${field}">${label}</button>`;
}
/** Inline link that opens `url` in the browser (allowlisted hosts only). */
export function extLink(url, text, cls = "lw-link") {
    return `<a class="${cls}" href="${url}" data-open-url="${url}">${text}</a>`;
}
/** The settings row markup (Settings → Speech to text). */
export const DOWNLOAD_ROW = `
<div class="fl-row lw-dl-row" id="dl_row" hidden>
  <div class="fl-row-text">
    <div class="fl-row-title"><label for="dl_model">Download a model</label></div>
    <div class="fl-row-desc">From Hugging Face, checked against a fingerprint built into this version. Only when you press Download; nothing about you is sent.<span class="lw-status-sub"><span class="lw-dl-bar" id="dl_bar" hidden><span></span></span><span id="dl_status"></span></span></div>
  </div>
  <div class="fl-row-control">
    <select id="dl_model" class="fl-select lw-w220" aria-label="Model to download"></select>
    <button type="button" class="fl-btn fl-btn-secondary" id="dl_start">Download…</button>
    <button type="button" class="fl-btn fl-btn-secondary" id="dl_stop" hidden>Cancel</button>
  </div>
</div>`;
