// Model files on this PC: native pickers ("Browse" next to model paths),
// help links that open in the browser (open_external, allowlisted hosts
// only), and the speech-model downloader (D31): the catalogue, the confirm
// dialog (OnboardDownloadConfirm.dc.html), progress from `model:download`
// and the minimal "Download a model" row in Settings → Speech to text; the
// AI cleanup models (GGUF, engine "llm") get the same row in Settings → AI
// cleanup, plus Use and Delete. Onboarding uses the same state and dialog. A download starts only from
// the dialog's Download button, never by itself (D9). "Import from file…" next to each row installs
// the same catalogue model from a folder, .zip or file on this PC (model_import_start, no network),
// checked against the same fingerprints; its progress comes as the same `model:download` events.
import { $, askDialog, html } from "../lib/dom.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { prefsStore } from "../lib/prefs-store.js";
let catalog = [];
/** undefined = not asked yet. */
let serverLib;
const progress = new Map();
const listeners = [];
const notify = () => listeners.forEach((l) => l());
/** "148 MB" (decimal, like the sizes download pages show). */
export function mb(bytes) {
    return `${Math.max(1, Math.round(bytes / 1e6))} MB`;
}
/** "312 of 661 MB · about 20 seconds left · from huggingface.co". */
export function progressLine(p, entry) {
    const imp = p.via === "import" || p.state === "importing";
    if (p.state === "verifying")
        return "Checking the fingerprint…";
    if (p.state === "error")
        return `${imp ? "Import" : "Download"} failed: ${p.error ?? "unknown error"}`;
    if (p.state === "cancelled")
        return `${imp ? "Import" : "Download"} cancelled. Nothing was kept.`;
    if (p.state === "done")
        return imp ? "Imported and checked." : "Downloaded and checked.";
    const got = Math.round(p.received / 1e6);
    let left = "";
    const secs = p.started ? (Date.now() - p.started) / 1000 : 0;
    if (secs > 2 && p.received > 0) {
        const rest = ((p.total - p.received) / p.received) * secs;
        left = rest < 60 ? ` · about ${Math.max(1, Math.round(rest / 5) * 5)} seconds left` : ` · about ${Math.round(rest / 60)} min left`;
    }
    if (imp)
        return `Copying ${got} of ${mb(p.total)}${left} · from this PC`;
    const host = entry?.from_server || p.id.startsWith("server:") ? "your server" : entry ? new URL(entry.files[0]?.url ?? entry.page).host : "";
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
    /** Can this build import anything from a file (any build with a local engine)? */
    canImport() {
        return catalog.some((c) => c.importable);
    },
    progress(id) {
        return progress.get(id);
    },
    busy(id) {
        return [...progress.values()].some((p) => (!id || p.id === id) && (p.state === "downloading" || p.state === "importing" || p.state === "verifying"));
    },
    onChange(l) {
        listeners.push(l);
    },
    async refresh() {
        catalog = await invoke("model_catalog");
        notify();
        return catalog;
    },
    /** A server is set: downloads come from its library (D41). */
    fromServer() {
        return catalog.some((c) => c.from_server);
    },
    server() {
        return serverLib;
    },
    /** The server's library (`server_models`; asks the server). */
    async refreshServer() {
        serverLib = await invoke("server_models").catch((e) => ({ configured: true, error: String(e), models: [] }));
        notify();
        return serverLib;
    },
    /** Download (after the confirm dialog) or, when installed, re-check and select a server model. */
    async serverStart(id) {
        const m = serverLib?.models.find((x) => x.id === id);
        if (!m || !m.slot || models.busy())
            return;
        if (!m.installed && !(await confirmServer(m)))
            return;
        const pid = m.catalogue_id ?? serverProgressId(m.id);
        progress.set(pid, { id: pid, received: 0, total: m.size, state: m.installed ? "verifying" : "downloading", started: Date.now() });
        notify();
        try {
            if (m.catalogue_id)
                await invoke("model_download_start", { id: m.catalogue_id });
            else
                await invoke("server_model_download", { id: m.id });
        }
        catch (err) {
            progress.set(pid, { id: pid, received: 0, total: m.size, state: "error", error: String(err), started: 0 });
            notify();
        }
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
    /** "Import from file…": the import dialog, then the picker, then model_import_start. */
    async importFlow(id) {
        const e = models.entry(id);
        if (!e || !e.importable || models.busy())
            return;
        const path = await askImport(e);
        if (!path)
            return;
        progress.set(id, { id, received: 0, total: e.size, state: "importing", started: Date.now(), via: "import" });
        notify();
        try {
            await invoke("model_import_start", { id, path });
        }
        catch (err) {
            progress.set(id, { id, received: 0, total: e.size, state: "error", error: String(err), started: 0, via: "import" });
            notify();
        }
    },
};
/** `model:download` id of a custom server model (server_library::progress_id). */
export const serverProgressId = (id) => `server:${id}`;
/** File names of the model's single file (whisper ggml, GGUF): "*.bin" / "*.gguf". */
function singleFilePattern(e) {
    if (e.engine !== "whisper" && e.engine !== "llm")
        return null;
    const ext = e.files[0]?.name.split(".").pop();
    return ext ? `*.${ext}` : null;
}
/** The import dialog: what is needed, then a folder or a .zip / file picker. The picked path, or null. */
function askImport(e) {
    $("imp_title").textContent = `Import ${e.title}`;
    const list = $("imp_files");
    list.replaceChildren(...e.files.map((f) => {
        const li = document.createElement("li");
        li.className = "lw-mono";
        li.textContent = `${f.name} · ${mb(f.size)}${f.extra ? " · optional, for auto-detect" : ""}`;
        return li;
    }));
    const single = singleFilePattern(e);
    $("imp_file").textContent = single ? "Choose .zip or model file…" : "Choose .zip…";
    const d = $("imp_dialog");
    return new Promise((resolve) => {
        const done = (pick) => {
            for (const b of ["imp_folder", "imp_file", "imp_cancel"])
                $(b).onclick = null;
            d.oncancel = null;
            d.close();
            if (!pick)
                return resolve(null);
            const filters = [
                { name: single ? "Model file or zip" : "Zip", pattern: single ? `${single};*.zip` : "*.zip" },
                { name: "All files", pattern: "*.*" },
            ];
            const title = pick === "folder" ? `Choose the folder with ${e.title}` : `Choose ${e.title} (.zip${single ? " or model file" : ""})`;
            pickPath(pick, title, pick === "file" ? filters : undefined)
                .then(resolve)
                .catch((err) => {
                console.error("pick_path", err);
                resolve(null);
            });
        };
        $("imp_folder").onclick = () => done("folder");
        $("imp_file").onclick = () => done("file");
        $("imp_cancel").onclick = () => done(null);
        d.oncancel = (ev) => {
            ev.preventDefault();
            done(null);
        };
        d.showModal();
        $("imp_cancel").focus();
    });
}
/** Speech models (Settings → Speech to text, onboarding). */
const isSpeech = (c) => c.engine !== "llm";
function confirm(e) {
    const many = e.files.length > 1;
    return askDownload(e.title, many ? e.page : e.files[0].url, e.size, e.files.length, e.dest, CHECK_PINNED);
}
/** The same dialog for a model from the server's library. */
function confirmServer(m) {
    return askDownload(m.title, "Your server's model library", m.size, m.files, m.dest, m.catalogue_id ? CHECK_PINNED : CHECK_SERVER);
}
function askDownload(title, from, size, files, dest, check) {
    $("dl_title").textContent = `Download ${title}?`;
    $("dl_from").textContent = from;
    $("dl_size").textContent = files > 1 ? `${mb(size)} in ${files} files` : mb(size);
    $("dl_dest").textContent = dest;
    $("dl_check").textContent = check;
    $("dl_ok").textContent = `Download ${mb(size)}`;
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
const CHECK_PINNED = "Against a SHA-256 fingerprint built into this version. If it doesn't match, the file is deleted and never loaded.";
const CHECK_SERVER = "Against the SHA-256 your server lists for it, over your key and the server's certificate. If it doesn't match, the file is deleted and never loaded.";
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
    <dt>Checked</dt><dd id="dl_check">${CHECK_PINNED}</dd>
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
const IMPORT_DIALOG = `
<dialog class="fl-dialog lw-dl-dialog" id="imp_dialog" aria-labelledby="imp_title">
  <div class="lw-dl-head">
    <h2 class="fl-serif lw-dl-title" id="imp_title">Import</h2>
    <p class="lw-dl-lead">From a folder, a .zip or the model file you copied to this PC. Nothing is downloaded.</p>
  </div>
  <dl class="lw-dl-facts">
    <dt>Needs</dt><dd><ul class="lw-imp-files" id="imp_files"></ul></dd>
    <dt>Checked</dt><dd>Against the SHA-256 fingerprints built into this version. If any file doesn't match, nothing is installed. Other or custom models can't be imported here.</dd>
  </dl>
  <div class="lw-dialog-actions">
    <button type="button" class="fl-btn fl-btn-ghost" id="imp_cancel">Cancel</button>
    <button type="button" class="fl-btn fl-btn-secondary" id="imp_folder">Choose folder…</button>
    <button type="button" class="fl-btn fl-btn-primary" id="imp_file">Choose .zip…</button>
  </div>
</dialog>`;
// ------------------------------------------------------------ Settings → Speech to text row
function renderSettingsRow() {
    const sel = document.getElementById("dl_model");
    if (!sel)
        return;
    const row = $("dl_row");
    row.hidden = !models.canDownload() && !models.canImport();
    const keep = sel.value;
    sel.replaceChildren(...catalog
        .filter((c) => (c.available || c.importable) && !c.language && isSpeech(c))
        .map((c) => new Option(`${c.title} · ${mb(c.size)}${c.installed ? " · downloaded" : ""}`, c.id)));
    if (keep && catalog.some((c) => c.id === keep))
        sel.value = keep;
    const id = sel.value;
    const p = models.progress(id);
    const busy = models.busy();
    const e = models.entry(id);
    $("dl_start").hidden = models.busy(id) || !e?.available;
    $("dl_start").disabled = busy;
    $("dl_import").hidden = models.busy(id) || !e?.importable;
    $("dl_import").disabled = busy;
    $("dl_stop").hidden = !models.busy(id);
    const line = $("dl_status");
    const bar = $("dl_bar");
    bar.hidden = !models.busy(id);
    if (p && models.busy(id))
        bar.firstElementChild.style.width = `${p.total ? Math.round((p.received / p.total) * 100) : 0}%`;
    if (!p) {
        line.textContent = e?.from_server && !e.installed ? FROM_SERVER_HINT : "";
        line.className = e?.from_server ? "hint" : "";
        return;
    }
    line.textContent = progressLine(p, models.entry(id));
    line.className = `hint ${p.state === "error" ? "err" : p.state === "done" ? "ok" : ""}`;
}
/** Under a catalogue row when a server is set (D41). */
const FROM_SERVER_HINT = "A server is set, so models come only from its library: see From your server below, or Import from file.";
// ------------------------------------------------------------ Settings → AI cleanup row
/** The file the app loads when no cleanup model is set (core_adapter DEFAULT_LLM_FILE, D11). */
const DEFAULT_LLM_FILE = "qwen2.5-1.5b-instruct-q4_k_m.gguf";
/** The previous default, still loaded when it is the only one there (core_adapter LEGACY_LLM_FILE). */
const LEGACY_LLM_FILE = "qwen2.5-0.5b-instruct-q4_k_m.gguf";
const isFile = (c, f) => c.dest.endsWith(`\\${f}`) || c.dest.endsWith(`/${f}`);
/** "Deleted." after a delete, until the selection or the catalogue changes. */
let llmNote = "";
const llmModels = () => catalog.filter((c) => c.engine === "llm");
/** undefined = not asked yet, null = asking. */
let llmBackend;
/** Ask once which device cleanup would use (the backend loads the engine plugin for it). */
function askLlmBackend() {
    if (llmBackend !== undefined)
        return;
    llmBackend = null;
    invoke("llm_backend")
        .then((b) => {
        llmBackend = b;
    })
        .catch((err) => {
        llmBackend = { gpu: false, backend: "", device: "", plugin: null, error: String(err) };
    })
        .finally(renderLlmRow);
}
/** The recommended cleanup model for this PC: the GPU pick when a GPU runs cleanup, else the CPU pick. */
export function recommendedLlm(list, gpu) {
    return list.find((c) => (gpu ? c.recommended_gpu : c.recommended));
}
/** "This PC: …" under the cleanup model list. */
function hardwareLine(b, rec) {
    if (!b)
        return "";
    const pick = rec ? ` Recommended: ${rec.title}.` : "";
    if (b.gpu)
        return `This PC runs AI cleanup on its graphics chip (${b.device}, ${b.backend}).${pick}`;
    if (!b.plugin)
        return `No AI cleanup engine found to check this PC's graphics chip; assuming the CPU.${pick}`;
    return `No graphics chip found for AI cleanup: it runs on the CPU.${pick}`;
}
/** Local cleanup runs with this catalogue model (path set to it, or empty = the default file). */
export function llmInUse(c, p = prefsStore.get()) {
    if (!p?.llm_enabled)
        return false;
    const path = p.llm_model_path?.trim();
    if (path)
        return path === c.dest;
    // Empty = the default file, or the old default when only that one is there (core_adapter default_llm_path).
    const list = llmModels();
    const legacyOnly = !list.some((x) => x.installed && isFile(x, DEFAULT_LLM_FILE)) &&
        list.some((x) => x.installed && isFile(x, LEGACY_LLM_FILE));
    return isFile(c, legacyOnly ? LEGACY_LLM_FILE : DEFAULT_LLM_FILE);
}
function renderLlmRow() {
    const sel = document.getElementById("llm_dl_model");
    if (!sel)
        return;
    const list = llmModels();
    $("llm_dl_row").hidden = list.length === 0;
    if (list.length)
        askLlmBackend();
    const rec = recommendedLlm(list, Boolean(llmBackend?.gpu));
    const keep = sel.value;
    // The recommendation moves once the backend answers (GPU → Llama 3B).
    const keepIt = keep && list.some((c) => c.id === keep) && sel.dataset.rec === rec?.id;
    sel.dataset.rec = rec?.id ?? "";
    sel.replaceChildren(...list.map((c) => new Option(`${c.title} · ${mb(c.size)}${c.installed ? (llmInUse(c) ? " · in use" : " · downloaded") : c === rec ? " · recommended" : ""}`, c.id)));
    if (keepIt)
        sel.value = keep;
    else {
        // Open on the model in use, else the one recommended for this PC.
        const pick = list.find((c) => c.installed && llmInUse(c)) ?? rec;
        if (pick)
            sel.value = pick.id;
    }
    $("llm_dl_hw").textContent = hardwareLine(llmBackend, rec);
    const e = models.entry(sel.value);
    const start = $("llm_dl_start");
    const stop = $("llm_dl_stop");
    const use = $("llm_dl_use");
    const del = $("llm_dl_delete");
    const imp = $("llm_dl_import");
    const bar = $("llm_dl_bar");
    const line = $("llm_dl_status");
    const busy = Boolean(e && models.busy(e.id));
    const inUse = Boolean(e && e.installed && llmInUse(e));
    start.hidden = !e || e.installed || busy || !e.available;
    start.disabled = models.busy();
    if (e)
        start.textContent = `Download ${mb(e.size)}…`;
    imp.hidden = !e || e.installed || busy || !e.importable;
    imp.disabled = models.busy();
    stop.hidden = !busy;
    bar.hidden = !busy;
    use.hidden = !e || !e.installed || busy || inUse;
    del.hidden = !e || !e.installed || busy;
    const set = (text, cls = "") => {
        line.textContent = text;
        line.className = `hint${cls ? ` ${cls}` : ""}`;
    };
    if (!e)
        return set("");
    const p = models.progress(e.id);
    if (p && busy) {
        bar.firstElementChild.style.width = `${p.total ? Math.round((p.received / p.total) * 100) : 0}%`;
        return set(progressLine(p, e));
    }
    if (p?.state === "error")
        return set(progressLine(p, e), "err");
    if (llmNote)
        return set(llmNote);
    if (p?.state === "cancelled" && !e.installed)
        return set(progressLine(p, e));
    if (!e.installed) {
        line.replaceChildren();
        line.className = "hint";
        if (e.from_server) {
            line.append(`${e.desc}. Not downloaded yet. A server is set, so models come only from its library: Settings → Speech to text → From your server${e.importable ? ", or Import from file" : ""}.`);
            return;
        }
        line.append(`${e.desc}. Not downloaded yet${e.available ? "." : ": "}`);
        if (!e.available)
            line.append(html(extLink(e.page, "get it from its model page")), e.importable ? ", then Import from file." : ".");
        if (e.license)
            line.append(` ${e.license}.`);
        return;
    }
    if (inUse)
        return set(`${e.desc}. Downloaded and in use for cleanup on this PC.`, "ok");
    set(`${e.desc}. Downloaded. Press Use to clean up with it.`);
}
async function useLlm(id) {
    const e = models.entry(id);
    if (!e?.installed)
        return;
    llmNote = "";
    await prefsStore.save({ llm_enabled: true, llm_model_path: e.dest }).catch((err) => {
        llmNote = `Could not switch: ${String(err)}`;
    });
    renderLlmRow();
}
async function deleteLlm(id) {
    const e = models.entry(id);
    if (!e?.installed || models.busy(e.id))
        return;
    $("llm_del_title").textContent = `Delete ${e.title}?`;
    $("llm_del_body").textContent = llmInUse(e)
        ? `Frees ${mb(e.size)}. It is the model AI cleanup on this PC uses, so that turns off; the formatting rules keep working. You can download it again any time.`
        : `Frees ${mb(e.size)}. You can download it again any time.`;
    if (!(await askDialog("llm_del_confirm", "llm_del_ok", "llm_del_cancel")))
        return;
    try {
        await invoke("model_delete", { id });
        llmNote = "Deleted.";
    }
    catch (err) {
        llmNote = `Could not delete: ${String(err)}`;
    }
    progress.delete(id);
    await prefsStore.load().catch(() => { });
    await models.refresh().catch(() => { });
}
/** Settings → AI cleanup → On this PC: download / use / delete a cleanup model. */
export const LLM_DOWNLOAD_ROW = `
<div class="fl-row lw-dl-row" id="llm_dl_row" data-needs="local" hidden>
  <div class="fl-row-text">
    <div class="fl-row-title"><label for="llm_dl_model">Download a model</label></div>
    <div class="fl-row-desc">From Hugging Face, checked against a fingerprint built into this version. Only when you press Download; nothing about you is sent. A downloaded or imported model turns on cleanup on this PC.<span class="lw-status-sub"><span class="lw-dl-bar" id="llm_dl_bar" hidden><span></span></span><span id="llm_dl_status"></span></span><span class="lw-status-sub hint" id="llm_dl_hw"></span></div>
  </div>
  <div class="fl-row-control">
    <select id="llm_dl_model" class="fl-select lw-w220" aria-label="Cleanup model to download"></select>
    <button type="button" class="fl-btn fl-btn-secondary" id="llm_dl_start">Download…</button>
    <button type="button" class="fl-btn fl-btn-secondary" id="llm_dl_import" hidden>Import from file…</button>
    <button type="button" class="fl-btn fl-btn-secondary" id="llm_dl_use" hidden>Use</button>
    <button type="button" class="fl-btn fl-btn-secondary" id="llm_dl_delete" hidden>Delete…</button>
    <button type="button" class="fl-btn fl-btn-secondary" id="llm_dl_stop" hidden>Cancel</button>
  </div>
</div>`;
const LLM_DELETE_DIALOG = `
<dialog id="llm_del_confirm" class="fl-dialog" aria-labelledby="llm_del_title">
  <h3 id="llm_del_title">Delete?</h3>
  <p id="llm_del_body"></p>
  <div class="lw-dialog-actions">
    <button type="button" class="fl-btn fl-btn-secondary" id="llm_del_cancel">Cancel</button>
    <button type="button" class="fl-btn fl-btn-destructive" id="llm_del_ok">Delete</button>
  </div>
</dialog>`;
/** prefs.hebrew_model → catalogue id. */
export const HEBREW_IDS = {
    zipformer: "zipformer-he-int8",
    parakeet_he: "parakeet-tdt-0.6b-v3-he-int8",
    ivrit_turbo: "ivrit-whisper-large-v3-turbo-q5_0",
};
let heStatus = null;
/** Where whisper would run on this PC (`whisper_backend`); undefined = not asked, null = asking. */
let whisperBackend;
/** Ask once (the backend loads the GPU whisper plugin, flow_whisper_vulkan.dll, to list the GPUs). */
function askWhisperBackend() {
    if (whisperBackend !== undefined)
        return;
    whisperBackend = null;
    invoke("whisper_backend")
        .then((b) => {
        whisperBackend = b;
    })
        .catch((err) => {
        whisperBackend = { gpu: false, backend: "CPU", device: "CPU", plugin: null, error: String(err) };
    })
        .finally(renderHebrewRow);
}
const onGpu = (b) => b.backend.toLowerCase() !== "cpu";
/** "Runs on …" for a loaded Hebrew model; `whisper` = the model is a whisper one (GPU possible). */
export function runsOnLine(b, whisper) {
    if (!b)
        return "";
    if (onGpu(b))
        return ` Runs on the graphics chip (${b.device}, ${b.backend}).`;
    if (!whisper)
        return " Runs on the CPU.";
    // Older backends without cpu_why: the full error, else no reason.
    const why = b.cpu_why ?? (b.gpu_error ? `the graphics chip failed (${b.gpu_error})` : "");
    return ` Runs on the CPU${why ? `: ${why}` : ""}, so expect slow dictations; Zipformer is faster.`;
}
/** Where a Hebrew model would run before it is loaded, from the whisper probe. */
function wouldRunLine(e, rec) {
    if (e.engine !== "whisper")
        return "";
    const b = whisperBackend;
    if (!b)
        return "";
    if (b.gpu)
        return ` This PC runs it on its graphics chip (${b.device}, ${b.backend}).`;
    const why = b.cpu_why ?? "no graphics chip found";
    return ` It would run on the CPU, slowly: ${why}.${rec && rec.id !== e.id ? ` Recommended here: ${rec.title}.` : ""}`;
}
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
    if (choice !== "none" && prefsStore.get()?.hebrew_model_dir?.trim())
        return true;
    return Boolean(hebrewEntry(choice)?.installed);
}
function renderHebrewRow() {
    const sel = document.getElementById("hebrew_model");
    if (!sel)
        return;
    if (models.entry(HEBREW_IDS.ivrit_turbo))
        askWhisperBackend();
    // "(recommended)" follows where whisper runs: ivrit turbo on a GPU, Zipformer on the CPU.
    const hebrew = catalog.filter((c) => c.language === "he");
    const rec = hebrew.find((c) => (whisperBackend?.gpu ? c.recommended_gpu : c.recommended));
    for (const o of Array.from(sel.options)) {
        const c = models.entry(HEBREW_IDS[o.value] ?? "");
        const base = o.dataset.label ?? o.textContent ?? "";
        o.dataset.label = base;
        o.textContent = c && c === rec ? `${base} (recommended)` : base;
    }
    const e = hebrewEntry(sel.value);
    const wantHe = hebrewChosen(prefsStore.get()?.languages);
    const line = $("he_status");
    const credit = $("he_credit");
    const start = $("he_download");
    const stop = $("he_stop");
    const imp = $("he_import");
    const bar = $("he_bar");
    const busy = Boolean(e && models.busy(e.id));
    const complete = e ? (e.complete ?? e.installed) : false;
    start.hidden = !e || complete || busy || !e.available;
    start.disabled = models.busy();
    imp.hidden = !e || complete || busy || !e.importable;
    imp.disabled = models.busy();
    stop.hidden = !busy;
    bar.hidden = !busy;
    credit.replaceChildren();
    if (e?.license) {
        credit.append(`${e.license} · `, html(extLink(e.page, "model page")));
    }
    credit.hidden = !e;
    if (e)
        start.textContent = `Download ${mb(e.missing || e.size)}…`;
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
    const custom = prefsStore.get()?.hebrew_model_dir?.trim();
    if (custom) {
        set(`Uses ${custom} (from your server).`, "ok");
        return;
    }
    if (!e.installed) {
        const how = e.from_server
            ? "a server is set, so get it from its library (From your server below)"
            : e.available
                ? "press Download"
                : e.importable
                    ? "get it from its model page, then Import from file"
                    : "get it from its model page";
        set(`Not downloaded yet: ${how} (${mb(e.size)}).${wantHe ? " Until then Hebrew uses the model above." : ""}${wouldRunLine(e, rec)}`, wantHe ? "err" : "");
        if (!e.available)
            credit.append(" · ", html(extLink(e.page, "Download page")));
        return;
    }
    // Installed before the language-ID model came with it (H6).
    const noLid = complete
        ? ""
        : ` Auto-detect needs a ${mb(e.missing ?? 0)} language-ID model: ${e.available ? "press Download" : "get it from the Whisper model page"}${e.importable ? ` or import ${e.files.find((f) => f.extra)?.name ?? "it"}` : ""}.`;
    switch (heStatus?.state) {
        case "ready":
            set(`Ready for Hebrew (loaded in ${heStatus.load_ms} ms).${runsOnLine(heStatus.backend, e.engine === "whisper")}${noLid}`, "ok");
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
    <div class="fl-row-desc">Used when the language is Hebrew (Settings → General), or on Detect automatically when you speak Hebrew; every other language keeps the model above. Zipformer is small and fast on any PC; ivrit.ai Whisper turbo is the most accurate but needs a graphics chip (GPU). The download includes a 32 MB Whisper model that tells Hebrew from English.<span class="lw-status-sub"><span class="lw-dl-bar" id="he_bar" hidden><span></span></span><span id="he_status"></span></span><span class="lw-status-sub lw-he-credit" id="he_credit" hidden></span></div>
  </div>
  <div class="fl-row-control">
    <select id="hebrew_model" class="fl-select lw-w220" data-save="prefs" aria-label="Hebrew model">
      <option value="zipformer">Zipformer</option>
      <option value="ivrit_turbo">ivrit.ai Whisper turbo</option>
      <option value="parakeet_he">Parakeet</option>
      <option value="none">None</option>
    </select>
    <button type="button" class="fl-btn fl-btn-secondary" id="he_download" hidden>Download…</button>
    <button type="button" class="fl-btn fl-btn-secondary" id="he_import" hidden>Import from file…</button>
    <button type="button" class="fl-btn fl-btn-secondary" id="he_stop" hidden>Cancel</button>
  </div>
</div>`;
// ------------------------------------------------------------ Settings → Speech to text: From your server (D41)
const KIND_LABEL = {
    stt: "Speech to text",
    hebrew: "Hebrew",
    llm: "AI cleanup",
    lid: "Language detection",
    vad: "Voice activity",
};
const SLOT_LABEL = {
    stt_whisper: "the model on this PC (Whisper)",
    stt_parakeet: "the model on this PC (Parakeet)",
    hebrew_zipformer: "the Hebrew model (streaming)",
    hebrew_parakeet: "the Hebrew model (Parakeet)",
    hebrew_whisper: "the Hebrew model (Whisper)",
    llm: "AI cleanup on this PC",
    lid: "language detection (auto-detect → Hebrew)",
};
function serverRow(m) {
    const pid = m.catalogue_id ?? serverProgressId(m.id);
    const p = models.progress(pid);
    const busy = models.busy(pid);
    const row = document.createElement("div");
    row.className = "lw-srv-model";
    row.dataset.srvId = m.id;
    const text = document.createElement("div");
    text.className = "lw-srv-text";
    const title = document.createElement("div");
    title.className = "fl-row-title";
    title.textContent = m.title;
    const meta = document.createElement("div");
    meta.className = "fl-row-desc";
    meta.textContent = `${KIND_LABEL[m.kind]}${m.language ? ` · ${m.language}` : ""} · ${mb(m.size)}${m.engine ? ` · ${m.engine}` : ""}${m.catalogue_id ? " · same as the built-in model" : ""}`;
    const status = document.createElement("div");
    status.className = "hint lw-srv-status";
    text.append(title, meta, status);
    const ctl = document.createElement("div");
    ctl.className = "fl-row-control";
    const btn = (label, act) => {
        const b = document.createElement("button");
        b.type = "button";
        b.className = "fl-btn fl-btn-secondary";
        b.textContent = label;
        b.dataset.srvAct = act;
        b.dataset.srvId = m.id;
        return b;
    };
    if (!m.slot) {
        status.textContent = `Can't be used here: ${m.reason ?? "unknown"}.`;
    }
    else if (busy) {
        const bar = html(`<span class="lw-dl-bar"><span style="width:${p && p.total ? Math.round((p.received / p.total) * 100) : 0}%"></span></span>`);
        status.append(bar, p ? progressLine(p, models.entry(m.catalogue_id ?? "")) : "");
        ctl.append(btn("Cancel", "cancel"));
    }
    else {
        if (p?.state === "error") {
            status.textContent = progressLine(p);
            status.classList.add("err");
        }
        else if (p?.state === "cancelled" && !m.installed) {
            status.textContent = progressLine(p);
        }
        else if (m.in_use) {
            status.textContent = `Downloaded and in use for ${SLOT_LABEL[m.slot]}.`;
            status.classList.add("ok");
        }
        else if (m.installed) {
            status.textContent = `Downloaded. Press Use for ${SLOT_LABEL[m.slot]}.`;
        }
        else {
            status.textContent = `For ${SLOT_LABEL[m.slot]}.`;
        }
        if (!m.installed) {
            const b = btn(`Download ${mb(m.size)}…`, "start");
            b.disabled = models.busy();
            ctl.append(b);
        }
        else if (!m.in_use) {
            const b = btn("Use", "start");
            b.disabled = models.busy();
            ctl.append(b);
        }
    }
    row.append(text, ctl);
    return row;
}
function renderServerModels() {
    const box = document.getElementById("srv_models");
    if (!box)
        return;
    const group = $("srv_group");
    const lib = serverLib;
    group.hidden = !lib?.configured;
    if (!lib?.configured)
        return;
    const note = $("srv_note");
    if (lib.error) {
        note.textContent = `Could not read your server's library: ${lib.error}`;
        note.className = "hint err";
    }
    else if (!lib.models.length) {
        note.textContent = "Your server lists no models. Add them to its library folder (server setup guide, section 10).";
        note.className = "hint";
    }
    else {
        note.textContent = "";
        note.className = "hint";
    }
    box.replaceChildren(...(lib.models ?? []).map(serverRow));
}
/** Settings → Speech to text: the server's model library (shown when a server is set). */
export const SERVER_MODELS_GROUP = `
<section class="lw-section" id="srv_group" data-needs="local" hidden>
  <h3 class="fl-group-title lw-title-badge">From your server <button type="button" class="fl-btn fl-btn-ghost lw-btn-sm" id="srv_refresh">Refresh</button></h3>
  <div class="fl-group">
    <div class="fl-row">
      <div class="fl-row-text"><div class="fl-row-desc">A server is set, so models download from its library instead of Hugging Face, including models you added to it. Only when you press Download; built-in models are checked against the fingerprints in this version, your own against the fingerprint your server lists.<span class="lw-status-sub" id="srv_note"></span></div></div>
    </div>
    <div id="srv_models" class="lw-srv-list"></div>
  </div>
</section>`;
export const modelsFeature = {
    init() {
        document.body.append(html(DIALOG));
        document.body.append(html(LLM_DELETE_DIALOG));
        document.body.append(html(IMPORT_DIALOG));
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
        document.getElementById("dl_import")?.addEventListener("click", () => void models.importFlow($("dl_model").value));
        models.onChange(renderSettingsRow);
        models.onChange(renderLlmRow);
        prefsStore.subscribe(() => renderLlmRow());
        const llmSel = () => $("llm_dl_model").value;
        document.getElementById("llm_dl_model")?.addEventListener("change", () => {
            llmNote = "";
            renderLlmRow();
        });
        document.getElementById("llm_dl_start")?.addEventListener("click", () => {
            llmNote = "";
            void models.confirmAndStart(llmSel());
        });
        document.getElementById("llm_dl_stop")?.addEventListener("click", () => void models.cancel(llmSel()));
        document.getElementById("llm_dl_import")?.addEventListener("click", () => {
            llmNote = "";
            void models.importFlow(llmSel());
        });
        document.getElementById("llm_dl_use")?.addEventListener("click", () => void useLlm(llmSel()));
        document.getElementById("llm_dl_delete")?.addEventListener("click", () => void deleteLlm(llmSel()));
        models.onChange(renderHebrewRow);
        models.onChange(renderServerModels);
        document.getElementById("srv_refresh")?.addEventListener("click", () => void models.refreshServer());
        document.addEventListener("click", (ev) => {
            const b = ev.target.closest("[data-srv-act]");
            if (!b)
                return;
            const id = b.dataset.srvId ?? "";
            if (b.dataset.srvAct === "start")
                void models.serverStart(id);
            else {
                const m = serverLib?.models.find((x) => x.id === id);
                if (m)
                    void models.cancel(m.catalogue_id ?? serverProgressId(m.id));
            }
        });
        prefsStore.subscribe(() => renderHebrewRow());
        document.getElementById("hebrew_model")?.addEventListener("change", renderHebrewRow);
        document.getElementById("he_download")?.addEventListener("click", () => {
            const e = hebrewEntry($("hebrew_model").value);
            if (e)
                void models.confirmAndStart(e.id);
        });
        document.getElementById("he_import")?.addEventListener("click", () => {
            const e = hebrewEntry($("hebrew_model").value);
            if (e)
                void models.importFlow(e.id);
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
            const prev = progress.get(p.id);
            const started = prev?.started || Date.now();
            // An import keeps its wording through verifying / done / error.
            const via = p.state === "importing" || (prev?.via === "import" && p.state !== "downloading") ? "import" : undefined;
            progress.set(p.id, { ...p, started, via });
            if (p.state === "done" || p.state === "error" || p.state === "cancelled") {
                // The backend pointed the engine at the new model: take its prefs.
                if (p.state === "done")
                    void prefsStore.load().catch(() => { });
                void models.refresh().catch(() => { });
                if (serverLib?.configured)
                    void models.refreshServer();
            }
            notify();
        });
    },
    async load() {
        const info = await invoke("get_model_status").catch(() => ({ hebrew: null }));
        heStatus = info.hebrew ?? null;
        await models.refresh();
        if (models.fromServer())
            void models.refreshServer();
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
    <div class="fl-row-desc">From Hugging Face, or Import from file (a folder, .zip or model file you copied here, no network); either way checked against a fingerprint built into this version. Only when you press Download; nothing about you is sent.<span class="lw-status-sub"><span class="lw-dl-bar" id="dl_bar" hidden><span></span></span><span id="dl_status"></span></span></div>
  </div>
  <div class="fl-row-control">
    <select id="dl_model" class="fl-select lw-w220" aria-label="Model to download"></select>
    <button type="button" class="fl-btn fl-btn-secondary" id="dl_start">Download…</button>
    <button type="button" class="fl-btn fl-btn-secondary" id="dl_import" hidden>Import from file…</button>
    <button type="button" class="fl-btn fl-btn-secondary" id="dl_stop" hidden>Cancel</button>
  </div>
</div>`;
